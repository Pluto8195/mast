use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileOrigin {
    pub(super) repository_relative_path: String,
    pub(super) repository_path: Option<String>,
    pub(super) nesting_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SubmoduleGroup {
    pub(super) repository_path: String,
    pub(super) parent_repository_path: Option<String>,
    pub(super) old_revision: Option<String>,
    pub(super) new_revision: Option<String>,
    pub(super) additions: u32,
    pub(super) deletions: u32,
    pub(super) file_count: usize,
    pub(super) truncated: bool,
    /// The parent gitlink's post-change SHA carried Git's `-dirty` suffix.
    /// Expanded children replace that SHA row, so the group must retain it.
    pub(super) dirty: bool,
    pub(super) nesting_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DiffTreeNode {
    File(u32),
    Submodule {
        group: u32,
        children: Vec<DiffTreeNode>,
    },
}

pub(super) fn nested_display_path(repository_path: &str, path: &str) -> String {
    if path.is_empty() {
        repository_path.to_string()
    } else {
        format!(
            "{}/{}",
            repository_path.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

pub(super) fn submodule_local_path<'a>(
    repository_path: &'a str,
    parent_repository_path: Option<&str>,
) -> Option<&'a str> {
    match parent_repository_path {
        None => Some(repository_path),
        Some(parent) => repository_path.strip_prefix(parent)?.strip_prefix('/'),
    }
}

pub(super) fn gitlink_has_dirty_marker(file: &FileDiff) -> bool {
    file.hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .any(|line| line.kind == LineKind::Add && line.text.trim_end().ends_with("-dirty"))
}

/// Parse root and nested patches into one file arena plus an ordered tree.
/// Expanded group nodes replace their owning gitlink file; unavailable
/// sections are deliberately ignored so the root SHA-only file remains.
pub(super) fn parse_diff_tree(
    root_patch: &str,
    submodules: &[SubmoduleDiffSummary],
) -> (
    Vec<FileDiff>,
    Vec<FileOrigin>,
    Vec<SubmoduleGroup>,
    Vec<DiffTreeNode>,
) {
    let mut files = parse_patch(root_patch);
    let mut origins = files
        .iter()
        .map(|file| FileOrigin {
            repository_relative_path: file.path.clone(),
            repository_path: None,
            nesting_depth: 0,
        })
        .collect::<Vec<_>>();
    let root_files = (0..files.len() as u32).collect::<Vec<_>>();
    let mut repository_files: HashMap<String, Vec<u32>> = HashMap::new();
    let mut groups = Vec::new();

    for section in submodules.iter().filter(|section| section.expanded) {
        let section_files = parse_patch(&section.patch);
        // An expanded section without renderable children must not replace
        // the root/parent gitlink. This occurs when a combined payload is
        // truncated before the nested patch arrives; the SHA transition is
        // still the only useful fallback in that case.
        if section_files.is_empty() {
            continue;
        }
        let depth = {
            let mut depth = 0usize;
            let mut parent = section.parent_repository_path.as_deref();
            let mut seen = std::collections::HashSet::new();
            while let Some(path) = parent {
                if !seen.insert(path) || depth >= 16 {
                    break;
                }
                depth += 1;
                parent = submodules
                    .iter()
                    .find(|candidate| candidate.repository_path == path)
                    .and_then(|candidate| candidate.parent_repository_path.as_deref());
            }
            depth
        };
        let group_ix = groups.len() as u32;
        groups.push(SubmoduleGroup {
            repository_path: section.repository_path.clone(),
            parent_repository_path: section.parent_repository_path.clone(),
            old_revision: section.old_revision.clone(),
            new_revision: section.new_revision.clone(),
            additions: section.additions,
            deletions: section.deletions,
            file_count: section.files.len(),
            truncated: section.truncated,
            dirty: false,
            nesting_depth: depth,
        });

        let mut indices = Vec::with_capacity(section_files.len());
        for mut file in section_files {
            let repository_relative_path = file.path.clone();
            file.path = nested_display_path(&section.repository_path, &file.path);
            file.old_path = file
                .old_path
                .take()
                .map(|path| nested_display_path(&section.repository_path, &path));
            indices.push(files.len() as u32);
            files.push(file);
            origins.push(FileOrigin {
                repository_relative_path,
                repository_path: Some(section.repository_path.clone()),
                nesting_depth: depth + 1,
            });
        }
        repository_files.insert(section.repository_path.clone(), indices);
        debug_assert_eq!(group_ix as usize + 1, groups.len());
    }

    // Expanded groups replace their parent gitlink rows. Preserve the one
    // extra state encoded only in those rows: Git's post-change `-dirty`
    // suffix. This works recursively because nested section files share the
    // same arena and carry their owning repository in `origins`.
    for group in &mut groups {
        let local_path = submodule_local_path(
            &group.repository_path,
            group.parent_repository_path.as_deref(),
        );
        let owner_files = match group.parent_repository_path.as_deref() {
            None => Some(root_files.as_slice()),
            Some(parent) => repository_files.get(parent).map(Vec::as_slice),
        };
        group.dirty = local_path
            .zip(owner_files)
            .and_then(|(local_path, owner_files)| {
                owner_files.iter().find_map(|file_ix| {
                    let ix = *file_ix as usize;
                    (origins.get(ix)?.repository_relative_path == local_path)
                        .then(|| files.get(ix))
                        .flatten()
                })
            })
            .is_some_and(gitlink_has_dirty_marker);
    }

    fn build_repository(
        parent: Option<&str>,
        file_indices: &[u32],
        origins: &[FileOrigin],
        groups: &[SubmoduleGroup],
        repository_files: &HashMap<String, Vec<u32>>,
        ancestors: &mut Vec<String>,
    ) -> Vec<DiffTreeNode> {
        let candidate_groups = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| group.parent_repository_path.as_deref() == parent)
            .filter_map(|(ix, group)| {
                submodule_local_path(&group.repository_path, parent)
                    .map(|local| (local.to_string(), ix))
            })
            .collect::<Vec<_>>();
        let mut used = std::collections::HashSet::new();
        let mut nodes = Vec::new();
        for &file_ix in file_indices {
            let Some(origin) = origins.get(file_ix as usize) else {
                continue;
            };
            let Some(group_ix) = candidate_groups.iter().find_map(|(path, group_ix)| {
                (path == &origin.repository_relative_path).then_some(*group_ix)
            }) else {
                nodes.push(DiffTreeNode::File(file_ix));
                continue;
            };
            let group = &groups[group_ix];
            if ancestors.contains(&group.repository_path) {
                nodes.push(DiffTreeNode::File(file_ix));
                continue;
            }
            used.insert(group_ix);
            ancestors.push(group.repository_path.clone());
            let children = build_repository(
                Some(&group.repository_path),
                repository_files
                    .get(&group.repository_path)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                origins,
                groups,
                repository_files,
                ancestors,
            );
            ancestors.pop();
            nodes.push(DiffTreeNode::Submodule {
                group: group_ix as u32,
                children,
            });
        }
        // A truncated parent patch can omit the gitlink header while the
        // structured section still arrived. Keep that section reachable.
        for (_, group_ix) in candidate_groups {
            if used.contains(&group_ix) {
                continue;
            }
            let group = &groups[group_ix];
            if ancestors.contains(&group.repository_path) {
                continue;
            }
            ancestors.push(group.repository_path.clone());
            let children = build_repository(
                Some(&group.repository_path),
                repository_files
                    .get(&group.repository_path)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                origins,
                groups,
                repository_files,
                ancestors,
            );
            ancestors.pop();
            nodes.push(DiffTreeNode::Submodule {
                group: group_ix as u32,
                children,
            });
        }
        nodes
    }

    let layout = build_repository(
        None,
        &root_files,
        &origins,
        &groups,
        &repository_files,
        &mut Vec::new(),
    );
    (files, origins, groups, layout)
}

pub(super) fn logical_file_count(layout: &[DiffTreeNode]) -> usize {
    layout
        .iter()
        .map(|node| match node {
            DiffTreeNode::File(_) => 1,
            DiffTreeNode::Submodule { children, .. } => logical_file_count(children),
        })
        .sum()
}

/// Count only files present in the display tree; expanded submodules replace
/// their parent gitlink in that tree but leave it in the file arena.
pub(super) fn logical_hunk_count(layout: &[DiffTreeNode], files: &[FileDiff]) -> usize {
    layout
        .iter()
        .map(|node| match node {
            DiffTreeNode::File(index) => files
                .get(*index as usize)
                .map_or(0, |file| file.hunks.len()),
            DiffTreeNode::Submodule { children, .. } => logical_hunk_count(children, files),
        })
        .sum()
}

/// Generated dependency snapshots are usually reviewed through their source manifests.
pub(super) fn is_generated_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "Cargo.lock"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "bun.lock"
            | "bun.lockb"
            | "Gemfile.lock"
            | "Pipfile.lock"
            | "poetry.lock"
            | "uv.lock"
            | "composer.lock"
            | "go.sum"
            | "Podfile.lock"
            | "mix.lock"
    ) || name.ends_with(".min.js")
        || name.ends_with(".min.css")
}

