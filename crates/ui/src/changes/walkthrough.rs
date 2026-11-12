//! Presentation and coverage of an ordered review over an existing diff.
use super::{Changes, DiffRow, FileDiff, review_state::hunk_identity};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::{cell::Cell, rc::Rc};

use gpui::{
    App, Context, Entity, FocusHandle, KeyBinding, Render, Subscription, Task, Window, actions,
    div, prelude::*, px,
};
use mast_proto::{
    ArtifactContentEncoding, ArtifactContentParams, ArtifactView, ReviewStep, ReviewTarget,
};

use crate::theme::Theme;

actions!(
    review_walkthrough,
    [
        NextStep,
        PreviousStep,
        NextHunk,
        PreviousHunk,
        MarkReviewed,
        ToggleMode
    ]
);
const KEY_CONTEXT: &str =
    "ReviewWalkthrough && !Input && !Composer && !MessageComposer && !PaletteSearch";

pub(crate) fn bind_keys(cx: &mut App, keymap: &crate::settings::KeymapConfig) {
    super::explanation::bind_keys(cx, keymap);
    use crate::settings::ShortcutId as S;
    for (id, action) in [
        (
            S::ReviewNextStep,
            Box::new(NextStep) as Box<dyn gpui::Action>,
        ),
        (S::ReviewPreviousStep, Box::new(PreviousStep)),
        (S::ReviewNextStepArrow, Box::new(NextStep)),
        (S::ReviewPreviousStepArrow, Box::new(PreviousStep)),
        (S::ReviewNextHunk, Box::new(NextHunk)),
        (S::ReviewPreviousHunk, Box::new(PreviousHunk)),
        (S::ReviewMarkReviewed, Box::new(MarkReviewed)),
        (S::ReviewToggleMode, Box::new(ToggleMode)),
        (S::ExplainDiff, Box::new(super::explanation::ExplainDiff)),
    ] {
        let combo = keymap.get(id);
        if combo.is_empty() {
            continue;
        }
        let combo = crate::settings::platform_combo(combo);
        let combo = if gpui::Keystroke::parse(&combo).is_ok() {
            combo
        } else {
            crate::settings::platform_combo(id.default_combo())
        };
        let predicate = Some(std::rc::Rc::new(
            gpui::KeyBindingContextPredicate::parse(KEY_CONTEXT).unwrap(),
        ));
        if let Ok(binding) = KeyBinding::load(
            &combo,
            action,
            predicate,
            false,
            None,
            &gpui::DummyKeyboardMapper,
        ) {
            cx.bind_keys([binding]);
        }
    }
}

/// Loads the chat's current review artifact and owns its diff for the lifetime
/// of the surface, including while the surface moves between hosts.
pub(crate) struct ReviewHost {
    view: Option<Entity<Walkthrough>>,
    error: Option<String>,
    state: Entity<crate::state::AppState>,
    shell: gpui::WeakEntity<crate::shell::Shell>,
    chat_id: String,
    target: Option<ReviewTarget>,
    _state_observer: Subscription,
    _shell_observer: Option<Subscription>,
    _load: Task<()>,
}

