//! Plan questions → Required Actions (Artifacts phase 3 D).
//!
//! `ArtifactQuestionSync::reconcile` runs on the artifact's owning device and
//! keeps the work item's Required Actions in step with the plan's open
//! questions. It is event-driven (see [`ArtifactQuestionSync::spawn_loop`]):
//! once on boot, whenever an outline or artifact row changes (a revision
//! published, an answer or pick recorded, a Required Action created or
//! resolved, here or on another device), right after an answer or pick, and
//! on a slow [`SAFETY_SWEEP`]. Passes are serialised.
//!
//! - one artifact-keyed plan request lists every open question; a revision
//!   updates that request in place, and a free-text reply goes to its origin
//!   Chat without being recorded as an answer to individual questions;
//! - legacy per-question and roll-up requests remain usable. A person resolving
//!   one records the answer it represented, as before; no new legacy request
//!   is created. A legacy request resolves when its questions are no longer open.
//!
//! The origin Chat is the Chat that published the latest revision (else the
//! one that created the artifact) so Reply & resolve reaches it; without a
//! live Chat the request is system-origin. Serialized passes prevent duplicate
//! creates, while a resolved request can be created again.

use std::sync::Arc;
use std::time::Duration;

use mast_proto::artifacts::{ArtifactQuestionAnswer, PlanQuestion, PlanQuestionStatus};
use mast_proto::work_outline::{WorkOutlineActorKind, derive_id};
use mast_proto::work_outline_user_actions::{
    CreateUserActionRequestParams, ResolveUserActionRequestParams, UserActionOriginKind,
    UserActionRequestOrigin, UserActionRequestView, user_action_limits,
};
use mast_proto::{ArtifactView, WorkOutlineMutation};

use crate::artifacts::ArtifactsService;
use crate::work_outline::WorkOutlineService;

/// Registry changes closer together than this share one pass.
const DEBOUNCE: Duration = Duration::from_millis(50);
/// Safety net: every registry write is an event, but a pass can still read a
/// revision's snapshot before its file lands (the row is written first), and a
/// lagged or dropped watch would otherwise leave questions stale until the
/// next change. One pass this often bounds that.
pub const SAFETY_SWEEP: Duration = Duration::from_secs(300);
/// Serialises passes: the background loop, an answer's immediate pass and
/// tests all call `reconcile`, and two concurrent passes would both create a
/// request for the same question.
static PASS: std::sync::Mutex<()> = std::sync::Mutex::new(());
const KEY_PREFIX: &str = "artq:";
/// Dedupe key of a Required Action about an options artifact's choice
/// (`artopt:<artifactId>`); an agent raises it, a pick resolves it.
const OPTION_KEY_PREFIX: &str = "artopt:";
pub(crate) fn approval_key(artifact_id: &str) -> String { format!("artplan:{artifact_id}") }

fn refers_to_plan_approval(request: &UserActionRequestView, artifact: &ArtifactView) -> bool {
    let text = request.text.to_ascii_lowercase();
    let approval = request.kind == Some(mast_proto::UserActionKind::Approve)
        || text.starts_with("approve plan") || text.starts_with("approve the plan");
    approval && (request.links.iter().any(|link| matches!(&link.target, mast_proto::RequestLinkTarget::Artifact { id } if id == &artifact.id))
        || mast_proto::artifacts::artifact_links(request.info.as_deref().unwrap_or_default()).iter().any(|link| link.target == artifact.id))
}

pub(crate) fn approval_info(info: Option<&str>, artifact: &ArtifactView) -> String {
    let body = info.unwrap_or_default().split("\n\nOpen questions:\n").next().unwrap_or_default()
        .lines().filter(|line| !line.starts_with("Plan revision: ") && !line.starts_with("Sheet revision: ")).collect::<Vec<_>>().join("\n");
    let suffix = format!("Plan revision: {}", artifact.current_revision);
    let max = user_action_limits::MAX_REQUEST_INFO_CHARS.saturating_sub(suffix.chars().count() + 1);
    if body.is_empty() { suffix } else { format!("{}\n{suffix}", truncate(&body, max)) }
}