/// Keep very large file bodies out of the virtual list until requested.
pub(super) fn is_large_file(file: &FileDiff) -> bool {
    const MAX_EXPANDED_LINES: usize = 2_000;
    file.hunks
        .iter()
        .map(|hunk| hunk.lines.len())
        .sum::<usize>()
        > MAX_EXPANDED_LINES
}

#[cfg(test)]
mod tests {
    use super::{DiffLine, FileDiff, Hunk, LineKind, is_generated_file, is_large_file};

    #[test]
    fn hunk_count_excludes_replaced_gitlinks() {
        let mut gitlink = FileDiff::new("sub".into(), None);
        gitlink.hunks.push(Hunk {
            header: "@@".into(),
            lines: Vec::new(),
        });
        let mut nested = FileDiff::new("sub/a".into(), None);
        nested.hunks.extend([
            Hunk {
                header: "@@".into(),
                lines: Vec::new(),
            },
            Hunk {
                header: "@@".into(),
                lines: Vec::new(),
            },
        ]);
        let layout = vec![super::DiffTreeNode::Submodule {
            group: 0,
            children: vec![super::DiffTreeNode::File(1)],
        }];
        assert_eq!(super::logical_hunk_count(&layout, &[gitlink, nested]), 2);
    }

    #[test]
    fn large_file_collapses_only_above_the_limit() {
        let line = DiffLine {
            kind: LineKind::Add,
            old_no: None,
            new_no: Some(1),
            text: "x".into(),
        };
        let mut file = FileDiff::new("src/a.rs".into(), None);
        file.hunks.push(Hunk {
            header: "@@".into(),
            lines: vec![line.clone(); 2_000],
        });
        assert!(!is_large_file(&file));
        file.hunks[0].lines.push(line);
        assert!(is_large_file(&file));
    }

