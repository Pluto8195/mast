//! Settings → Workflow pack: export the selected Space's instructions and
//! project templates to a file, and import one into the Space as preview →
//! confirmed apply → undo. Every write goes through the engine
//! (`WorkflowPack*` RPC); the page never changes a Space itself.

use gpui::{
    AnyElement, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use mast_proto::workflow_pack::{WorkflowPackScopePreview, WorkflowPackTemplatePreview};
use mast_rpc::methods;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::icons;
use crate::popover;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

/// What happens to a destination scope or template that differs from the pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Skip,
    Append,
    Replace,
}

impl Choice {
    fn wire(self) -> &'static str {
        match self {
            Choice::Skip => "skip",
            Choice::Append => "append",
            Choice::Replace => "replace",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Choice::Skip => "Keep mine",
            Choice::Append => "Add missing",
            Choice::Replace => "Replace",
        }
    }
}

/// How one incoming scope relates to the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// Nothing to import, or already identical.
    Unchanged,
    /// The destination has no instructions there.
    New,
    /// Both sides have different instructions: the user chooses.
    Conflict,
}

pub fn scope_kind(scope: &WorkflowPackScopePreview) -> ScopeKind {
    if scope.incoming.is_empty() || scope.current == scope.incoming {
        ScopeKind::Unchanged
    } else if scope.current.is_empty() {
        ScopeKind::New
    } else {
        ScopeKind::Conflict
    }
}

/// A preview as the engine returns it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub preview_id: String,
    pub scopes: Vec<WorkflowPackScopePreview>,
    pub templates: Vec<WorkflowPackTemplatePreview>,
}

/// A preview plus the user's choice for each scope and template.
#[derive(Debug, Clone)]
pub struct ImportReview {
    pub preview: ImportPreview,
    pub scope_choices: Vec<Choice>,
    pub template_choices: Vec<Choice>,
}

impl ImportReview {
    /// Every conflict starts at "keep mine", so applying never overwrites
    /// anything the user did not choose.
    pub fn new(preview: ImportPreview) -> Self {
        Self {
            scope_choices: vec![Choice::Skip; preview.scopes.len()],
            template_choices: vec![Choice::Skip; preview.templates.len()],
            preview,
        }
    }

    /// How many scopes and templates applying would change.
    pub fn change_count(&self) -> usize {
        let scopes = self
            .preview
            .scopes
            .iter()
            .zip(&self.scope_choices)
            .filter(|(scope, choice)| match scope_kind(scope) {
                ScopeKind::Unchanged => false,
                ScopeKind::New => true,
                ScopeKind::Conflict => **choice != Choice::Skip,
            })
            .count();
        let templates = self
            .preview
            .templates
            .iter()
            .zip(&self.template_choices)
            .filter(|(template, choice)| template.current.is_none() || **choice != Choice::Skip)
            .count();
        scopes + templates
    }

    /// The `resolutions` argument of `WorkflowPackApply`.
    pub fn resolutions(&self) -> Value {
        let scopes: Vec<Value> = self
            .preview
            .scopes
            .iter()
            .zip(&self.scope_choices)
            .filter(|(scope, _)| scope_kind(scope) == ScopeKind::Conflict)
            .map(|(scope, choice)| json!({ "itemPath": scope.item_path, "resolution": choice.wire() }))
            .collect();
        let templates: Vec<Value> = self
            .preview
            .templates
            .iter()
            .zip(&self.template_choices)
            .filter(|(template, _)| template.current.is_some())
            .map(|(template, choice)| json!({ "name": template.name, "resolution": choice.wire() }))
            .collect();
        json!({ "scopes": scopes, "templates": templates })
    }
}