fn approval_question_info(info: Option<&str>, artifact: &ArtifactView, sheet_revision: Option<u32>, questions: &[&PlanQuestion]) -> String {
    let link = format!("[Open plan]({})", artifact.url());
    let detail = info.unwrap_or_default().split("\n\nOpen questions:\n").next().unwrap_or_default()
        .lines().filter(|line| !line.starts_with("Plan revision: ") && !line.starts_with("Sheet revision: ") && *line != link)
        .collect::<Vec<_>>().join("\n");
    let sheet = sheet_revision.map(|revision| format!("\nSheet revision: {revision}")).unwrap_or_default();
    let header = format!("Plan revision: {}{sheet}\n{link}", artifact.current_revision);
    let questions = if questions.is_empty() { String::new() } else {
        format!("\n\nOpen questions:\n{}", questions.iter().map(|q| format!("- {}", one_line(&q.text))).collect::<Vec<_>>().join("\n"))
    };
    let limit = user_action_limits::MAX_REQUEST_INFO_CHARS;
    let question_budget = limit.saturating_sub(header.chars().count());
    let questions = if question_budget == 0 { String::new() } else { truncate(&questions, question_budget) };
    let detail_budget = limit.saturating_sub(header.chars().count() + questions.chars().count() + 1);
    let detail = if detail_budget == 0 || detail.is_empty() { String::new() } else { format!("\n{}", truncate(&detail, detail_budget)) };
    format!("{header}{detail}{questions}")
}
const RESOLVED_IN_ACTIONS: &str = "Resolved in Required Actions";

#[derive(Clone)]
pub struct ArtifactQuestionSync {
    artifacts: ArtifactsService,
    outline: Arc<WorkOutlineService>,
}

pub fn question_key(artifact_id: &str, question_id: &str) -> String {
    format!("{KEY_PREFIX}{artifact_id}:{question_id}")
}