    #[test]
    fn compact_folders_collapse_and_restore_descendants() {
        let files = vec![
            FileDiff::new("src/hooks/nested/a.rs".into(), None),
            FileDiff::new("src/hooks/nested/b.rs".into(), None),
            FileDiff::new("src/other.rs".into(), None),
        ];
        let mut collapsed = std::collections::HashSet::new();
        let (rows, folders) = super::file_tree_rows(&files, &[0, 1, 2], &collapsed);
        assert!(rows.iter().any(
            |row| matches!(row, super::FileTreeRow::Folder { label, .. } if label == "hooks/nested")
        ));
        assert!(folders.contains(&"src/hooks/nested/".to_string()));
        collapsed.insert("src/hooks/nested/".into());
        let (rows, _) = super::file_tree_rows(&files, &[0, 1, 2], &collapsed);
        assert!(rows.iter().any(|row| matches!(
            row,
            super::FileTreeRow::Folder {
                count: 2,
                collapsed: true,
                ..
            }
        )));
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row, super::FileTreeRow::File { index: 0 | 1, .. }))
        );
        collapsed.clear();
        let (rows, _) = super::file_tree_rows(&files, &[0, 1, 2], &collapsed);
        assert!(
            rows.iter()
                .any(|row| matches!(row, super::FileTreeRow::File { index: 0, .. }))
        );
    }

    #[test]
    fn generated_file_detection_uses_file_names() {
        assert!(is_generated_file("nested/Cargo.lock"));
        assert!(is_generated_file("web/app.min.js"));
        assert!(!is_generated_file("src/lock.rs"));
        assert!(!is_generated_file("Cargo.toml"));
    }
}

fn ordered_file_indices(nodes: &[DiffTreeNode], output: &mut Vec<usize>) {
    for node in nodes {
        match node {
            DiffTreeNode::File(index) => output.push(*index as usize),
            DiffTreeNode::Submodule { children, .. } => ordered_file_indices(children, output),
        }
    }
}

fn file_matches_filter(file: &FileDiff, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || file.path.to_lowercase().contains(&query)
        || file
            .old_path
            .as_ref()
            .is_some_and(|path| path.to_lowercase().contains(&query))
}

pub(super) struct FileListResize;
struct FileListResizeGhost;
impl Render for FileListResizeGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

#[derive(Clone)]
enum FileTreeRow {
    Folder {
        path: String,
        label: String,
        depth: usize,
        count: usize,
        collapsed: bool,
    },
    File {
        index: usize,
        depth: usize,
    },
}

