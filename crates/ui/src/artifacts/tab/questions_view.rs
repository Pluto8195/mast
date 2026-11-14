//! The artifact tab's plan question state: answering in place, the
//! questions / drift / "updated since you last viewed" badge row, and the
//! per-artifact visit record. Decisions live in `artifacts::questions_ui`.
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyElement, Context, Entity, Focusable, SharedString, Subscription, Task, Window, div,
    prelude::*, px,
};
use mast_proto::{
    ArtifactDriftParams, ArtifactDriftReply, ArtifactKind,
    ArtifactView, PlanQuestion, extract_questions,
};

use super::ArtifactTab;
use crate::artifacts::questions_ui::{self, DriftGate};
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::markdown::render::AnswerUi;
use crate::theme::Theme;
use crate::work_outline::chips::TextTooltip;

/// What the tab remembers about the plan's questions.
#[derive(Default)]
pub(crate) struct QuestionState {
    /// The question whose answer input is open (upper-cased id).
    pub(super) answering: Option<String>,
    input: Option<Entity<ComposerInput>>,
    busy: bool,
    /// Answers sent from this tab, shown until the snapshot carries them.
    local: HashMap<String, String>,
    /// What the Markdown preview was last given.
    synced: Option<(Vec<(String, String)>, Vec<(String, String)>, Option<String>, bool)>,
    cached: Option<(u64, Vec<PlanQuestion>)>,
    /// Answers baked into the HTML document on screen (`data-answer`).
    pub(super) html_answers: HashMap<String, String>,
    html_drafts: HashMap<String, String>,
    drift: Option<ArtifactDriftReply>,
    gate: DriftGate,
    /// The revision last recorded as viewed, and the one the previous
    /// visit left (the marker's `from`).
    visit_recorded: u32,
    visit_base: Option<u32>,
    pub(super) changed: Option<(u32, u32)>,
    tasks: Vec<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl ArtifactTab {
    /// The plan's questions from the shown document, cached per load.
    pub(crate) fn plan_questions(&mut self, kind: ArtifactKind) -> Vec<PlanQuestion> {
        if self.loading {
            return Vec::new();
        }
        if let Some((generation, questions)) = &self.questions.cached
            && *generation == self.generation
        {
            return questions.clone();
        }
        let questions = self
            .shown_text()
            .map(|text| extract_questions(kind, &text))
            .unwrap_or_default();
        self.questions.cached = Some((self.generation, questions.clone()));
        questions
    }

    /// Everything the tab keeps in step with the snapshot; cheap when
    /// nothing changed. Called from render.
    pub(super) fn track_questions(
        &mut self,
        artifact: &ArtifactView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.note_visit(artifact, cx);
        self.maybe_fetch_drift(artifact, cx);
        self.sync_answers(artifact, window, cx);
    }

    // ------------------------------------------------------------------
    // Changed since you last viewed
    // ------------------------------------------------------------------

    /// Compute the marker from the stored visit, then record the current
    /// revision so the marker survives until the next visit.
    fn note_visit(&mut self, artifact: &ArtifactView, cx: &mut Context<Self>) {
        let current = artifact.current_revision;
        if current == 0 || current <= self.questions.visit_recorded {
            return;
        }
        if self.questions.visit_recorded == 0 {
            self.questions.visit_base = crate::settings::current(cx)
                .artifact_last_viewed
                .get(&artifact.id)
                .copied();
        }
        self.questions.changed = questions_ui::changed_since(self.questions.visit_base, current);
        self.questions.visit_recorded = current;
        let id = artifact.id.clone();
        cx.defer(move |cx| {
            let mut moved = false;
            crate::settings::update(crate::settings::SavePolicy::Debounced, cx, |settings| {
                moved = settings.artifact_last_viewed.insert(id, current) != Some(current);
            });
            // The list markers read this record; repaint them only when it moved.
            if moved {
                cx.refresh_windows();
            }
        });
    }

    /// Open Compare from the last viewed revision to the current one.
    pub(crate) fn open_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((from, _)) = self.questions.changed.take() else {
            return;
        };
        self.compare_since(from, window, cx);
    }