/// An export awaiting the user's review of what was redacted.
#[derive(Debug, Clone)]
pub struct ExportReview {
    pub json: String,
    pub scopes: usize,
    pub templates: usize,
    pub redactions: usize,
    pub share: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptSummary {
    #[serde(default)]
    pub items_created: usize,
    #[serde(default)]
    pub instructions_added: usize,
    #[serde(default)]
    pub instructions_archived: usize,
    #[serde(default)]
    pub templates_added: usize,
    #[serde(default)]
    pub templates_replaced: usize,
}

impl ReceiptSummary {
    /// "2 instructions added · 1 template replaced", for the receipt row.
    pub fn describe(&self) -> String {
        fn plural(count: usize, one: &str, many: &str) -> String {
            format!("{count} {}", if count == 1 { one } else { many })
        }
        let mut parts = Vec::new();
        if self.instructions_added > 0 {
            parts.push(format!("{} added", plural(self.instructions_added, "instruction", "instructions")));
        }
        if self.instructions_archived > 0 {
            parts.push(format!("{} replaced", plural(self.instructions_archived, "instruction", "instructions")));
        }
        if self.templates_added > 0 {
            parts.push(format!("{} added", plural(self.templates_added, "template", "templates")));
        }
        if self.templates_replaced > 0 {
            parts.push(format!("{} replaced", plural(self.templates_replaced, "template", "templates")));
        }
        if self.items_created > 0 {
            parts.push(format!("{} created", plural(self.items_created, "item", "items")));
        }
        parts.join(" · ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptRow {
    pub id: String,
    pub created_at: i64,
    #[serde(default)]
    pub undone_at: Option<i64>,
    #[serde(default)]
    pub summary: ReceiptSummary,
}

#[derive(Deserialize)]
struct ReceiptList {
    receipts: Vec<ReceiptRow>,
}

#[derive(Debug, Clone)]
enum Stage {
    Idle,
    /// A request is in flight; the text says what.
    Working(&'static str),
    Export(ExportReview),
    Import(ImportReview),
}

pub struct WorkflowPackPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    stage: Stage,
    receipts: Vec<ReceiptRow>,
    error: Option<SharedString>,
    notice: Option<SharedString>,
    task: Option<Task<()>>,
    /// The Space the receipts were last loaded for.
    receipts_space: Option<String>,
    _observe: Subscription,
}

impl WorkflowPackPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&state, |this: &mut Self, state, cx| {
            let space = state.read(cx).selected_space.clone();
            if space != this.receipts_space {
                this.stage = Stage::Idle;
                this.refresh_receipts(cx);
                cx.notify();
            }
        });
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            stage: Stage::Idle,
            receipts: Vec::new(),
            error: None,
            notice: None,
            task: None,
            receipts_space: None,
            _observe: observe,
        };
        page.refresh_receipts(cx);
        page
    }

    /// A page showing fixed data, without an engine, for previews.
    #[cfg(feature = "snapshot")]
    pub(crate) fn fixture(
        state: Entity<AppState>,
        review: ImportReview,
        receipts: Vec<ReceiptRow>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&state, |_, _, _| {});
        Self {
            state,
            scroll: widgets::PageScroll::default(),
            stage: Stage::Import(review),
            receipts,
            error: None,
            notice: None,
            task: None,
            receipts_space: None,
            _observe: observe,
        }
    }

    fn space_id(&self, cx: &Context<Self>) -> Option<String> {
        self.state.read(cx).selected_space.clone()
    }

    fn space_name(&self, cx: &Context<Self>) -> Option<SharedString> {
        let state = self.state.read(cx);
        let id = state.selected_space.as_deref()?;
        let space = state.spaces.iter().find(|space| space.id == id)?;
        space
            .name
            .clone()
            .or_else(|| {
                std::path::Path::new(&space.path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .map(SharedString::from)
    }

    /// Run one RPC, then hand its result to `done`. One request at a time.
    fn request(
        &mut self,
        working: &'static str,
        method: &'static str,
        params: Value,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<Value, String>, &mut Context<Self>) + 'static,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.error = Some("Mast is not connected to an engine".into());
            cx.notify();
            return;
        };
        self.error = None;
        self.notice = None;
        self.stage = Stage::Working(working);
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(method, params)
                .await
                .map_err(|error| error.to_string());
            this.update(cx, |page, cx| {
                if matches!(page.stage, Stage::Working(_)) {
                    page.stage = Stage::Idle;
                }
                done(page, result, cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn refresh_receipts(&mut self, cx: &mut Context<Self>) {
        let space = self.space_id(cx);
        self.receipts_space = space.clone();
        let (Some(space), Some(engine)) = (space, self.state.read(cx).engine().cloned()) else {
            self.receipts.clear();
            return;
        };
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::WORKFLOW_PACK_RECEIPTS, json!({ "spaceId": space }))
                .await;
            this.update(cx, |page, cx| {
                if let Ok(value) = result
                    && let Ok(list) = serde_json::from_value::<ReceiptList>(value)
                {
                    page.receipts = list.receipts;
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn start_export(&mut self, share: bool, cx: &mut Context<Self>) {
        let Some(space) = self.space_id(cx) else {
            return;
        };
        let mode = if share { "share" } else { "full" };
        self.request(
            "Preparing the export…",
            methods::WORKFLOW_PACK_EXPORT_PREVIEW,
            json!({ "spaceId": space, "mode": mode }),
            cx,
            move |page, result, _| match result {
                Ok(value) => {
                    let pack = value.get("pack").cloned().unwrap_or(Value::Null);
                    let count = |key: &str| pack.get(key).and_then(Value::as_array).map_or(0, Vec::len);
                    let redactions = value
                        .get("redactions")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    page.stage = Stage::Export(ExportReview {
                        json: serde_json::to_string_pretty(&pack).unwrap_or_default(),
                        scopes: count("scopes"),
                        templates: count("templates"),
                        redactions,
                        share,
                    });
                }
                Err(error) => page.error = Some(format!("Export failed: {error}").into()),
            },
        );
    }

    fn save_export(&mut self, cx: &mut Context<Self>) {
        let Stage::Export(review) = &self.stage else {
            return;
        };
        let json = review.json.clone();
        let directory = std::env::var_os("HOME")
            .map(|home| std::path::PathBuf::from(home).join("Documents"))
            .unwrap_or_default();
        let receiver = cx.prompt_for_new_path(&directory, Some("workflow-pack.json"));
        self.task = Some(cx.spawn(async move |this, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(path))) => path,
                _ => return,
            };
            let result = std::fs::write(&path, json).map_err(|error| error.to_string());
            this.update(cx, |page, cx| {
                match result {
                    Ok(()) => {
                        page.stage = Stage::Idle;
                        page.notice = Some(format!("Saved {}", path.display()).into());
                    }
                    Err(error) => page.error = Some(format!("Could not save the pack: {error}").into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn choose_import_file(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose Workflow Pack".into()),
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(mut paths))) => paths.pop(),
                _ => None,
            };
            let Some(path) = path else {
                return;
            };
            this.update(cx, |page, cx| page.preview_import(path, cx)).ok();
        }));
    }

    fn preview_import(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        let Some(space) = self.space_id(cx) else {
            return;
        };
        self.request(
            "Reading the pack…",
            methods::WORKFLOW_PACK_IMPORT_PREVIEW,
            json!({ "spaceId": space, "path": path }),
            cx,
            |page, result, _| match result.and_then(|value| {
                serde_json::from_value::<ImportPreview>(value).map_err(|error| error.to_string())
            }) {
                Ok(preview) => page.stage = Stage::Import(ImportReview::new(preview)),
                Err(error) => page.error = Some(format!("Could not read the pack: {error}").into()),
            },
        );
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let (Some(space), Stage::Import(review)) = (self.space_id(cx), &self.stage) else {
            return;
        };
        if review.change_count() == 0 {
            return;
        }
        let params = json!({
            "spaceId": space,
            "previewId": review.preview.preview_id,
            "resolutions": review.resolutions(),
        });
        let restore = review.clone();
        self.request(
            "Applying…",
            methods::WORKFLOW_PACK_APPLY,
            params,
            cx,
            move |page, result, cx| match result {
                Ok(_) => {
                    page.notice = Some("Imported. You can undo it below.".into());
                    page.refresh_receipts(cx);
                }
                Err(error) => {
                    // Keep the review so the user can adjust and retry.
                    page.stage = Stage::Import(restore);
                    page.error = Some(format!("Import failed, nothing was changed: {error}").into());
                }
            },
        );
    }

    fn undo(&mut self, receipt_id: String, cx: &mut Context<Self>) {
        let Some(space) = self.space_id(cx) else {
            return;
        };
        self.request(
            "Undoing…",
            methods::WORKFLOW_PACK_UNDO,
            json!({ "spaceId": space, "receiptId": receipt_id }),
            cx,
            |page, result, cx| match result {
                Ok(_) => {
                    page.notice = Some("Import undone.".into());
                    page.refresh_receipts(cx);
                }
                Err(error) => {
                    page.error =
                        Some(format!("Could not undo, nothing was changed: {error}").into())
                }
            },
        );
    }

    fn set_choice(&mut self, template: bool, index: usize, choice: Choice, cx: &mut Context<Self>) {
        if let Stage::Import(review) = &mut self.stage {
            let choices = if template {
                &mut review.template_choices
            } else {
                &mut review.scope_choices
            };
            if let Some(slot) = choices.get_mut(index) {
                *slot = choice;
                cx.notify();
            }
        }
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }
}

impl popover::ScrollRailHost for WorkflowPackPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

fn path_label(path: &[String]) -> SharedString {
    if path.is_empty() {
        "Space root".into()
    } else {
        path.join(" / ").into()
    }
}

fn count_label(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn fragment(theme: &Theme, text: impl Into<SharedString>) -> AnyElement {
    let _ = theme;
    div().child(text.into()).into_any_element()
}

impl WorkflowPackPage {
    fn action_row(
        &self,
        theme: &Theme,
        id: &'static str,
        icon: &'static str,
        title: &'static str,
        detail: SharedString,
        actions: Vec<AnyElement>,
    ) -> gpui::Div {
        widgets::card_row(theme, id == "export")
            .child(widgets::row_tile(theme, icon))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(widgets::row_title(theme, title))
                    .child(widgets::meta_line(theme, vec![fragment(theme, detail)])),
            )
            .child(div().flex().flex_row().gap(px(8.0)).children(actions))
    }

    fn render_choices(
        &self,
        theme: &Theme,
        template: bool,
        index: usize,
        selected: Choice,
        options: &[Choice],
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_row()
            .gap(px(6.0))
            .children(options.iter().map(|&choice| {
                let id = format!("workflow-pack-choice-{template}-{index}-{}", choice.wire());
                crate::components::settings_choice(
                    theme,
                    id.into(),
                    choice.label(),
                    selected == choice,
                    true,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_choice(template, index, choice, cx);
                }))
            }))
    }

    fn render_import(
        &self,
        theme: &Theme,
        review: &ImportReview,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut card = widgets::section_card(theme);
        let mut first = true;
        for (index, scope) in review.preview.scopes.iter().enumerate() {
            let kind = scope_kind(scope);
            if kind == ScopeKind::Unchanged {
                continue;
            }
            let detail = match kind {
                ScopeKind::New => {
                    let mut text = count_label(scope.incoming.len(), "instruction", "instructions");
                    if let Some(from) = scope.missing_from {
                        let missing = scope.item_path.get(from..).unwrap_or_default().join(" / ");
                        text = format!("{text} · creates {missing}");
                    }
                    format!("New: {text}")
                }
                _ => format!(
                    "Conflict: you have {}, the pack has {}",
                    count_label(scope.current.len(), "instruction", "instructions"),
                    scope.incoming.len()
                ),
            };
            let mut row = widgets::card_row(theme, first)
                .child(widgets::row_tile(theme, icons::DOCUMENT))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(widgets::row_title(theme, path_label(&scope.item_path)))
                        .child(widgets::meta_line(theme, vec![fragment(theme, detail)])),
                );
            if kind == ScopeKind::Conflict {
                row = row.child(self.render_choices(
                    theme,
                    false,
                    index,
                    review.scope_choices[index],
                    &[Choice::Skip, Choice::Append, Choice::Replace],
                    cx,
                ));
            } else {
                row = row.child(widgets::badge_active(theme, "New"));
            }
            card = card.child(row);
            first = false;
        }
        for (index, template) in review.preview.templates.iter().enumerate() {
            let conflict = template.current.is_some();
            let mut row = widgets::card_row(theme, first)
                .child(widgets::row_tile(theme, icons::CHECKLIST))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(widgets::row_title(theme, format!("Template: {}", template.name)))
                        .child(widgets::meta_line(
                            theme,
                            vec![fragment(
                                theme,
                                if conflict {
                                    "Conflict: you have a template with this name"
                                } else {
                                    "New template"
                                },
                            )],
                        )),
                );
            if conflict {
                row = row.child(self.render_choices(
                    theme,
                    true,
                    index,
                    review.template_choices[index],
                    &[Choice::Skip, Choice::Replace],
                    cx,
                ));
            } else {
                row = row.child(widgets::badge_active(theme, "New"));
            }
            card = card.child(row);
            first = false;
        }
        let changes = review.change_count();
        if first {
            card = card.child(
                widgets::card_row(theme, true).child(widgets::row_title(
                    theme,
                    "Nothing to import: this Space already has everything in the pack.",
                )),
            );
        }
        let apply_label = format!("Apply {}", count_label(changes, "change", "changes"));
        let apply = popover::btn_primary(theme, &apply_label)
            .id("workflow-pack-apply")
            .when(changes == 0, |el| el.opacity(0.4))
            .on_click(cx.listener(|this, _, _, cx| this.apply(cx)));
        let cancel = crate::components::settings_compact_action(theme, "Cancel", "workflow-pack-cancel")
            .on_click(cx.listener(|this, _, _, cx| {
                this.stage = Stage::Idle;
                cx.notify();
            }));
        div()
            .child(card)
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.0))
                    .child(cancel)
                    .child(apply),
            )
            .into_any_element()
    }

    fn render_export(&self, theme: &Theme, review: &ExportReview, cx: &mut Context<Self>) -> AnyElement {
        let redaction = if review.redactions == 0 {
            "No credentials or personal details were detected. Detection is not complete: read the file before you share it.".to_string()
        } else {
            format!(
                "{} replaced with a placeholder. Detection is not complete: read the file before you share it.",
                count_label(review.redactions, "value was", "values were")
            )
        };
        let save = popover::btn_primary(theme, "Save…")
            .id("workflow-pack-save")
            .on_click(cx.listener(|this, _, _, cx| this.save_export(cx)));
        let cancel = crate::components::settings_compact_action(theme, "Cancel", "workflow-pack-cancel")
            .on_click(cx.listener(|this, _, _, cx| {
                this.stage = Stage::Idle;
                cx.notify();
            }));
        div()
            .child(
                widgets::section_card(theme).child(
                    widgets::card_row(theme, true)
                        .child(widgets::row_tile(theme, icons::DOCUMENT_ADD))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(widgets::row_title(
                                    theme,
                                    format!(
                                        "{} · {}",
                                        count_label(review.scopes, "instruction scope", "instruction scopes"),
                                        count_label(review.templates, "template", "templates")
                                    ),
                                ))
                                .child(widgets::meta_line(theme, vec![fragment(theme, redaction)])),
                        ),
                ),
            )
            .when(review.share, |el| {
                el.child(widgets::warning_strip(
                    theme,
                    "Share mode also replaces email addresses and home folder paths.",
                ))
            })
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.0))
                    .child(cancel)
                    .child(save),
            )
            .into_any_element()
    }

    fn render_receipts(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let now = chrono::Utc::now();
        let mut card = widgets::section_card(theme);
        if self.receipts.is_empty() {
            card = card.child(widgets::card_row(theme, true).child(widgets::row_title(
                theme,
                "No imports yet",
            )));
        }
        for (index, receipt) in self.receipts.iter().enumerate() {
            let when = chrono::DateTime::from_timestamp_millis(receipt.created_at)
                .map(|created| crate::state::format_time_ago(created, now))
                .unwrap_or_default();
            let summary = receipt.summary.describe();
            let id = receipt.id.clone();
            let mut row = widgets::card_row(theme, index == 0)
                .child(widgets::row_tile(theme, icons::ARCHIVE_UP_MINIMALISTIC))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(widgets::row_title(theme, format!("Imported {when} ago")))
                        .child(widgets::meta_line(theme, vec![fragment(theme, summary)])),
                );
            row = if receipt.undone_at.is_some() {
                row.child(widgets::badge(theme, "Undone"))
            } else {
                row.child(
                    crate::components::settings_compact_action(
                        theme,
                        "Undo",
                        format!("workflow-pack-undo-{index}"),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.undo(id.clone(), cx))),
                )
            };
            card = card.child(row);
        }
        card.into_any_element()
    }
}

