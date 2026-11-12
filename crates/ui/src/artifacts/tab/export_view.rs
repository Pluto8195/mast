//! Export the displayed revision without the embedded Mast bridge.
use super::{ArtifactBody, ArtifactTab, WebKind, document, model};
use crate::{popover, theme::Theme};
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, prelude::*, px};
use mast_proto::{ArtifactKind, ArtifactView};

#[derive(Clone, Copy)]
enum ExportFormat {
    Html,
    Pdf,
    Markdown,
}

fn formats(kind: ArtifactKind) -> &'static [(&'static str, ExportFormat)] {
    const VISUAL: &[(&str, ExportFormat)] =
        &[("HTML", ExportFormat::Html), ("PDF", ExportFormat::Pdf)];
    const STRUCTURED: &[(&str, ExportFormat)] = &[
        ("HTML", ExportFormat::Html),
        ("PDF", ExportFormat::Pdf),
        ("Copy as markdown", ExportFormat::Markdown),
    ];
    const MARKDOWN: &[(&str, ExportFormat)] = &[("Copy as markdown", ExportFormat::Markdown)];
    match kind {
        ArtifactKind::Plan | ArtifactKind::DecisionSheet => STRUCTURED,
        ArtifactKind::HtmlFragment
        | ArtifactKind::HtmlPage
        | ArtifactKind::Mermaid
        | ArtifactKind::Options => VISUAL,
        ArtifactKind::Markdown => MARKDOWN,
        _ => &[],
    }
}