fn file_tree_rows(
    files: &[FileDiff],
    indices: &[usize],
    collapsed: &std::collections::HashSet<String>,
) -> (Vec<FileTreeRow>, Vec<String>) {
    fn visit(
        files: &[FileDiff],
        indices: &[usize],
        prefix: &str,
        depth: usize,
        collapsed: &std::collections::HashSet<String>,
        rows: &mut Vec<FileTreeRow>,
        folders: &mut Vec<String>,
        visible: bool,
    ) {
        let mut groups: std::collections::BTreeMap<String, Vec<usize>> =
            std::collections::BTreeMap::new();
        for &index in indices {
            let relative = files[index]
                .path
                .strip_prefix(prefix)
                .unwrap_or(&files[index].path);
            if let Some((directory, _)) = relative.split_once('/') {
                groups.entry(directory.to_owned()).or_default().push(index);
            } else if visible {
                rows.push(FileTreeRow::File { index, depth });
            }
        }
        for (directory, members) in groups {
            let mut path = format!("{prefix}{directory}/");
            let mut label = directory;
            loop {
                let mut next = None;
                let mut can_compact = true;
                for &index in &members {
                    let relative = files[index].path.strip_prefix(&path).unwrap_or("");
                    let Some((part, _)) = relative.split_once('/') else {
                        can_compact = false;
                        break;
                    };
                    if next.as_deref().is_some_and(|previous| previous != part) {
                        can_compact = false;
                        break;
                    }
                    next = Some(part.to_owned());
                }
                if !can_compact {
                    break;
                }
                let Some(part) = next else {
                    break;
                };
                label.push('/');
                label.push_str(&part);
                path.push_str(&part);
                path.push('/');
            }
            let is_collapsed = collapsed.contains(&path);
            folders.push(path.clone());
            if visible {
                rows.push(FileTreeRow::Folder {
                    path: path.clone(),
                    label,
                    depth,
                    count: members.len(),
                    collapsed: is_collapsed,
                });
            }
            visit(
                files,
                &members,
                &path,
                depth + 1,
                collapsed,
                rows,
                folders,
                visible && !is_collapsed,
            );
        }
    }
    let mut rows = Vec::new();
    let mut folders = Vec::new();
    visit(
        files,
        indices,
        "",
        0,
        collapsed,
        &mut rows,
        &mut folders,
        true,
    );
    (rows, folders)
}

impl Changes {
    fn file_tree_key(&self) -> String {
        let owner = self.owner.as_ref().map(|owner| (&owner.chat_id, &owner.device_id, &owner.cwd, &owner.checkout_id, &owner.range));
        format!("{owner:?}:{:?}:{:?}:{:?}", self.pinned_cwd, self.scope, self.base_ref)
    }

    #[cfg(feature = "snapshot")]
    pub(crate) fn collapse_snapshot_file_tree(&mut self, cx: &mut Context<Self>) {
        let key = self.file_tree_key();
        self.file_tree_collapsed
            .entry(key)
            .or_default()
            .insert("src/".into());
        cx.notify();
    }

    pub(super) fn jump_to_file(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(file) = self
            .parsed
            .as_ref()
            .and_then(|parsed| parsed.files.get(index))
        else {
            return;
        };
        let mut needs_reflatten = false;
        if let Some(fold) = self.folds.get_mut(&file.path) {
            needs_reflatten = fold.collapsed || fold.toggled_at.is_some();
            fold.collapsed = false;
            fold.toggled_at = None;
        }
        needs_reflatten |= self.submodule_folds.values().any(|collapsed| *collapsed);
        self.submodule_folds.clear();
        if needs_reflatten {
            self.reflatten(cx);
        }
        if let Some(start) = self
            .row_ranges
            .get(index)
            .and_then(|range| range.as_ref())
            .map(|range| range.start)
        {
            self.list.scroll_to(gpui::ListOffset {
                item_ix: start,
                offset_in_item: px(0.0),
            });
        }
        cx.notify();
    }

    pub(super) fn jump_to_hunk(&mut self, file: usize, hunk: usize, cx: &mut Context<Self>) {
        self.jump_to_file(file, cx);
        if let Some(row) = self.rows.iter().position(|row| {
            *row == DiffRow::HunkHeader {
                file: file as u32,
                hunk: hunk as u32,
            }
        }) {
            self.list.scroll_to(gpui::ListOffset {
                item_ix: row,
                offset_in_item: px(0.0),
            });
        }
    }

