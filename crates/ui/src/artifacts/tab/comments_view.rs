//! The artifact tab's comments on HTML and Mermaid pages (Artifacts phase 4
//! G): what the frame reports (selection, picked element, marker clicks,
//! in-place question answers), the draft composer, the comments list, and
//! keeping the page's markers in step with the row.
//!
//! The tab never edits the document: comments live on the artifact row
//! (`ArtifactView.comments`), saved with `ArtifactAddComment` /
//! `ArtifactResolveComment`. A new comment also messages the chat that
//! published the revision, like an answer does.
use gpui::{
    AnyElement, Context, Entity, Focusable, SharedString, Subscription, Task, Window, div,
    prelude::*, px,
};
use mast_proto::artifacts::comment_message;
use mast_proto::{
    ArtifactAddCommentParams, ArtifactComment, ArtifactResolveCommentParams, ArtifactView,
    CommentAnchor,
};

use super::ArtifactTab;
use crate::artifacts::bridge::FrameMessage;
use crate::artifacts::comments::{self, CommentRow, CommentState};
use crate::artifacts::model::ArtifactBody;
use crate::artifacts::questions_ui;
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::theme::Theme;
use crate::icons;

/// A comment being written: where it points and what it quotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Draft {
    pub(crate) anchor: CommentAnchor,
    pub(crate) quote: String,
}