impl Render for WorkflowPackPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let space = self.space_name(cx);
        let has_space = self.space_id(cx).is_some();
        // Starting another export or import just replaces an open review.
        let idle = !matches!(self.stage, Stage::Working(_));
        let subtitle = match &space {
            Some(name) => format!(
                "Move the instructions and project templates of {name} to another Space. Imports are previewed first and can be undone."
            ),
            None => "Select a Space in the sidebar to export or import its workflow.".to_string(),
        };

        let actions_enabled = has_space && idle;
        let export_actions = vec![
            crate::components::settings_compact_action(&theme, "Export for sharing", "workflow-pack-export-share")
                .when(!actions_enabled, |el| el.opacity(0.4))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if actions_enabled {
                        this.start_export(true, cx);
                    }
                }))
                .into_any_element(),
            crate::components::settings_compact_action(&theme, "Export all", "workflow-pack-export-full")
                .when(!actions_enabled, |el| el.opacity(0.4))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if actions_enabled {
                        this.start_export(false, cx);
                    }
                }))
                .into_any_element(),
        ];
        let import_actions = vec![
            crate::components::settings_compact_action(&theme, "Choose file…", "workflow-pack-import")
                .when(!actions_enabled, |el| el.opacity(0.4))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if actions_enabled {
                        this.choose_import_file(cx);
                    }
                }))
                .into_any_element(),
        ];
        let start = widgets::section_card(&theme)
            .child(self.action_row(
                &theme,
                "export",
                icons::DOCUMENT_ADD,
                "Export",
                "Write this Space's instructions and templates to a file".into(),
                export_actions,
            ))
            .child(self.action_row(
                &theme,
                "import",
                icons::ARCHIVE_UP_MINIMALISTIC,
                "Import",
                "Preview a pack file before anything changes".into(),
                import_actions,
            ));

        let stage: Option<AnyElement> = match self.stage.clone() {
            Stage::Idle => None,
            Stage::Working(text) => Some(
                widgets::section_card(&theme)
                    .child(widgets::card_row(&theme, true).child(widgets::row_title(&theme, text)))
                    .into_any_element(),
            ),
            Stage::Export(review) => Some(self.render_export(&theme, &review, cx)),
            Stage::Import(review) => Some(self.render_import(&theme, &review, cx)),
        };
        let receipts = self.render_receipts(&theme, cx);

        let scrollbar = popover::rail(self, "workflow-pack-scrollbar", &theme, cx);
        div()
            .id("workflow-pack-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                div()
                    .id("workflow-pack-page")
                    .debug_selector(|| "workflow-pack-page".into())
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll.scroll)
                    .child(
                        widgets::page_column()
                            .child(widgets::page_header(&theme, "Workflow pack", None))
                            .child(widgets::page_subtitle(&theme, subtitle))
                            .when_some(self.error.clone(), |el, message| {
                                el.child(
                                    widgets::error_strip(&theme, message)
                                        .id("workflow-pack-error")
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.error = None;
                                            cx.notify();
                                        })),
                                )
                            })
                            .when_some(self.notice.clone(), |el, message| {
                                el.child(widgets::warning_strip(&theme, message))
                            })
                            .child(start)
                            .children(stage)
                            .child(widgets::field_label(&theme, "Recent imports").mt(px(32.0)))
                            .child(receipts),
                    ),
            )
            .children(scrollbar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(path: &[&str], current: &[&str], incoming: &[&str]) -> WorkflowPackScopePreview {
        WorkflowPackScopePreview {
            item_path: path.iter().map(|part| part.to_string()).collect(),
            work_item_id: None,
            missing_from: None,
            current: current.iter().map(|text| text.to_string()).collect(),
            incoming: incoming.iter().map(|text| text.to_string()).collect(),
        }
    }

    fn review(scopes: Vec<WorkflowPackScopePreview>) -> ImportReview {
        ImportReview::new(ImportPreview { preview_id: "p".into(), scopes, templates: vec![] })
    }

    #[test]
    fn scopes_are_classified_against_the_destination() {
        assert_eq!(scope_kind(&scope(&[], &[], &[])), ScopeKind::Unchanged);
        assert_eq!(scope_kind(&scope(&[], &["a"], &["a"])), ScopeKind::Unchanged);
        assert_eq!(scope_kind(&scope(&[], &[], &["a"])), ScopeKind::New);
        assert_eq!(scope_kind(&scope(&[], &["b"], &["a"])), ScopeKind::Conflict);
    }

    #[test]
    fn conflicts_default_to_keeping_the_destination_and_need_a_choice_to_change() {
        let mut review = review(vec![scope(&[], &["mine"], &["theirs"]), scope(&["Tasks"], &[], &["new"])]);
        assert_eq!(review.change_count(), 1, "only the new scope applies by default");
        review.scope_choices[0] = Choice::Replace;
        assert_eq!(review.change_count(), 2);
        let resolutions = review.resolutions();
        assert_eq!(
            resolutions["scopes"],
            json!([{ "itemPath": [], "resolution": "replace" }])
        );
        assert_eq!(resolutions["templates"], json!([]));
    }

    #[test]
    fn receipt_summaries_read_in_plain_words() {
        let summary = ReceiptSummary {
            items_created: 1,
            instructions_added: 2,
            instructions_archived: 0,
            templates_added: 1,
            templates_replaced: 0,
        };
        assert_eq!(summary.describe(), "2 instructions added · 1 template added · 1 item created");
    }
}