    // ------------------------------------------------------------------
    // Drift
    // ------------------------------------------------------------------

    fn maybe_fetch_drift(&mut self, artifact: &ArtifactView, cx: &mut Context<Self>) {
        if artifact.current_revision == 0
            || self.loading
            || !self
                .questions
                .gate
                .due(artifact.current_revision, Instant::now())
        {
            return;
        }
        let state = self.state.read(cx);
        let target_device_id = (state.local_device_id.as_deref() != Some(&artifact.device_id)
            && !artifact.device_id.is_empty())
        .then(|| artifact.device_id.clone());
        let call = state.artifact_actions().drift(ArtifactDriftParams {
            artifact_id: artifact.id.clone(),
            target_device_id,
        });
        let task = cx.spawn(async move |this, cx| {
            let reply = call.await;
            this.update(cx, |this, cx| {
                // Failures (an older host) just leave no commits badge.
                this.questions.drift = reply.ok();
                cx.notify();
            })
            .ok();
        });
        self.questions.tasks.push(task);
    }

    // ------------------------------------------------------------------
    // Answering
    // ------------------------------------------------------------------

    fn answer_map(&self, artifact: &ArtifactView) -> HashMap<String, String> {
        let mut map = self.questions.local.clone();
        map.extend(questions_ui::answer_texts(artifact));
        map
    }

    fn question_drafts(&self, artifact: &ArtifactView, cx: &gpui::App) -> HashMap<String, String> {
        crate::settings::current(cx).artifact_question_drafts.get(&artifact.id)
            .filter(|(revision, _)| *revision == artifact.current_revision)
            .map(|(_, drafts)| drafts.clone()).unwrap_or_default()
    }

    /// Give the Markdown preview the answers and the Answer affordance
    /// (only while the current revision is on screen).
    fn sync_answers(
        &mut self,
        artifact: &ArtifactView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode != super::Mode::Compare
            && matches!(self.body, Some(super::ArtifactBody::Web { .. }))
            && self.option_shown.is_none()
        {
            // HTML cards show answered state from the document itself; a
            // changed answer set re-presents the page.
            let answers = self.answer_map(artifact);
            if answers != self.questions.html_answers {
                // A live page updates its cards in place (no reload, the
                // scroll position stays); otherwise re-present it with the
                // answers baked in.
                let in_place = self.comments.ready
                    && self.web.as_ref().is_some_and(|web| web.supports_scripts());
                self.questions.html_answers = answers.clone();
                if in_place {
                    let script = crate::artifacts::bridge::frame_command_script(
                        &serde_json::json!({ "mastAnswers": answers }),
                    );
                    if let Some(web) = &mut self.web {
                        web.run_script(&script, cx);
                    }
                } else {
                    self.present(window, cx);
                }
            }
            let drafts = self.question_drafts(artifact, cx);
            if self.comments.ready && drafts != self.questions.html_drafts {
                self.questions.html_drafts = drafts.clone();
                let script = crate::artifacts::bridge::frame_command_script(&serde_json::json!({"mastQuestionDrafts": drafts}));
                if let Some(web) = &mut self.web { web.run_script(&script, cx); }
            }
            return;
        }
        let Some(preview) = self.markdown.clone() else {
            return;
        };
        let live = self.shown_or_current_revision(artifact) == artifact.current_revision;
        let answers = self.answer_map(artifact);
        let mut sig: Vec<(String, String)> = answers.clone().into_iter().collect();
        sig.sort();
        let drafts = self.question_drafts(artifact, cx);
        let mut draft_sig: Vec<_> = drafts.clone().into_iter().collect();
        draft_sig.sort();
        let editing = live.then(|| self.questions.answering.clone()).flatten();
        let signature = (sig, draft_sig, editing.clone(), live);
        if self.questions.synced.as_ref() == Some(&signature) {
            return;
        }
        self.questions.synced = Some(signature);
        let ui = live.then(|| {
            let owner = cx.weak_entity();
            let begin = owner.clone();
            let send = owner.clone();
            let cancel = owner.clone();
            let undo = owner;
            AnswerUi {
                answers: Rc::new(answers),
                drafts: Rc::new(drafts),
                editing,
                input: self
                    .questions
                    .input
                    .clone()
                    .map(|input| gpui::AnyView::from(input)),
                begin: Rc::new(move |id, window, cx| {
                    begin
                        .update(cx, |this, cx| this.begin_answer(id, window, cx))
                        .ok();
                }),
                send: Rc::new(move |id, _, cx| {
                    send.update(cx, |this, cx| this.send_answer(id, cx)).ok();
                }),
                cancel: Rc::new(move |_, cx| {
                    cancel.update(cx, |this, cx| this.cancel_answer(cx)).ok();
                }),
                undo: Rc::new(move |id, _, cx| { undo.update(cx, |this, cx| this.undo_question_draft(id, cx)).ok(); }),
            }
        });
        preview.update(cx, |preview, cx| preview.set_question_answers(ui, cx));
    }