fn suggested_name(title: &str, revision: u32, extension: &str) -> String {
    let name: String = title
        .chars()
        .map(|c| {
            if c == '/' || c == ':' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim().trim_matches('.');
    format!(
        "{} r{revision}.{extension}",
        if name.is_empty() { "Artifact" } else { name }
    )
}

fn safe_destination(path: &std::path::Path) -> bool {
    let Some(home) = std::env::var_os("HOME") else {
        return false;
    };
    let store = std::path::PathBuf::from(home).join(".zeron");
    let store = store.canonicalize().unwrap_or(store);
    let resolved = path
        .canonicalize()
        .ok()
        .or_else(|| Some(path.parent()?.canonicalize().ok()?.join(path.file_name()?)));
    resolved.is_some_and(|path| !path.starts_with(store))
}

impl ArtifactTab {
    pub(super) fn render_export_button(
        &self,
        artifact: &ArtifactView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let choices = formats(artifact.kind);
        if choices.is_empty() || self.body.is_none() {
            return None;
        }
        let mut trigger = crate::components::browser_parts::artifact_picker_button(
            "artifact-export",
            "Export".into(),
            self.export_open.is_open(),
            theme,
        )
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(|this, _, _, _| this.export_open.note_trigger_press()),
        )
        .on_click(cx.listener(|this, _, _, cx| {
            if this.export_open.take_press_was_open() {
                this.close_export_menu(cx);
            } else {
                this.export_open.open(());
            }
            cx.notify();
        }));
        if self.export_open.get().is_some() {
            let menu = popover::popover_card(theme)
                .id("artifact-export-menu")
                .w(px(190.0))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.close_export_menu(cx);
                }))
                .children(choices.iter().map(|(label, format)| {
                    let format = *format;
                    let id = match format {
                        ExportFormat::Html => "artifact-export-html",
                        ExportFormat::Pdf => "artifact-export-pdf",
                        ExportFormat::Markdown => "artifact-export-markdown",
                    };
                    popover::menu_row(theme, false, id)
                        .id(id)
                        .debug_selector(move || id.into())
                        .child(*label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start_export(format, window, cx)
                        }))
                }))
                .into_any_element();
            trigger = trigger.child(popover::anchored_menu_below_end(
                "artifact-export-popup",
                menu,
                self.export_open.closing_since(),
            ));
        }
        Some(trigger.into_any_element())
    }

    fn close_export_menu(&mut self, cx: &mut Context<Self>) {
        if self.export_open.begin_close() {
            popover::reap_popup(cx, |this: &mut Self| &mut this.export_open);
            cx.notify();
        }
    }

    fn start_export(&mut self, format: ExportFormat, window: &mut Window, cx: &mut Context<Self>) {
        self.close_export_menu(cx);
        self.export_saved = None;
        let Some(artifact) = self.artifact(cx) else {
            return;
        };
        let Some(source) = self.shown_text() else {
            return;
        };
        let decision_revision = self.shown.unwrap_or(artifact.current_revision);
        let revision = self
            .content
            .as_ref()
            .and_then(|reply| reply.as_ref().ok())
            .map(|reply| reply.revision)
            .unwrap_or(decision_revision);
        let decisions: Vec<_> = artifact
            .decisions
            .iter()
            .filter(|d| d.revision == decision_revision)
            .cloned()
            .collect();
        if matches!(format, ExportFormat::Markdown) {
            let text = match artifact.kind {
                ArtifactKind::Markdown => Ok(source),
                ArtifactKind::Plan => {
                    mast_proto::plan::Plan::parse(&source).map(|p| mast_proto::plan::markdown(&p))
                }
                ArtifactKind::DecisionSheet => {
                    mast_proto::decision_sheet::DecisionSheet::parse(&source)
                        .map(|s| mast_proto::decision_sheet::markdown(&s, &decisions))
                }
                _ => Err("This artifact cannot be copied as Markdown".into()),
            };
            match text {
                Ok(text) => {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    self.notice = Some("Copied as markdown".into());
                }
                Err(error) => self.notice = Some(error),
            }
            cx.notify();
            return;
        }
        let (kind, body) = match (&self.body, artifact.kind) {
            (_, ArtifactKind::DecisionSheet) => {
                match mast_proto::decision_sheet::DecisionSheet::parse(&source) {
                    Ok(mut sheet) => {
                        sheet.apply_decisions(&decisions);
                        (
                            WebKind::DecisionSheet,
                            mast_proto::decision_sheet_render::render(&sheet),
                        )
                    }
                    Err(error) => {
                        self.notice = Some(error);
                        cx.notify();
                        return;
                    }
                }
            }
            (Some(ArtifactBody::Web { kind, source }), _) => (*kind, source.clone()),
            (Some(ArtifactBody::Options(manifest)), _) => {
                let calls: Vec<_> = manifest
                    .options
                    .iter()
                    .map(|option| {
                        let params = self.content_params(self.shown, Some(option.path.clone()), cx);
                        let label = if option.label.is_empty() {
                            option.id.clone()
                        } else {
                            option.label.clone()
                        };
                        (
                            label,
                            self.state.read(cx).artifact_actions().content(params),
                        )
                    })
                    .collect();
                let title = artifact.title.clone();
                self._export = Some(cx.spawn_in(window, async move |this, cx| {
                    let mut body = String::new();
                    for (label, call) in calls {
                        match call.await {
                            Ok(reply) => {
                                let Some(source) = model::source_text(&reply) else {
                                    this.update_in(cx, |this, _, cx| {
                                        this.notice = Some("Option content is not UTF-8".into());
                                        cx.notify();
                                    })
                                    .ok();
                                    return;
                                };
                                body.push_str(&format!(
                                    "<section><h2>{}</h2>{}</section>",
                                    crate::browser::fragment::escape_text(&label),
                                    source
                                ));
                            }
                            Err(error) => {
                                this.update_in(cx, |this, _, cx| {
                                    this.notice = Some(error.text());
                                    cx.notify();
                                })
                                .ok();
                                return;
                            }
                        }
                    }
                    this.update_in(cx, |this, window, cx| {
                        this.save_export(
                            format,
                            WebKind::Fragment,
                            body,
                            title,
                            revision,
                            window,
                            cx,
                        )
                    })
                    .ok();
                }));
                return;
            }
            _ => return,
        };
        self.save_export(
            format,
            kind,
            body,
            artifact.title.clone(),
            revision,
            window,
            cx,
        );
    }

    fn save_export(
        &mut self,
        format: ExportFormat,
        kind: WebKind,
        body: String,
        title: String,
        revision: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pdf = matches!(format, ExportFormat::Pdf);
        let html = document::export_document(kind, &body, &title, pdf);
        let ext = if pdf { "pdf" } else { "html" };
        let name = suggested_name(&title, revision, ext);
        let Some(home) = std::env::var_os("HOME") else {
            self.notice = Some("Downloads folder is unavailable".into());
            cx.notify();
            return;
        };
        let download_dir = std::path::PathBuf::from(home).join("Downloads");
        let receiver = cx.prompt_for_new_path(&download_dir, Some(&name));
        self._export = Some(cx.spawn_in(window, async move |this, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) => return,
                _ => {
                    this.update_in(cx, |this, _, cx| {
                        this.notice = Some("Could not open save panel".into());
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let result = if !safe_destination(&path) {
                Err("Cannot save in the Mast data directory".to_owned())
            } else if pdf {
                #[cfg(target_os = "macos")]
                {
                    super::super::render_check::webkit::export_pdf(
                        html,
                        path.clone(),
                        cx.background_executor().clone(),
                    )
                    .await
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Err("PDF export is not available on this platform".into())
                }
            } else {
                std::fs::write(&path, html).map_err(|e| e.to_string())
            };
            this.update_in(cx, |this, _, cx| {
                match result {
                    Ok(()) => {
                        this.notice = Some(format!(
                            "Saved {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ));
                        this.export_saved = Some(path);
                    }
                    Err(error) => this.notice = Some(format!("Export failed: {error}")),
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::suggested_name;

    #[test]
    fn suggested_names_include_the_revision_and_extension() {
        assert_eq!(suggested_name("A / B", 12, "pdf"), "A _ B r12.pdf");
        assert_eq!(suggested_name("...", 0, "html"), "Artifact r0.html");
    }
}