/// What the tab remembers about comments.
#[derive(Default)]
pub(crate) struct CommentsState {
    /// The list panel is open.
    pub(super) open: bool,
    /// Comment mode: hovering outlines an element, a click captures it.
    pub(super) pick: bool,
    /// The page's scripts are listening (reset on every document load).
    pub(super) ready: bool,
    /// The reader's current text selection in the page.
    pub(super) selection: Option<Draft>,
    pub(super) draft: Option<Draft>,
    input: Option<Entity<ComposerInput>>,
    busy: bool,
    /// The comment the page last asked us to show.
    pub(super) focused: Option<String>,
    /// What the page was last given, to avoid repeating it.
    pushed: Option<String>,
    cached: Option<(u64, Vec<ArtifactComment>, Vec<CommentRow>)>,
    tasks: Vec<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl ArtifactTab {
    /// Comments make sense for HTML and Mermaid artifacts, on screen as a
    /// page.
    pub(super) fn comments_available(&self, artifact: &ArtifactView) -> bool {
        comments::applies(artifact)
            && self.mode == super::Mode::Rendered
            && self.option_shown.is_none()
            && matches!(self.body, Some(ArtifactBody::Web { .. }))
    }

    /// Only the current revision takes new comments.
    fn can_comment(&self, artifact: &ArtifactView) -> bool {
        self.comments_available(artifact)
            && self.shown_or_current_revision(artifact) == artifact.current_revision
    }

    /// The rows for the comments against the source on screen (cached per
    /// document and comment set).
    pub(crate) fn comment_rows(&mut self, artifact: &ArtifactView) -> Vec<CommentRow> {
        if let Some((generation, comments, rows)) = &self.comments.cached
            && *generation == self.generation
            && *comments == artifact.comments
        {
            return rows.clone();
        }
        let source = match &self.body {
            Some(ArtifactBody::Web { source, .. }) if !self.loading => Some(source.as_str()),
            _ => None,
        };
        let rows = comments::rows(artifact.kind, source, &artifact.comments);
        self.comments.cached = Some((self.generation, artifact.comments.clone(), rows.clone()));
        rows
    }

    #[cfg(test)]
    pub(crate) fn comments_document_loading_for_tests(&mut self) {
        self.comments_document_loading();
    }

    #[cfg(test)]
    pub(crate) fn comment_rows_for_tests(&mut self, artifact: &ArtifactView) -> Vec<CommentRow> {
        self.comment_rows(artifact)
    }

    /// A new document is loading: the page must say it is ready again.
    pub(super) fn comments_document_loading(&mut self) {
        self.comments.ready = false;
        self.comments.pushed = None;
        self.comments.selection = None;
    }

    // ------------------------------------------------------------------
    // The page's messages
    // ------------------------------------------------------------------

    /// One message from the page's frame (raw JSON from the bridge). Unknown
    /// or malformed messages are ignored.
    pub(crate) fn on_frame_message(
        &mut self,
        json: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = FrameMessage::parse(json) else {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(json)
                && value.get("t").and_then(|v| v.as_str()) == Some("sheet-stage") {
                let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("");
                self.sheet_refusal("The stage request was invalid.", Some(id), cx);
            }
            return;
        };
        match message {
            FrameMessage::Ready => {
                self.comments.ready = true;
                self.comments.pushed = None;
                self.comments.selection = None;
                cx.notify();
            }
            FrameMessage::Selection { anchor, quote } => {
                self.comments.selection = Some(Draft { anchor, quote });
                cx.notify();
            }
            FrameMessage::SelectionCleared => {
                if self.comments.selection.take().is_some() {
                    cx.notify();
                }
            }
            FrameMessage::Draft { anchor, quote } => {
                self.set_pick_mode(false, cx);
                self.begin_comment(Draft { anchor, quote }, window, cx);
            }
            FrameMessage::FocusComment(id) => {
                self.comments.open = true;
                self.comments.focused = Some(id);
                cx.notify();
            }
            FrameMessage::QuestionUndo(id) => {
                if self.artifact(cx).is_some_and(|artifact| self.can_comment(&artifact)) { self.undo_question_draft(id, cx); }
            }
            FrameMessage::SheetStage { id, outcome, note, override_recommendation } => {
                self.stage_sheet_decision(id, outcome, note, override_recommendation, cx);
            }
            FrameMessage::SheetUndo(id) => self.undo_sheet_decision(Some(id), cx),
            FrameMessage::SheetDiscard => self.undo_sheet_decision(None, cx),
            FrameMessage::SheetSend => self.send_sheet_decisions(cx),
            FrameMessage::Answer { id, text } => {
                // Only the current revision's questions take answers (the
                // Markdown cards do the same).
                if self
                    .artifact(cx)
                    .is_some_and(|artifact| self.can_comment(&artifact))
                {
                    self.stage_question_answer(id, text, cx)
                }
            }
        }
    }

    fn current_sheet(&self, cx: &gpui::App) -> Result<(ArtifactView, mast_proto::decision_sheet::DecisionSheet), String> {
        let artifact = self.artifact(cx).ok_or("The artifact is unavailable.")?;
        if artifact.kind != mast_proto::ArtifactKind::DecisionSheet { return Err("This artifact is not a decision sheet.".into()); }
        if self.shown_or_current_revision(&artifact) != artifact.current_revision { return Err("This revision is no longer current.".into()); }
        if self.mode != super::Mode::Rendered || self.loading { return Err("The decision sheet is not ready.".into()); }
        let source = self.shown_text().ok_or("The decision sheet content is unavailable.")?;
        let sheet = mast_proto::decision_sheet::DecisionSheet::parse(&source).map_err(|error| format!("Could not read the decision sheet: {error}"))?;
        Ok((artifact, sheet))
    }

    fn sheet_refusal(&mut self, reason: &str, row_id: Option<&str>, cx: &mut Context<Self>) {
        tracing::warn!(artifact_id = %self.artifact_id, reason, "decision sheet action refused");
        self.notice = Some(reason.to_owned());
        let message = match row_id {
            Some(id) => serde_json::json!({"mastSheetStageRejected": {"id": id, "reason": reason}}),
            None => serde_json::json!({"mastSheetEnableSend": true}),
        };
        if let Some(web) = &mut self.web { web.run_script(&crate::artifacts::bridge::frame_command_script(&message), cx); }
        cx.notify();
    }

    fn stage_sheet_decision(&mut self, id: String, outcome: mast_proto::decision_sheet::DecisionOutcome, note: String, override_recommendation: bool, cx: &mut Context<Self>) {
        use mast_proto::decision_sheet::{SheetDecisionDraft, SheetDecisionDrafts};
        let (artifact, sheet) = match self.current_sheet(cx) {
            Ok(sheet) => sheet,
            Err(reason) => { self.sheet_refusal(&reason, Some(&id), cx); return; }
        };
        let reason = if !sheet.decision_rows().iter().any(|row| row.id == id && row.status.state == "open") {
            Some("This row is missing or no longer open.")
        } else if outcome == mast_proto::decision_sheet::DecisionOutcome::Rejected && !sheet.decision_rows().iter().any(|row| row.id == id && row.optional) {
            Some("This row cannot be omitted.")
        } else if override_recommendation && (outcome != mast_proto::decision_sheet::DecisionOutcome::Decided || note.trim().is_empty()) {
            Some("Your answer is required.")
        } else if artifact.decisions.iter().any(|saved| saved.row_id == id && saved.revision == artifact.current_revision) {
            Some("This row has already been sent.")
        } else { None };
        if let Some(reason) = reason { self.sheet_refusal(reason, Some(&id), cx); return; }
        crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
            let draft = settings.artifact_decision_drafts.entry(artifact.id).or_insert_with(|| SheetDecisionDrafts { revision: artifact.current_revision, decisions: Vec::new() });
            if draft.revision != artifact.current_revision { *draft = SheetDecisionDrafts { revision: artifact.current_revision, decisions: Vec::new() }; }
            draft.decisions.retain(|decision| decision.row_id != id);
            draft.decisions.push(SheetDecisionDraft { row_id: id.clone(), outcome, override_recommendation, note: Some(note) });
        });
    }

    fn undo_sheet_decision(&mut self, row_id: Option<String>, cx: &mut Context<Self>) {
        let Ok((artifact, _)) = self.current_sheet(cx) else { return; };
        crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
            if let Some(id) = row_id {
                if let Some(draft) = settings.artifact_decision_drafts.get_mut(&artifact.id) {
                    draft.decisions.retain(|decision| decision.row_id != id);
                    if draft.decisions.is_empty() { settings.artifact_decision_drafts.remove(&artifact.id); }
                }
            } else { settings.artifact_decision_drafts.remove(&artifact.id); }
        });
    }

    fn send_sheet_decisions(&mut self, cx: &mut Context<Self>) {
        use mast_proto::decision_sheet::{SubmitSheetDecisionsParams, decision_batch_message};
        let (artifact, _) = match self.current_sheet(cx) { Ok(sheet) => sheet, Err(reason) => { self.sheet_refusal(&reason, None, cx); return; } };
        if self.comments.busy { self.sheet_refusal("A decision batch is already being sent.", None, cx); return; }
        let draft = crate::settings::current(cx).artifact_decision_drafts.get(&artifact.id).cloned();
        let Some(draft) = draft.filter(|d| d.revision == artifact.current_revision && !d.decisions.is_empty()) else { self.sheet_refusal("There are no staged decisions for this revision.", None, cx); return; };
        let chat = artifact.current_owner_chat_id().map(str::to_owned).filter(|id| self.state.read(cx).chats.iter().any(|row| row.id == *id && !row.archived));
        let Some(chat) = chat else {
            self.sheet_refusal("The current owner chat is unavailable. Decisions remain staged.", None, cx);
            return;
        };
        self.comments.busy = true;
        let call = self.state.read(cx).artifact_actions().submit_sheet_decisions(SubmitSheetDecisionsParams {
            artifact_id: artifact.id.clone(), revision: draft.revision, decisions: draft.decisions,
        });
        let task = cx.spawn(async move |this, cx| {
            let reply = call.await;
            this.update(cx, |this, cx| {
                this.comments.busy = false;
                match reply {
                    Err(error) => {
                        this.sheet_refusal(&format!("Couldn't save decisions: {}", error.text()), None, cx);
                    }
                    Ok(reply) => {
                        crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| { settings.artifact_decision_drafts.remove(&artifact.id); });
                        if let Some(web) = &mut this.web { web.run_script(&crate::artifacts::bridge::frame_command_script(&serde_json::json!({"mastSheetSent": reply.decisions})), cx); }
                        {
                            let message = decision_batch_message(&artifact.title, &reply.decisions);
                            let delivery = crate::work_outline::required_actions_view::deliver_user_message(this.state.clone(), chat, message, crate::work_outline::actions::new_operation_id(), cx);
                            let task = cx.spawn(async move |this, cx| {
                                if let Err(error) = delivery.await { this.update(cx, |this, cx| { this.notice = Some(format!("Decisions saved, but the chat wasn't told: {error}")); cx.notify(); }).ok(); }
                            });
                            this.comments.tasks.push(task);
                        }
                    }
                }
                cx.notify();
            }).ok();
        });
        self.comments.tasks.push(task);
    }

    // ------------------------------------------------------------------
    // Writing a comment
    // ------------------------------------------------------------------

    pub(crate) fn set_pick_mode(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.comments.pick == on {
            return;
        }
        self.comments.pick = on;
        if let Some(web) = &mut self.web {
            web.run_script(&comments::mode_script(on), cx);
        }
        cx.notify();
    }

    /// Start a comment on the reader's current selection.
    pub(crate) fn comment_on_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = self.comments.selection.clone() {
            self.begin_comment(draft, window, cx);
        }
    }

    fn begin_comment(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        let Some(artifact) = self.artifact(cx) else {
            return;
        };
        if !self.can_comment(&artifact) {
            return;
        }
        let input = match self.comments.input.clone() {
            Some(input) => input,
            None => {
                let input = cx.new(|cx| {
                    ComposerInput::new("Comment", cx)
                        .with_text_metrics(12.0, 18.0)
                        .with_auto_grow()
                });
                let subscription =
                    cx.subscribe(&input, |this: &mut Self, _, event, cx| match event {
                        ComposerInputEvent::Submitted | ComposerInputEvent::ModifiedSubmitted => {
                            this.submit_comment(cx)
                        }
                        _ => {}
                    });
                self.comments.subscriptions.push(subscription);
                self.comments.input = Some(input.clone());
                input
            }
        };
        input.update(cx, |input, cx| input.set_text("", cx));
        let handle = input.focus_handle(cx);
        if let Some(web) = &mut self.web {
            web.focus_chrome(cx);
        }
        window.focus(&handle, cx);
        self.comments.draft = Some(draft);
        self.comments.open = true;
        cx.notify();
    }

    pub(crate) fn cancel_comment(&mut self, cx: &mut Context<Self>) {
        self.comments.draft = None;
        cx.notify();
    }

    fn typed_comment(&self, cx: &gpui::App) -> String {
        self.comments
            .input
            .as_ref()
            .map(|input| input.read(cx).text().trim().to_owned())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn set_comment_text_for_tests(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(input) = self.comments.input.clone() {
            input.update(cx, |input, cx| input.set_text(text, cx));
        }
    }

    /// Save the draft on the row, then tell the chat that published the
    /// revision.
    pub(crate) fn submit_comment(&mut self, cx: &mut Context<Self>) {
        let text = self.typed_comment(cx);
        let Some(draft) = self.comments.draft.clone() else {
            return;
        };
        if text.is_empty() || self.comments.busy {
            return;
        }
        let Some(artifact) = self.artifact(cx) else {
            return;
        };
        self.comments.busy = true;
        let call = self
            .state
            .read(cx)
            .artifact_actions()
            .add_comment(ArtifactAddCommentParams {
                artifact_id: artifact.id.clone(),
                anchor: draft.anchor,
                quote: draft.quote,
                text,
            });
        let task = cx.spawn(async move |this, cx| {
            let reply = call.await;
            this.update(cx, |this, cx| {
                this.comments.busy = false;
                match reply {
                    Ok(reply) => this.comment_recorded(&artifact, reply.comment, cx),
                    Err(error) => {
                        this.notice = Some(format!(
                            "Couldn\u{2019}t save the comment: {}",
                            error.text()
                        ));
                        cx.notify();
                    }
                }
            })
            .ok();
        });
        self.comments.tasks.push(task);
    }

    fn comment_recorded(
        &mut self,
        artifact: &ArtifactView,
        comment: ArtifactComment,
        cx: &mut Context<Self>,
    ) {
        self.comments.draft = None;
        self.comments.selection = None;
        self.comments.pushed = None;
        // Shown until the snapshot carries it.
        self.comments.focused = Some(comment.id.clone());
        let chat = questions_ui::answer_target_chat(artifact).filter(|chat| {
            self.state
                .read(cx)
                .chats
                .iter()
                .any(|row| row.id == *chat && !row.archived)
        });
        match chat {
            Some(chat) => {
                let delivery = crate::work_outline::required_actions_view::deliver_user_message(
                    self.state.clone(),
                    chat,
                    comment_message(&artifact.title, comment.revision, &comment),
                    crate::work_outline::actions::new_operation_id(),
                    cx,
                );
                let task = cx.spawn(async move |this, cx| {
                    if let Err(message) = delivery.await {
                        this.update(cx, |this, cx| {
                            this.notice = Some(format!(
                                "Comment saved, but the chat wasn\u{2019}t told: {message}"
                            ));
                            cx.notify();
                        })
                        .ok();
                    }
                });
                self.comments.tasks.push(task);
            }
            None => {
                self.notice = Some(
                    "Comment saved. The chat that published this artifact is unavailable, so it wasn\u{2019}t told."
                        .to_owned(),
                );
            }
        }
        cx.notify();
    }

    pub(crate) fn resolve_comment(&mut self, id: String, resolved: bool, cx: &mut Context<Self>) {
        let Some(artifact) = self.artifact(cx) else {
            return;
        };
        let call =
            self.state
                .read(cx)
                .artifact_actions()
                .resolve_comment(ArtifactResolveCommentParams {
                    artifact_id: artifact.id,
                    comment_id: id,
                    resolved,
                });
        let task = cx.spawn(async move |this, cx| {
            if let Err(error) = call.await {
                this.update(cx, |this, cx| {
                    this.notice = Some(format!(
                        "Couldn\u{2019}t update the comment: {}",
                        error.text()
                    ));
                    cx.notify();
                })
                .ok();
            }
        });
        self.comments.tasks.push(task);
    }

    /// Show a comment in the page (open comments only).
    pub(crate) fn focus_comment(&mut self, id: &str, cx: &mut Context<Self>) {
        self.comments.focused = Some(id.to_owned());
        if let Some(web) = &mut self.web {
            web.run_script(&comments::focus_script(id), cx);
        }
        cx.notify();
    }

    // ------------------------------------------------------------------
    // Keeping the page's markers in step with the row
    // ------------------------------------------------------------------

    /// Push the markers when the page is ready and they changed. Called from
    /// render; cheap when nothing changed.
    pub(super) fn sync_comment_markers(&mut self, artifact: &ArtifactView, cx: &mut Context<Self>) {
        if !self.comments.ready || !self.comments_available(artifact) {
            return;
        }
        let rows = self.comment_rows(artifact);
        let script = comments::markers_script(&rows);
        if self.comments.pushed.as_deref() == Some(script.as_str()) {
            return;
        }
        self.comments.pushed = Some(script.clone());
        if let Some(web) = &mut self.web {
            web.run_script(&script, cx);
            if self.comments.pick {
                web.run_script(&comments::mode_script(true), cx);
            }
        }
    }

    // ------------------------------------------------------------------
    // Toolbar and panel
    // ------------------------------------------------------------------

    /// The toolbar's comment buttons (none for kinds without comments).
    pub(super) fn render_comments_toolbar(
        &mut self,
        artifact: &ArtifactView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if !self.comments_available(artifact) {
            return Vec::new();
        }
        let rows = self.comment_rows(artifact);
        let open = comments::open_count(&rows);
        let can = self.can_comment(artifact);
        let label = |id: &'static str,
                     text: SharedString,
                     on: bool,
                     enabled: bool|
         -> gpui::Stateful<gpui::Div> {
            crate::components::browser_parts::artifact_comment_control(id, text, on, enabled, theme)
        };
        let mut out = Vec::new();
        if self.comments.selection.is_some() && can {
            out.push(
                label(
                    "artifact-comment-selection",
                    "Comment on selection".into(),
                    false,
                    true,
                )
                .on_click(cx.listener(|this, _, window, cx| this.comment_on_selection(window, cx)))
                .into_any_element(),
            );
        }
        out.push(
            label(
                "artifact-comment-pick",
                "Pick element".into(),
                self.comments.pick,
                can,
            )
            .when(can, |el| {
                el.on_click(cx.listener(|this, _, _, cx| {
                    let on = !this.comments.pick;
                    this.set_pick_mode(on, cx)
                }))
            })
            .into_any_element(),
        );
        let count: SharedString = if open == 0 {
            "Comments".into()
        } else {
            format!("Comments {open}").into()
        };
        out.push(
            label("artifact-comments-toggle", count, self.comments.open, true)
                .child(icons::icon(icons::CHAT_ROUND_LINE).size(px(11.0)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.comments.open = !this.comments.open;
                    cx.notify();
                }))
                .into_any_element(),
        );
        out
    }

    /// The body with the comments list beside it while the panel is open.
    pub(super) fn with_comments_panel(
        &mut self,
        body: AnyElement,
        artifact: &ArtifactView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_comment_markers(artifact, cx);
        if !self.comments_available(artifact) || !self.comments.open {
            return body;
        }
        let panel = self.render_comments_panel(artifact, theme, cx);
        div()
            .size_full()
            .flex()
            .flex_row()
            .child(div().flex_1().min_w_0().h_full().relative().child(body))
            .child(panel)
            .into_any_element()
    }

    fn render_comments_panel(
        &mut self,
        artifact: &ArtifactView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = self.comment_rows(artifact);
        let can = self.can_comment(artifact);
        let mut list = div()
            .id("artifact-comments-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(8.0));
        if let Some(draft) = self.comments.draft.clone() {
            list = list.child(self.render_draft(&draft, theme, cx));
        }
        if rows.is_empty() && self.comments.draft.is_none() {
            list = list.child(
                div()
                    .debug_selector(|| "artifact-comments-empty".into())
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .child(if can {
                        "Select text in the page, or pick an element, to comment on it."
                    } else {
                        "No comments."
                    }),
            );
        }
        // Outdated first so they are not missed, then newest last.
        let mut ordered: Vec<&CommentRow> = rows.iter().collect();
        ordered.sort_by_key(|row| match row.state {
            CommentState::Outdated => 0,
            CommentState::Open => 1,
            CommentState::Resolved => 2,
        });
        for row in ordered {
            list = list.child(self.render_comment_row(row, theme, cx));
        }
        div()
            .id("artifact-comments-panel")
            .debug_selector(|| "artifact-comments-panel".into())
            .flex_none()
            .w(px(280.0))
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_none()
                    .px(px(10.0))
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .items_center()
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .child(div().flex_1().child("Comments"))
                    .child(
                        div()
                            .id("artifact-comments-close")
                            .cursor_pointer()
                            .child(icons::icon(icons::CLOSE).size(px(10.0)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.comments.open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(list)
            .into_any_element()
    }

    fn render_draft(&self, draft: &Draft, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let input = self.comments.input.clone();
        let quote: SharedString = draft.quote.clone().into();
        crate::components::browser_parts::artifact_comment_card("artifact-comment-draft".into(), true, false, false, theme)
            .when(!draft.quote.is_empty(), |el| {
                el.child(
                    div()
                        .line_clamp(3)
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_muted)
                        .child(quote),
                )
            })
            .children(input.map(|input| div().child(input).into_any_element()))
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(
                        action_button("artifact-comment-add", "Comment", true, theme)
                            .on_click(cx.listener(|this, _, _, cx| this.submit_comment(cx))),
                    )
                    .child(
                        action_button("artifact-comment-cancel", "Cancel", false, theme)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_comment(cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_comment_row(
        &self,
        row: &CommentRow,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let comment = &row.comment;
        let id = comment.id.clone();
        let color = match row.state {
            CommentState::Open => theme.accent,
            CommentState::Outdated => theme.warning,
            CommentState::Resolved => theme.text_muted,
        };
        let focused = self.comments.focused.as_deref() == Some(comment.id.as_str());
        let resolved = row.state == CommentState::Resolved;
        let quote: SharedString = if comment.quote.is_empty() {
            comment.anchor.exact.clone().into()
        } else {
            comment.quote.clone().into()
        };
        let who = match (resolved, comment.resolved_by.as_deref()) {
            (true, Some("agent")) => comment
                .addressed_in
                .map(|rev| format!("Addressed in r{rev}"))
                .unwrap_or_else(|| "Addressed".to_owned()),
            (true, _) => "Resolved".to_owned(),
            _ => format!("r{}", comment.revision),
        };
        let select_id = id.clone();
        let toggle_id = id.clone();
        let selector: SharedString = format!("artifact-comment-{}", comment.id).into();
        let toggle_selector: SharedString =
            format!("artifact-comment-toggle-{}", comment.id).into();
        crate::components::browser_parts::artifact_comment_card(selector, false, focused, resolved, theme)
            .when(row.state == CommentState::Open, |el| {
                el.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.focus_comment(&select_id, cx)))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(11.0))
                            .text_color(theme.text)
                            .child(SharedString::from(comment.id.clone())),
                    )
                    .child(
                        crate::components::browser_parts::artifact_comment_badge(row.state.label(), color, theme)
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(crate::typography::ui_rems(10.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(who)),
                    ),
            )
            .when(!quote.is_empty(), |el| {
                el.child(
                    div()
                        .line_clamp(2)
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_muted)
                        .child(quote),
                )
            })
            .child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text)
                    .child(SharedString::from(comment.text.clone())),
            )
            .child(
                div().flex().child(
                    action_button(
                        toggle_selector,
                        if resolved { "Reopen" } else { "Resolve" },
                        false,
                        theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.resolve_comment(toggle_id.clone(), !resolved, cx)
                    })),
                ),
            )
            .into_any_element()
    }
}

fn action_button(
    id: impl Into<SharedString>,
    label: &'static str,
    primary: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    crate::components::browser_parts::artifact_action_button(id.into(), label, primary, theme)
}