    pub(super) fn render_file_list(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(parsed) = self.parsed.as_ref() else {
            return div().into_any_element();
        };
        let query = self.file_filter.read(cx).text().to_string();
        let mut indices = Vec::new();
        ordered_file_indices(&parsed.layout, &mut indices);
        let files = parsed.files.clone();
        let identities = parsed.hunk_identities.clone();
        let review_state = self.review_state.clone();
        let indices = indices
            .into_iter()
            .filter(|index| {
                files
                    .get(*index)
                    .is_some_and(|file| file_matches_filter(file, &query))
            })
            .collect::<Vec<_>>();
        let tree_key = self.file_tree_key();
        let collapsed = self
            .file_tree_collapsed
            .get(&tree_key)
            .cloned()
            .unwrap_or_default();
        let (tree_rows, folders) = file_tree_rows(&files, &indices, &collapsed);
        let rows_key = tree_key.clone();
        let collapse_key = tree_key.clone();
        let expand_key = tree_key.clone();
        let count = tree_rows.len();
        let entity = cx.entity();
        let row_theme = theme.clone();
        let entries =
            gpui::uniform_list("changes-file-list-entries", count, move |range, _, app| {
                entity.update(app, |_this, cx| {
                    let theme = &row_theme;
                    range
                        .filter_map(|position| tree_rows.get(position).cloned())
                        .map(|row| {
                            let (index, depth) = match row {
                                FileTreeRow::File { index, depth } => (index, depth),
                                FileTreeRow::Folder {
                                    path,
                                    label,
                                    depth,
                                    count,
                                    collapsed,
                                } => {
                                    let selector = format!("changes-folder-{path}");
                                    let toggle_path = path.clone();
                                    let toggle_key = rows_key.clone();
                                    return div()
                                        .id(SharedString::from(selector.clone()))
                                        .debug_selector(move || selector.clone())
                                        .flex()
                                        .items_center()
                                        .gap(px(5.0))
                                        .pl(px(6.0 + depth as f32 * 14.0))
                                        .pr(px(6.0))
                                        .py(px(4.0))
                                        .cursor_pointer()
                                        .when(true, |element| {
                                            crate::components::changes_file_row(element, theme)
                                        })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.file_tree_focused_path = Some(toggle_path.clone());
                                            window.focus(&this.file_tree_focus, cx);
                                            let collapsed = this
                                                .file_tree_collapsed
                                                .entry(toggle_key.clone())
                                                .or_default();
                                            if !collapsed.remove(&toggle_path) {
                                                collapsed.insert(toggle_path.clone());
                                            }
                                            cx.notify();
                                        }))
                                        .child(if collapsed { "▸" } else { "▾" })
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_size(px(11.0))
                                                .child(SharedString::from(label)),
                                        )
                                        .children(collapsed.then(|| {
                                            div()
                                                .text_size(px(10.0))
                                                .text_color(theme.text_muted)
                                                .child(SharedString::from(count.to_string()))
                                        }))
                                        .into_any_element();
                                }
                            };
                            let file = &files[index];
                            let viewed = review_state.as_ref().is_some_and(|state| {
                                if file.hunks.is_empty() {
                                    state.viewed_files.contains(&file.path)
                                } else {
                                    identities[index]
                                        .iter()
                                        .all(|identity| state.viewed_hunks.contains(identity))
                                }
                            });
                            let label = match &file.old_path {
                                Some(old) if old != &file.path => format!("{old} → {}", file.path),
                                _ => file
                                    .path
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or(&file.path)
                                    .to_owned(),
                            };
                            div()
                                .id(("changes-file-list-entry", index))
                                .debug_selector(move || format!("changes-file-list-entry-{index}"))
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .pl(px(6.0 + depth as f32 * 14.0))
                                .pr(px(6.0))
                                .py(px(4.0))
                                .cursor_pointer()
                                .when(true, |element| {
                                    crate::components::changes_file_row(element, theme)
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.jump_to_file(index, cx);
                                    window.focus(&this.diff_focus, cx);
                                }))
                                .child(
                                    div()
                                        .id(("changes-file-viewed", index))
                                        .cursor_pointer()
                                        .text_color(theme.text_muted)
                                        .when(true, |el| {
                                            crate::components::review_text_action(el, theme)
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.toggle_review_file(index, cx);
                                        }))
                                        .child(if review_state.is_none() {
                                            "–"
                                        } else if viewed {
                                            "☑"
                                        } else {
                                            "□"
                                        }),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(11.0))
                                        .text_color(theme.text_dim)
                                        .child(SharedString::from(label)),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(add_color(theme))
                                        .child(SharedString::from(format!("+{}", file.additions))),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(del_color(theme))
                                        .child(SharedString::from(format!("−{}", file.deletions))),
                                )
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                })
            })
            .flex_1()
            .min_h_0();
        div()
            .id("changes-file-list")
            .w(px(self.file_list_width))
            .relative()
            .track_focus(&self.file_tree_focus)
            .on_key_down(
                cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    if !this.file_tree_focus.is_focused(window) {
                        return;
                    }
                    if let Some(path) = this.file_tree_focused_path.clone() {
                        let collapsed = this
                            .file_tree_collapsed
                            .entry(tree_key.clone())
                            .or_default();
                        match event.keystroke.key.as_str() {
                            "left" => {
                                collapsed.insert(path);
                                cx.stop_propagation();
                                cx.notify();
                            }
                            "right" => {
                                collapsed.remove(&path);
                                cx.stop_propagation();
                                cx.notify();
                            }
                            _ => {}
                        }
                    }
                }),
            )
            .flex_none()
            .min_h_0()
            .flex()
            .flex_col()
            .when(!crate::components::is_horizon(theme), |element| {
                element.border_r_1().border_color(theme.border)
            })
            .when(true, |element| {
                crate::components::changes_file_list(element, theme)
            })
            .child(popover::search_input_frame(
                theme,
                self.file_filter.clone().into_any_element(),
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .px(px(6.0))
                    .child(
                        div()
                            .id("changes-folders-collapse")
                            .cursor_pointer()
                            .child("Collapse all")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.file_tree_collapsed
                                    .entry(collapse_key.clone())
                                    .or_default()
                                    .extend(folders.iter().cloned());
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("changes-folders-expand")
                            .cursor_pointer()
                            .child("Expand all")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.file_tree_collapsed.remove(&expand_key);
                                cx.notify();
                            })),
                    ),
            )
            .child(entries)
            .child(
                div()
                    .id("changes-file-list-resize")
                    .debug_selector(|| "changes-file-list-resize".to_string())
                    .absolute()
                    .right(px(-3.0))
                    .top_0()
                    .bottom_0()
                    .w(px(6.0))
                    .cursor_col_resize()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            this.file_list_resize_anchor =
                                Some((f32::from(event.position.x), this.file_list_width));
                            cx.stop_propagation();
                        }),
                    )
                    .on_drag(FileListResize, |_, _, _, cx| {
                        cx.new(|_| FileListResizeGhost)
                    })
                    .on_mouse_up(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, _| this.file_list_resize_anchor = None),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    #[test]
    fn filter_matches_both_rename_paths_case_insensitively() {
        let mut file = FileDiff::new("src/new.rs".into(), Some("src/old.rs".into()));
        file.status = FileStatus::Renamed;
        assert!(file_matches_filter(&file, "OLD"));
        assert!(file_matches_filter(&file, "new.rs"));
        assert!(!file_matches_filter(&file, "missing"));
    }
}