impl ReviewHost {
    pub(crate) fn new(
        state: Entity<crate::state::AppState>,
        chat_id: String,
        artifact: &ArtifactView,
        shell: gpui::WeakEntity<crate::shell::Shell>,
        cx: &mut Context<Self>,
    ) -> Self {
        let changes = cx.new(|cx| {
            let mut changes = Changes::for_chat(state.clone(), chat_id.clone(), cx);
            changes.explanation_shell = Some(shell.clone());
            if changes.scope == super::DiffScope::WorkingTree {
                changes.set_scope(super::DiffScope::Branch, cx);
            }
            changes
        });
        let state_observer = cx.observe(&state, |_, _, cx| cx.notify());
        let shell_observer = shell
            .upgrade()
            .map(|shell| cx.observe(&shell, |_, _, cx| cx.notify()));
        let target_device_id = (state.read(cx).local_device_id.as_deref()
            != Some(artifact.device_id.as_str()))
        .then(|| artifact.device_id.clone());
        let call = state
            .read(cx)
            .artifact_actions()
            .content(ArtifactContentParams {
                artifact_id: artifact.id.clone(),
                revision: None,
                companion: None,
                target_device_id,
            });
        let load = cx.spawn(async move |this, cx| {
            let reply = call.await;
            let _ = this.update(cx, |this, cx| {
                let review = reply.map_err(|error| error.text()).and_then(|reply| {
                    if reply.encoding != ArtifactContentEncoding::Utf8 {
                        return Err("Review content is not UTF-8".to_string());
                    }
                    serde_json::from_str::<ReviewDocument>(&reply.content)
                        .map_err(|error| format!("Invalid review: {error}"))
                });
                match review.and_then(ReviewDocument::into_review) {
                    Ok((target, review)) => {
                        this.target = Some(target.clone());
                        changes.update(cx, |changes, cx| {
                            changes.walkthrough_target = Some(target);
                            changes.ensure_review_state(cx);
                        });
                        this.view = Some(cx.new(|cx| Walkthrough::new(review, changes, cx)))
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        Self {
            view: None,
            error: None,
            state,
            shell,
            chat_id,
            target: None,
            _state_observer: state_observer,
            _shell_observer: shell_observer,
            _load: load,
        }
    }
}

impl Render for ReviewHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let (Some(view), Some(target)) = (&self.view, &self.target) {
            let pr = match target {
                ReviewTarget::PullRequest {
                    repo,
                    number,
                    head_sha,
                    base_sha,
                } => Some(mast_proto::ReviewPullRequest {
                    repo: repo.clone(),
                    number: *number,
                    head_sha: head_sha.clone(),
                    base_sha: base_sha.clone(),
                }),
                ReviewTarget::Branch { .. } => None,
            };
            let cached = self.shell.upgrade().and_then(|shell| {
                pr.as_ref()
                    .and_then(|pr| shell.read(cx).cached_review_head(&self.chat_id, pr))
            });
            let stale = if let (Some(head), Some(pr)) = (cached, pr.as_ref()) {
                (!head.eq_ignore_ascii_case(&pr.head_sha)).then_some(head)
            } else if let Some(pr) = pr {
                let card = crate::artifacts::model::ReviewCard {
                    pr,
                    summary: String::new(),
                    highlights: Vec::new(),
                };
                crate::artifacts::model::review_head_drift(
                    &card,
                    self.state.read(cx).chat_ref_targets(&self.chat_id),
                )
                .map(str::to_owned)
            } else if let ReviewTarget::Branch {
                checkout_path: Some(path),
                branch: Some(branch),
                head_sha,
                ..
            } = target
            {
                let state = self.state.read(cx);
                let head = state
                    .chat_ref_targets(&self.chat_id)
                    .iter()
                    .find(|ref_target| {
                        ref_target.path == *path && ref_target.head_branch == *branch
                    })
                    .and_then(|ref_target| ref_target.head_sha.clone())
                    .or_else(|| {
                        state
                            .selected_execution_member(&self.chat_id)
                            .filter(|member| {
                                member.checkout_path == *path
                                    && member.branch.as_deref() == Some(branch)
                            })
                            .and_then(|member| member.head_sha)
                    });
                head.filter(|head| !head.eq_ignore_ascii_case(head_sha))
            } else {
                None
            };
            view.update(cx, |view, cx| {
                if view.stale_head != stale {
                    view.stale_head = stale;
                    cx.notify();
                }
            });
        }
        div()
            .size_full()
            .when_some(self.view.clone(), |el, view| el.child(view))
            .when(self.view.is_none(), |el| {
                el.child(
                    self.error
                        .clone()
                        .unwrap_or_else(|| "Loading review…".into()),
                )
            })
    }
}

/// Identifies a hunk in the displayed diff, independently of the target kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TourHunk {
    pub file: usize,
    pub hunk: usize,
    pub identity: String,
}

/// ArtifactContent normalizes legacy hashes when the reviewed commits are
/// available. An unmatched hash remains uncovered rather than guessing by line.
fn step_covers_hunk(step: &ReviewStep, file: &FileDiff, hunk: usize) -> bool {
    if step.file != file.path {
        return false;
    }
    let Some(hunk) = file.hunks.get(hunk) else {
        return false;
    };
    let identity = hunk_identity(&file.path, hunk);
    step.hunks
        .iter()
        .any(|anchor| anchor.hunk_hash.as_deref() == Some(identity.as_str()))
}

#[derive(Clone, Debug)]
pub(super) struct TourCoverage {
    pub steps: Vec<Vec<TourHunk>>,
    complete: Vec<bool>,
    pub uncovered: Vec<TourHunk>,
}

impl TourCoverage {
    pub fn new(steps: &[ReviewStep], files: &[FileDiff]) -> Self {
        let mut covered = HashSet::new();
        let complete = steps
            .iter()
            .map(|step| {
                !step.hunks.is_empty()
                    && step.hunks.iter().all(|range| {
                        let mut single = step.clone();
                        single.hunks = vec![range.clone()];
                        files.iter().any(|file| {
                            (0..file.hunks.len())
                                .any(|index| step_covers_hunk(&single, file, index))
                        })
                    })
            })
            .collect();
        let steps = steps
            .iter()
            .map(|step| {
                files
                    .iter()
                    .enumerate()
                    .flat_map(|(file_index, file)| {
                        file.hunks
                            .iter()
                            .enumerate()
                            .filter_map(move |(hunk_index, hunk)| {
                                step_covers_hunk(step, file, hunk_index).then(|| TourHunk {
                                    file: file_index,
                                    hunk: hunk_index,
                                    identity: hunk_identity(&file.path, hunk),
                                })
                            })
                    })
                    .inspect(|hunk| {
                        covered.insert((hunk.file, hunk.hunk));
                    })
                    .collect()
            })
            .collect();
        let uncovered = files
            .iter()
            .enumerate()
            .flat_map(|(file_index, file)| {
                file.hunks.iter().enumerate().filter_map({
                    let covered = &covered;
                    move |(hunk_index, hunk)| {
                        (!covered.contains(&(file_index, hunk_index))).then(|| TourHunk {
                            file: file_index,
                            hunk: hunk_index,
                            identity: hunk_identity(&file.path, hunk),
                        })
                    }
                })
            })
            .collect();
        Self {
            steps,
            complete,
            uncovered,
        }
    }

    pub fn reviewed_count(&self, viewed: &BTreeSet<String>) -> usize {
        self.steps
            .iter()
            .enumerate()
            .filter(|(index, hunks)| {
                self.complete[*index]
                    && !hunks.is_empty()
                    && hunks.iter().all(|hunk| viewed.contains(&hunk.identity))
            })
            .count()
    }
}

/// A target-independent tour component. The owner supplies the same diff and
/// viewed identities used by Changes; no state is persisted in the sidecar.
pub(crate) struct Walkthrough {
    review: TourReview,
    files: Arc<Vec<FileDiff>>,
    coverage: Option<TourCoverage>,
    viewed: BTreeSet<String>,
    seen: HashSet<String>,
    target_selected: bool,
    selected: usize,
    note_expanded: bool,
    note_height: Rc<Cell<f32>>,
    tour: bool,
    stale_head: Option<String>,
    focus: FocusHandle,
    changes: Option<Entity<Changes>>,
    _changes_observer: Option<Subscription>,
    step_input: Option<Entity<crate::composer::ComposerInput>>,
    _step_input_events: Option<Subscription>,
    step_panel: Option<super::explanation::ExplanationPanel>,
    step_task: Option<Task<()>>,
    step_cancelled: Option<Arc<std::sync::atomic::AtomicBool>>,
}

/// Presentation data independent of whether the review targets a PR or branch.
struct TourReview {
    steps: Vec<ReviewStep>,
    highlights: Vec<mast_proto::ReviewHighlight>,
    questions: Vec<serde_json::Value>,
    head: String,
    target_key: String,
}

#[derive(serde::Deserialize)]
struct ReviewDocument {
    target: Option<ReviewTarget>,
    pr: Option<serde_json::Value>,
    steps: Vec<ReviewStep>,
    #[serde(default)]
    highlights: Vec<mast_proto::ReviewHighlight>,
    #[serde(default)]
    questions: Vec<serde_json::Value>,
}

impl ReviewDocument {
    fn into_review(self) -> Result<(ReviewTarget, TourReview), String> {
        let target = match self.target {
            Some(target) => target,
            None => {
                let mut pr = self.pr.ok_or("Review has no target")?;
                pr.as_object_mut()
                    .ok_or("Invalid legacy review target")?
                    .insert("kind".into(), "pr".into());
                serde_json::from_value(pr)
                    .map_err(|error| format!("Invalid legacy review target: {error}"))?
            }
        };
        let head = match &target {
            ReviewTarget::PullRequest { head_sha, .. } | ReviewTarget::Branch { head_sha, .. } => {
                head_sha.clone()
            }
        };
        let target_key = target.state_key();
        Ok((
            target,
            TourReview {
                steps: self.steps,
                highlights: self.highlights,
                questions: self.questions,
                head,
                target_key,
            },
        ))
    }
}

fn note_body() -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(12.0))
        .line_height(crate::typography::ui_rems(16.0))
}

fn review_question_text(question: &serde_json::Value) -> Option<String> {
    question
        .as_str()
        .or_else(|| question.get("question").and_then(|value| value.as_str()))
        .or_else(|| question.get("text").and_then(|value| value.as_str()))
        .map(str::to_owned)
}

impl Walkthrough {
    fn cancel_step_question(&mut self) {
        if let Some(cancelled) = self.step_cancelled.take() {
            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(task) = self.step_task.take() {
            task.detach();
        }
        self.step_panel = None;
    }

    fn ask_step(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.step_input.as_ref() else {
            return;
        };
        let question = input.read(cx).text().trim().to_owned();
        if question.is_empty() {
            return;
        }
        let Some(step) = self.review.steps.get(self.selected) else {
            return;
        };
        let Some(changes) = self.changes.as_ref() else {
            return;
        };
        let changes = changes.read(cx);
        let Some(chat_id) = changes.owner_chat_id_owned() else {
            return;
        };
        let state = changes.state.clone();
        let message = super::explanation::step_question(step, &question);
        self.cancel_step_question();
        self.step_input
            .as_ref()
            .unwrap()
            .update(cx, |input, cx| input.set_text("", cx));
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.step_cancelled = Some(cancelled.clone());
        self.step_panel = Some(super::explanation::ExplanationPanel::Waiting {
            queued: false,
            chat_id: None,
        });
        let send = super::explanation::send_question(state.clone(), chat_id.clone(), message, cx);
        self.step_task = Some(cx.spawn(async move |this, cx| {
            let sent = match send.await {
                Ok(sent) => sent,
                Err(error) => {
                    if !cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = this.update(cx, |this, cx| {
                            if let Some(input) = &this.step_input
                                && input.read(cx).text().is_empty()
                            {
                                input.update(cx, |input, cx| input.set_text(question, cx));
                            }
                            this.step_panel =
                                Some(super::explanation::ExplanationPanel::Error(error));
                            cx.notify();
                        });
                    }
                    return;
                }
            };
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let watch = match this.update(cx, |this, cx| {
                this.step_panel = Some(super::explanation::ExplanationPanel::Waiting {
                    queued: sent.queued,
                    chat_id: Some(chat_id.clone()),
                });
                cx.notify();
                super::explanation::watch_question(
                    state,
                    chat_id.clone(),
                    sent.message_id,
                    cancelled.clone(),
                    cx,
                )
            }) {
                Ok(task) => task,
                Err(_) => return,
            };
            let reply = watch.await;
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.step_panel = Some(match reply {
                    Ok(text) => super::explanation::ExplanationPanel::Answer { chat_id, text },
                    Err(error) => super::explanation::ExplanationPanel::Error(error),
                });
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn new(review: TourReview, changes: Entity<Changes>, cx: &mut Context<Self>) -> Self {
        changes.update(cx, |changes, _| {
            changes.review_walkthrough = true;
            changes.review_tour = true;
        });
        let observer = cx.observe(&changes, |_, _, cx| cx.notify());
        let step_input = cx.new(|cx| {
            crate::composer::ComposerInput::new("Ask about this step", cx).with_single_line()
        });
        let step_events = cx.subscribe(&step_input, |this: &mut Self, _, event, cx| {
            if matches!(
                event,
                crate::composer::ComposerInputEvent::Submitted
                    | crate::composer::ComposerInputEvent::ModifiedSubmitted
            ) {
                this.ask_step(cx);
            }
        });
        Self {
            review,
            files: Arc::new(Vec::new()),
            coverage: None,
            viewed: BTreeSet::new(),
            seen: HashSet::new(),
            target_selected: false,
            selected: 0,
            note_expanded: false,
            note_height: Rc::new(Cell::new(0.0)),
            tour: true,
            stale_head: None,
            focus: cx.focus_handle(),
            changes: Some(changes),
            _changes_observer: Some(observer),
            step_input: Some(step_input),
            _step_input_events: Some(step_events),
            step_panel: None,
            step_task: None,
            step_cancelled: None,
        }
    }

    fn coverage(&mut self) -> &TourCoverage {
        self.coverage
            .get_or_insert_with(|| TourCoverage::new(&self.review.steps, &self.files))
    }

    fn select_step(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.review.steps.len() {
            return;
        }
        if self.selected != index {
            self.cancel_step_question();
            self.note_expanded = false;
            self.note_height.set(0.0);
        }
        self.selected = index;
        let coverage = self.coverage().clone();
        if let (Some(changes), Some(hunk)) = (&self.changes, coverage.steps[index].first()) {
            changes.update(cx, |changes, cx| {
                changes.jump_to_hunk(hunk.file, hunk.hunk, cx)
            });
        }
        cx.notify();
    }

    fn move_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let index = if forward {
            self.selected.saturating_add(1)
        } else {
            self.selected.saturating_sub(1)
        };
        self.select_step(index, cx);
    }

    fn move_hunk(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(changes) = &self.changes {
            changes.update(cx, |changes, cx| changes.step_diff(forward, false, cx));
        }
    }

    fn toggle_mode(&mut self, cx: &mut Context<Self>) {
        self.tour = !self.tour;
        if let Some(changes) = &self.changes {
            changes.update(cx, |changes, cx| {
                changes.review_tour = self.tour;
                cx.notify();
            });
        }
        cx.notify();
    }

    fn mark_current_hunk(&mut self, cx: &mut Context<Self>) {
        if self.tour && !self.target_selected {
            return;
        }
        if let Some(changes) = &self.changes {
            changes.update(cx, |changes, cx| {
                let top = changes
                    .list
                    .logical_scroll_top()
                    .item_ix
                    .min(changes.rows.len().saturating_sub(1));
                let file = changes.rows.get(top).and_then(|row| row.file());
                let hunk = changes
                    .rows
                    .iter()
                    .take(top + 1)
                    .rev()
                    .find_map(|row| match row {
                        DiffRow::HunkHeader {
                            file: candidate,
                            hunk,
                        } if Some(*candidate as usize) == file => {
                            Some((*candidate as usize, *hunk as usize))
                        }
                        _ => None,
                    });
                if let Some((file, hunk)) = hunk {
                    changes.toggle_review_hunk(file, hunk, cx);
                }
            });
        }
    }

    fn mark_step(&mut self, cx: &mut Context<Self>) {
        if !self.target_selected {
            return;
        }
        let coverage = self.coverage().clone();
        let Some(hunks) = coverage.steps.get(self.selected) else {
            return;
        };
        if hunks.is_empty() || !coverage.complete[self.selected] {
            return;
        }
        let mark = !hunks
            .iter()
            .all(|hunk| self.viewed.contains(&hunk.identity));
        if let Some(changes) = &self.changes {
            changes.update(cx, |changes, cx| {
                let mut handled = HashSet::new();
                for hunk in hunks {
                    if handled.insert(&hunk.identity)
                        && self.viewed.contains(&hunk.identity) != mark
                    {
                        changes.toggle_review_hunk(hunk.file, hunk.hunk, cx);
                    }
                }
            });
        }
    }
}

impl Render for Walkthrough {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let controls = self
            .changes
            .as_ref()
            .map(|changes| changes.update(cx, |changes, cx| changes.render_header_controls(cx)));
        if let Some(changes) = &self.changes {
            changes.update(cx, |changes, cx| changes.ensure_content(cx));
            let changes = changes.read(cx);
            if let Some(parsed) = &changes.parsed {
                if !Arc::ptr_eq(&self.files, &parsed.files) {
                    self.files = parsed.files.clone();
                    self.coverage = None;
                }
            } else if !self.files.is_empty() {
                self.files = Arc::new(Vec::new());
                self.coverage = None;
            }
            self.target_selected =
                changes.review_target.as_deref() == Some(self.review.target_key.as_str());
            self.viewed = changes
                .review_state
                .as_ref()
                .filter(|state| self.target_selected && state.target == self.review.target_key)
                .map(|state| state.viewed_hunks.clone())
                .unwrap_or_default();
            self.seen = if self.target_selected {
                changes.seen_hunks.clone()
            } else {
                HashSet::new()
            };
        }
        let coverage = self.coverage().clone();
        let selected_reviewed = coverage.complete.get(self.selected).copied().unwrap_or(false)
            && coverage.steps.get(self.selected).is_some_and(|hunks| !hunks.is_empty() && hunks.iter().all(|hunk| self.viewed.contains(&hunk.identity)));
        let progress = format!(
            "{} / {} reviewed",
            coverage.reviewed_count(&self.viewed),
            self.review.steps.len()
        );
        let mut steps = div().id("review-steps").flex().flex_col().gap_1();
        for (index, step) in self.review.steps.iter().enumerate() {
            let selected = index == self.selected;
            let hunks = &coverage.steps[index];
            let reviewed = coverage.complete[index]
                && !hunks.is_empty()
                && hunks
                    .iter()
                    .all(|hunk| self.viewed.contains(&hunk.identity));
            let seen = !reviewed && hunks.iter().any(|hunk| self.seen.contains(&hunk.identity));
            steps = steps.child(
                div()
                    .id(("review-step", index))
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .when(true, |row| crate::components::walkthrough_step_row(row, &theme, selected))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus, cx);
                        this.select_step(index, cx);
                    }))
                    .child(
                        div()
                            .text_color(if seen {
                                theme.text_faint
                            } else {
                                theme.text_muted
                            })
                            .child(if reviewed {
                                "✓"
                            } else if seen {
                                "·"
                            } else {
                                " "
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(format!(
                                "{}. {}{}",
                                index + 1,
                                step.title,
                                if !self.files.is_empty() && !coverage.complete[index] {
                                    " · Outdated"
                                } else {
                                    ""
                                }
                            )),
                    )
                    .child(
                        div()
                            .text_color(match step.risk {
                                mast_proto::ReviewRisk::High => theme.danger,
                                mast_proto::ReviewRisk::Med => theme.warning,
                                _ => theme.text_muted,
                            })
                            .child("•"),
                    ),
            );
        }
        let mut uncovered = div()
            .id("review-uncovered")
            .debug_selector(|| "review-uncovered".into())
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_color(theme.text_muted).child("Not in the tour"));
        for hunk in &coverage.uncovered {
            let (file, index) = (hunk.file, hunk.hunk);
            uncovered = uncovered.child(
                div()
                    .id(gpui::SharedString::from(format!(
                        "uncovered-hunk-{file}-{index}"
                    )))
                    .cursor_pointer()
                    .when(true, |el| crate::components::review_text_action(el, &theme))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(changes) = &this.changes {
                            changes.update(cx, |changes, cx| changes.jump_to_hunk(file, index, cx));
                        }
                    }))
                    .child(format!(
                        "{}  {}",
                        self.files[file].path, self.files[file].hunks[index].header
                    )),
            );
        }
        let note = self.review.steps.get(self.selected).map(|step| {
            let note_content = crate::components::walkthrough_note_content(
                step.what.clone(), step.why.clone(), step.risk_note.clone(),
                self.note_height.clone(), cx.entity_id(), &theme,
            );
            let clamped_height = f32::from(crate::typography::ui_rems(64.0).to_pixels(window.rem_size()));
            div()
                .id("review-note")
                .debug_selector(|| "review-note".into())
                .flex()
                .flex_col()
                .gap_2()
                .text_size(crate::typography::ui_rems(12.0))
                .when(true, |el| crate::components::walkthrough_guidance(el, &theme, |el| {
                    el.child(format!(
                        "Step {} of {}",
                        self.selected + 1,
                        self.review.steps.len()
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("review-previous-step")
                                    .cursor_pointer()
                                    .when(true, |el| crate::components::review_text_action(el, &theme))
                                    .child(crate::components::review_action_label("Previous", &theme))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.move_step(false, cx)),
                                    ),
                            )
                            .child(
                                div()
                                    .id("review-next-step")
                                    .cursor_pointer()
                                    .when(true, |el| crate::components::review_text_action(el, &theme))
                                    .child(crate::components::review_action_label("Next", &theme))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.move_step(true, cx)),
                                    ),
                            ),
                    )
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .when(true, |el| crate::components::walkthrough_title(el, &theme))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(crate::components::walkthrough_heading(&step.title, &theme)),
                        )
                        .child(
                            div()
                                .id("review-risk")
                                .px_2()
                                .when(true, |el| crate::components::walkthrough_chip(el, &theme))
                                .text_color(theme.text_muted)
                                .child(format!("{:?}", step.risk)),
                        ),
                )
                .child(
                    note_body()
                        .id("review-note-body")
                        .debug_selector(|| "review-note-body".into())
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when(!self.note_expanded, |el| {
                            el.max_h(crate::typography::ui_rems(64.0)).overflow_hidden()
                        })
                        .child(note_content),
                )
                .when(crate::components::walkthrough_note_more_visible(&theme, self.note_expanded, self.note_height.get(), clamped_height), |el| el.child(
                    div()
                        .id("review-note-more")
                        .debug_selector(|| "review-note-more".into())
                        .cursor_pointer()
                        .text_color(theme.text_muted)
                        .when(true, |el| crate::components::walkthrough_note_more(el, &theme))
                        .child(if self.note_expanded { "Less" } else { "More" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.note_expanded = !this.note_expanded;
                            cx.notify();
                        })),
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(step.tags.iter().map(|tag| {
                            div()
                                .px_2()
                                .when(true, |el| crate::components::walkthrough_chip(el, &theme))
                                .text_color(theme.text_muted)
                                .child(tag.clone())
                        })),
                )
                .when(!self.review.highlights.is_empty(), |el| {
                    el.child(
                        div()
                            .text_color(theme.text_muted)
                            .text_size(crate::typography::ui_rems(11.0))
                            .child(crate::components::walkthrough_section_label("HIGHLIGHTS", &theme)),
                    )
                    .children(self.review.highlights.iter().map(|highlight| {
                        div()
                            .text_color(theme.text_muted)
                            .child(highlight.text.clone())
                    }))
                })
                .when(!self.review.questions.is_empty(), |el| {
                    el.child(
                        div()
                            .text_color(theme.text_muted)
                            .text_size(crate::typography::ui_rems(11.0))
                            .child(crate::components::walkthrough_section_label("QUESTIONS", &theme)),
                    )
                    .children(
                        self.review
                            .questions
                            .iter()
                            .filter_map(review_question_text)
                            .map(|question| div().text_color(theme.text_muted).child(question)),
                    )
                })
                .when_some(self.step_input.clone(), |el, input| {
                    el.child(
                        div()
                            .id("review-step-question")
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().child(input))
                            .child(
                                div()
                                    .id("review-step-ask")
                                    .cursor_pointer()
                                    .when(true, |el| crate::components::review_text_action(el, &theme))
                                    .child(crate::components::review_action_label("Ask", &theme))
                                    .on_click(cx.listener(|this, _, _, cx| this.ask_step(cx))),
                            ),
                    )
                })
                .when_some(self.step_panel.as_ref(), |el, panel| {
                    let (label, chat_id, waiting) = match panel {
                        super::explanation::ExplanationPanel::Waiting { queued, chat_id } => (
                            if *queued {
                                "Queued".to_owned()
                            } else {
                                "Waiting for reply…".to_owned()
                            },
                            chat_id.clone(),
                            true,
                        ),
                        super::explanation::ExplanationPanel::Answer { chat_id, text } => {
                            (text.clone(), Some(chat_id.clone()), false)
                        }
                        super::explanation::ExplanationPanel::Error(error) => {
                            (error.clone(), None, false)
                        }
                    };
                    let mut reply = div()
                        .id("review-step-reply")
                        .debug_selector(|| "review-step-reply".into())
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(label);
                    if waiting {
                        reply = reply.child(crate::components::changes_loading(&theme, "step-question-spinner", cx.entity_id(), cx));
                    }
                    if let Some(chat_id) = chat_id {
                        reply = reply.child(
                            div()
                                .id("review-step-open-chat")
                                .cursor_pointer()
                                .when(true, |el| crate::components::review_text_action(el, &theme))
                                .child(crate::components::review_action_label("Open in chat", &theme))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(changes) = &this.changes {
                                        changes.update(cx, |changes, cx| {
                                            changes.open_explanation_chat(chat_id.clone(), cx)
                                        });
                                    }
                                })),
                        );
                    }
                    reply = reply.child(
                        div()
                            .id("review-step-close")
                            .cursor_pointer()
                            .when(true, |el| crate::components::review_text_action(el, &theme))
                            .child(crate::components::review_action_label(if waiting { "Cancel" } else { "Close" }, &theme))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_step_question();
                                cx.notify();
                            })),
                    );
                    el.child(reply)
                })
                .child(
                    div()
                        .id("review-mark-step")
                        .cursor_pointer()
                        .when(true, |el| crate::components::walkthrough_review_action(el, &theme, selected_reviewed))
                        .child(crate::components::review_action_label("Mark reviewed", &theme))
                        .on_click(cx.listener(|this, _, _, cx| this.mark_step(cx))),
                )
        });
        div()
            .id("review-walkthrough")
            .track_focus(&self.focus)
            .key_context("ReviewWalkthrough")
            .on_action(cx.listener(|this, _: &NextStep, _, cx| this.move_step(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousStep, _, cx| this.move_step(false, cx)))
            .on_action(cx.listener(|this, _: &NextHunk, _, cx| this.move_hunk(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousHunk, _, cx| this.move_hunk(false, cx)))
            .on_action(cx.listener(|this, _: &MarkReviewed, _, cx| this.mark_current_hunk(cx)))
            .on_action(cx.listener(|this, _: &ToggleMode, _, cx| this.toggle_mode(cx)))
            .on_action(
                cx.listener(|this, _: &super::explanation::ExplainDiff, _, cx| {
                    if let Some(changes) = &this.changes {
                        changes.update(cx, |changes, cx| changes.explain_focused_hunk(cx));
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text)
            .bg(theme.bg)
            .children(crate::components::walkthrough_empty_decoration(&theme, self.files.is_empty()))
            .children(crate::ui_style::help_card(
                "review-walkthrough",
                "Follow each step to understand the change, then mark it reviewed.",
                &theme,
                cx,
            ))
            .when(!self.target_selected && self.changes.is_some(), |view| {
                view.child(
                    div().flex().flex_col().gap(px(12.0))
                        .text_color(theme.text_muted)
                        .child("Select the review target in Changes to track progress.")
                        .children(crate::components::walkthrough_empty_decoration(&theme, true)),
                )
            })
            .when_some(self.stale_head.clone(), |view, head| {
                let before: String = self.review.head.chars().take(7).collect();
                let after: String = head.chars().take(7).collect();
                view.child(
                    div()
                        .id("review-stale")
                        .debug_selector(|| "review-stale".into())
                        .p_2()
                        .bg(theme.surface_raised)
                        .text_color(theme.warning)
                        .child(format!("Stale review: head moved {before} → {after}")),
                )
            })
            .when_some(controls, |view, controls| {
                view.child(crate::surface_chrome::toolbar(&theme).child(controls))
            })
            .when(true, |view| crate::components::walkthrough_hints(view, &theme, |view| {
                    let keymap = crate::settings::current(cx).keymap;
                    view.child(
                        div().flex().gap(px(6.0)).children(
                            [
                                (crate::settings::ShortcutId::ReviewPreviousStep, "Previous"),
                                (crate::settings::ShortcutId::ReviewNextStep, "Next"),
                                (crate::settings::ShortcutId::ReviewMarkReviewed, "Reviewed"),
                            ]
                            .into_iter()
                            .filter_map(|(id, label)| {
                                crate::work_outline::hints::shortcut_combo(&keymap, id).map(
                                    |combo| {
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(3.0))
                                            .child(crate::work_outline::hints::kbd_chip(
                                                &theme, &combo,
                                            ))
                                            .child(label)
                                    },
                                )
                            }),
                        ),
                    )
                }))
            .child(
                div()
                    .id("review-progress")
                    .debug_selector(|| "review-progress".into())
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(theme.text_muted)
                    .when(true, |el| crate::components::walkthrough_progress(el, &theme))
                    .child(crate::components::walkthrough_progress_label(progress, &theme))
                    .child(
                        div()
                            .id("review-mode")
                            .cursor_pointer()
                            .when(true, |el| crate::components::walkthrough_mode(el, &theme))
                            .child(if self.tour { "Free diff" } else { "Tour" })
                            .on_click(cx.listener(|this, _, window, cx| {
                                window.focus(&this.focus, cx);
                                this.toggle_mode(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .when(self.tour, |row| {
                        row.child(
                            div()
                                .id("review-navigation")
                                .w(px(180.))
                                .min_h_0()
                                .overflow_y_scroll()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .when(true, |el| crate::components::walkthrough_navigation(el, &theme))
                                .child(steps)
                                .child(uncovered),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .gap_3()
                            .when_some(self.changes.clone(), |row, changes| {
                                row.child(div().flex_1().min_w_0().child(changes))
                            })
                            .when(self.tour, |row| {
                                row.child(
                                    div()
                                        .id("review-notes")
                                        .when(self.changes.is_some(), |el| el.w(px(240.)).min_w(px(200.)).max_w(px(280.)))
                                        .when(self.changes.is_none(), |el| el.flex_1().max_w(px(800.)).mx_auto())
                                        .min_h_0()
                                        .overflow_y_scroll()
                                        .when(true, |el| crate::components::walkthrough_notes(el, &theme))
                                        .children(note),
                                )
                            }),
                    ),
            )
    }
}

#[cfg(feature = "snapshot")]
pub(crate) fn snapshot_fixture(cx: &mut App) -> Entity<impl Render + use<>> {
    snapshot_fixture_with_changes(std::env::var_os("MAST_SNAPSHOT_POPULATED_CHANGES").is_some(), cx)
}

#[cfg(feature = "snapshot")]
pub(crate) fn snapshot_fixture_with_changes(populated: bool, cx: &mut App) -> Entity<impl Render + use<>> {
    let document: ReviewDocument = serde_json::from_str(r#"{
        "pr":{"repo":"o/r","number":1,"headSha":"a","baseSha":"b"},
        "steps":[{"id":"s1","title":"Review the data flow","file":"src/main.rs",
        "hunks":[{"newStart":1,"newEnd":1}],"what":"Update the request handler",
        "why":"Keep the view synchronized","risk":"low","riskNote":"Check error handling","tags":[]}],
        "highlights":[],"questions":[]
    }"#).unwrap();
    let (_, review) = document.into_review().unwrap();
    let changes = populated.then(|| super::snapshot_populated_changes(cx));
    let populated = changes.is_some();
    if let Some(changes) = &changes {
        changes.update(cx, |changes, _| changes.review_target = Some(review.target_key.clone()));
    }
    cx.new(|cx| Walkthrough {
        review, files: Arc::new(Vec::new()), coverage: None,
        viewed: BTreeSet::new(), seen: HashSet::new(), target_selected: true,
        selected: 0, note_expanded: false, note_height: Rc::new(Cell::new(0.0)), tour: !populated, stale_head: None,
        focus: cx.focus_handle(), changes, _changes_observer: None,
        step_input: None, _step_input_events: None, step_panel: None,
        step_task: None, step_cancelled: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::parse_patch;
    use mast_proto::ReviewSidecar;

    impl From<ReviewSidecar> for TourReview {
        fn from(sidecar: ReviewSidecar) -> Self {
            Self {
                steps: sidecar.steps,
                highlights: sidecar.highlights,
                questions: sidecar.questions,
                head: sidecar.pr.head_sha,
                target_key: "pr:o/r#1".into(),
            }
        }
    }

    fn fixture() -> (ReviewSidecar, Vec<FileDiff>) {
        let mut review: ReviewSidecar = serde_json::from_str(
            r#"{
          "schema":"mast.review/v1", "pr":{"repo":"o/r","number":1,"headSha":"a","baseSha":"b"},
          "summary":"Summary", "steps":[{"id":"s1","title":"First", "file":"a.rs",
          "hunks":[{"newStart":1,"newEnd":1}], "what":"Change", "why":"Reason",
          "risk":"low", "riskNote":"Check it", "tags":[]}],
          "highlights":[], "questions":[], "suggestedComments":[]
        }"#,
        )
        .unwrap();
        let files = parse_patch(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n@@ -9 +9 @@\n-before\n+after\n",
        );
        review.steps[0].hunks[0].hunk_hash = Some(hunk_identity("a.rs", &files[0].hunks[0]));
        (review, files)
    }

    #[test]
    fn note_body_uses_secondary_text_size() {
        let mut body = note_body();
        let mut secondary = div().text_size(crate::typography::ui_rems(12.0));
        assert_eq!(
            body.style().text.font_size,
            secondary.style().text.font_size
        );
    }

    #[test]
    fn review_questions_accept_string_and_object_text() {
        assert_eq!(
            review_question_text(&serde_json::json!("Why?")),
            Some("Why?".into())
        );
        assert_eq!(
            review_question_text(&serde_json::json!({"question": "How?"})),
            Some("How?".into())
        );
    }

    #[test]
    fn review_document_preserves_highlights_and_questions() {
        let (mut review, _) = fixture();
        review.highlights.push(
            serde_json::from_value(serde_json::json!({
                "kind": "risk", "severity": "high", "text": "Check migration",
                "anchor": {"file": "a.rs", "side": "right", "startLine": 1, "endLine": 1}
            }))
            .unwrap(),
        );
        review.questions.push(serde_json::json!("Is this safe?"));
        let document: ReviewDocument =
            serde_json::from_value(serde_json::to_value(review).unwrap()).unwrap();
        let (_, tour) = document.into_review().unwrap();
        assert_eq!(tour.highlights[0].text, "Check migration");
        assert_eq!(
            review_question_text(&tour.questions[0]).as_deref(),
            Some("Is this safe?")
        );
    }

    #[test]
    fn coverage_and_progress_use_manual_hunk_marks() {
        let (review, files) = fixture();
        let coverage = TourCoverage::new(&review.steps, &files);
        assert_eq!(coverage.steps[0].len(), 1);
        assert_eq!(coverage.uncovered.len(), 1);
        let mut viewed = BTreeSet::new();
        assert_eq!(coverage.reviewed_count(&viewed), 0);
        viewed.insert(coverage.steps[0][0].identity.clone());
        assert_eq!(coverage.reviewed_count(&viewed), 1);
        assert_eq!(coverage.uncovered[0].hunk, 1);
    }

    #[test]
    fn shared_hash_survives_moved_lines_and_rejects_changed_hunks() {
        let (mut review, files) = fixture();
        review.steps[0].hunks[0].hunk_hash = Some(hunk_identity("a.rs", &files[0].hunks[0]));
        let moved = parse_patch(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -50 +50 @@\n-old\n+new\n",
        );
        let changed = parse_patch(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+different\n",
        );
        assert_eq!(TourCoverage::new(&review.steps, &moved).steps[0].len(), 1);
        let coverage = TourCoverage::new(&review.steps, &changed);
        assert!(coverage.steps[0].is_empty());
        assert_eq!(coverage.uncovered.len(), 1);
    }

    #[test]
    fn branch_document_needs_no_placeholder_pr() {
        let document: ReviewDocument = serde_json::from_str(r#"{"target":{"kind":"branch","checkoutPath":"/repo","branch":"feature","headSha":"abc"},"steps":[]}"#).unwrap();
        let (target, review) = document.into_review().unwrap();
        assert!(matches!(target, ReviewTarget::Branch { .. }));
        assert_eq!(review.head, "abc");
    }

    #[test]
    fn missing_anchor_cannot_count_as_reviewed() {
        let (mut review, files) = fixture();
        review.steps[0].hunks.push(mast_proto::ReviewHunk {
            new_start: 50,
            new_end: 51,
            hunk_hash: None,
        });
        let coverage = TourCoverage::new(&review.steps, &files);
        let viewed = coverage.steps[0]
            .iter()
            .map(|hunk| hunk.identity.clone())
            .collect();
        assert_eq!(coverage.reviewed_count(&viewed), 0);
    }

    #[test]
    fn unnormalized_hash_does_not_match_by_line_number() {
        let (mut review, files) = fixture();
        review.steps[0].hunks[0].hunk_hash = Some("legacy-hash".into());
        let coverage = TourCoverage::new(&review.steps, &files);
        assert!(coverage.steps[0].is_empty());
        assert_eq!(coverage.uncovered.len(), 2);
    }

    #[test]
    fn review_shortcut_defaults_are_valid() {
        use crate::settings::ShortcutId as S;
        for id in [
            S::ReviewNextStep,
            S::ReviewPreviousStep,
            S::ReviewNextStepArrow,
            S::ReviewPreviousStepArrow,
            S::ReviewNextHunk,
            S::ReviewPreviousHunk,
            S::ReviewMarkReviewed,
            S::ReviewToggleMode,
        ] {
            assert!(
                gpui::Keystroke::parse(&crate::settings::platform_combo(id.default_combo()))
                    .is_ok(),
                "{id:?}"
            );
        }
    }

    #[gpui::test]
    fn industrial_walkthrough_renders_light_and_dark(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::for_selection(crate::theme::Appearance::Light, "industrial-light", Default::default(), mast_theme::SurfacePreference::Opaque).with_ui_style(crate::ui_style::UiStyle::INDUSTRIAL));
        });
        let (mut review, files) = fixture();
        review.steps[0].what = "Short".into();
        review.steps[0].why.clear();
        review.steps[0].risk_note.clear();
        let (view, vcx) = cx.add_window_view(|_, cx| Walkthrough {
            review: review.into(),
            files: Arc::new(files),
            coverage: None,
            viewed: BTreeSet::new(),
            seen: HashSet::new(),
            target_selected: true,
            selected: 0,
            note_expanded: false,
            note_height: Rc::new(Cell::new(0.0)),
            tour: true,
            stale_head: None,
            focus: cx.focus_handle(),
            changes: None,
            _changes_observer: None,
            step_input: None,
            _step_input_events: None,
            step_panel: None,
            step_task: None,
            step_cancelled: None,
        });
        for (appearance, id) in [
            (crate::theme::Appearance::Light, "industrial-light"),
            (crate::theme::Appearance::Dark, "industrial-dark"),
        ] {
            vcx.update(|window, cx| {
                cx.set_global(
                    Theme::for_selection(
                        appearance,
                        id,
                        Default::default(),
                        mast_theme::SurfacePreference::Opaque,
                    )
                    .with_ui_style(crate::ui_style::UiStyle::INDUSTRIAL),
                );
                window.refresh();
                window.draw(cx).clear();
            });
            assert!(vcx.debug_bounds("review-progress").is_some());
            assert!(vcx.debug_bounds("review-note").is_some());
        }
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("review-note-more").is_none());
        view.update(vcx, |view, cx| { view.review.steps[0].what = "Long note ".repeat(80); cx.notify(); });
        vcx.update(|window, cx| window.draw(cx).clear());
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("review-note-more").is_some());
    }

    #[gpui::test]
    fn walkthrough_renders_step_note_and_coverage(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        let (mut review, files) = fixture();
        review.steps[0].what = "Long note ".repeat(30);
        let (view, vcx) = cx.add_window_view(|_, cx| Walkthrough {
            review: review.into(),
            files: Arc::new(files),
            coverage: None,
            viewed: BTreeSet::new(),
            seen: HashSet::new(),
            target_selected: true,
            selected: 0,
            note_expanded: false,
            note_height: Rc::new(Cell::new(0.0)),
            tour: true,
            stale_head: None,
            focus: cx.focus_handle(),
            changes: None,
            _changes_observer: None,
            step_input: None,
            _step_input_events: None,
            step_panel: None,
            step_task: None,
            step_cancelled: None,
        });
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("review-progress").is_some());
        assert!(vcx.debug_bounds("review-note").is_some());
        let collapsed = vcx.debug_bounds("review-note-body").unwrap().size.height;
        let more = vcx.debug_bounds("review-note-more").unwrap().center();
        vcx.simulate_click(more, gpui::Modifiers::default());
        assert!(view.read_with(vcx, |view, _| view.note_expanded));
        vcx.update(|window, cx| window.draw(cx).clear());
        let expanded = vcx.debug_bounds("review-note-body").unwrap().size.height;
        assert!(expanded > collapsed);
        let more = vcx.debug_bounds("review-note-more").unwrap().center();
        vcx.simulate_click(more, gpui::Modifiers::default());
        assert!(!view.read_with(vcx, |view, _| view.note_expanded));
        assert!(vcx.debug_bounds("review-uncovered").is_some());
        assert!(vcx.debug_bounds("review-stale").is_none());
        view.update(vcx, |view, cx| {
            view.stale_head = Some("def456789".into());
            cx.notify();
        });
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("review-stale").is_some());
        view.update(vcx, |view, cx| {
            view.stale_head = None;
            view.step_panel = Some(super::super::explanation::ExplanationPanel::Answer {
                chat_id: "chat".into(),
                text: "A short answer".into(),
            });
            cx.notify();
        });
        vcx.update(|window, cx| window.draw(cx).clear());
        assert!(vcx.debug_bounds("review-step-reply").is_some());
    }

    #[gpui::test]
    fn step_question_sends_identity_and_hunks_to_review_chat(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let (review, _) = fixture();
        let (view, vcx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| {
                let mut state = crate::state::AppState::new();
                state.work_outline_actions_override =
                    Some(crate::work_outline::WorkOutlineActions::scripted(Arc::new(
                        move |method, params| {
                            recorded.lock().unwrap().push((method.to_owned(), params));
                            Ok(serde_json::json!({"id":"message"}))
                        },
                    )));
                state
            });
            let changes = cx.new(|cx| {
                let mut changes = Changes::new(state, cx);
                changes.owner = Some(super::super::ChangesTarget {
                    chat_id: "review-chat".into(),
                    device_id: "local".into(),
                    cwd: None,
                    checkout_id: None,
                    branch: None,
                    unconfirmed: None,
                    exact_checkout: false,
                    range: None,
                });
                changes
            });
            Walkthrough::new(review.into(), changes, cx)
        });
        view.update(vcx, |view, cx| {
            view.step_input
                .as_ref()
                .unwrap()
                .update(cx, |input, cx| input.set_text("Why now?", cx));
            view.ask_step(cx);
        });
        vcx.run_until_parked();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, mast_rpc::methods::QUEUE_MESSAGE);
        assert_eq!(calls[0].1["chatId"], "review-chat");
        let text = calls[0].1["text"].as_str().unwrap();
        assert!(text.contains("s1: First"));
        assert!(text.contains("Hunks:"));
        assert!(text.ends_with("Why now?"));
    }

    #[gpui::test]
    fn review_keys_follow_the_persisted_binding(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            let mut keymap = crate::settings::KeymapConfig::default();
            keymap.review_next_step = "x".into();
            bind_keys(cx, &keymap);
        });
        let (mut review, files) = fixture();
        let mut second = review.steps[0].clone();
        second.id = "s2".into();
        review.steps.push(second);
        let (view, vcx) = cx.add_window_view(|_, cx| Walkthrough {
            review: review.into(),
            files: Arc::new(files),
            coverage: None,
            viewed: BTreeSet::new(),
            seen: HashSet::new(),
            target_selected: true,
            selected: 0,
            note_expanded: false,
            note_height: Rc::new(Cell::new(0.0)),
            tour: true,
            stale_head: None,
            focus: cx.focus_handle(),
            changes: None,
            _changes_observer: None,
            step_input: None,
            _step_input_events: None,
            step_panel: None,
            step_task: None,
            step_cancelled: None,
        });
        vcx.update(|window, cx| {
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
            window.draw(cx).clear();
        });
        vcx.simulate_keystrokes("n");
        assert_eq!(view.read_with(vcx, |view, _| view.selected), 0);
        vcx.simulate_keystrokes("x");
        assert_eq!(view.read_with(vcx, |view, _| view.selected), 1);
        vcx.simulate_keystrokes("t");
        assert!(!view.read_with(vcx, |view, _| view.tour));
    }

    #[gpui::test]
    fn review_keys_do_not_intercept_the_diff_filter(cx: &mut gpui::TestAppContext) {
        use crate::changes::{
            FileHorizontalState, ParsedDiff, file_list::parse_diff_tree,
            review_state::hunk_identities,
        };
        use std::sync::Arc;
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            bind_keys(cx, &crate::settings::KeymapConfig::default());
        });
        let (review, _) = fixture();
        let (view, vcx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| crate::state::AppState::new());
            let changes = cx.new(|cx| Changes::for_checkout(state, "/repo".into(), cx));
            changes.update(cx, |changes, cx| {
                let patch =
                    "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";
                let (files, origins, submodules, layout) = parse_diff_tree(patch, &[]);
                let horizontal = files.iter().map(FileHorizontalState::new).collect();
                changes.scoped = Some(mast_proto::CheckoutDiff {
                    checkout_id: "checkout".into(),
                    device_id: "device".into(),
                    cwd: "/repo".into(),
                    patch: patch.into(),
                    files: Vec::new(),
                    submodules: Vec::new(),
                    additions: 1,
                    deletions: 1,
                    truncated: false,
                    checksum: "sum".into(),
                    updated_at: chrono::Utc::now(),
                });
                changes.scoped_for = Some("local|||/repo|workingTree|||".into());
                changes.parsed = Some(ParsedDiff {
                    key: "checkout:sum:wt".into(),
                    truncated: false,
                    additions: 1,
                    deletions: 1,
                    file_count: 1,
                    horizontal,
                    hunk_identities: Arc::new(hunk_identities(&files)),
                    files: Arc::new(files),
                    origins: Arc::new(origins),
                    submodules: Arc::new(submodules),
                    layout: Arc::new(layout),
                });
                changes.reflatten(cx);
            });
            let mut view = Walkthrough::new(review.into(), changes, cx);
            view.toggle_mode(cx);
            view
        });
        vcx.update(|window, cx| window.draw(cx).clear());
        let filter = view.read_with(vcx, |view, cx| {
            view.changes.as_ref().unwrap().read(cx).file_filter.clone()
        });
        vcx.update(|window, cx| {
            let focus = gpui::Focusable::focus_handle(filter.read(cx), cx);
            window.focus(&focus, cx);
        });
        vcx.simulate_keystrokes("n v t e");
        assert_eq!(
            filter.read_with(vcx, |input, _| input.text().to_owned()),
            "nvte"
        );
        assert!(!view.read_with(vcx, |view, _| view.tour));
    }
}