    pub(crate) fn begin_answer(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let input = match self.questions.input.clone() {
            Some(input) => input,
            None => {
                let input = cx.new(|cx| {
                    ComposerInput::new("Your answer", cx)
                        .with_text_metrics(12.0, 18.0)
                        .with_auto_grow()
                });
                let subscription =
                    cx.subscribe(&input, |this: &mut Self, _, event, cx| match event {
                        ComposerInputEvent::Submitted | ComposerInputEvent::ModifiedSubmitted => {
                            if let Some(id) = this.questions.answering.clone() {
                                this.send_answer(id, cx);
                            }
                        }
                        _ => {}
                    });
                self.questions.subscriptions.push(subscription);
                self.questions.input = Some(input.clone());
                input
            }
        };
        let current = self.artifact(cx).and_then(|artifact| self.question_drafts(&artifact, cx).get(&id.to_ascii_uppercase()).cloned()).unwrap_or_default();
        input.update(cx, |input, cx| input.set_text(&current, cx));
        let handle = input.focus_handle(cx);
        window.focus(&handle, cx);
        self.questions.answering = Some(id.to_ascii_uppercase());
        self.questions.synced = None;
        cx.notify();
    }

    pub(crate) fn cancel_answer(&mut self, cx: &mut Context<Self>) {
        self.questions.answering = None;
        self.questions.synced = None;
        cx.notify();
    }

    /// The typed text of the answer input (tests, Send).
    fn typed_answer(&self, cx: &gpui::App) -> String {
        self.questions
            .input
            .as_ref()
            .map(|input| input.read(cx).text().trim().to_owned())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn typed_answer_for_tests(&self, cx: &gpui::App) -> String {
        self.questions
            .input
            .as_ref()
            .map(|input| input.read(cx).text().to_owned())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn set_answer_text_for_tests(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(input) = self.questions.input.clone() {
            input.update(cx, |input, cx| input.set_text(text, cx));
        }
    }

    /// Record the typed answer, then tell the chat that published the plan.
    pub(crate) fn send_answer(&mut self, id: String, cx: &mut Context<Self>) {
        let answer = self.typed_answer(cx);
        self.stage_question_answer(id, answer, cx);
    }

    /// Stage an answer on this device. No engine call or chat message occurs.
    pub(crate) fn stage_question_answer(&mut self, id: String, answer: String, cx: &mut Context<Self>) {
        if self.questions.busy { return; }
        let id = id.to_ascii_uppercase();
        let answer = answer.trim().chars().take(mast_proto::artifacts::MAX_ANSWER_CHARS).collect::<String>();
        let Some(artifact) = self.artifact(cx) else { return; };
        if answer.is_empty() || self.shown_or_current_revision(&artifact) != artifact.current_revision
            || !self.plan_questions(artifact.kind).iter().any(|q| q.id == id && q.status == mast_proto::PlanQuestionStatus::Open) { return; }
        crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
            let entry = settings.artifact_question_drafts.entry(artifact.id).or_insert_with(|| (artifact.current_revision, HashMap::new()));
            if entry.0 != artifact.current_revision { *entry = (artifact.current_revision, HashMap::new()); }
            entry.1.insert(id, answer);
        });
        self.questions.answering = None;
        self.questions.synced = None;
        cx.notify();
    }