impl Changes {
    pub(super) fn diff_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if !self.diff_focus.is_focused(window)
            || modifiers.platform
            || modifiers.control
            || modifiers.alt
        {
            return;
        }
        if event.keystroke.key == "v" && !modifiers.shift && !self.review_walkthrough {
            let top = self
                .list
                .logical_scroll_top()
                .item_ix
                .min(self.rows.len().saturating_sub(1));
            let current_file = self.rows.get(top).and_then(|row| row.file());
            let current_hunk = self
                .rows
                .iter()
                .take(top + 1)
                .rev()
                .find_map(|row| match row {
                    DiffRow::HunkHeader { file, hunk } if Some(*file as usize) == current_file => {
                        Some((*file as usize, *hunk as usize))
                    }
                    _ => None,
                });
            if let Some((file, hunk)) = current_hunk {
                self.toggle_review_hunk(file, hunk, cx);
            } else if let Some(file) = current_file {
                self.toggle_review_file(file, cx);
            }
            cx.stop_propagation();
            window.prevent_default();
            return;
        }
        let forward = match event.keystroke.key.as_str() {
            "j" => true,
            "k" => false,
            _ => return,
        };
        self.step_diff(forward, modifiers.shift, cx);
        cx.stop_propagation();
        window.prevent_default();
    }

    pub(super) fn step_diff(&mut self, forward: bool, files_only: bool, cx: &mut Context<Self>) {
        self.explanation_selection = None;
        self.explanation_unified_rows = None;
        self.explanation_hunk = None;
        self.cancel_explanation();
        let Some(parsed) = self.parsed.as_ref() else {
            return;
        };
        let mut files = Vec::new();
        ordered_file_indices(&parsed.layout, &mut files);
        let top = self
            .list
            .logical_scroll_top()
            .item_ix
            .min(self.rows.len().saturating_sub(1));
        let current_file = self.rows.get(top).and_then(|row| row.file());
        if files_only {
            let position =
                current_file.and_then(|file| files.iter().position(|index| *index == file));
            let target = match (position, forward) {
                (Some(index), true) => files.get(index + 1),
                (Some(index), false) => index.checked_sub(1).and_then(|index| files.get(index)),
                (None, true) => files.first(),
                (None, false) => files.last(),
            }
            .copied();
            if let Some(index) = target {
                self.jump_to_file(index, cx);
            }
            return;
        }
        let hunks: Vec<(usize, usize)> = files
            .iter()
            .flat_map(|index| (0..parsed.files[*index].hunks.len()).map(move |hunk| (*index, hunk)))
            .collect();
        let current_hunk = self
            .rows
            .iter()
            .take(top + 1)
            .rev()
            .find_map(|row| match row {
                DiffRow::HunkHeader { file, hunk } if Some(*file as usize) == current_file => {
                    Some((*file as usize, *hunk as usize))
                }
                DiffRow::FileHeader { .. } => None,
                _ => None,
            });
        let position =
            current_hunk.and_then(|item| hunks.iter().position(|candidate| *candidate == item));
        let target = match (position, forward) {
            (Some(index), true) => hunks.get(index + 1),
            (Some(index), false) => index.checked_sub(1).and_then(|index| hunks.get(index)),
            (None, true) => {
                match current_file.and_then(|file| files.iter().position(|index| *index == file)) {
                    Some(position) => files[position..]
                        .iter()
                        .find_map(|file| hunks.iter().find(|(index, _)| index == file)),
                    None => hunks.first(),
                }
            }
            (None, false) => {
                match current_file.and_then(|file| files.iter().position(|index| *index == file)) {
                    Some(position) => files[..position]
                        .iter()
                        .rev()
                        .find_map(|file| hunks.iter().rev().find(|(index, _)| index == file)),
                    None => hunks.last(),
                }
            }
        }
        .copied();
        if let Some((file, hunk)) = target {
            self.jump_to_file(file, cx);
            if let Some(row) = self.rows.iter().position(|row| {
                *row == DiffRow::HunkHeader {
                    file: file as u32,
                    hunk: hunk as u32,
                }
            }) {
                self.list.scroll_to(gpui::ListOffset {
                    item_ix: row,
                    offset_in_item: px(0.0),
                });
            }
        }
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    #[gpui::test]
    fn folder_row_toggles_files(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        let (changes, vcx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Changes::for_checkout(state, "/repo".into(), cx)
        });
        vcx.update(|_, cx| changes.update(cx, |changes, cx| {
            let patch = "diff --git a/src/hooks/a b/src/hooks/a\n--- a/src/hooks/a\n+++ b/src/hooks/a\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/src/hooks/b b/src/hooks/b\n--- a/src/hooks/b\n+++ b/src/hooks/b\n@@ -1 +1 @@\n-c\n+d\n";
            let (files, origins, submodules, layout) = parse_diff_tree(patch, &[]);
            let horizontal = files.iter().map(FileHorizontalState::new).collect();
            changes.scoped = Some(CheckoutDiff { checkout_id: "checkout".into(), device_id: "device".into(), cwd: "/repo".into(), patch: patch.into(), files: Vec::new(), submodules: Vec::new(), additions: 2, deletions: 2, truncated: false, checksum: "sum".into(), updated_at: chrono::Utc::now() });
            changes.parsed = Some(ParsedDiff { key: "test".into(), truncated: false, additions: 2, deletions: 2, file_count: 2, horizontal, hunk_identities: Arc::new(hunk_identities(&files)), files: Arc::new(files), origins: Arc::new(origins), submodules: Arc::new(submodules), layout: Arc::new(layout) });
            changes.reflatten(cx);
        }));
        vcx.update(|window, cx| window.draw(cx).clear());
        let folder = vcx
            .debug_bounds("changes-folder-src/hooks/")
            .unwrap()
            .center();
        assert!(vcx.debug_bounds("changes-file-list-entry-0").is_some());
        vcx.simulate_click(folder, gpui::Modifiers::default());
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("changes-file-list-entry-0").is_none());
        changes.update(vcx, |changes, _| changes.scope = DiffScope::LatestTurn);
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("changes-file-list-entry-0").is_some());
        changes.update(vcx, |changes, _| changes.scope = DiffScope::WorkingTree);
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("changes-file-list-entry-0").is_none());
        vcx.simulate_keystrokes("right");
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("changes-file-list-entry-0").is_some());
        vcx.simulate_keystrokes("left");
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("changes-file-list-entry-0").is_none());
        vcx.simulate_click(folder, gpui::Modifiers::default());
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("changes-file-list-entry-0").is_some());
        let start = vcx.debug_bounds("changes-file-list-resize").unwrap().center();
        let modifiers = gpui::Modifiers::default();
        let left = gpui::MouseButton::Left;
        vcx.simulate_mouse_down(start, left, modifiers);
        vcx.simulate_mouse_move(start + gpui::point(px(4.0), px(0.0)), Some(left), modifiers);
        vcx.simulate_mouse_move(start + gpui::point(px(450.0), px(0.0)), Some(left), modifiers);
        vcx.simulate_mouse_up(start + gpui::point(px(450.0), px(0.0)), left, modifiers);
        changes.read_with(vcx, |changes, _| assert_eq!(changes.file_list_width, 600.0));
    }

    #[gpui::test]
    fn file_list_filters_rename_paths_and_jumps(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        let (changes, vcx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Changes::for_checkout(state, "/repo".into(), cx)
        });
        vcx.update(|_, cx| changes.update(cx, |changes, cx| {
            let patch = "diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/other.rs b/other.rs\n--- a/other.rs\n+++ b/other.rs\n@@ -1 +1 @@\n-a\n+b\n";
            let (files, origins, submodules, layout) = parse_diff_tree(patch, &[]);
            let horizontal = files.iter().map(FileHorizontalState::new).collect();
            changes.scoped = Some(CheckoutDiff { checkout_id: "checkout".into(), device_id: "device".into(), cwd: "/repo".into(), patch: patch.into(), files: Vec::new(), submodules: Vec::new(), additions: 2, deletions: 2, truncated: false, checksum: "sum".into(), updated_at: chrono::Utc::now() });
            changes.parsed = Some(ParsedDiff { key: "test".into(), truncated: false, additions: 2, deletions: 2, file_count: 2, horizontal, hunk_identities: Arc::new(hunk_identities(&files)), files: Arc::new(files), origins: Arc::new(origins), submodules: Arc::new(submodules), layout: Arc::new(layout) });
            changes.reflatten(cx);
        }));
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("changes-file-list-entry-0").is_some());
        assert!(vcx.debug_bounds("changes-file-list-entry-1").is_some());
        let filter = changes.read_with(vcx, |changes, _| changes.file_filter.clone());
        filter.update(vcx, |input, cx| input.set_text("old.rs", cx));
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("changes-file-list-entry-0").is_some());
        assert!(vcx.debug_bounds("changes-file-list-entry-1").is_none());
        let row = vcx
            .debug_bounds("changes-file-list-entry-0")
            .unwrap()
            .center();
        vcx.simulate_click(row, gpui::Modifiers::default());
        changes.read_with(vcx, |changes, _| {
            assert!(matches!(
                changes.rows[changes.list.logical_scroll_top().item_ix],
                DiffRow::FileHeader { file: 0 }
            ))
        });
    }

    #[gpui::test]
    fn file_list_virtualizes_rows_and_filters_distant_files(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        let (changes, vcx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Changes::for_checkout(state, "/repo".into(), cx)
        });
        vcx.update(|_, cx| {
            changes.update(cx, |changes, cx| {
                let patch = crate::perf::data::unified_diff(300 * 210);
                let (files, origins, submodules, layout) = parse_diff_tree(&patch, &[]);
                let horizontal = files.iter().map(FileHorizontalState::new).collect();
                changes.scoped = Some(CheckoutDiff {
                    checkout_id: "checkout".into(),
                    device_id: "device".into(),
                    cwd: "/repo".into(),
                    patch,
                    files: Vec::new(),
                    submodules: Vec::new(),
                    additions: 0,
                    deletions: 0,
                    truncated: false,
                    checksum: "sum".into(),
                    updated_at: chrono::Utc::now(),
                });
                changes.parsed = Some(ParsedDiff {
                    key: "test".into(),
                    truncated: false,
                    additions: 0,
                    deletions: 0,
                    file_count: files.len(),
                    horizontal,
                    hunk_identities: Arc::new(hunk_identities(&files)),
                    files: Arc::new(files),
                    origins: Arc::new(origins),
                    submodules: Arc::new(submodules),
                    layout: Arc::new(layout),
                });
                changes.reflatten(cx);
            })
        });
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("changes-file-list-entry-0").is_some());
        assert!(vcx.debug_bounds("changes-file-list-entry-299").is_none());
        let filter = changes.read_with(vcx, |changes, _| changes.file_filter.clone());
        filter.update(vcx, |input, cx| input.set_text("mod300.rs", cx));
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("changes-file-list-entry-299").is_some());
    }

    #[gpui::test]
    fn navigation_moves_between_hunks_and_files(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        let (changes, vcx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Changes::new(state, cx)
        });
        vcx.update(|window, cx| changes.update(cx, |changes, cx| {
            let patch = "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n@@ -10 +10 @@\n-before\n+after\ndiff --git a/b b/b\n--- a/b\n+++ b/b\n@@ -1 +1 @@\n-x\n+y\n";
            let (files, origins, submodules, layout) = parse_diff_tree(patch, &[]);
            let horizontal = files.iter().map(FileHorizontalState::new).collect();
            changes.parsed = Some(ParsedDiff {
                key: "test".into(), truncated: false, additions: 3, deletions: 3, file_count: 2,
                horizontal, hunk_identities: Arc::new(hunk_identities(&files)), files: Arc::new(files), origins: Arc::new(origins),
                submodules: Arc::new(submodules), layout: Arc::new(layout),
            });
            changes.reflatten(cx);
            changes.review_state = Some(mast_proto::DiffReviewState { target: "test".into(), ..Default::default() });
            window.focus(&changes.diff_focus, cx);
            let key = |name| gpui::KeyDownEvent { keystroke: gpui::Keystroke::parse(name).unwrap(), is_held: false, prefer_character_input: false };
            changes.diff_key_down(&key("k"), window, cx);
            assert!(matches!(changes.rows[changes.list.logical_scroll_top().item_ix], DiffRow::FileHeader { file: 0 }));
            let rows = changes.rows.as_ptr();
            changes.diff_key_down(&key("j"), window, cx);
            assert_eq!(changes.rows.as_ptr(), rows, "an expanded target keeps the row model");
            assert!(matches!(changes.rows[changes.list.logical_scroll_top().item_ix], DiffRow::HunkHeader { file: 0, hunk: 0 }));
            changes.diff_key_down(&key("v"), window, cx);
            assert_eq!(changes.review_state.as_ref().unwrap().viewed_hunks.len(), 1);
            changes.diff_key_down(&key("j"), window, cx);
            assert!(matches!(changes.rows[changes.list.logical_scroll_top().item_ix], DiffRow::HunkHeader { file: 0, hunk: 1 }));
            changes.diff_key_down(&key("shift-j"), window, cx);
            assert!(matches!(changes.rows[changes.list.logical_scroll_top().item_ix], DiffRow::FileHeader { file: 1 }));
        }));
    }
}
