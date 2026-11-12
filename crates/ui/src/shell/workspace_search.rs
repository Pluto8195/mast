use super::*;
use crate::files::client::{FilesRequestContext, WorkspaceFilesClient};
use mast_proto::{
    SearchWorkspaceContentRequest, SearchWorkspaceFilesRequest, WorkspaceContentSearchFrame,
    WorkspaceEntryKind,
};

pub(super) struct IndexedPath {
    path: String,
    lower_path: String,
    lower_name: String,
}

fn file_score(path: &IndexedPath, query: &str) -> Option<i64> {
    let name = &path.lower_name;
    let full = &path.lower_path;
    if name == query {
        return Some(10_000);
    }
    if name.starts_with(query) {
        return Some(8_000 - name.len() as i64);
    }
    if let Some(index) = name.find(query) {
        return Some(6_000 - index as i64 - name.len() as i64);
    }
    if let Some(index) = full.find(query) {
        return Some(4_000 - index as i64 - full.len() as i64);
    }
    let mut query_chars = query.chars();
    let mut wanted = query_chars.next()?;
    let mut gaps = 0i64;
    for character in full.chars() {
        if character == wanted {
            match query_chars.next() {
                Some(next) => wanted = next,
                None => return Some(2_000 - gaps - full.len() as i64),
            }
        } else {
            gaps += 1;
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SearchMode {
    Files,
    Content,
    Line,
}
#[derive(Clone)]
struct ResultRow {
    path: String,
    line: Option<u32>,
    preview: String,
    ranges: Vec<(usize, usize)>,
    changed: bool,
}
pub(super) struct WorkspaceSearch {
    chat_id: String,
    context: Option<FilesRequestContext>,
    mode: SearchMode,
    input: Entity<ComposerInput>,
    include: Entity<ComposerInput>,
    exclude: Entity<ComposerInput>,
    previous_focus: Option<FocusHandle>,
    focus_pending: bool,
    active: usize,
    list: gpui::ListState,
    rows: Vec<ResultRow>,
    changed: Vec<String>,
    has_diff: bool,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    changed_only: bool,
    status: String,
    generation: u64,
    query_started: std::time::Instant,
    rendered_generation: Option<u64>,
    task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub(super) fn prewarm_workspace_search(&mut self, cx: &mut Context<Self>) {
        let context = FilesRequestContext::for_chat(self.state.read(cx), &self.active_chat);
        let engine = self.state.read(cx).engine().cloned();
        let (Some(context), Some(engine)) = (context, engine) else {
            self.workspace_search_index_key = None;
            self.workspace_search_index_engine = None;
            self.workspace_search_index_task = None;
            return;
        };
        let key = format!(
            "{}\0{}",
            context.target_device_id.as_deref().unwrap_or(""),
            context.cwd
        );
        if self.workspace_search_index_key.as_ref() == Some(&key)
            && self
                .workspace_search_index_engine
                .as_ref()
                .is_some_and(|old| old.same_connection(&engine))
        {
            return;
        }
        if !self
            .workspace_search_index_engine
            .as_ref()
            .is_some_and(|old| old.same_connection(&engine))
        {
            self.workspace_search_indexes.clear();
        }
        self.workspace_search_index_key = Some(key.clone());
        self.workspace_search_index_engine = Some(engine.clone());
        let client = WorkspaceFilesClient::new(engine, context);
        self.workspace_search_index_task = Some(cx.spawn(async move |this, cx| {
            let mut stream = match client.watch_search_files().await {
                Ok(stream) => stream,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        if this.workspace_search_index_key.as_ref() == Some(&key) {
                            this.workspace_search_indexes.remove(&key);
                            this.workspace_search_index_error =
                                Some((key.clone(), error.to_string()));
                            if this
                                .workspace_search
                                .as_ref()
                                .is_some_and(|search| search.mode == SearchMode::Files)
                            {
                                this.update_workspace_search(cx);
                            }
                        }
                    });
                    return;
                }
            };
            while let Some(mut frame) = stream.recv().await {
                if let Some(error) = frame.get("error").and_then(|value| value.as_str()) {
                    let _ = this.update(cx, |this, cx| {
                        if this.workspace_search_index_key.as_ref() == Some(&key) {
                            this.workspace_search_indexes.remove(&key);
                            this.workspace_search_index_error =
                                Some((key.clone(), error.to_owned()));
                            if this
                                .workspace_search
                                .as_ref()
                                .is_some_and(|search| search.mode == SearchMode::Files)
                            {
                                this.update_workspace_search(cx);
                            }
                        }
                    });
                    continue;
                }
                let Some(files) = frame
                    .as_object_mut()
                    .and_then(|object| object.remove("files"))
                else {
                    continue;
                };
                let entries = cx
                    .background_executor()
                    .spawn(async move {
                        serde_json::from_value::<Vec<String>>(files)
                            .ok()
                            .map(|paths| {
                                std::sync::Arc::new(
                                    paths
                                        .into_iter()
                                        .map(|path| IndexedPath {
                                            lower_name: path
                                                .rsplit('/')
                                                .next()
                                                .unwrap_or(&path)
                                                .to_lowercase(),
                                            lower_path: path.to_lowercase(),
                                            path,
                                        })
                                        .collect(),
                                )
                            })
                    })
                    .await;
                let Some(entries) = entries else {
                    continue;
                };
                if this
                    .update(cx, |this, cx| {
                        if this.workspace_search_index_key.as_ref() != Some(&key) {
                            return false;
                        }
                        this.workspace_search_index_error = None;
                        this.workspace_search_indexes.insert(key.clone(), entries);
                        if this.workspace_search.is_some() {
                            this.update_workspace_search(cx);
                        }
                        true
                    })
                    .unwrap_or(false)
                    == false
                {
                    break;
                }
            }
            let _ = this.update(cx, |this, cx| {
                if this.workspace_search_index_key.as_ref() == Some(&key) {
                    this.workspace_search_indexes.remove(&key);
                    this.workspace_search_index_error =
                        Some((key.clone(), "Workspace file index disconnected".into()));
                    if this
                        .workspace_search
                        .as_ref()
                        .is_some_and(|search| search.mode == SearchMode::Files)
                    {
                        this.update_workspace_search(cx);
                    }
                }
            });
        }));
    }
    pub(super) fn workspace_recent_key(&self, cx: &App) -> String {
        let state = self.state.read(cx);
        let checkout = state
            .chat_diff_targets(&self.active_chat)
            .iter()
            .find(|target| target.selected)
            .map(|target| target.path.as_str())
            .unwrap_or("");
        format!("{}\0{checkout}", self.active_chat)
    }
    pub(super) fn open_workspace_search(
        &mut self,
        mode: SearchMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_chat.is_empty() {
            cx.propagate();
            return;
        }
        if mode == SearchMode::Line {
            let focused = match self.resolved_right_active(cx) {
                RightSurface::File(id) => self
                    .file_surfaces
                    .get(&id)
                    .is_some_and(|file| file.read(cx).editor_is_focused(window, cx)),
                RightSurface::Diff(id) => self
                    .diffs
                    .get(&id)
                    .is_some_and(|diff| diff.read(cx).diff_is_focused(window, cx)),
                _ => false,
            };
            if !focused {
                cx.propagate();
                return;
            }
        }
        let input = cx.new(|cx| {
            ComposerInput::with_context(
                match mode {
                    SearchMode::Files => "Search files…",
                    SearchMode::Content => "Search in files…",
                    SearchMode::Line => "Line number…",
                },
                "WorkspaceSearch",
                cx,
            )
        });
        let include =
            cx.new(|cx| ComposerInput::with_context("Files to include", "WorkspaceSearch", cx));
        let exclude =
            cx.new(|cx| ComposerInput::with_context("Files to exclude", "WorkspaceSearch", cx));
        let mut subscriptions = Vec::new();
        for entity in [&input, &include, &exclude] {
            subscriptions.push(cx.subscribe(entity, |this, _, event, cx| {
                if matches!(event, ComposerInputEvent::Edited) {
                    this.update_workspace_search(cx);
                }
            }));
        }
        let diff = self.active_changes(cx).or_else(|| {
            self.right_tabs
                .get(&self.panel_key(cx))
                .and_then(|tabs| {
                    tabs.iter().find_map(|tab| match tab {
                        RightSurface::Diff(id) => self.diffs.get(id),
                        _ => None,
                    })
                })
                .cloned()
        });
        if let Some(diff) = &diff {
            subscriptions.push(cx.observe(diff, |this, diff, cx| {
                let Some(paths) = diff.read(cx).changed_paths() else {
                    return;
                };
                if let Some(search) = this.workspace_search.as_mut()
                    && (!search.has_diff || search.changed != paths)
                {
                    search.has_diff = true;
                    search.changed = paths;
                    this.update_workspace_search(cx);
                }
            }));
        }
        let changed = diff.and_then(|diff| diff.read(cx).changed_paths());
        let has_diff = changed.is_some();
        let changed = changed.unwrap_or_default();
        self.workspace_search = Some(WorkspaceSearch {
            chat_id: self.active_chat.clone(),
            context: FilesRequestContext::for_chat(self.state.read(cx), &self.active_chat),
            mode,
            input,
            include,
            exclude,
            previous_focus: window.focused(cx),
            focus_pending: true,
            active: 0,
            list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(300.0)),
            rows: Vec::new(),
            changed,
            has_diff,
            case_sensitive: false,
            whole_word: false,
            regex: false,
            changed_only: false,
            status: String::new(),
            generation: 0,
            query_started: std::time::Instant::now(),
            rendered_generation: None,
            task: None,
            _subscriptions: subscriptions,
        });
        // Re-subscribe on an explicit open so the host requests a rate-limited
        // refresh even when this checkout was already prewarmed.
        self.workspace_search_index_key = None;
        self.prewarm_workspace_search(cx);
        self.update_workspace_search(cx);
        cx.notify();
    }

    pub(super) fn close_workspace_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.workspace_search.take() {
            if let Some(focus) = search.previous_focus {
                window.focus(&focus, cx);
            }
            cx.notify();
        }
    }

    fn update_workspace_search(&mut self, cx: &mut Context<Self>) {
        let recent_key = self.workspace_recent_key(cx);
        let Some(search) = self.workspace_search.as_mut() else {
            return;
        };
        if search.chat_id != self.active_chat {
            search.task = None;
            search.rows.clear();
            search.status = "The active chat changed; reopen search.".into();
            cx.notify();
            return;
        }
        if search.context != FilesRequestContext::for_chat(self.state.read(cx), &self.active_chat) {
            search.task = None;
            search.rows.clear();
            search.status = "The workspace changed; reopen search.".into();
            cx.notify();
            return;
        }
        search.task = None;
        search.generation = search.generation.wrapping_add(1);
        search.query_started = std::time::Instant::now();
        let generation = search.generation;
        search.active = 0;
        search.list.reset(0);
        search.rows.clear();
        search.status.clear();
        let mode = search.mode;
        let raw = search.input.read(cx).text().trim().to_owned();
        if mode == SearchMode::Line {
            cx.notify();
            return;
        }
        let query = if mode == SearchMode::Files {
            parse_line_suffix(&raw).0
        } else {
            raw.as_str()
        };
        let recents = self
            .workspace_file_recents
            .get(&recent_key)
            .cloned()
            .unwrap_or_default();
        if query.is_empty() {
            if mode == SearchMode::Files {
                let mut paths = search.changed.clone();
                for path in recents {
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
                search.rows = paths
                    .into_iter()
                    .map(|path| ResultRow {
                        changed: search.changed.contains(&path),
                        path,
                        line: None,
                        preview: String::new(),
                        ranges: Vec::new(),
                    })
                    .collect();
            }
            search
                .list
                .reset_with_uniform_height(search.rows.len(), px(42.0));
            cx.notify();
            return;
        }
        if mode == SearchMode::Files {
            if query.chars().count() > 256 {
                search.status = "Query must not exceed 256 characters".into();
                cx.notify();
                return;
            }
            if search.context.is_none() {
                search.status = "No workspace available".into();
                cx.notify();
                return;
            }
            if self.state.read(cx).engine().is_none() {
                search.status = "Workspace service unavailable".into();
                cx.notify();
                return;
            }
            let Some(index) = self
                .workspace_search_index_key
                .as_ref()
                .and_then(|key| self.workspace_search_indexes.get(key))
            else {
                let error = self
                    .workspace_search_index_error
                    .as_ref()
                    .filter(|(key, _)| Some(key) == self.workspace_search_index_key.as_ref())
                    .map(|(_, error)| error.clone());
                if error.is_some() {
                    // An older remote engine may not support the index stream.
                    // Keep quick open usable through its existing search RPC.
                    let context = search.context.clone().unwrap();
                    let engine = self.state.read(cx).engine().cloned().unwrap();
                    let client = WorkspaceFilesClient::new(engine, context.clone());
                    let request = SearchWorkspaceFilesRequest {
                        target: context.target,
                        query: query.to_owned(),
                        include_ignored: false,
                        limit: Some(200),
                    };
                    let changed = search.changed.clone();
                    search.status = "Searching…".into();
                    search.task = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(150))
                            .await;
                        let result = client.search(request).await;
                        let _ = this.update(cx, |this, cx| {
                            let Some(search) = this
                                .workspace_search
                                .as_mut()
                                .filter(|search| search.generation == generation)
                            else {
                                return;
                            };
                            match result {
                                Ok(matches) => {
                                    search.rows = matches
                                        .into_iter()
                                        .filter(|entry| entry.kind == WorkspaceEntryKind::File)
                                        .map(|entry| ResultRow {
                                            changed: changed.contains(&entry.path),
                                            path: entry.path,
                                            line: None,
                                            preview: String::new(),
                                            ranges: Vec::new(),
                                        })
                                        .collect();
                                    search.rows.sort_by_key(|row| {
                                        (
                                            !row.changed,
                                            recents
                                                .iter()
                                                .position(|path| path == &row.path)
                                                .unwrap_or(usize::MAX),
                                        )
                                    });
                                    search
                                        .list
                                        .reset_with_uniform_height(search.rows.len(), px(42.0));
                                    search.status = if search.rows.is_empty() {
                                        "No files found".into()
                                    } else {
                                        String::new()
                                    };
                                }
                                Err(error) => search.status = error.to_string(),
                            }
                            cx.notify();
                        });
                    }));
                } else {
                    search.status = "Indexing files…".into();
                }
                cx.notify();
                return;
            };
            let started = std::time::Instant::now();
            let lower = query.to_lowercase();
            let mut matches: Vec<_> = index
                .iter()
                .filter_map(|path| file_score(path, &lower).map(|score| (path, score)))
                .collect();
            let compare = |(a, a_score): &(&IndexedPath, i64),
                           (b, b_score): &(&IndexedPath, i64)| {
                b_score
                    .cmp(a_score)
                    .then_with(|| a.lower_path.cmp(&b.lower_path))
                    .then_with(|| a.path.cmp(&b.path))
            };
            if matches.len() > 200 {
                matches.select_nth_unstable_by(200, compare);
            }
            matches.truncate(200);
            matches.sort_unstable_by(compare);
            search.rows = matches
                .into_iter()
                .map(|(path, _)| ResultRow {
                    changed: search.changed.contains(&path.path),
                    path: path.path.clone(),
                    line: None,
                    preview: String::new(),
                    ranges: Vec::new(),
                })
                .collect();
            search.rows.sort_by_key(|row| {
                (
                    !row.changed,
                    recents
                        .iter()
                        .position(|path| path == &row.path)
                        .unwrap_or(usize::MAX),
                )
            });
            search
                .list
                .reset_with_uniform_height(search.rows.len(), px(42.0));
            search.status = if search.rows.is_empty() {
                "No files found".into()
            } else {
                String::new()
            };
            tracing::debug!(
                elapsed_us = started.elapsed().as_micros(),
                files = index.len(),
                "workspace files matched"
            );
            cx.notify();
            return;
        }
        let Some(context) = FilesRequestContext::for_chat(self.state.read(cx), &self.active_chat)
        else {
            search.status = "No workspace available".into();
            cx.notify();
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            search.status = "Workspace service unavailable".into();
            cx.notify();
            return;
        };
        let client = WorkspaceFilesClient::new(engine, context.clone());
        let changed = search.changed.clone();
        let include = split_globs(search.include.read(cx).text());
        let exclude = split_globs(search.exclude.read(cx).text());
        let request = SearchWorkspaceContentRequest {
            target: context.target.clone(),
            query: query.to_owned(),
            case_sensitive: search.case_sensitive,
            whole_word: search.whole_word,
            regex: search.regex,
            include,
            exclude,
            changed_files: search.changed_only.then_some(changed.clone()),
        };
        search.status = "Searching…".into();
        search.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            {
                match client.search_content(request).await {
                    Ok(mut stream) => {
                        let mut first_batch = true;
                        while let Some(value) = stream.recv().await {
                            if !first_batch {
                                cx.background_executor().timer(Duration::from_millis(16)).await;
                            }
                            first_batch = false;
                            let mut frames = vec![value];
                            while frames.len() < 64 {
                                match stream.try_recv() { Ok(value) => frames.push(value), Err(_) => break }
                            }
                            let result = this.update(cx, |this, cx| {
                                let Some(search) = this
                                    .workspace_search
                                    .as_mut()
                                    .filter(|s| s.generation == generation)
                                else {
                                    return false;
                                };
                                for frame in frames.into_iter().map(serde_json::from_value::<WorkspaceContentSearchFrame>) { match frame {
                                    Ok(WorkspaceContentSearchFrame::Progress { files }) => search.status = format!("Searching… {files} files"),
                                    Ok(WorkspaceContentSearchFrame::Match {
                                        path,
                                        line,
                                        preview,
                                        ranges,
                                    }) => search.rows.push(ResultRow {
                                        changed: changed.contains(&path),
                                        path,
                                        line: u32::try_from(line).ok(),
                                        preview,
                                        ranges,
                                    }),
                                    Ok(WorkspaceContentSearchFrame::Complete {
                                        matches,
                                        files,
                                        skipped_files,
                                        truncated,
                                    }) => {
                                        search.status = if truncated {
                                            format!(
                                                "Showing first {matches} matches in {files} files"
                                            )
                                        } else {
                                            format!("{matches} matches in {files} files")
                                        };
                                        if skipped_files > 0 { search.status.push_str(&format!("; {skipped_files} files could not be searched")); }
                                    }
                                    Ok(WorkspaceContentSearchFrame::Error { message }) => {
                                        search.status = message
                                    }
                                    Err(error) => search.status = error.to_string(),
                                }}
                                let count = search.list.item_count();
                                if search.rows.len() > count {
                                    search.list.splice(count..count, search.rows.len() - count);
                                    search.list.clone().with_uniform_item_height(px(42.0));
                                }
                                cx.notify();
                                true
                            });
                            if !result.unwrap_or(false) {
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        let _ = this.update(cx, |this, cx| {
                            if let Some(search) = this
                                .workspace_search
                                .as_mut()
                                .filter(|s| s.generation == generation)
                            {
                                search.status = error.to_string();
                                cx.notify();
                            }
                        });
                    }
                }
            }
        }));
        cx.notify();
    }

    fn select_workspace_result(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(search) = self.workspace_search.as_ref() else {
            return;
        };
        if search.chat_id != self.active_chat {
            self.close_workspace_search(window, cx);
            return;
        }
        if search.context != FilesRequestContext::for_chat(self.state.read(cx), &self.active_chat) {
            self.close_workspace_search(window, cx);
            return;
        }
        if search.mode == SearchMode::Line {
            let Ok(line) = search.input.read(cx).text().trim().parse::<u32>() else {
                return;
            };
            let surface = self.resolved_right_active(cx);
            self.close_workspace_search(window, cx);
            match surface {
                RightSurface::File(id) => {
                    if let Some(file) = self.file_surfaces.get(&id) {
                        file.update(cx, |file, cx| file.reveal_editor_location(line, None, cx));
                    }
                }
                RightSurface::Diff(id) => {
                    if let Some(diff) = self.diffs.get(&id) {
                        diff.update(cx, |diff, cx| diff.go_to_line(line, cx));
                    }
                }
                _ => (),
            }
            return;
        }
        let Some(row) = search.rows.get(index).cloned() else {
            return;
        };
        let line = row
            .line
            .or_else(|| parse_line_suffix(search.input.read(cx).text()).1);
        self.close_workspace_search(window, cx);
        self.add_file_surface_at(row.path, line, None, window, cx);
    }

    pub(super) fn render_workspace_search(
        &mut self,
        viewport: gpui::Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let search = self.workspace_search.as_mut()?;
        if !search.rows.is_empty() && search.rendered_generation != Some(search.generation) {
            tracing::debug!(mode = ?search.mode, elapsed_ms = search.query_started.elapsed().as_millis(), rows = search.rows.len(), "workspace search results prepared for rendering");
            search.rendered_generation = Some(search.generation);
        }
        if std::mem::take(&mut search.focus_pending) {
            window.focus(&search.input.focus_handle(cx), cx);
        }
        let mode = search.mode;
        let list = search.list.clone();
        let theme = Theme::of(cx).for_popup();
        let mut card = div()
            .id("workspace-search")
            .w(px(640.0_f32.min(f32::from(viewport.width) - 32.0)))
            .max_h(px(600.0_f32.min(f32::from(viewport.height) - 32.0)))
            .flex()
            .flex_col()
            .p(px(16.0))
            .gap(px(8.0))
            .rounded(px(12.0))
            .bg(popover::surface_bg(&theme))
            .text_color(theme.text)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => this.close_workspace_search(window, cx),
                    "enter" => {
                        let index = this.workspace_search.as_ref().map_or(0, |s| s.active);
                        this.select_workspace_result(index, window, cx);
                    }
                    "up" | "down" => {
                        if let Some(search) = this.workspace_search.as_mut() {
                            let n = search.rows.len();
                            if n > 0 {
                                search.active = if event.keystroke.key == "down" {
                                    (search.active + 1) % n
                                } else {
                                    (search.active + n - 1) % n
                                };
                                search.list.scroll_to_reveal_item(search.active);
                                cx.notify();
                            }
                        }
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .on_mouse_down_out(
                cx.listener(|this, _, window, cx| this.close_workspace_search(window, cx)),
            )
            .child(search.input.clone());
        if mode == SearchMode::Content {
            let toggles = [
                ("Case", search.case_sensitive, 0usize),
                ("Word", search.whole_word, 1),
                ("Regex", search.regex, 2),
                ("Only changed files", search.changed_only, 3),
            ];
            let mut buttons = div().flex().gap(px(8.0));
            for (label, selected, id) in toggles {
                if id == 3 && !search.has_diff {
                    continue;
                }
                buttons = buttons.child(
                    div()
                        .id(("search-option", id))
                        .role(gpui::Role::Button)
                        .aria_label(label)
                        .cursor_pointer()
                        .px(px(6.0))
                        .rounded(px(4.0))
                        .when(selected, |el| el.bg(theme.surface_raised))
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(search) = this.workspace_search.as_mut() {
                                match id {
                                    0 => search.case_sensitive = !search.case_sensitive,
                                    1 => search.whole_word = !search.whole_word,
                                    2 => search.regex = !search.regex,
                                    _ => search.changed_only = !search.changed_only,
                                };
                            }
                            this.update_workspace_search(cx);
                        })),
                );
            }
            card = card
                .child(buttons)
                .child(search.include.clone())
                .child(search.exclude.clone());
        }
        let results = gpui::list(list, cx.processor(Self::render_workspace_search_row))
            .h(px(
                320.0_f32.min((f32::from(viewport.height) - 200.0).max(80.0))
            ))
            .with_sizing_behavior(gpui::ListSizingBehavior::Auto);
        card = card
            .when(mode != SearchMode::Line, |card| card.child(results))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .child(search.status.clone()),
            );
        Some(popover::modal(
            "workspace-search-overlay",
            viewport,
            card.into_any_element(),
        ))
    }
    fn render_workspace_search_row(
        &mut self,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(search) = self.workspace_search.as_ref() else {
            return gpui::Empty.into_any_element();
        };
        let Some(row) = search.rows.get(index).cloned() else {
            return gpui::Empty.into_any_element();
        };
        let previous = index
            .checked_sub(1)
            .and_then(|index| search.rows.get(index));
        let header = match search.mode {
            SearchMode::Content if previous.is_none_or(|previous| previous.path != row.path) => {
                Some(row.path.clone())
            }
            SearchMode::Files
                if previous.is_none_or(|previous| previous.changed != row.changed) =>
            {
                Some(
                    if row.changed {
                        "Changed in this diff"
                    } else {
                        "Files"
                    }
                    .to_owned(),
                )
            }
            _ => None,
        };
        let theme = Theme::of(cx).for_popup();
        let label = if let Some(line) = row.line {
            let prefix = format!("{line}: ");
            let offset = prefix.len();
            gpui::StyledText::new(format!("{prefix}{}", row.preview))
                .with_highlights(row.ranges.iter().map(|&(start, end)| {
                    (
                        offset + start..offset + end,
                        gpui::HighlightStyle {
                            color: Some(theme.code_text),
                            background_color: Some(theme.code_wash),
                            ..Default::default()
                        },
                    )
                }))
                .into_any_element()
        } else {
            row.path.clone().into_any_element()
        };
        div()
            .id(("workspace-search-result", index))
            .role(gpui::Role::Button)
            .aria_label(row.path.clone())
            .h(px(42.0))
            .w_full()
            .px(px(8.0))
            .flex()
            .flex_col()
            .justify_end()
            .cursor_pointer()
            .when(index == search.active, |el| el.bg(theme.surface_raised))
            .when_some(header, |el, header| {
                el.child(
                    div()
                        .text_size(px(10.0))
                        .text_color(theme.text_muted)
                        .truncate()
                        .child(header),
                )
            })
            .child(div().truncate().child(label))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_workspace_result(index, window, cx)
            }))
            .into_any_element()
    }
}
fn parse_line_suffix(query: &str) -> (&str, Option<u32>) {
    match query.rsplit_once(':') {
        Some((path, line)) if !path.is_empty() => match line.parse::<u32>() {
            Ok(line) if line > 0 => (path, Some(line)),
            _ => (query, None),
        },
        _ => (query, None),
    }
}
fn split_globs(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn line_suffix_and_globs() {
        assert_eq!(
            parse_line_suffix("src/main.rs:42"),
            ("src/main.rs", Some(42))
        );
        assert_eq!(parse_line_suffix("src/main.rs:0"), ("src/main.rs:0", None));
        assert_eq!(split_globs("*.rs, src/** , "), vec!["*.rs", "src/**"]);
        let path = IndexedPath {
            path: "src/Workspace.rs".into(),
            lower_path: "src/workspace.rs".into(),
            lower_name: "workspace.rs".into(),
        };
        assert!(file_score(&path, "workspace").unwrap() > file_score(&path, "ws").unwrap());
        assert!(file_score(&path, "missing").is_none());
    }

    #[gpui::test]
    fn search_debounces_edits_and_falls_back_for_older_engines(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (mut wire, engine, _guard) = crate::admin::tests::Wire::new();
        cx.update(|cx| {
            settings::init(settings::UiSettings::default(), dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let host = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: mast_proto::HarnessId::Mock,
                },
                cx,
            )
        });
        host.update(cx, |shell, window, cx| {
            shell.debug_gate = Some(GatePhase::Ready);
            shell.state.update(cx, |state, cx| {
                state.chats = vec![serde_json::from_value(serde_json::json!({"id":"chat", "deviceId":"local", "cwd":"/tmp", "archived":false, "createdAt":Utc::now()})).unwrap()];
                state.set_test_engine(engine);
                state.select_chat(Some("chat".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.open_workspace_search(SearchMode::Content, window, cx);
        }).unwrap();
        cx.run_until_parked();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        host.update(cx, |shell, window, cx| {
            let input = &shell.workspace_search.as_ref().unwrap().input;
            window.focus(&input.focus_handle(cx), cx);
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        cx.simulate_input(host.into(), "a");
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        assert!(
            wire.sent()
                .iter()
                .all(|(method, _)| method != mast_rpc::methods::SEARCH_WORKSPACE_CONTENT)
        );
        cx.simulate_input(host.into(), "b");
        cx.run_until_parked();
        host.update(cx, |shell, _, cx| {
            let search = shell.workspace_search.as_ref().unwrap();
            assert_eq!(search.input.read(cx).text(), "ab");
            assert_eq!(search.status, "Searching…");
        })
        .unwrap();
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        assert!(
            wire.sent()
                .iter()
                .all(|(method, _)| method != mast_rpc::methods::SEARCH_WORKSPACE_CONTENT)
        );
        cx.executor().advance_clock(Duration::from_millis(60));
        cx.run_until_parked();
        let searches: Vec<_> = wire
            .sent()
            .into_iter()
            .filter(|(method, _)| method == mast_rpc::methods::SEARCH_WORKSPACE_CONTENT)
            .collect();
        assert_eq!(searches.len(), 1);
        assert_eq!(searches[0].1["query"], "ab");
        host.update(cx, |shell, window, cx| {
            shell.close_workspace_search(window, cx);
            shell.open_workspace_search(SearchMode::Files, window, cx);
            let key = shell.workspace_search_index_key.clone().unwrap();
            shell.workspace_search_indexes.remove(&key);
            shell.workspace_search_index_error = Some((key, "unsupported".into()));
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        host.update(cx, |shell, window, cx| {
            let input = &shell.workspace_search.as_ref().unwrap().input;
            window.focus(&input.focus_handle(cx), cx);
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        cx.simulate_input(host.into(), "file");
        cx.run_until_parked();
        let mut subscriptions = 0;
        while let Ok(frame) = wire.requests.try_recv() {
            let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
            // Dropping the prewarmed stream also sends a cancellation frame.
            if frame["method"] == mast_rpc::methods::WATCH_WORKSPACE_SEARCH_FILES {
                subscriptions += 1;
            }
        }
        assert_eq!(subscriptions, 1, "reopening requests a fresh index subscription");
        cx.executor().advance_clock(Duration::from_millis(160));
        cx.run_until_parked();
        let mut searches = Vec::new();
        while let Ok(frame) = wire.requests.try_recv() {
            let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
            if frame["method"] == mast_rpc::methods::SEARCH_WORKSPACE_FILES {
                searches.push(frame["params"].clone());
            }
        }
        assert_eq!(searches.len(), 1);
        assert_eq!(searches[0]["query"], "file");
    }

    #[gpui::test]
    #[ignore]
    fn measure_workspace_search_latency(cx: &mut TestAppContext) {
        // The in-memory RPC service runs on Tokio threads, so this benchmark
        // intentionally permits external wakeups while retaining a controlled clock.
        cx.executor().allow_parking();
        cx.executor().forbid_parking();
        let dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let core = mast_engine::EngineCore::assemble(
            dir.path(),
            std::sync::Arc::new(mast_engine::HarnessRegistry::new()),
            mast_proto::HarnessId::Mock,
            None,
        )
        .unwrap();
        let root =
            std::env::var("MAST_BENCH_ROOT").expect("MAST_BENCH_ROOT must name the checkout");
        core.workspace
            .create_space("space", &core.device_id, &root, None, true)
            .unwrap();
        core.workspace
            .create_chat("chat", Some("space"), None, None, None)
            .unwrap();
        let chat = core.workspace.chat("chat").unwrap().unwrap();
        let device_id = core.device_id.clone();
        let engine = crate::state::EngineHandle::from_test_client(mast_rpc::memory_client(
            core.rpc_service(),
        ));
        cx.update(|cx| {
            settings::init(settings::UiSettings::default(), dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let host = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: mast_proto::HarnessId::Mock,
                },
                cx,
            )
        });
        host.update(cx, |shell, window, cx| {
            shell.debug_gate = Some(GatePhase::Ready);
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat];
                state.local_device_id = Some(device_id);
                state.set_test_engine(engine);
                state.select_chat(Some("chat".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.open_workspace_search(SearchMode::Files, window, cx);
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        let prewarm_started = std::time::Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(10));
            cx.executor().advance_clock(Duration::from_millis(10));
            cx.run_until_parked();
            if host
                .read_with(cx, |shell, _| !shell.workspace_search_indexes.is_empty())
                .unwrap()
            {
                break;
            }
            assert!(prewarm_started.elapsed() < Duration::from_secs(20));
        }
        eprintln!("index prewarm: {:?}", prewarm_started.elapsed());
        for ch in "workspace".chars() {
            let start = std::time::Instant::now();
            cx.simulate_input(host.into(), &ch.to_string());
            loop {
                std::thread::sleep(Duration::from_millis(10));
                cx.executor().advance_clock(Duration::from_millis(10));
                cx.run_until_parked();
                cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
                    .unwrap();
                let done = host
                    .read_with(cx, |shell, _| {
                        shell
                            .workspace_search
                            .as_ref()
                            .is_some_and(|search| !search.rows.is_empty())
                    })
                    .unwrap();
                if done {
                    break;
                }
                assert!(start.elapsed() < Duration::from_secs(10));
            }
            eprintln!("query character {ch}: {:?}", start.elapsed());
        }
        host.update(cx, |shell, window, cx| {
            shell.close_workspace_search(window, cx);
            shell.open_workspace_search(SearchMode::Content, window, cx);
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        let content_started = std::time::Instant::now();
        cx.simulate_input(host.into(), "WorkspaceSearch");
        let mut first = None;
        loop {
            std::thread::sleep(Duration::from_millis(10));
            cx.executor().advance_clock(Duration::from_millis(10));
            cx.run_until_parked();
            cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
                .unwrap();
            let (has_results, complete) = host
                .read_with(cx, |shell, _| {
                    let search = shell.workspace_search.as_ref().unwrap();
                    (
                        !search.rows.is_empty(),
                        search.status.contains("matches in"),
                    )
                })
                .unwrap();
            if has_results && first.is_none() {
                first = Some(content_started.elapsed());
            }
            if complete {
                break;
            }
            assert!(content_started.elapsed() < Duration::from_secs(20));
        }
        eprintln!(
            "content first: {:?}; complete: {:?}",
            first,
            content_started.elapsed()
        );
        host.update(cx, |shell, window, cx| {
            shell.close_workspace_search(window, cx);
            shell.workspace_search_index_task = None;
        })
        .unwrap();
        runtime.block_on(core.shutdown());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn configured_shortcuts_open_the_search_overlays(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            settings::init(settings::UiSettings::default(), dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let host = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: mast_proto::HarnessId::Mock,
                },
                cx,
            )
        });
        host.update(cx, |shell, _, cx| {
            shell.debug_gate = Some(GatePhase::Ready);
            shell.state.update(cx, |state, cx| {
                state.chats = vec![serde_json::from_value(serde_json::json!({"id":"chat", "deviceId":"local", "cwd":"/tmp", "archived":false, "createdAt":Utc::now()})).unwrap()];
                state.select_chat(Some("chat".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
        }).unwrap();
        cx.run_until_parked();
        cx.update_window(host.into(), |_, window, cx| {
            window.draw(cx).clear();
            let context = gpui::KeyContext::parse("Input").unwrap();
            let replace =
                window.bindings_for_action_in_context(&gpui_base::input::Replace, context);
            let new_key = gpui::Keystroke::parse(&platform_combo("mod-alt-f")).unwrap();
            let old_key = gpui::Keystroke::parse(&platform_combo("mod-shift-f")).unwrap();
            assert!(replace.iter().any(|binding| {
                binding.match_keystrokes(std::slice::from_ref(&new_key)) == Some(false)
            }));
            assert!(!replace.iter().any(|binding| {
                binding.match_keystrokes(std::slice::from_ref(&old_key)) == Some(false)
            }));
        })
        .unwrap();
        cx.simulate_keystrokes(host.into(), &platform_combo("mod-o"));
        host.update(cx, |shell, window, cx| {
            let input = &shell.workspace_search.as_ref().unwrap().input;
            input.update(cx, |input, cx| input.set_text("hello world", cx));
            window.focus(&input.focus_handle(cx), cx);
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        cx.simulate_keystrokes(host.into(), "backspace");
        host.update(cx, |shell, _, cx| {
            assert_eq!(
                shell
                    .workspace_search
                    .as_ref()
                    .unwrap()
                    .input
                    .read(cx)
                    .text(),
                "hello worl"
            );
        })
        .unwrap();
        cx.simulate_keystrokes(host.into(), "left delete alt-backspace");
        host.update(cx, |shell, _, cx| {
            assert_eq!(
                shell
                    .workspace_search
                    .as_ref()
                    .unwrap()
                    .input
                    .read(cx)
                    .text(),
                "hello "
            );
        })
        .unwrap();
        cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into())));
        cx.simulate_keystrokes(host.into(), "cmd-a cmd-v");
        host.update(cx, |shell, _, cx| {
            assert_eq!(
                shell
                    .workspace_search
                    .as_ref()
                    .unwrap()
                    .input
                    .read(cx)
                    .text(),
                "pasted"
            );
        })
        .unwrap();
        host.update(cx, |shell, window, cx| {
            assert!(
                shell
                    .workspace_search
                    .as_ref()
                    .is_some_and(|search| search.mode == SearchMode::Files)
            );
            let search = shell.workspace_search.as_mut().unwrap();
            search.rows = ["one.rs", "two.rs"]
                .into_iter()
                .map(|path| ResultRow {
                    path: path.into(),
                    line: None,
                    preview: String::new(),
                    ranges: Vec::new(),
                    changed: false,
                })
                .collect();
            search.list.reset_with_uniform_height(2, px(42.0));
            window.focus(&search.input.focus_handle(cx), cx);
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        cx.simulate_keystrokes(host.into(), "down");
        host.update(cx, |shell, _, _| {
            assert_eq!(shell.workspace_search.as_ref().unwrap().active, 1)
        })
        .unwrap();
        cx.simulate_keystrokes(host.into(), "up enter");
        host.update(cx, |shell, window, cx| {
            assert!(shell.workspace_search.is_none());
            shell.open_workspace_search(SearchMode::Files, window, cx);
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        cx.simulate_keystrokes(host.into(), "escape");
        host.update(cx, |shell, _, _| assert!(shell.workspace_search.is_none()))
            .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        cx.simulate_keystrokes(host.into(), &platform_combo("mod-shift-f"));
        host.update(cx, |shell, window, cx| {
            let input = &shell.workspace_search.as_ref().unwrap().input;
            input.update(cx, |input, cx| input.set_text("hello world", cx));
            window.focus(&input.focus_handle(cx), cx);
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        cx.simulate_keystrokes(host.into(), "backspace");
        host.update(cx, |shell, _, cx| {
            assert_eq!(
                shell
                    .workspace_search
                    .as_ref()
                    .unwrap()
                    .input
                    .read(cx)
                    .text(),
                "hello worl"
            );
        })
        .unwrap();
        cx.simulate_keystrokes(host.into(), "left delete alt-backspace");
        host.update(cx, |shell, _, cx| {
            assert_eq!(
                shell
                    .workspace_search
                    .as_ref()
                    .unwrap()
                    .input
                    .read(cx)
                    .text(),
                "hello "
            );
        })
        .unwrap();
        cx.simulate_keystrokes(host.into(), "cmd-a cmd-v");
        host.update(cx, |shell, _, cx| {
            assert_eq!(
                shell
                    .workspace_search
                    .as_ref()
                    .unwrap()
                    .input
                    .read(cx)
                    .text(),
                "pasted"
            );
        })
        .unwrap();
        for include in [true, false] {
            host.update(cx, |shell, window, cx| {
                let search = shell.workspace_search.as_ref().unwrap();
                let input = if include {
                    &search.include
                } else {
                    &search.exclude
                };
                input.update(cx, |input, cx| input.set_text("*.rs", cx));
                window.focus(&input.focus_handle(cx), cx);
            })
            .unwrap();
            cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
                .unwrap();
            cx.simulate_keystrokes(host.into(), "backspace");
            host.update(cx, |shell, _, cx| {
                let search = shell.workspace_search.as_ref().unwrap();
                let input = if include {
                    &search.include
                } else {
                    &search.exclude
                };
                assert_eq!(input.read(cx).text(), "*.r");
            })
            .unwrap();
        }
        host.update(cx, |shell, _, _| {
            let search = shell.workspace_search.as_mut().unwrap();
            search.rows = ["one.rs", "two.rs"]
                .into_iter()
                .map(|path| ResultRow {
                    path: path.into(),
                    line: Some(1),
                    preview: String::new(),
                    ranges: Vec::new(),
                    changed: false,
                })
                .collect();
            search.list.reset_with_uniform_height(2, px(42.0));
            search.active = 0;
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        cx.simulate_keystrokes(host.into(), "down");
        host.update(cx, |shell, _, _| {
            assert_eq!(shell.workspace_search.as_ref().unwrap().active, 1)
        })
        .unwrap();
        cx.simulate_keystrokes(host.into(), "up");
        host.update(cx, |shell, _, _| {
            assert_eq!(shell.workspace_search.as_ref().unwrap().active, 0)
        })
        .unwrap();
        cx.simulate_keystrokes(host.into(), "escape");
        host.update(cx, |shell, window, cx| {
            assert!(shell.workspace_search.is_none());
            shell.open_workspace_search(SearchMode::Content, window, cx);
            let search = shell.workspace_search.as_mut().unwrap();
            search.rows.push(ResultRow {
                path: "one.rs".into(),
                line: Some(1),
                preview: String::new(),
                ranges: Vec::new(),
                changed: false,
            });
            search.list.reset_with_uniform_height(1, px(42.0));
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        cx.simulate_keystrokes(host.into(), "enter");
        host.update(cx, |shell, window, cx| {
            assert!(shell.workspace_search.is_none());
            shell.open_workspace_search(SearchMode::Content, window, cx);
        })
        .unwrap();
        host.update(cx, |shell, _, _| {
            assert!(
                shell
                    .workspace_search
                    .as_ref()
                    .is_some_and(|search| search.mode == SearchMode::Content)
            )
        })
        .unwrap();
        host.update(cx, |shell, window, cx| {
            shell.close_workspace_search(window, cx);
            let mut map = KeymapConfig::default();
            map.quick_open = "mod-alt-o".into();
            apply_keymap(cx, &map, ComposerSendBehavior::default());
        })
        .unwrap();
        cx.update_window(host.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        cx.simulate_keystrokes(host.into(), &platform_combo("mod-alt-o"));
        host.update(cx, |shell, _, _| {
            assert!(
                shell
                    .workspace_search
                    .as_ref()
                    .is_some_and(|search| search.mode == SearchMode::Files)
            )
        })
        .unwrap();
    }

    #[gpui::test]
    fn overlays_open_results_at_the_requested_line(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            settings::init(settings::UiSettings::default(), dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let host = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: mast_proto::HarnessId::Mock,
                },
                cx,
            )
        });
        host.update(cx, |shell, window, cx| {
            shell.active_chat = "chat".into();
            shell.open_workspace_search(SearchMode::Files, window, cx);
            let input = shell.workspace_search.as_ref().unwrap().input.clone();
            input.update(cx, |input, cx| input.set_text("src/main.rs:42", cx));
            shell.workspace_search.as_mut().unwrap().rows = vec![ResultRow {
                path: "src/main.rs".into(),
                line: None,
                preview: String::new(),
                ranges: Vec::new(),
                changed: false,
            }];
            shell.select_workspace_result(0, window, cx);
            let id = match shell.resolved_right_active(cx) {
                RightSurface::File(id) => id,
                _ => panic!("file not opened"),
            };
            assert_eq!(
                shell.file_surfaces[&id]
                    .read(cx)
                    .pending_location_for_test(),
                Some((42, None))
            );
            shell.open_workspace_search(SearchMode::Content, window, cx);
            let input = shell.workspace_search.as_ref().unwrap().input.clone();
            input.update(cx, |input, cx| input.set_text("needle", cx));
            shell.workspace_search.as_mut().unwrap().rows = vec![ResultRow {
                path: "src/main.rs".into(),
                line: Some(19),
                preview: "needle".into(),
                ranges: vec![(0, 6)],
                changed: false,
            }];
            shell.select_workspace_result(0, window, cx);
            assert_eq!(
                shell.file_surfaces[&id]
                    .read(cx)
                    .pending_location_for_test(),
                Some((19, None))
            );
            shell.open_workspace_search(SearchMode::Files, window, cx);
            shell.workspace_search.as_mut().unwrap().mode = SearchMode::Line;
            let input = shell.workspace_search.as_ref().unwrap().input.clone();
            input.update(cx, |input, cx| input.set_text("7", cx));
            shell.select_workspace_result(0, window, cx);
            assert_eq!(
                shell.file_surfaces[&id]
                    .read(cx)
                    .pending_location_for_test(),
                Some((7, None))
            );
        })
        .unwrap();
    }
}