    pub(crate) fn undo_question_draft(&mut self, id: String, cx: &mut Context<Self>) {
        if self.questions.busy { return; }
        let artifact_id = self.artifact_id.clone();
        crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
            if let Some((_, drafts)) = settings.artifact_question_drafts.get_mut(&artifact_id) {
                drafts.remove(&id);
                if drafts.is_empty() { settings.artifact_question_drafts.remove(&artifact_id); }
            }
        });
        self.questions.synced = None;
        cx.notify();
    }

    pub(crate) fn discard_question_drafts(&mut self, cx: &mut Context<Self>) {
        if self.questions.busy { return; }
        let id = self.artifact_id.clone();
        crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| { settings.artifact_question_drafts.remove(&id); });
        self.questions.synced = None;
        cx.notify();
    }

    pub(crate) fn send_question_batch(&mut self, cx: &mut Context<Self>) {
        let Some(artifact) = self.artifact(cx) else { return; };
        let drafts = self.question_drafts(&artifact, cx);
        if drafts.is_empty() || self.questions.busy { return; }
        let chat = artifact.current_owner_chat_id().map(str::to_owned).filter(|id| self.state.read(cx).chats.iter().any(|row| row.id == *id && !row.archived));
        let Some(chat) = chat else {
            self.notice = Some("The current owner chat is unavailable. Answers remain staged.".into());
            cx.notify();
            return;
        };
        let mut answers: Vec<_> = drafts.iter().map(|(id, answer)| mast_proto::artifacts::PlanQuestionDraft { question_id: id.clone(), answer: answer.clone() }).collect();
        answers.sort_by(|a, b| a.question_id.cmp(&b.question_id));
        let message = format!("Answers for {}: {}", artifact.title, answers.iter().map(|a| format!("{}: {}", a.question_id, a.answer)).collect::<Vec<_>>().join("; "));
        self.questions.busy = true;
        let call = self.state.read(cx).artifact_actions().answer_questions(mast_proto::artifacts::ArtifactAnswerQuestionsParams { artifact_id: artifact.id.clone(), revision: artifact.current_revision, answers });
        let task = cx.spawn(async move |this, cx| {
            let reply = call.await;
            this.update(cx, |this, cx| {
                this.questions.busy = false;
                match reply {
                    Err(error) => this.notice = Some(format!("Couldn't save answers: {}", error.text())),
                    Ok(reply) => {
                        for answer in &reply.answers { this.questions.local.insert(answer.id.clone(), answer.answer.clone()); }
                        this.discard_question_drafts(cx);
                        {
                            let delivery = crate::work_outline::required_actions_view::deliver_user_message(this.state.clone(), chat, message, crate::work_outline::actions::new_operation_id(), cx);
                            let task = cx.spawn(async move |this, cx| { if let Err(error) = delivery.await { this.update(cx, |this, cx| { this.notice = Some(format!("Answers saved, but the chat wasn't told: {error}")); cx.notify(); }).ok(); } });
                            this.questions.tasks.push(task);
                        }
                    }
                }
                cx.notify();
            }).ok();
        });
        self.questions.tasks.push(task);
    }

    pub(super) fn render_question_batch_bar(&self, artifact: &ArtifactView, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let count = self.question_drafts(artifact, cx).len();
        if count == 0 || self.shown_or_current_revision(artifact) != artifact.current_revision { return None; }
        Some(div().flex_none().flex().items_center().gap(px(12.0)).px(px(12.0)).py(px(8.0)).border_t_1().border_color(theme.border).text_color(theme.text)
            .child(format!("{count} answers ready"))
            .child(div().id("question-batch-send").debug_selector(|| "question-batch-send".into()).cursor_pointer().child(format!("Send {count} answers")).on_click(cx.listener(|this, _, _, cx| this.send_question_batch(cx))))
            .child(div().id("question-batch-discard").cursor_pointer().child("Discard").on_click(cx.listener(|this, _, _, cx| this.discard_question_drafts(cx))))
            .into_any_element())
    }

    // ------------------------------------------------------------------
    // Badge row
    // ------------------------------------------------------------------

    /// "N open questions", the drift badge and the changed-since chip.
    pub(super) fn render_badges(
        &mut self,
        artifact: &ArtifactView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let questions = self.plan_questions(artifact.kind);
        let answers = self.answer_map(artifact);
        let answer_rows: Vec<_> = answers
            .iter()
            .map(|(id, text)| mast_proto::ArtifactQuestionAnswer {
                id: id.clone(),
                answer: text.clone(),
                at: i64::MAX,
                ..Default::default()
            })
            .collect();
        // Snapshot answers keep their own times; local-only ones are newer
        // than any revision.
        let snapshot_ids: Vec<&str> = artifact.answers.iter().map(|a| a.id.as_str()).collect();
        let rows: Vec<_> = artifact
            .answers
            .iter()
            .cloned()
            .chain(
                answer_rows
                    .into_iter()
                    .filter(|a| !snapshot_ids.iter().any(|id| id.eq_ignore_ascii_case(&a.id))),
            )
            .collect();
        let open = questions_ui::open_questions_label(&questions, &rows);
        let revision_at = artifact.current().map(|revision| revision.at).unwrap_or(0);
        let drift = questions_ui::drift_badge(
            &questions,
            &rows,
            artifact.current_revision,
            revision_at,
            self.questions.drift.as_ref(),
            crate::artifacts::model::now_ms(),
        );
        let changed = self.questions.changed;
        if open.is_none() && drift.is_none() && changed.is_none() {
            return None;
        }
        let mut row = div()
            .flex_none()
            .px(px(12.0))
            .py(px(5.0))
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .border_b_1()
            .border_color(theme.border);
        if let Some(label) = open {
            row = row.child(
                badge(
                    "artifact-open-questions",
                    label,
                    None,
                    theme.text_muted,
                    theme,
                )
                .into_any_element(),
            );
        }
        if let Some(badge_model) = drift {
            row = row.child(
                badge(
                    "artifact-drift-badge",
                    badge_model.label,
                    Some(badge_model.tooltip),
                    theme.warning,
                    theme,
                )
                .into_any_element(),
            );
        }
        if let Some((from, to)) = changed {
            let note = artifact
                .current()
                .map(|revision| revision.note.clone())
                .filter(|note| !note.trim().is_empty());
            row = row.child(
                badge(
                    "artifact-changed-since",
                    questions_ui::changed_since_label(from, to),
                    note,
                    theme.accent,
                    theme,
                )
                .cursor_pointer()
                .on_click(cx.listener(|this, _, window, cx| this.open_changes(window, cx)))
                .into_any_element(),
            );
        }
        Some(row.into_any_element())
    }
}

fn badge(
    selector: &'static str,
    label: String,
    tooltip: Option<String>,
    color: gpui::Hsla,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let tooltip = tooltip.map(SharedString::from);
    crate::components::browser_parts::artifact_question_badge(selector, label, color, theme)
        .when_some(tooltip, |el, tip| {
            el.tooltip(move |_, cx| cx.new(|_| TextTooltip(tip.clone())).into())
        })
}