pub fn rollup_key(artifact_id: &str) -> String {
    question_key(artifact_id, "rollup")
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `Q-n` ids at the start of each `info` line of a roll-up request.
fn ids_in_info(info: &str) -> Vec<String> {
    info.lines()
        .filter_map(|line| {
            let id = line.split(':').next()?.trim();
            (id.len() > 2 && id[..2].eq_ignore_ascii_case("q-")).then(|| id.to_ascii_uppercase())
        })
        .collect()
}

impl ArtifactQuestionSync {
    pub fn new(artifacts: ArtifactsService) -> Self {
        let outline = Arc::new(WorkOutlineService::new(artifacts.workspace().clone()));
        Self { artifacts, outline }
    }

    /// One pass over this device's plan artifacts. Returns the number of
    /// Required Action writes (create or resolve) plus recorded answers.
    pub fn reconcile_all(&self) -> usize {
        let ws = self.artifacts.workspace();
        let mut changes = 0;
        for artifact in ws.all_artifacts() {
            if artifact.device_id != self.artifacts.device_id() {
                continue;
            }
            changes += self.reconcile(&artifact.id);
        }
        changes
    }

    /// Reconcile on boot, then whenever the outline or artifact rows change
    /// (the registry change watch, filtered by the outline kinds' generation
    /// so chat and presence churn costs nothing), plus every [`SAFETY_SWEEP`].
    pub fn spawn_loop(&self) {
        let sync = self.clone();
        let ws = self.artifacts.workspace().clone();
        let mut changes = ws.watch_changes();
        tokio::spawn(async move {
            let mut sweep = tokio::time::interval(SAFETY_SWEEP);
            sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            sweep.tick().await; // the boot pass below stands in for the first tick
            let mut seen: Option<u64> = None;
            loop {
                // Read before the pass: our own writes move the marker, so one
                // more pass runs (writing nothing) and the marker then settles.
                let generation = ws.work_outline_kinds_generation();
                if seen != Some(generation) {
                    seen = Some(generation);
                    let sync = sync.clone();
                    let _ = tokio::task::spawn_blocking(move || sync.reconcile_all()).await;
                    continue;
                }
                tokio::select! {
                    changed = changes.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        tokio::time::sleep(DEBOUNCE).await;
                        changes.borrow_and_update();
                    }
                    _ = sweep.tick() => seen = None,
                }
            }
        });
    }

    /// After a pick (or un-pick): resolve the open requests about the choice,
    /// the plan question the manifest names, and `artopt:<id>` requests.
    /// An un-pick resolves nothing (the plan question re-opens by itself).
    pub fn after_pick(&self, artifact_id: &str, picked: bool) -> usize {
        let ws = self.artifacts.workspace();
        let Some(artifact) = ws.artifact(artifact_id) else {
            return 0;
        };
        let Some(item) = artifact.work_item_id.clone() else {
            return 0;
        };
        // The plan question the pick answered resolves on the plan's pass.
        for plan in ws
            .artifacts_of_space(&artifact.space_id)
            .into_iter()
            .filter(|a| a.pinned_as_plan && a.work_item_id.as_deref() == Some(item.as_str()))
        {
            self.reconcile(&plan.id);
        }
        if !picked {
            return 0;
        }
        let key = format!("{OPTION_KEY_PREFIX}{}", artifact.id);
        let snapshot = self.outline.snapshot(&artifact.space_id);
        let open: Vec<UserActionRequestView> = snapshot
            .user_action_requests
            .iter()
            .filter(|r| r.work_item_id == item && r.is_open())
            .filter(|r| r.dedupe_key.as_deref() == Some(key.as_str()))
            .cloned()
            .collect();
        let pick = artifact.picked.as_ref();
        let note = match pick {
            Some(p) => format!("Picked option {} in Mast", p.option_id),
            None => "Picked in Mast".to_owned(),
        };
        open.iter()
            .filter(|r| self.resolve(r, note.clone()))
            .count()
    }

    /// Reconcile one artifact; see the module docs.
    pub fn reconcile(&self, artifact_id: &str) -> usize {
        let _pass = PASS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let ws = self.artifacts.workspace();
        let Some(artifact) = ws.artifact(artifact_id) else {
            return 0;
        };
        let Some(item) = artifact.work_item_id.clone() else {
            return 0;
        };
        if artifact.device_id != self.artifacts.device_id() || artifact.archived_at.is_some() {
            return 0;
        }
        if artifact.kind == mast_proto::ArtifactKind::DecisionSheet {
            return self.reconcile_sheet(&artifact, &item);
        }
        if !(artifact.pinned_as_plan || artifact.role.as_deref() == Some("plan")) {
            return 0;
        }
        let questions = self.artifacts.plan_questions(&artifact);
        let sheet_revision = self.artifacts.plan_sheet_revision(&artifact);
        let snapshot = self.outline.snapshot(&artifact.space_id);
        let origin = self.origin_for(&artifact);
        let approval_key = approval_key(&artifact.id);
        let mut changes = 0;
        let prefix = format!("{KEY_PREFIX}{}:", artifact.id);
        let requests: Vec<&UserActionRequestView> = snapshot
            .user_action_requests
            .iter()
            .filter(|r| r.work_item_id == item)
            .filter(|r| {
                r.dedupe_key
                    .as_deref()
                    .is_some_and(|k| k.starts_with(&prefix))
            })
            .collect();

        // 1. A person's resolution answers the question(s).
        let mut answers = artifact.answers.clone();
        let mut current_answers = self.artifacts.current_plan_answers(&artifact);
        let mut answered_now: Vec<ArtifactQuestionAnswer> = Vec::new();
        for request in requests.iter().filter(|r| !r.is_open()) {
            let Some(resolution) = request.resolution.as_ref() else {
                continue;
            };
            if resolution.by.kind == WorkOutlineActorKind::System {
                continue;
            }
            let key = request.dedupe_key.clone().unwrap_or_default();
            let ids: Vec<String> = if key == rollup_key(&artifact.id) {
                ids_in_info(request.info.as_deref().unwrap_or(""))
            } else {
                vec![key[prefix.len()..].to_ascii_uppercase()]
            };
            let note = resolution
                .note
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or(RESOLVED_IN_ACTIONS);
            for id in ids {
                let open_in_doc = questions
                    .iter()
                    .any(|q| q.id == id && q.status == PlanQuestionStatus::Open);
                let already = current_answers.iter().any(|a| a.id.eq_ignore_ascii_case(&id));
                // Only a resolution that postdates the question's current
                // revision can answer it; older ones belong to a request
                // for an earlier incarnation of the same id.
                if open_in_doc && !already && resolution.at >= self.artifacts.question_opened_at(&artifact, &id).1 {
                    let answer = ArtifactQuestionAnswer {
                        id: id.clone(),
                        answer: truncate(note, mast_proto::artifacts::MAX_ANSWER_CHARS),
                        at: resolution.at,
                        by: resolution.by.clone(),
                        revision: artifact.current_revision,
                    };
                    answers.retain(|existing| !existing.id.eq_ignore_ascii_case(&id));
                    answers.push(answer.clone());
                    current_answers.push(answer.clone());
                    answered_now.push(answer);
                }
            }
        }
        if !answered_now.is_empty() {
            let excess = answers
                .len()
                .saturating_sub(mast_proto::artifacts::MAX_ANSWERS);
            answers.drain(..excess);
            match ws.set_artifact_answers(&artifact.id, &answers) {
                Ok(_) => changes += answered_now.len(),
                Err(error) => {
                    tracing::warn!(artifact = %artifact.id, %error, "artifact answers write failed");
                    answers = artifact.answers.clone();
                    current_answers = self.artifacts.current_plan_answers(&artifact);
                }
            }
        }

        let mut open = mast_proto::artifacts::open_unanswered(&questions, &current_answers);

        // Requests made by older versions remain usable until their own
        // questions are answered; no new per-question requests are created.
        for request in requests.iter().filter(|r| r.is_unresolved()) {
            let key = request.dedupe_key.as_deref().unwrap_or_default();
            let ids = if key == rollup_key(&artifact.id) {
                ids_in_info(request.info.as_deref().unwrap_or_default())
            } else {
                vec![key[prefix.len()..].to_ascii_uppercase()]
            };
            if !open.iter().any(|q| ids.iter().any(|id| q.id.eq_ignore_ascii_case(id))) {
                let id = (key != rollup_key(&artifact.id)).then(|| &key[prefix.len()..]);
                changes += self.resolve(request, self.why_resolved(&artifact, &questions, &answers, id)) as usize;
            }
        }

        // Existing legacy cards remain the source of their questions until
        // resolved; do not introduce a second card for the same decisions.
        let covered: std::collections::HashSet<String> = requests.iter().filter(|r| r.is_unresolved()).flat_map(|request| {
            let key = request.dedupe_key.as_deref().unwrap_or_default();
            if key == rollup_key(&artifact.id) { ids_in_info(request.info.as_deref().unwrap_or_default()) }
            else { vec![key[prefix.len()..].to_ascii_uppercase()] }
        }).collect();
        open.retain(|question| !covered.contains(&question.id.to_ascii_uppercase()));

        let approvals: Vec<_> = snapshot.user_action_requests.iter()
            .filter(|r| r.work_item_id == item && !r.deduped && (r.dedupe_key.as_deref() == Some(approval_key.as_str()) || refers_to_plan_approval(r, &artifact)))
            .collect();
        let existing = approvals.iter().find(|r| r.is_open()).copied().or_else(|| {
            snapshot.user_action_requests.iter().find(|r| r.work_item_id == item && r.is_unresolved() && refers_to_plan_approval(r, &artifact))
        });
        let text = if open.is_empty() {
            format!("Approve plan: {}", one_line(&artifact.title))
        } else {
            format!("{} open question{} in {}", open.len(), if open.len() == 1 { "" } else { "s" }, one_line(&artifact.title))
        };
        let text = truncate(&text, user_action_limits::MAX_REQUEST_TEXT_CHARS);
        if let Some(request) = existing {
            if open.is_empty() && request.kind != Some(mast_proto::UserActionKind::Approve) && !refers_to_plan_approval(request, &artifact) {
                changes += self.resolve(request, "No open questions left in the plan".to_owned()) as usize;
            } else {
                let generated_count = request.text.split_once(' ').is_some_and(|(count, suffix)| {
                    count.parse::<usize>().is_ok() && (suffix.starts_with("open question in ") || suffix.starts_with("open questions in "))
                });
                let text = if generated_count { text } else { request.text.clone() };
                let info = approval_question_info(request.info.as_deref(), &artifact, sheet_revision, &open);
                let mut updated = request.clone();
                updated.dedupe_key = Some(approval_key.clone());
                if request.dedupe_key.as_deref() != Some(approval_key.as_str()) {
                    updated.kind = Some(mast_proto::UserActionKind::Approve);
                }
                if request.text != text || request.info.as_deref() != Some(info.as_str())
                    || request.chat_id != origin.chat_id || request.origin_kind != origin.kind
                    || request.dedupe_key != updated.dedupe_key {
                    changes += self.update(&updated, &origin, text, info) as usize;
                }
            }
        } else if !open.is_empty() {
            let previous = approvals.iter().filter(|r| !r.is_open())
                .max_by_key(|r| r.resolution.as_ref().map_or(0, |resolution| resolution.at));
            let reopened = match previous {
                None => true,
                Some(previous) => {
                    let info = previous.info.as_deref().unwrap_or_default();
                    let plan_reopened = info.lines()
                        .find_map(|line| line.strip_prefix("Plan revision: ").and_then(|value| value.parse::<u32>().ok()))
                        .or_else(|| previous.resolution.as_ref().and_then(|resolution| artifact.revisions.iter().find(|revision| revision.at <= resolution.at).map(|revision| revision.id)))
                        .filter(|revision| *revision < artifact.current_revision)
                        .is_some_and(|revision| open.iter().any(|q| self.artifacts.question_opened_at(&artifact, &q.id).0 > revision));
                    let sheet_reopened = sheet_revision.zip(info.lines().find_map(|line| line.strip_prefix("Sheet revision: ").and_then(|value| value.parse::<u32>().ok())))
                        .is_some_and(|(current, previous)| current > previous);
                    plan_reopened || sheet_reopened
                },
            };
            if reopened {
                let info = approval_question_info(previous.and_then(|r| r.info.as_deref()), &artifact, sheet_revision, &open);
                let request = CreateUserActionRequestParams {
                    work_item_id: item.clone(), origin, text, info: Some(info), dedupe_key: Some(approval_key),
                    operation_id: uuid::Uuid::new_v4().simple().to_string(),
                    options: previous.map_or_else(Vec::new, |r| r.options.clone()),
                    links: previous.map_or_else(Vec::new, |r| r.links.clone()),
                    facts: previous.map_or_else(Vec::new, |r| r.facts.clone()),
                    kind: previous.and_then(|r| r.kind).or(Some(mast_proto::UserActionKind::Ask)),
                    priority: previous.and_then(|r| r.priority),
                };
                changes += self.apply(WorkOutlineMutation::CreateUserActionRequest(request)) as usize;
            }
        }
        changes
    }

    /// Keep one Required Action for all unanswered rows in a sheet.
    fn reconcile_sheet(&self, artifact: &ArtifactView, item: &str) -> usize {
        let Ok(bytes) = self.artifacts.current_bytes(artifact) else { return 0; };
        let Ok(source) = std::str::from_utf8(&bytes) else { return 0; };
        let Ok(sheet) = mast_proto::decision_sheet::DecisionSheet::parse(source) else { return 0; };
        let open: Vec<_> = sheet.decision_rows().into_iter()
            .filter(|row| row.status.state == "open" && !artifact.decisions.iter().any(|decision| decision.row_id == row.id && decision.revision == artifact.current_revision)).collect();
        let key = format!("artsheet:{}", artifact.id);
        let snapshot = self.outline.snapshot(&artifact.space_id);
        let requests: Vec<_> = snapshot.user_action_requests.iter()
            .filter(|request| request.work_item_id == item && request.dedupe_key.as_deref() == Some(key.as_str()))
            .collect();
        let text = truncate(&format!("Decide {} open rows in {}", open.len(), one_line(&artifact.title)), user_action_limits::MAX_REQUEST_TEXT_CHARS);
        let info = format!("[Open sheet]({})\nRevision {}", artifact.url(), artifact.current_revision);
        let mut current = false;
        let mut changes = 0;
        for request in requests.iter().copied().filter(|request| request.is_open()) {
            if open.is_empty() {
                changes += self.resolve(request, "No open rows left in the sheet".to_owned()) as usize;
            } else if !current && !request.deduped {
                current = true;
                let origin = self.origin_for(artifact);
                if request.text != text || request.info.as_deref() != Some(info.as_str())
                    || request.chat_id != origin.chat_id || request.origin_kind != origin.kind {
                    changes += self.update(request, &origin, text.clone(), info.clone()) as usize;
                }
            }
        }
        let resolved: Vec<_> = requests.iter().filter(|request| !request.is_open()).collect();
        let newest_revision = resolved.iter().filter_map(|request| request.info.as_deref().unwrap_or_default().lines()
            .find_map(|line| line.strip_prefix("Revision ").and_then(|value| value.parse::<u32>().ok()))).max();
        let newer_revision = resolved.is_empty() || newest_revision.is_some_and(|revision| artifact.current_revision > revision);
        if !open.is_empty() && !current && newer_revision {
            changes += self.create(item, &self.origin_for(artifact), text, info, key) as usize;
        }
        changes
    }

    fn why_resolved(
        &self,
        artifact: &ArtifactView,
        questions: &[PlanQuestion],
        answers: &[ArtifactQuestionAnswer],
        id: Option<&str>,
    ) -> String {
        let Some(id) = id else {
            return "No open questions left in the plan".to_owned();
        };
        match questions.iter().find(|q| q.id == id) {
            None => format!(
                "{id} was removed from the plan (rev {})",
                artifact.current_revision
            ),
            Some(q) if q.status == PlanQuestionStatus::Resolved => {
                format!(
                    "{id} is resolved in the plan (rev {})",
                    artifact.current_revision
                )
            }
            Some(_) if answers.iter().any(|a| a.id.eq_ignore_ascii_case(id)) => {
                let answer = answers.iter().find(|a| a.id.eq_ignore_ascii_case(id));
                let text = answer.map(|a| one_line(&a.answer)).unwrap_or_default();
                truncate(
                    &format!("Answered in Mast: {text}"),
                    user_action_limits::MAX_NOTE_CHARS,
                )
            }
            Some(q) if q.status == PlanQuestionStatus::Answered => {
                format!("{id} is marked answered in the plan")
            }
            Some(_) => format!("{id} is no longer a blocking question"),
        }
    }

    /// The Chat that published the latest revision, when it is live and in
    /// the artifact's Space; else the artifact's first publisher; else system.
    fn origin_for(&self, artifact: &ArtifactView) -> UserActionRequestOrigin {
        let ws = self.artifacts.workspace();
        if let Some(chat_id) = artifact.current_owner_chat_id().map(str::to_owned) {
            let live = ws.chat(&chat_id).ok().flatten().is_some_and(|c| {
                !c.archived && c.space_id.as_deref() == Some(artifact.space_id.as_str())
            });
            if live {
                return UserActionRequestOrigin {
                    kind: UserActionOriginKind::Chat,
                    chat_id,
                    message_id: None,
                    tool_call_id: None,
                };
            }
        }
        UserActionRequestOrigin {
            kind: UserActionOriginKind::System,
            chat_id: String::new(),
            message_id: None,
            tool_call_id: None,
        }
    }

    fn create(
        &self,
        item: &str,
        origin: &UserActionRequestOrigin,
        text: String,
        info: String,
        dedupe_key: String,
    ) -> bool {
        // A resolved request stays resolved. A fresh operation lets the same
        // question reopen even if its content and revision have not changed.
        let info = truncate(&info, user_action_limits::MAX_REQUEST_INFO_CHARS);
        let dedupe_key = truncate(&dedupe_key, user_action_limits::MAX_DEDUPE_KEY_CHARS);
        let mutation =
            WorkOutlineMutation::CreateUserActionRequest(CreateUserActionRequestParams {
            options: Vec::new(),
            links: Vec::new(),
            facts: Vec::new(),
            kind: Some(mast_proto::work_outline_user_actions::UserActionKind::Ask),
            priority: None,
                work_item_id: item.to_owned(),
                origin: origin.clone(),
                text,
                info: Some(info),
                dedupe_key: Some(dedupe_key),
                operation_id: uuid::Uuid::new_v4().simple().to_string(),
            });
        self.apply(mutation)
    }

    fn update(&self, request: &UserActionRequestView, origin: &UserActionRequestOrigin, text: String, info: String) -> bool {
        let update = mast_proto::UpdateUserActionRequestParams {
            user_action_request_id: request.id.clone(),
            request: CreateUserActionRequestParams {
                work_item_id: request.work_item_id.clone(), origin: origin.clone(), text, info: Some(info),
                dedupe_key: request.dedupe_key.clone(), operation_id: uuid::Uuid::new_v4().simple().to_string(),
                options: request.options.clone(), links: request.links.clone(), facts: request.facts.clone(),
                kind: request.kind, priority: request.priority,
            },
        };
        self.apply(WorkOutlineMutation::UpdateUserActionRequest(update))
    }

    fn resolve(&self, request: &UserActionRequestView, note: String) -> bool {
        self.apply(WorkOutlineMutation::ResolveUserActionRequest(
            ResolveUserActionRequestParams {
                outcome: None,
                values: None,
                via: None,
                user_action_request_id: request.id.clone(),
                note: Some(truncate(&note, user_action_limits::MAX_NOTE_CHARS)),
                answer: None,
                operation_id: derive_id(&format!("artq-resolve\0{}", request.id), "artq"),
            },
        ))
    }

    fn apply(&self, mutation: WorkOutlineMutation) -> bool {
        let actor = self.outline.system_actor();
        match self.outline.mutate_as(&mutation, &actor) {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(
                    code = error.code.as_str(),
                    "artifact question sync: write failed"
                );
                false
            }
        }
    }
}
