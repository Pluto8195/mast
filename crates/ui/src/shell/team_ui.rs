//! The side panel's Team tab: one row per worker of an orchestrating chat,
//! a roll-up line on top, and the Item tab's compact "top workers" block. Both
//! read one [`TeamFacts`] sheet (`crate::team`), so they cannot disagree.
//!
//! ⌥⌘T opens the tab from anywhere; a plain `t` does the same while the side
//! panel itself (not a text field inside it) holds focus. Both do nothing for
//! a chat without workers.
use super::*;
use crate::components::shell_parts::team_button;

use crate::shell::chat_item_ui::{ChipTone, chip_palette};
use crate::team::{
    ITEM_TAB_WORKERS, SUGGEST_ARCHIVE_AT, TeamFacts, TeamInput, WorkerFacts, WorkerPr,
    team,
};
use crate::tracker_lifecycle::{LifecycleRow, LifecycleStage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
enum TrackerFilter {
    #[default]
    All,
    NeedsYou,
    Stage(LifecycleStage),
}

#[derive(Clone)]
struct TrackerSourceObservation {
    chat_id: String,
    space_id: String,
    snapshot_revision: u64,
    account: String,
    account_revision: u64,
    project_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TrackerCleanup {
    LinkIssue { item_id: String, identifier: String },
    PossibleDuplicate { item_id: String, identifier: String },
    StaleRequest { request_id: String },
}

#[derive(Default)]
pub(super) struct TrackerState {
    key: Option<(String, Option<u64>, u64, u64, u64)>,
    chat_scope: Option<(String, u64, Option<String>, bool)>,
    health_key: Option<(String, String, u64, u64, u64, u64)>,
    health_task: Option<Task<()>>,
    health_watch_task: Option<Task<()>>,
    health_item_prs: std::collections::HashMap<String, Vec<mast_engine::pr_ticket_cache::PrStatus>>,
    health_entries: Vec<mast_engine::work_outline_health::HealthEntry>,
    local_cleanup: Vec<TrackerCleanup>,
    health_incomplete: bool,
    health_expanded: std::collections::HashSet<String>,
    health_targets: std::sync::Arc<Vec<(String, String)>>,
    health_titles: std::collections::HashMap<String, String>,
    health_completed: std::collections::HashSet<String>,
    health_issue_urls: std::collections::HashMap<String, String>,
    health_link_url: Option<String>,
    facts: std::sync::Arc<TeamFacts>,
    all_workers: std::sync::Arc<TeamFacts>,
    not_started: Vec<crate::board::BoardItem>,
    completed_items: Vec<crate::board::BoardItem>,
    unassigned_groups: std::collections::HashMap<(TrackerFilter, LifecycleStage), Vec<crate::board::BoardItem>>,
    unassigned_rows: std::collections::HashMap<String, LifecycleRow>,
    unassigned_prs: std::collections::HashMap<String, mast_engine::pr_ticket_cache::PrStatus>,
    unassigned_pending: Vec<crate::board::BoardItem>,
    linear_completed_items: std::collections::HashSet<String>,
    other_chats: Vec<WorkerFacts>,
    additional_chats: std::collections::HashMap<String, Vec<WorkerFacts>>,
    expanded_chats: std::collections::HashSet<String>,
    grouped_rows: std::collections::HashMap<(TrackerFilter, LifecycleStage), Vec<usize>>,
    stale_rows: std::collections::HashMap<TrackerFilter, Vec<usize>>,
    lifecycle_rows: std::collections::HashMap<String, LifecycleRow>,
    linear_projects: std::collections::HashMap<(String, String, u64), Vec<mast_proto::linear::LinearIssueSummary>>,
    linear_more: std::collections::HashMap<(String, String, u64), bool>,
    linear_loading: std::collections::HashSet<(String, String, u64)>,
    linear_errors: std::collections::HashMap<(String, String, u64), crate::linear::client::LinearUiError>,
    linear_sync_task: Option<Task<()>>,
    linear_project_linked: bool,
    can_link_linear_project: bool,
    available_project_context: Option<(String, String, String, u64)>,
    project_picker: Option<(String, String, String, u64)>,
    project_choices: Option<std::sync::Arc<Vec<mast_proto::linear::LinearProjectSummary>>>,
    project_picker_error: Option<String>,
    project_linking: bool,
    source_observation: Option<TrackerSourceObservation>,
    source_label: String,
    source_notice: Option<String>,
    other_expanded: bool,
    stale_expanded: bool,
    done_expanded: bool,
    pub(super) counts: [usize; 6],
    /// Chats whose link statuses the last rebuild wanted; later replies top up from this.
    link_ids: Vec<String>,
    needs_you: usize,
    metadata: std::collections::HashMap<String, (String, String, Option<String>)>,
    requests: std::collections::HashMap<String, String>,
    stalled: std::collections::HashMap<String, (String, bool)>,
    todos: std::collections::HashMap<String, mast_proto::TodoProgress>,
    review_progress: std::collections::HashMap<(String, Option<String>), (usize, usize)>,
    review_targets: std::collections::HashMap<String, (String, Option<String>)>,
    review_generation: u64,
    review_task: Option<Task<()>>,
    filter: TrackerFilter,
    wake_settings_open: bool,
    #[cfg(test)]
    pub(super) rebuilds: usize,
}

fn tracker_matches(row: &LifecycleRow, filter: TrackerFilter) -> bool {
    match filter {
        TrackerFilter::All => true,
        TrackerFilter::NeedsYou => row.needs_you,
        TrackerFilter::Stage(stage) => row.stage == stage,
    }
}

fn tracker_sync_revision(value: &serde_json::Value, initial: bool, seen: &mut Option<u64>) -> bool {
    let Some(revision) = value.as_u64() else { return false; };
    if initial && seen.is_some_and(|previous| revision < previous) { *seen = None; }
    if revision == 0 || *seen == Some(revision) { return false; }
    *seen = Some(revision);
    true
}

fn tracker_headline(ticket: Option<&crate::chat_metadata::HeaderTicket>, fallback: &str) -> String {
    ticket.and_then(|ticket| ticket.title.as_deref()).filter(|title| !title.is_empty()).unwrap_or(fallback).to_owned()
}

fn stale_review_request(request: &mast_proto::UserActionRequestView) -> bool {
    let text = request.text.trim_start().to_ascii_lowercase();
    request.is_unresolved() && (request.effective_kind() == mast_proto::work_outline_user_actions::UserActionKind::Handoff
        || text.starts_with("ready for review") || text.starts_with("ready to merge"))
}

fn tracker_todo_label(progress: mast_proto::TodoProgress) -> String {
    format!("{}/{}", progress.done, progress.total)
}

fn unassigned_lifecycle(item_completed: bool, linear_completed: Option<bool>, prs: &[mast_engine::pr_ticket_cache::PrStatus]) -> LifecycleRow {
    let prs: Vec<_> = prs.iter().map(|pr| crate::tracker_lifecycle::PullRequestFacts {
        state_known: true,
        open: pr.state.eq_ignore_ascii_case("open"),
        merged: pr.state.eq_ignore_ascii_case("merged"),
        approved: !pr.is_draft && pr.review_decision.as_deref() == Some("APPROVED"),
        changes_requested: pr.review_decision.as_deref() == Some("CHANGES_REQUESTED"),
        checks_passing: pr.checks == Some(mast_engine::pr_ticket_cache::ChecksStatus::Passing),
    }).collect();
    crate::tracker_lifecycle::derive_ticket_lifecycle(&[], &prs, Some(item_completed), linear_completed)
}

fn tracker_select_rows(facts: &mut TeamFacts, snapshot: Option<&mast_proto::WorkOutlineSnapshot>, chat_id: &str) -> (Vec<WorkerFacts>, std::collections::HashMap<String, Vec<WorkerFacts>>) {
    let mut other_chats = Vec::new();
    let mut additional_chats = std::collections::HashMap::new();
    if let Some(snapshot) = snapshot {
        let root = crate::chat_work_context::assigned_item(snapshot, chat_id);
        let mut assignments = std::collections::HashMap::new();
        for item in &snapshot.items {
            for id in &item.chat_ids { assignments.insert(id.as_str(), item); }
        }
        facts.workers.sort_by_key(|worker| {
            let rank = match worker.state { crate::team::WorkerState::Waiting => 0, crate::team::WorkerState::Working => 1, _ => 2 };
            (worker.archived, rank, std::cmp::Reverse(worker.last_active))
        });
        let mut selected = std::collections::HashMap::<String, usize>::new();
        let mut rows: Vec<WorkerFacts> = Vec::new();
        for worker in facts.workers.drain(..) {
            let Some(item) = assignments.get(worker.chat_id.as_str()) else { other_chats.push(worker); continue; };
            if Some(item.id.as_str()) == root || !item.child_ids.is_empty() { other_chats.push(worker); continue; }
            if let Some(&index) = selected.get(&item.id) {
                for pr in &worker.prs { if !rows[index].prs.iter().any(|existing| existing.url == pr.url) { rows[index].prs.push(pr.clone()); } }
                additional_chats.entry(rows[index].chat_id.clone()).or_insert_with(Vec::new).push(worker);
            } else {
                selected.insert(item.id.clone(), rows.len());
                rows.push(worker);
            }
        }
        rows.sort_by_key(|worker| (worker.tracker_group(), std::cmp::Reverse(worker.last_active), worker.chat_id.clone()));
        facts.workers = rows;
    }
    (other_chats, additional_chats)
}

fn tracker_reconcile_project(
    snapshot: &mast_proto::WorkOutlineSnapshot,
    root: &str,
    items: &[(crate::board::BoardItem, bool)],
    workers: &TeamFacts,
    issues: &[mast_proto::linear::LinearIssueSummary],
) -> (TeamFacts, Vec<WorkerFacts>, std::collections::HashMap<String, Vec<WorkerFacts>>, Vec<crate::board::BoardItem>, std::collections::HashMap<String, (Option<bool>, Option<bool>)>) {
    use crate::tracker_tickets::{BranchChat, LocalTicket, reconcile_tickets};
    let item_map: std::collections::HashMap<_, _> = items.iter().map(|(item, _)| (item.id.as_str(), item)).collect();
    let worker_map: std::collections::HashMap<_, _> = workers.workers.iter().enumerate().map(|(index, worker)| (worker.chat_id.as_str(), (index, worker))).collect();
    let snapshot_items: std::collections::HashMap<_, _> = snapshot.items.iter().map(|item| (item.id.as_str(), item)).collect();
    let assignments: std::collections::HashMap<_, _> = snapshot.items.iter().flat_map(|item| item.chat_ids.iter().map(move |id| (id.as_str(), item))).collect();
    let mut references = std::collections::HashMap::<&str, &mast_proto::WorkItemExternalReferenceView>::new();
    for reference in snapshot.external_references.iter().filter(|reference| reference.provider == mast_proto::linear::LINEAR_PROVIDER
        && reference.reference_type == mast_proto::linear::LINEAR_ISSUE_REFERENCE) {
        references.entry(reference.work_item_id.as_str()).and_modify(|prior| {
            if (reference.created_at, &reference.id) < (prior.created_at, &prior.id) { *prior = reference; }
        }).or_insert(reference);
    }
    let local: Vec<_> = items.iter().filter_map(|(item, _)| {
        let source = snapshot_items.get(item.id.as_str())?;
        Some(LocalTicket { item_id: item.id.clone(), issue_identifier: references.get(item.id.as_str()).map(|reference| reference.external_id.clone())
            .or_else(|| crate::tracker_tickets::title_issue_identifier(&item.title).filter(|id| issues.iter().any(|issue| issue.identifier == *id)).map(str::to_owned)),
            chat_ids: source.chat_ids.iter().filter(|id| worker_map.get(id.as_str()).is_some_and(|(_, worker)| !worker.archived)).cloned().collect() })
    }).collect();
    let branches: Vec<_> = workers.live().filter(|worker| assignments.get(worker.chat_id.as_str())
        .is_none_or(|item| item.id != root && item.child_ids.is_empty()))
        .filter_map(|worker| Some(BranchChat { chat_id: worker.chat_id.clone(), branch: worker.branch.clone()? })).collect();
    let identifiers: Vec<_> = issues.iter().map(|issue| issue.identifier.clone()).collect();
    let issue_map: std::collections::HashMap<_, _> = issues.iter().map(|issue| (issue.identifier.to_ascii_uppercase(), issue)).collect();
    let mut result = TeamFacts::default();
    let mut additional = std::collections::HashMap::new();
    let mut empty = Vec::new();
    let mut completion = std::collections::HashMap::new();
    let mut selected = std::collections::HashSet::new();
    for row in reconcile_tickets(&identifiers, &local, &branches) {
        let issue = row.issue_identifier.as_ref().and_then(|id| issue_map.get(&id.to_ascii_uppercase()).copied());
        let mut chats: Vec<_> = row.chat_ids.iter().filter_map(|id| worker_map.get(id.as_str()).copied()).collect();
        chats.sort_by_key(|(index, _)| *index);
        if let Some((_, first)) = chats.first().copied() {
            let mut primary = first.clone();
            selected.insert(primary.chat_id.clone());
            let mut others = Vec::new();
            for (_, worker) in chats.into_iter().skip(1) {
                selected.insert(worker.chat_id.clone());
                for pr in &worker.prs { if !primary.prs.iter().any(|existing| existing.url == pr.url) { primary.prs.push(pr.clone()); } }
                others.push(worker.clone());
            }
            if let Some(issue) = issue {
                primary.ticket = Some(crate::chat_metadata::HeaderTicket { identifier: issue.identifier.clone(), title: Some(issue.title.clone()),
                    url: Some(issue.url.clone()), state: Some(issue.state.name.clone()) });
            } else if let Some(identifier) = &row.issue_identifier {
                let reference = row.item_ids.iter().find_map(|id| references.get(id.as_str()).copied());
                primary.ticket = Some(crate::chat_metadata::HeaderTicket { identifier: identifier.clone(),
                    title: reference.and_then(|reference| reference.cached_title.clone()),
                    url: reference.map(|reference| reference.url.clone()).filter(|url| !url.is_empty()), state: None });
            } else {
                primary.ticket = row.item_ids.iter().find_map(|id| item_map.get(id.as_str()))
                    .and_then(|item| crate::tracker_tickets::title_issue_identifier(&item.title))
                    .map(|identifier| crate::chat_metadata::HeaderTicket { identifier: identifier.into(), title: None, url: None, state: None });
            }
            let item_done = (!row.item_ids.is_empty()).then(|| row.item_ids.iter().any(|id|
                snapshot_items.get(id.as_str()).is_some_and(|item| item.effective_status == mast_proto::WorkItemEffectiveStatus::Completed)));
            completion.insert(primary.chat_id.clone(), (item_done, issue.map(|issue| issue.state.state_type.is_closed())));
            if !others.is_empty() { additional.insert(primary.chat_id.clone(), others); }
            result.workers.push(primary);
        } else {
            let mut item = row.item_ids.first().and_then(|id| item_map.get(id.as_str()).copied()).cloned().unwrap_or_else(|| crate::board::BoardItem {
                id: format!("linear:{}", row.issue_identifier.as_deref().unwrap_or_default()), title: String::new(),
                status: crate::board::BoardStatus::Todo, updated_at: 0, created_at: 0, worker: None, worker_title: None,
                report: None, ticket: None, request_id: None,
            });
            if let Some(issue) = issue {
                item.title = issue.title.clone();
                item.ticket = Some(issue.identifier.clone());
                if issue.state.state_type.is_closed() { item.status = crate::board::BoardStatus::Done; }
            } else {
                item.ticket = row.issue_identifier.clone().or_else(|| crate::tracker_tickets::title_issue_identifier(&item.title).map(str::to_owned));
            }
            if row.item_ids.iter().any(|id| item_map.get(id.as_str()).is_some_and(|item| item.status == crate::board::BoardStatus::Done)) {
                item.status = crate::board::BoardStatus::Done;
            }
            empty.push(item);
        }
    }
    let other = workers.workers.iter().filter(|worker| !worker.archived && !selected.contains(&worker.chat_id)).cloned().collect();
    (result, other, additional, empty, completion)
}

fn tracker_primary_pr(prs: &[WorkerPr]) -> Option<&WorkerPr> {
    prs.iter().min_by_key(|pr| if pr.state.eq_ignore_ascii_case("open") { 0 } else if pr.state.eq_ignore_ascii_case("merged") { 1 } else if pr.state.eq_ignore_ascii_case("closed") { 2 } else { 3 })
}

fn tracker_pr_reference(pr: &WorkerPr) -> String {
    crate::chat_work_context::parse_pr_url(&pr.url).map(|reference| format!("{}#{}", reference.repo, reference.number))
        .unwrap_or_else(|| if pr.number > 0 { format!("#{}", pr.number) } else { pr.url.clone() })
}

fn reviewed_files_at_head(state: Option<mast_proto::DiffReviewState>, head: Option<&str>) -> usize {
    state.filter(|state| head.is_some_and(|head| state.head_sha.as_deref().is_some_and(|saved| saved.eq_ignore_ascii_case(head))))
        .map_or(0, |state| state.viewed_files.len())
}

fn tracker_stage_color(stage: LifecycleStage, index: usize, theme: &Theme) -> gpui::Hsla {
    if stage == LifecycleStage::NotStarted || index > stage.index() { theme.border }
    else if stage == LifecycleStage::Done || index < stage.index() { theme.success }
    else { theme.accent }
}

fn tracker_stage_track(id: &str, stage: LifecycleStage, theme: &Theme) -> AnyElement {
    let labels = ["Not started", "Building", "Code review", "PR review", "Ready to merge", "Done"];
    let mut track = div().flex().items_center().gap(px(Theme::SPACE_XS));
    for (index, label) in labels.into_iter().enumerate() {
        let lit = stage != LifecycleStage::NotStarted && index <= stage.index();
        let color = tracker_stage_color(stage, index, theme);
        let tip = SharedString::from(format!("{label}: {}", if lit { "observed" } else { "not observed" }));
        track = track.child(crate::components::shell_parts::tracker_stage_indicator(SharedString::from(format!("tracker-stage-{id}-{index}")), color)
            .debug_selector(move || format!("tracker-stage-{label}-{lit}"))
            .tooltip(move |_, cx| cx.new(|_| SurfaceTabTooltip { text: tip.clone() }).into()));
    }
    track = track.child(crate::components::shell_parts::tracker_stage_label(theme, stage.label()));
    track.into_any_element()
}

#[cfg(test)]
mod tracker_tests {
    use super::*;
    use crate::team::WorkerState;

    #[test]
    fn stale_review_classification_preserves_questions() {
        let request = |text: &str, kind: &str| -> mast_proto::UserActionRequestView {
            serde_json::from_value(serde_json::json!({ "id":"request", "text":text, "kind":kind })).unwrap()
        };
        assert!(stale_review_request(&request("Ready for review: example", "ask")));
        assert!(stale_review_request(&request("Ready to merge: example", "ask")));
        assert!(stale_review_request(&request("Please review", "handoff")));
        assert!(!stale_review_request(&request("How should holidays be loaded?", "ask")));
    }

    #[test]
    fn stage_track_uses_theme_colors() {
        for (appearance, variant) in [
            (crate::theme::Appearance::Light, "mast-light"), (crate::theme::Appearance::Dark, "mast-dark"),
            (crate::theme::Appearance::Light, "industrial-light"), (crate::theme::Appearance::Dark, "industrial-dark"),
            (crate::theme::Appearance::Light, "horizon-light"), (crate::theme::Appearance::Dark, "horizon-dark"),
        ] {
            let theme = Theme::for_selection(appearance, variant, mast_theme::AccentSelection::ThemeDefault, mast_theme::SurfacePreference::default());
            for index in 0..6 {
                assert_eq!(tracker_stage_color(LifecycleStage::NotStarted, index, &theme), theme.border);
                assert_eq!(tracker_stage_color(LifecycleStage::Done, index, &theme), theme.success);
                assert_eq!(tracker_stage_color(LifecycleStage::PrReview, index, &theme),
                    if index < 3 { theme.success } else if index == 3 { theme.accent } else { theme.border });
            }
        }
    }

    fn worker(id: &str, state: WorkerState) -> WorkerFacts {
        WorkerFacts { chat_id: id.into(), title: id.into(), work_item_title: Some(id.into()), state, last_active: None, activity: None,
            branch: None, prs: Vec::new(), ticket: None, diff: None, ready: None, archived: false, archivable: None }
    }

    #[test]
    fn tracker_filters_by_lifecycle_and_needs_you() {
        let worker = worker("first", WorkerState::Waiting);
        let row = crate::team::ticket_lifecycle(&[&worker], Some(false), None, false, chrono::Utc::now());
        assert!(tracker_matches(&row, TrackerFilter::NeedsYou));
        assert!(tracker_matches(&row, TrackerFilter::Stage(LifecycleStage::Building)));
        assert!(!tracker_matches(&row, TrackerFilter::Stage(LifecycleStage::Done)));
    }

    #[test]
    fn unassigned_ticket_uses_its_pr_even_without_a_chat() {
        let mut pr: mast_engine::pr_ticket_cache::PrStatus = serde_json::from_value(serde_json::json!({
            "number": 3, "state":"OPEN", "isDraft":false, "reviewDecision":"REVIEW_REQUIRED", "reviewers":[],
            "mergeable":"mergeable", "hasReviewerRequested":false
        })).unwrap();
        assert_eq!(unassigned_lifecycle(false, Some(true), &[pr.clone()]).stage, LifecycleStage::PrReview);
        pr.review_decision = Some("APPROVED".into());
        pr.checks = Some(mast_engine::pr_ticket_cache::ChecksStatus::Passing);
        assert_eq!(unassigned_lifecycle(false, Some(true), &[pr.clone()]).stage, LifecycleStage::PrReview);
        assert_eq!(unassigned_lifecycle(false, None, &[pr.clone()]).stage, LifecycleStage::ReadyToMerge);
        pr.state = "MERGED".into();
        assert!(unassigned_lifecycle(false, Some(false), &[pr]).merged_with_open_ticket);
    }

    #[test]
    fn tracker_sync_ignores_duplicates_and_accepts_engine_restarts() {
        let mut seen = None;
        assert!(!tracker_sync_revision(&serde_json::json!(0), true, &mut seen));
        assert!(tracker_sync_revision(&serde_json::json!(3), false, &mut seen));
        assert!(!tracker_sync_revision(&serde_json::json!(3), true, &mut seen));
        assert!(!tracker_sync_revision(&serde_json::json!(0), true, &mut seen));
        assert!(tracker_sync_revision(&serde_json::json!(1), false, &mut seen));
    }

    #[test]
    fn tracker_selects_open_pr_and_preserves_unloaded_reference() {
        let pr = |number, state: &str| WorkerPr { branch: None, head_sha: None, additions: None, deletions: None, number,
            url: format!("https://github.com/example/project/pull/{number}"), checks: None, review: None, draft: false, state: state.into() };
        let prs = vec![pr(1, "CLOSED"), pr(2, "MERGED"), pr(3, "OPEN"), pr(4, "")];
        assert_eq!(tracker_primary_pr(&prs).unwrap().number, 3);
        assert_eq!(tracker_pr_reference(&prs[3]), "project#4");
    }

    #[test]
    fn tracker_selects_one_chat_per_leaf_and_excludes_containers() {
        let snapshot: mast_proto::WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
            "spaceId": "fixture", "items": [
                {"id":"root", "spaceId":"fixture", "title":"Root", "chatIds":["coordinator", "planning"], "childIds":["container"]},
                {"id":"container", "spaceId":"fixture", "title":"Container", "chatIds":["container-chat"], "childIds":["item"]},
                {"id":"item", "spaceId":"fixture", "title":"Item", "chatIds":["idle", "waiting"]}
            ]
        })).unwrap();
        let mut facts = TeamFacts { workers: vec![worker("idle", WorkerState::Idle), worker("waiting", WorkerState::Waiting),
            worker("planning", WorkerState::Idle), worker("container-chat", WorkerState::Working)] };
        let (other, additional) = tracker_select_rows(&mut facts, Some(&snapshot), "coordinator");
        assert_eq!(facts.workers.len(), 1);
        assert_eq!(facts.workers[0].chat_id, "waiting");
        assert_eq!(additional["waiting"][0].chat_id, "idle");
        assert_eq!(other.len(), 2);
    }

    #[test]
    fn tracker_reconciles_linear_items_branches_and_local_tickets() {
        let snapshot: mast_proto::WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
            "spaceId": "fixture", "items": [
                {"id":"root", "spaceId":"fixture", "title":"Root", "childIds":["a","b"]},
                {"id":"a", "spaceId":"fixture", "title":"A", "parentWorkItemId":"root", "chatIds":["one"]},
                {"id":"b", "spaceId":"fixture", "title":"B", "parentWorkItemId":"root"}
            ], "externalReferences": [{"id":"ref", "spaceId":"fixture", "workItemId":"a", "provider":"linear", "referenceType":"issue", "externalId":"EX-1"}]
        })).unwrap();
        let item = |id: &str| crate::board::BoardItem { id: id.into(), title: id.into(), status: crate::board::BoardStatus::Todo,
            updated_at: 0, created_at: 0, worker: None, worker_title: None, report: None, ticket: None, request_id: None };
        let items = vec![(item("a"), false), (item("b"), true)];
        let mut branch = worker("two", WorkerState::Working);
        branch.branch = Some("feature/ex-1-work".into());
        let workers = TeamFacts { workers: vec![worker("one", WorkerState::Waiting), branch] };
        let issues = vec![mast_proto::linear::LinearIssueSummary { id: "i".into(), identifier: "EX-1".into(), title: "Issue".into(), ..Default::default() },
            mast_proto::linear::LinearIssueSummary { id: "j".into(), identifier: "EX-2".into(), title: "No worker".into(),
                state: mast_proto::linear::LinearState { state_type: mast_proto::linear::LinearStateType::Completed, ..Default::default() }, ..Default::default() }];
        let (rows, other, additional, empty, statuses) = tracker_reconcile_project(&snapshot, "root", &items, &workers, &issues);
        assert_eq!(rows.workers.len(), 1);
        assert_eq!(rows.workers[0].ticket.as_ref().unwrap().identifier, "EX-1");
        assert_eq!(additional["one"][0].chat_id, "two");
        assert!(other.is_empty());
        assert_eq!(statuses["one"], (Some(false), Some(false)));
        assert_eq!(empty.len(), 2);
        assert!(empty.iter().any(|item| item.ticket.as_deref() == Some("EX-2") && item.status == crate::board::BoardStatus::Done));
        assert!(empty.iter().any(|item| item.id == "b"));
    }

    #[test]
    fn titled_items_join_loaded_tickets_and_keep_unmatched_ids() {
        let snapshot: mast_proto::WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
            "spaceId":"fixture", "items":[
                {"id":"root", "spaceId":"fixture", "childIds":["a", "b"]},
                {"id":"a", "spaceId":"fixture", "parentWorkItemId":"root", "chatIds":["one"]},
                {"id":"b", "spaceId":"fixture", "parentWorkItemId":"root"}]
        })).unwrap();
        let item = |id: &str, title: &str| crate::board::BoardItem { id: id.into(), title: title.into(), status: crate::board::BoardStatus::Todo,
            updated_at: 0, created_at: 0, worker: None, worker_title: None, report: None, ticket: None, request_id: None };
        let items = vec![(item("a", "EX-1 Release notes"), false), (item("b", "EX-999 Calendar"), true)];
        let workers = TeamFacts { workers: vec![worker("one", WorkerState::Working)] };
        let issues = vec![mast_proto::linear::LinearIssueSummary { identifier:"EX-1".into(), title:"Release notes".into(), ..Default::default() }];
        let (rows, _, _, empty, _) = tracker_reconcile_project(&snapshot, "root", &items, &workers, &issues);
        assert_eq!(rows.workers.len(), 1);
        assert_eq!(rows.workers[0].ticket.as_ref().unwrap().identifier, "EX-1");
        assert_eq!(empty.len(), 1);
        assert_eq!(empty[0].ticket.as_deref(), Some("EX-999"));
    }

    #[test]
    fn tracker_preserves_explicit_issue_ids_outside_the_loaded_project() {
        let snapshot: mast_proto::WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
            "spaceId":"fixture", "items":[
                {"id":"root", "spaceId":"fixture", "childIds":["a", "b"]},
                {"id":"a", "spaceId":"fixture", "parentWorkItemId":"root", "chatIds":["one"]},
                {"id":"b", "spaceId":"fixture", "parentWorkItemId":"root"}],
            "externalReferences":[
                {"id":"r1", "spaceId":"fixture", "workItemId":"a", "provider":"linear", "referenceType":"issue", "externalId":"OUT-1"},
                {"id":"r2", "spaceId":"fixture", "workItemId":"b", "provider":"linear", "referenceType":"issue", "externalId":"OUT-2"}]
        })).unwrap();
        let item = |id: &str| crate::board::BoardItem { id: id.into(), title: id.into(), status: crate::board::BoardStatus::Todo,
            updated_at: 0, created_at: 0, worker: None, worker_title: None, report: None, ticket: None, request_id: None };
        let items = vec![(item("a"), false), (item("b"), true)];
        let workers = TeamFacts { workers: vec![worker("one", WorkerState::Working)] };
        let issues = vec![mast_proto::linear::LinearIssueSummary { identifier:"EX-1".into(), ..Default::default() }];
        let (rows, _, _, empty, _) = tracker_reconcile_project(&snapshot, "root", &items, &workers, &issues);
        assert_eq!(rows.workers[0].ticket.as_ref().unwrap().identifier, "OUT-1");
        assert!(empty.iter().any(|item| item.id == "b" && item.ticket.as_deref() == Some("OUT-2")));
    }

    #[test]
    fn tracker_headline_uses_ticket_title_or_work_item() {
        let mut ticket = crate::chat_metadata::HeaderTicket { identifier: "ENG-42".into(), title: Some("Review changes".into()), ..Default::default() };
        assert_eq!(tracker_headline(Some(&ticket), "Work item"), "Review changes");
        ticket.title = None;
        assert_eq!(tracker_headline(Some(&ticket), "Work item"), "Work item");
    }

    #[test]
    fn tracker_todo_label_is_a_count() {
        assert_eq!(tracker_todo_label(mast_proto::TodoProgress { done: 3, total: 7 }), "3/7");
    }

    #[test]
    fn tracker_review_marks_require_the_current_head() {
        let mut state = mast_proto::DiffReviewState { target: "pr:example/project#1".into(), head_sha: Some("abc".into()), ..Default::default() };
        state.viewed_files.insert("file.rs".into());
        assert_eq!(reviewed_files_at_head(Some(state.clone()), Some("ABC")), 1);
        assert_eq!(reviewed_files_at_head(Some(state.clone()), Some("def")), 0);
        assert_eq!(reviewed_files_at_head(Some(state), None), 0);
    }
}

impl Shell {
    #[cfg(test)]
    pub(super) fn set_tracker_project_for_test(&mut self, account: &str, project: &str, issues: Vec<mast_proto::linear::LinearIssueSummary>) {
        self.tracker.linear_projects.insert((account.into(), project.into(), 0), issues);
        self.tracker.key = None;
    }

    #[cfg(test)]
    pub(super) fn set_tracker_project_more_for_test(&mut self, account: &str, project: &str, more: bool) {
        self.tracker.linear_more.insert((account.into(), project.into(), 0), more);
        self.tracker.key = None;
    }

    #[cfg(test)]
    pub(super) fn tracker_source_notice_for_test(&self) -> Option<&str> {
        self.tracker.source_notice.as_deref()
    }

    #[cfg(test)]
    pub(super) fn tracker_all_workers_for_test(&self) -> std::sync::Arc<TeamFacts> {
        self.tracker.all_workers.clone()
    }

    #[cfg(test)]
    pub(super) fn set_tracker_health_for_test(&mut self, entries: Vec<mast_engine::work_outline_health::HealthEntry>, targets: Vec<(String, String)>) {
        self.tracker.health_entries = entries;
        self.tracker.health_titles = targets.iter().cloned().collect();
        self.tracker.health_targets = std::sync::Arc::new(targets);
    }

    #[cfg(test)]
    pub(super) fn set_tracker_item_prs_for_test(&mut self, item_id: &str, prs: Vec<mast_engine::pr_ticket_cache::PrStatus>) {
        self.tracker.health_item_prs.insert(item_id.to_owned(), prs);
        self.tracker.key = None;
    }

    #[cfg(test)]
    pub(super) fn tracker_has_item_for_test(&self, id: &str) -> bool {
        self.tracker.not_started.iter().chain(&self.tracker.completed_items).chain(&self.tracker.unassigned_pending).any(|item| item.id == id)
    }

    fn tracker_linear_project(&self, chat_id: &str, cx: &App) -> Option<(String, String, mast_proto::WorkItemExternalReferenceView, u64)> {
        let state = self.state.read(cx);
        let space = state.chats.iter().find(|chat| chat.id == chat_id)?.space_id.as_deref()?;
        let outline = state.work_outline(space);
        let snapshot = outline.snapshot()?;
        if !crate::linear::space_enabled(Some(snapshot)) { return None; }
        let item = crate::chat_work_context::assigned_item(snapshot, chat_id)?;
        let project = snapshot.linear_project_reference(item)?;
        if project.external_id.is_empty() { return None; }
        let account = crate::linear::store::account_scope(snapshot).to_owned();
        let revision = state.linear.revision(&account);
        Some((space.to_owned(), account, project.clone(), revision))
    }

    fn tracker_project_link_context(&self, chat_id: &str, cx: &App) -> Option<(String, String, String, u64)> {
        let state = self.state.read(cx);
        let space = state.chats.iter().find(|chat| chat.id == chat_id)?.space_id.as_deref()?;
        let outline = state.work_outline(space);
        let snapshot = outline.snapshot()?;
        if !crate::linear::space_enabled(Some(snapshot)) { return None; }
        let item = crate::chat_work_context::assigned_item(snapshot, chat_id)?;
        if snapshot.item(item).is_none_or(|item| item.archived_at.is_some()) || snapshot.linear_project_reference(item).is_some() { return None; }
        let account = crate::linear::store::account_scope(snapshot).to_owned();
        let revision = state.linear.revision(&account);
        Some((space.to_owned(), item.to_owned(), account, revision))
    }

    fn open_tracker_project_picker(&mut self, cx: &mut Context<Self>) {
        let Some(context) = self.tracker_project_link_context(&self.active_chat, cx) else { return; };
        if self.tracker.project_picker.as_ref() == Some(&context) {
            self.tracker.project_picker = None;
            cx.notify();
            return;
        }
        self.tracker.project_picker = Some(context.clone());
        self.tracker.project_choices = None;
        self.tracker.project_picker_error = None;
        let task = self.state.read(cx).work_outline_actions().linear_my_projects(&context.0, &context.2, crate::linear::client::ListRead::Cached);
        cx.spawn(async move |shell, cx| {
            let result = task.await;
            shell.update(cx, |shell, cx| {
                if shell.tracker.project_picker.as_ref() != Some(&context) { return; }
                match result {
                    Ok(projects) => shell.tracker.project_choices = Some(std::sync::Arc::new(projects)),
                    Err(error) => shell.tracker.project_picker_error = Some(error.user_text()),
                }
                cx.notify();
            }).ok();
        }).detach();
        cx.notify();
    }

    fn link_tracker_project(&mut self, project: mast_proto::linear::LinearProjectSummary, cx: &mut Context<Self>) {
        if self.tracker.project_linking { return; }
        let Some(context) = self.tracker.project_picker.clone() else { return; };
        if self.tracker_project_link_context(&self.active_chat, cx).as_ref() != Some(&context) {
            self.tracker.project_picker = None;
            cx.notify();
            return;
        }
        self.tracker.project_linking = true;
        self.tracker.project_picker_error = None;
        let task = self.state.read(cx).work_outline_actions().link_linear_project(mast_proto::LinkLinearProjectParams {
            work_item_id: context.1.clone(), project: project.to_ref(), operation_id: crate::work_outline::actions::new_operation_id(),
        });
        cx.spawn(async move |shell, cx| {
            let result = task.await;
            shell.update(cx, |shell, cx| {
                shell.tracker.project_linking = false;
                let same_picker = shell.tracker.project_picker.as_ref() == Some(&context);
                match result {
                    Ok(_) => {
                        if same_picker { shell.tracker.project_picker = None; }
                    }
                    Err(error) if same_picker => shell.tracker.project_picker_error = Some(error.user_text()),
                    Err(_) => {}
                }
                cx.notify();
            }).ok();
        }).detach();
        cx.notify();
    }

    pub(super) fn refresh_tracker_source_if_changed(&mut self, cx: &mut Context<Self>) {
        if !self.active_chat.is_empty() && self.right_pane_open(cx) && self.resolved_right_active(cx) == RightSurface::Team {
            let chat_id = self.active_chat.clone();
            self.load_tracker_health(&chat_id, cx);
        }
        let Some(previous) = self.tracker.source_observation.clone() else { return; };
        if self.active_chat != previous.chat_id || !self.right_pane_open(cx) || self.resolved_right_active(cx) != RightSurface::Team { return; }
        let state = self.state.read(cx);
        let outline = state.work_outline(&previous.space_id);
        let Some(snapshot) = outline.snapshot() else { return; };
        let account = crate::linear::store::account_scope(snapshot).to_owned();
        let revision = state.linear.revision(&account);
        if snapshot.revision == previous.snapshot_revision && account == previous.account && revision == previous.account_revision { return; }
        let snapshot_revision = snapshot.revision;
        let project = self.tracker_linear_project(&previous.chat_id, cx).map(|(_, _, reference, _)| reference.external_id);
        self.tracker.source_observation = Some(TrackerSourceObservation {
            chat_id: previous.chat_id.clone(), space_id: previous.space_id, snapshot_revision,
            account: account.clone(), account_revision: revision, project_id: project.clone(),
        });
        if project != previous.project_id || account != previous.account || revision != previous.account_revision {
            self.tracker.key = None;
            if project.is_some() {
                self.load_tracker_project(&previous.chat_id, false, cx);
                self.watch_tracker_sync(&previous.chat_id, cx);
            } else {
                self.stop_tracker_sync();
            }
        }
    }

    pub(super) fn load_tracker_health(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let Some(space) = state.chats.iter().find(|chat| chat.id == chat_id).and_then(|chat| chat.space_id.as_deref()) else { self.clear_tracker_health(); return; };
        let outline = state.work_outline(space);
        let Some(snapshot) = outline.snapshot() else { self.clear_tracker_health(); return; };
        let Some(root) = crate::chat_work_context::assigned_item(snapshot, chat_id) else { self.clear_tracker_health(); return; };
        let account_revision = state.linear.revision(crate::linear::store::account_scope(snapshot));
        let key = (space.to_owned(), root.to_owned(), snapshot.revision, state.board_status_revision, self.board_links_revision, account_revision);
        if self.tracker.health_key.as_ref() == Some(&key) { return; }
        let scoped = mast_engine::work_outline_health::subtree_items(snapshot, root);
        let targets = scoped.iter().filter(|item| item.child_ids.is_empty()).map(|item| (item.id.clone(), item.title.clone())).collect();
        let mut titles: std::collections::HashMap<_, _> = scoped.iter().map(|item| (item.id.clone(), item.title.clone())).collect();
        let completed = scoped.iter().filter(|item| item.effective_status == mast_proto::WorkItemEffectiveStatus::Completed).map(|item| item.id.clone()).collect();
        let scoped_ids: std::collections::HashSet<_> = scoped.iter().map(|item| item.id.as_str()).collect();
        let issue_urls = snapshot.external_references.iter().filter(|reference| scoped_ids.contains(reference.work_item_id.as_str())
            && reference.provider == mast_proto::linear::LINEAR_PROVIDER && reference.reference_type == mast_proto::linear::LINEAR_ISSUE_REFERENCE)
            .map(|reference| (reference.work_item_id.clone(), if reference.url.is_empty() { format!("https://linear.app/issue/{}", reference.external_id) } else { reference.url.clone() })).collect();
        if let Some(project) = snapshot.linear_project_reference(root) {
            let account = crate::linear::store::account_scope(snapshot).to_owned();
            if let Some(issues) = self.tracker.linear_projects.get(&(account, project.external_id.clone(), account_revision)) {
                titles.extend(issues.iter().map(|issue| (format!("linear:{}", issue.identifier.to_ascii_uppercase()), issue.title.clone())));
            }
        }
        let Some(engine) = state.engine().cloned() else {
            if self.tracker.health_key.as_ref().is_some_and(|old| old.0 != key.0 || old.1 != key.1) { self.clear_tracker_health(); }
            return;
        };
        if self.tracker.health_key.as_ref().is_none_or(|old| old.0 != key.0 || old.1 != key.1) {
            self.tracker.health_entries.clear();
            self.tracker.health_item_prs.clear();
            self.tracker.health_link_url = None;
        }
        self.tracker.health_key = Some(key.clone());
        self.tracker.health_targets = std::sync::Arc::new(targets);
        self.tracker.health_titles = titles;
        self.tracker.health_completed = completed;
        self.tracker.health_issue_urls = issue_urls;
        self.tracker.health_task = Some(cx.spawn(async move |shell, cx| {
            // Coalesce a burst of cached PR updates into one subtree read.
            cx.background_executor().timer(std::time::Duration::from_millis(150)).await;
            let result = engine.client().call(mast_rpc::methods::WORK_OUTLINE_HEALTH,
                serde_json::json!({ "spaceId": key.0.clone(), "workItemId": key.1.clone() })).await;
            let _ = shell.update(cx, |shell, cx| {
                if shell.tracker.health_key.as_ref() != Some(&key) { return; }
                match result {
                    Ok(value) => {
                        shell.tracker.health_entries = value.get("entries").cloned().and_then(|entries| serde_json::from_value(entries).ok()).unwrap_or_default();
                        shell.tracker.health_item_prs = value.get("itemPrs").cloned().and_then(|prs| serde_json::from_value(prs).ok()).unwrap_or_default();
                        shell.tracker.key = None;
                        shell.tracker.health_incomplete = value.get("unknown").and_then(|value| value.as_object())
                            .is_some_and(|unknown| unknown.values().any(|value| value.as_bool() == Some(true)));
                    }
                    Err(_) => { shell.tracker.health_entries.clear(); shell.tracker.health_item_prs.clear(); shell.tracker.health_incomplete = true; }
                }
                cx.notify();
            });
        }));
    }

    fn clear_tracker_health(&mut self) {
        self.tracker.health_key = None;
        self.tracker.health_task = None;
        self.tracker.health_entries.clear();
        self.tracker.health_item_prs.clear();
        self.tracker.health_link_url = None;
        self.tracker.health_targets = Default::default();
        self.tracker.health_titles.clear();
        self.tracker.health_completed.clear();
        self.tracker.health_issue_urls.clear();
        self.tracker.health_incomplete = false;
    }

    fn complete_tracker_health_item(&mut self, item_id: String, cx: &mut Context<Self>) {
        let task = self.state.read(cx).work_outline_actions().update_work_item(mast_proto::UpdateWorkItemParams {
            work_item_id: item_id,
            changes: mast_proto::WorkItemChanges { status: Some(mast_proto::WorkItemStatus::Completed), ..Default::default() },
            operation_id: crate::work_outline::actions::new_operation_id(),
        });
        cx.spawn(async move |shell, cx| {
            let result = task.await;
            let _ = shell.update(cx, |shell, cx| {
                if let Err(error) = result { shell.sidebar_notice = Some(error.message.into()); }
                shell.tracker.health_key = None;
                let chat_id = shell.active_chat.clone();
                shell.load_tracker_health(&chat_id, cx);
                cx.notify();
            });
        }).detach();
    }

    fn link_tracker_health_pr(&mut self, item_id: String, url: String, cx: &mut Context<Self>) {
        let Some(pr) = crate::chat_work_context::parse_pr_url(&url) else { return; };
        let task = self.state.read(cx).work_outline_actions().create_external_reference(mast_proto::CreateWorkItemExternalReferenceParams {
            work_item_id: item_id, provider: mast_proto::pr_review_sync::GITHUB_PROVIDER.into(),
            reference_type: mast_proto::pr_review_sync::GITHUB_PULL_REQUEST_REFERENCE.into(), external_id: pr.external_id(),
            url, cached_title: None, operation_id: crate::work_outline::actions::new_operation_id(),
        });
        self.tracker.health_link_url = None;
        cx.spawn(async move |shell, cx| {
            let result = task.await;
            let _ = shell.update(cx, |shell, cx| {
                if let Err(error) = result { shell.sidebar_notice = Some(error.message.into()); }
                shell.tracker.health_key = None;
                let chat_id = shell.active_chat.clone();
                shell.load_tracker_health(&chat_id, cx);
                cx.notify();
            });
        }).detach();
    }

    fn link_tracker_issue(&mut self, item_id: String, identifier: String, cx: &mut Context<Self>) {
        let task = self.state.read(cx).work_outline_actions().link_linear_issue(mast_proto::CreateWorkItemExternalReferenceParams {
            work_item_id: item_id, provider: mast_proto::linear::LINEAR_PROVIDER.into(),
            reference_type: mast_proto::linear::LINEAR_ISSUE_REFERENCE.into(), external_id: identifier.clone(),
            url: format!("https://linear.app/issue/{identifier}"), cached_title: None,
            operation_id: crate::work_outline::actions::new_operation_id(),
        });
        cx.spawn(async move |shell, cx| {
            let result = task.await;
            let _ = shell.update(cx, |shell, cx| {
                if let Err(error) = result { shell.sidebar_notice = Some(error.message.into()); }
                shell.tracker.key = None;
                cx.notify();
            });
        }).detach();
    }

    fn resolve_tracker_stale_request(&mut self, request_id: String, cx: &mut Context<Self>) {
        let task = self.state.read(cx).work_outline_actions().resolve_user_action_request(mast_proto::ResolveUserActionRequestParams {
            user_action_request_id: request_id, outcome: Some("done".into()),
            note: Some("PR merged".into()), answer: None, values: None, via: Some("tracker".into()),
            operation_id: crate::work_outline::actions::new_operation_id(),
        });
        cx.spawn(async move |shell, cx| {
            let result = task.await;
            let _ = shell.update(cx, |shell, cx| {
                if let Err(error) = result { shell.sidebar_notice = Some(error.message.into()); }
                shell.tracker.key = None;
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn load_tracker_project(&mut self, chat_id: &str, force: bool, cx: &mut Context<Self>) {
        self.load_tracker_health(chat_id, cx);
        let Some((space, account, reference, revision)) = self.tracker_linear_project(chat_id, cx) else { return; };
        let key = (account.clone(), reference.external_id.clone(), revision);
        if !self.tracker.linear_loading.insert(key.clone()) { return; }
        self.tracker.key = None;
        let actions = self.state.read(cx).work_outline_actions();
        let read = if force { crate::linear::client::ListRead::Force } else { crate::linear::client::ListRead::Cached };
        let cached_projects = self.state.read(cx).linear.projects.get(&account).cloned();
        let cached_match = cached_projects.as_ref().is_some_and(|projects| projects.iter().any(|project| project.id == reference.external_id)
            || crate::linear::project_links::repair_target(&reference, projects).is_some());
        let lookup = (!crate::linear::project_picker::looks_like_uuid(&reference.external_id) && !cached_match)
            .then(|| actions.linear_my_projects(&space, &account, crate::linear::client::ListRead::Cached));
        let needs_stored = !self.tracker.linear_projects.contains_key(&key);
        cx.spawn(async move |shell, cx| {
            let projects = match lookup {
                Some(task) => match task.await {
                    Ok(projects) => Some(projects),
                    Err(error) => {
                        shell.update(cx, |shell, cx| {
                            shell.tracker.linear_loading.remove(&key);
                            if shell.tracker.linear_errors.len() >= 32 && !shell.tracker.linear_errors.contains_key(&key)
                                && let Some(evicted) = shell.tracker.linear_errors.keys().next().cloned() { shell.tracker.linear_errors.remove(&evicted); }
                            shell.tracker.linear_errors.insert(key.clone(), error);
                            shell.tracker.key = None;
                            cx.notify();
                        }).ok();
                        return;
                    }
                },
                None => cached_projects,
            };
            if let Some(projects) = &projects {
                shell.update(cx, |shell, cx| {
                    shell.state.update(cx, |state, cx| {
                        if state.linear.revision(&account) == revision && state.linear.projects.get(&account) != Some(projects) {
                            state.linear.projects.insert(account.clone(), projects.clone());
                            cx.notify();
                        }
                    });
                }).ok();
            }
            let project = crate::linear::project_links::resolved_project_id(&reference, projects.as_deref());
            let stored = needs_stored.then(|| actions.linear_project_issue_page(&space, &account, &project, crate::linear::client::ListRead::Stored));
            let task = actions.linear_project_issue_page(&space, &account, &project, read);
            if let Some(stored) = stored && let Ok(page) = stored.await {
                shell.update(cx, |shell, cx| {
                    shell.tracker.linear_more.entry(key.clone()).or_insert(page.has_more);
                    shell.tracker.linear_projects.entry(key.clone()).or_insert(page.issues);
                    shell.tracker.key = None;
                    cx.notify();
                }).ok();
            }
            let result = task.await;
            shell.update(cx, |shell, cx| {
                shell.tracker.linear_loading.remove(&key);
                match result {
                    Ok(page) => {
                        if shell.tracker.linear_projects.len() >= 32 && !shell.tracker.linear_projects.contains_key(&key)
                            && let Some(evicted) = shell.tracker.linear_projects.keys().next().cloned() { shell.tracker.linear_projects.remove(&evicted); shell.tracker.linear_more.remove(&evicted); }
                        shell.tracker.linear_more.insert(key.clone(), page.has_more);
                        shell.tracker.linear_projects.insert(key.clone(), page.issues);
                        shell.tracker.linear_errors.remove(&key);
                        shell.tracker.key = None;
                        shell.tracker.health_key = None;
                        let chat_id = shell.active_chat.clone();
                        shell.load_tracker_health(&chat_id, cx);
                    }
                    Err(error) => {
                        if shell.tracker.linear_errors.len() >= 32 && !shell.tracker.linear_errors.contains_key(&key)
                            && let Some(evicted) = shell.tracker.linear_errors.keys().next().cloned() { shell.tracker.linear_errors.remove(&evicted); }
                        shell.tracker.linear_errors.insert(key, error);
                        shell.tracker.key = None;
                    }
                }
                cx.notify();
            }).ok();
        }).detach();
    }

    pub(super) fn stop_tracker_sync(&mut self) {
        self.tracker.linear_sync_task = None;
        self.tracker.health_watch_task = None;
    }

    /// The engine's PR cache published new results: chat link statuses read
    /// before it are stale, so mark them due (the fetch stays rate limited by
    /// `board_links_due`) and rebuild the rows once.
    pub(super) fn on_tracker_pr_cache_changed(&mut self, chat_id: &str, update: Option<&mast_engine::pr_ticket_cache::PrCacheUpdate>, cx: &mut Context<Self>) {
        self.expire_chat_header_links(update, cx);
        self.tracker.health_key = None;
        self.tracker.key = None;
        self.load_tracker_health(chat_id, cx);
        cx.notify();
    }

    fn watch_tracker_prs(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else { return; };
        let chat_id = chat_id.to_owned();
        self.tracker.health_watch_task = Some(cx.spawn(async move |shell, cx| {
            let mut attempt = 0;
            loop {
                let mut subscription = match engine.client().subscribe_checked(mast_rpc::methods::WATCH_TRACKER_PR_CACHE, serde_json::json!({})).await {
                    Ok(subscription) => subscription,
                    Err(mast_rpc::RpcError::UnknownMethod(_)) => return,
                    Err(_) => {
                        let delay = mast_harness::backoff::delay_for_attempt(std::time::Duration::from_secs(5), std::time::Duration::from_secs(300), attempt);
                        attempt = attempt.saturating_add(1);
                        cx.background_executor().timer(mast_harness::backoff::jittered(delay)).await;
                        continue;
                    }
                };
                while let Some(value) = subscription.recv().await {
                    attempt = 0;
                    let update = serde_json::from_value::<mast_engine::pr_ticket_cache::PrCacheUpdate>(value).ok();
                    if shell.update(cx, |shell, cx| {
                        if shell.active_chat == chat_id && shell.right_pane_open(cx) && shell.resolved_right_active(cx) == RightSurface::Team {
                            shell.on_tracker_pr_cache_changed(&chat_id, update.as_ref(), cx);
                        }
                    }).is_err() { return; }
                }
                let delay = mast_harness::backoff::delay_for_attempt(std::time::Duration::from_secs(5), std::time::Duration::from_secs(300), attempt);
                attempt = attempt.saturating_add(1);
                cx.background_executor().timer(mast_harness::backoff::jittered(delay)).await;
            }
        }));
    }

    pub(super) fn watch_tracker_sync(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.stop_tracker_sync();
        self.watch_tracker_prs(chat_id, cx);
        let Some((space, _, _, _)) = self.tracker_linear_project(chat_id, cx) else { return; };
        let Some(engine) = self.state.read(cx).engine().cloned() else { return; };
        let chat_id = chat_id.to_owned();
        self.tracker.linear_sync_task = Some(cx.spawn(async move |shell, cx| {
            let mut attempt = 0;
            let mut seen_revision = None;
            // This loop only reconnects a broken event stream; it never polls.
            loop {
                let params = serde_json::json!({ "spaceId": space });
                let mut subscription = match engine.client().subscribe_checked(mast_rpc::methods::WATCH_LINEAR_SYNC, params).await {
                    Ok(subscription) => subscription,
                    Err(mast_rpc::RpcError::UnknownMethod(_)) => return,
                    Err(error) => {
                        let message = error.to_string();
                        if message.find("linear/").and_then(|at| mast_proto::linear::LinearError::from_wire_message(&message[at..]))
                            .is_some_and(|error| matches!(error.code, mast_proto::linear::LinearErrorCode::NotFound | mast_proto::linear::LinearErrorCode::Invalid)) { return; }
                        let delay = mast_harness::backoff::delay_for_attempt(std::time::Duration::from_secs(5), std::time::Duration::from_secs(300), attempt);
                        attempt = attempt.saturating_add(1);
                        cx.background_executor().timer(mast_harness::backoff::jittered(delay)).await;
                        continue;
                    }
                };
                let mut first = true;
                while let Some(value) = subscription.recv().await {
                    let initial = first;
                    first = false;
                    if !tracker_sync_revision(&value, initial, &mut seen_revision) { continue; }
                    if !initial { attempt = 0; }
                    if shell.update(cx, |shell, cx| {
                        if shell.active_chat == chat_id && shell.right_pane_open(cx) && shell.resolved_right_active(cx) == RightSurface::Team {
                            shell.load_tracker_project(&chat_id, false, cx);
                        }
                    }).is_err() { return; }
                }
                let delay = mast_harness::backoff::delay_for_attempt(std::time::Duration::from_secs(5), std::time::Duration::from_secs(300), attempt);
                attempt = attempt.saturating_add(1);
                cx.background_executor().timer(mast_harness::backoff::jittered(delay)).await;
            }
        }));
    }

    pub(super) fn invalidate_tracker_review(&mut self) {
        self.tracker.key = None;
    }

    fn open_tracker_review(&mut self, chat_id: String, branch: Option<String>, pr_url: String, head_sha: Option<String>, cx: &mut Context<Self>) {
        self.open_request_diff(chat_id.clone(), branch.clone(), cx);
        let Some(pr) = crate::chat_work_context::parse_pr_url(&pr_url) else { return; };
        let Some(artifact) = self.review_artifact(cx) else { return; };
        let state = self.state.read(cx);
        let target_device_id = (state.local_device_id.as_deref() != Some(artifact.device_id.as_str()))
            .then(|| artifact.device_id.clone());
        let call = state.artifact_actions().content(mast_proto::ArtifactContentParams {
            artifact_id: artifact.id.clone(), revision: None, companion: None, target_device_id,
        });
        let expected_pr = format!("pr:{}", pr.external_id().to_lowercase());
        let expected_branch = branch.map(|branch| format!("branch:{}/{}#{branch}", pr.owner.to_lowercase(), pr.repo.to_lowercase()));
        cx.spawn(async move |shell, cx| {
            let Ok(reply) = call.await else { return; };
            if reply.encoding != mast_proto::ArtifactContentEncoding::Utf8 { return; }
            let Ok(document) = serde_json::from_str::<serde_json::Value>(&reply.content) else { return; };
            let target = document.get("target").cloned().or_else(|| {
                let mut pr = document.get("pr")?.clone();
                pr.as_object_mut()?.insert("kind".into(), "pr".into());
                Some(pr)
            });
            let Some(target) = target.and_then(|value| serde_json::from_value::<mast_proto::ReviewTarget>(value).ok()) else { return; };
            let target_head = match &target { mast_proto::ReviewTarget::PullRequest { head_sha, .. } | mast_proto::ReviewTarget::Branch { head_sha, .. } => head_sha };
            if !head_sha.as_deref().is_some_and(|head| head.eq_ignore_ascii_case(target_head)) { return; }
            let key = target.state_key();
            if key != expected_pr && expected_branch.as_deref() != Some(key.as_str()) { return; }
            let _ = shell.update(cx, |shell, cx| {
                if shell.active_chat == chat_id && matches!(shell.resolved_right_active(cx), RightSurface::Diff(_)) && shell.review_artifact(cx).as_ref().is_some_and(|current| current.id == artifact.id && current.current_revision == artifact.current_revision) {
                    shell.set_right_active(RightSurface::Review, cx);
                    cx.notify();
                }
            });
        }).detach();
    }

    pub(crate) fn board_links_revision(&self) -> u64 { self.board_links_revision }

    pub(crate) fn board_link_versions(&self, ids: &[String]) -> Vec<u64> {
        ids.iter().map(|id| self.board_link_versions.get(id).copied().unwrap_or(0)).collect()
    }

    pub(crate) fn board_link_statuses(&self, ids: &[String]) -> std::collections::HashMap<String, mast_engine::pr_ticket_cache::ChatLinkStatus> {
        ids.iter().filter_map(|id| Some((id.clone(), self.chat_header_links.get(id)?.status.clone()?))).collect()
    }

    fn board_links_due_all(&self, ids: &[String], cx: &App) -> Vec<String> {
        ids.iter().filter(|id| {
            let checkout = self.state.read(cx).selected_checkout_key(id);
            !self.chat_header_links_pending.contains(*id) && !self.chat_header_links.get(*id).is_some_and(|entry| entry.checkout == checkout && entry.fetched_at.elapsed() < CHAT_HEADER_LINK_REFRESH)
        }).cloned().collect()
    }

    pub(super) fn board_links_due(&self, ids: &[String], cx: &App) -> Vec<String> {
        let mut due = self.board_links_due_all(ids, cx);
        due.truncate(8usize.saturating_sub(self.chat_header_links_pending.len()));
        due
    }

    /// Fetch the chats this Tracker still owes a link status (filled only by a
    /// rebuild's due scan and by PR cache updates), at most 8 in flight. It
    /// never polls: nothing is owed unless one of those two put it there.
    pub(crate) fn top_up_board_links(&mut self, cx: &mut Context<Self>) {
        let capacity = 8usize.saturating_sub(self.chat_header_links_pending.len());
        let next: Vec<String> = self.board_links_owed.iter().take(capacity).cloned().collect();
        for id in next {
            self.board_links_owed.remove(&id);
            self.ensure_chat_header_links(id.clone(), cx);
            if self.chat_header_links_pending.contains(&id) { self.board_links_requested.insert(id); }
        }
    }

    pub(crate) fn request_board_links(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let ids = self.board_links_due(ids, cx);
        self.board_links_generation = self.board_links_generation.wrapping_add(1);
        let generation = self.board_links_generation;
        if ids.is_empty() { return; }
        cx.spawn(async move |shell, cx| {
            for id in ids {
                if !shell.update(cx, |shell, cx| { if shell.board_links_generation != generation { return false; } shell.ensure_chat_header_links(id, cx); true }).unwrap_or(false) { break; }
                cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
            }
        }).detach();
    }

    pub(crate) fn cancel_board_links(&mut self) { self.board_links_generation = self.board_links_generation.wrapping_add(1); }

    #[cfg(feature = "snapshot")]
    pub(crate) fn fixture_snapshot_team(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| {
            let titles = ["Validate the release", "Implement tracker rows", "Update the cache", "Review accessibility",
                "Add filtering", "Verify light themes", "Inspect status facts", "Test the expanded view",
                "EX-100 Validate the release", "EX-999 Draft release notes", "T4 Validate the release", "Prepare handoff",
                "Check exports", "Improve search", "Verify sorting", "Update labels", "Review tests", "Check errors", "Document setup"];
            for (index, title) in titles.into_iter().enumerate() {
                let id = format!("fixture-worker-{index}");
                state.chats.push(serde_json::from_value(serde_json::json!({
                    "id": id, "deviceId": "local", "spaceId": "fixture-space",
                    "parentChatId": "fixture-chat", "title": title,
                    "branch": format!("tracker/task-{index}"), "actualModel": "example-model",
                    "lastMessagePreview": "Reviewing [project#20](https://github.com/example/project/pull/20) changes",
                    "archived": false, "createdAt": (chrono::Utc::now() - chrono::Duration::hours(if index > 14 { 100 } else { 2 })).to_rfc3339()
                })).unwrap());
                if index < 5 {
                    state.begin_pending_send(&id, &format!("fixture-message-{index}"), chrono::Utc::now() + chrono::Duration::hours(1));
                }
                if index == 5 || index == 6 {
                    let now = chrono::Utc::now();
                    state.sessions.push(mast_proto::Session {
                        chat_id: id, device_id: "local".into(),
                        status: if index == 5 { mast_proto::SessionStatus::AwaitingInput } else { mast_proto::SessionStatus::Errored },
                        started_at: Some(now), updated_at: now, last_completed_turn: None,
                        awaiting_input: None, last_turn_tool_failures: None, todo_progress: Some(mast_proto::TodoProgress { done: 3, total: 7 }),
                    });
                }
            }
            let mut snapshot: mast_proto::WorkOutlineSnapshot = serde_json::from_value(serde_json::json!({
                "spaceId": "fixture-space", "available": true, "revision": 1,
                "items": [
                    {"id":"fixture-root", "spaceId":"fixture-space", "title":"Tracker", "chatIds":["fixture-chat"], "childIds":["fixture-todo", "fixture-done"]},
                    {"id":"fixture-todo", "spaceId":"fixture-space", "title":"Add documentation", "parentWorkItemId":"fixture-root"},
                    {"id":"fixture-done", "spaceId":"fixture-space", "title":"Complete design", "parentWorkItemId":"fixture-root", "effectiveStatus":"completed"}
                ]
            })).unwrap();
            snapshot.space_settings.linear_enabled = true;
            snapshot.external_references.push(serde_json::from_value(serde_json::json!({
                "id":"fixture-project-ref", "spaceId":"fixture-space", "workItemId":"fixture-root", "provider":"linear",
                "referenceType":"project", "externalId":"fixture-project", "cachedTitle":"Harbor workspace"
            })).unwrap());
            snapshot.items[0].chat_ids.push("fixture-worker-18".into());
            for (index, title) in titles.iter().enumerate().take(18) {
                if index == 15 || index == 17 { continue; }
                let id = format!("fixture-item-{index}");
                let chat_ids = if index == 16 { vec![format!("fixture-worker-{index}"), "fixture-worker-17".into()] } else { vec![format!("fixture-worker-{index}")] };
                snapshot.items[0].child_ids.push(id.clone());
                snapshot.items.push(serde_json::from_value(serde_json::json!({
                    "id": id, "spaceId": "fixture-space", "title": title, "parentWorkItemId": "fixture-root",
                    "chatIds": chat_ids, "effectiveStatus": if index == 3 || index == 13 { "completed" } else { "active" }
                })).unwrap());
                if index < 8 {
                    snapshot.external_references.push(serde_json::from_value(serde_json::json!({
                        "id": format!("fixture-issue-ref-{index}"), "spaceId":"fixture-space", "workItemId": format!("fixture-item-{index}"),
                        "provider":"linear", "referenceType":"issue", "externalId": format!("EX-{}", index + 100)
                    })).unwrap());
                }
            }
            snapshot.items[0].child_ids.push("fixture-container".into());
            snapshot.items.push(serde_json::from_value(serde_json::json!({
                "id": "fixture-container", "spaceId": "fixture-space", "title": "Planning", "parentWorkItemId": "fixture-root",
                "chatIds": ["fixture-worker-15"], "childIds": ["fixture-container-child"]
            })).unwrap());
            snapshot.items.push(serde_json::from_value(serde_json::json!({
                "id": "fixture-container-child", "spaceId": "fixture-space", "title": "Plan migration", "parentWorkItemId": "fixture-container"
            })).unwrap());
            for number in [20, 21, 44] {
                snapshot.external_references.push(serde_json::from_value(serde_json::json!({
                    "id": format!("fixture-pr-{number}"), "spaceId": "fixture-space", "workItemId": "fixture-item-0",
                    "provider": "github", "referenceType": "pullRequest", "externalId": format!("example/project#{number}"),
                    "url": format!("https://github.com/example/project/pull/{number}")
                })).unwrap());
            }
            snapshot.user_action_requests.push(mast_proto::UserActionRequestView {
                id: "fixture-handoff".into(), space_id: "fixture-space".into(), work_item_id: "fixture-root".into(),
                chat_id: "fixture-worker-7".into(), text: "Ready to merge: tracker/task-7".into(),
                kind: Some(mast_proto::work_outline_user_actions::UserActionKind::Handoff), priority: None,
                origin_kind: Default::default(), message_id: None, tool_call_id: None, info: None, options: Vec::new(),
                links: Vec::new(), facts: Vec::new(), dedupe_key: None, created_by: None, created_at: 1,
                state: mast_proto::UserActionRequestState::Open, resolution: None, deduped: false, origin_chat_available: true,
            });
            snapshot.user_action_requests.push(mast_proto::UserActionRequestView {
                id: "fixture-stale-review".into(), space_id: "fixture-space".into(), work_item_id: "fixture-item-3".into(),
                chat_id: "fixture-worker-3".into(), text: "Ready for review: https://github.com/example/project/pull/23".into(),
                kind: Some(mast_proto::work_outline_user_actions::UserActionKind::Ask), priority: None,
                origin_kind: Default::default(), message_id: None, tool_call_id: None, info: None, options: Vec::new(),
                links: Vec::new(), facts: Vec::new(), dedupe_key: None, created_by: None, created_at: 1,
                state: mast_proto::UserActionRequestState::Open, resolution: None, deduped: false, origin_chat_available: true,
            });
            snapshot.user_action_requests.push(mast_proto::UserActionRequestView {
                id: "fixture-merged-question".into(), space_id: "fixture-space".into(), work_item_id: "fixture-item-2".into(),
                chat_id: "fixture-worker-2".into(), text: "Which calendar should the report use?".into(),
                kind: Some(mast_proto::work_outline_user_actions::UserActionKind::Ask), priority: None,
                origin_kind: Default::default(), message_id: None, tool_call_id: None, info: None, options: Vec::new(),
                links: Vec::new(), facts: Vec::new(), dedupe_key: None, created_by: None, created_at: 1,
                state: mast_proto::UserActionRequestState::Open, resolution: None, deduped: false, origin_chat_available: true,
            });
            snapshot.user_action_requests.push(mast_proto::UserActionRequestView {
                id: "fixture-question".into(), space_id: "fixture-space".into(), work_item_id: "fixture-root".into(),
                chat_id: "fixture-worker-5".into(), text: "Use the compact layout first?".into(),
                kind: Some(mast_proto::work_outline_user_actions::UserActionKind::Ask), priority: None,
                origin_kind: Default::default(), message_id: None, tool_call_id: None, info: None, options: Vec::new(),
                links: Vec::new(), facts: Vec::new(), dedupe_key: None, created_by: None, created_at: 1,
                state: mast_proto::UserActionRequestState::Open, resolution: None, deduped: false, origin_chat_available: true,
            });
            state.set_work_outline_for_test(snapshot);
        });
        let titles = ["Validate the release", "Implement tracker rows", "Update the cache", "Review accessibility",
            "Add filtering", "Verify light themes", "Inspect status facts", "Test the expanded view"];
        let issues = titles.into_iter().enumerate().map(|(index, title)| mast_proto::linear::LinearIssueSummary {
            id: format!("fixture-issue-{index}"), identifier: format!("EX-{}", index + 100), title: title.into(),
            url: format!("https://linear.app/example/issue/EX-{}", index + 100),
            state: mast_proto::linear::LinearState { state_type: if index == 3 { mast_proto::linear::LinearStateType::Completed } else { mast_proto::linear::LinearStateType::Started }, ..Default::default() },
            ..Default::default()
        }).collect();
        self.tracker.linear_projects.insert(("default".into(), "fixture-project".into(), 0), issues);
        self.tracker.health_titles = [("fixture-item-16".into(), "Review tests".into()), ("fixture-container".into(), "Planning".into()),
            ("fixture-item-2".into(), "Update the cache".into())].into_iter().collect();
        self.tracker.health_targets = std::sync::Arc::new(vec![("fixture-item-2".into(), "Update the cache".into())]);
        self.tracker.health_entries = vec![
            mast_engine::work_outline_health::HealthEntry::SeveralLiveChats { item_id: "fixture-item-16".into(), chat_ids: vec!["fixture-worker-16".into(), "fixture-worker-17".into()] },
            mast_engine::work_outline_health::HealthEntry::ChatOnContainer { item_id: "fixture-container".into(), chat_id: "fixture-worker-15".into() },
            mast_engine::work_outline_health::HealthEntry::MergedPrOnOpenItem { item_id: "fixture-item-2".into(), url: "https://github.com/example/project/pull/22".into() },
            mast_engine::work_outline_health::HealthEntry::UnlinkedPr { url: "https://github.com/example/project/pull/42".into() },
            mast_engine::work_outline_health::HealthEntry::WorkerOnDefaultBranch { chat_id: "fixture-worker-12".into() },
            mast_engine::work_outline_health::HealthEntry::StaleChat { chat_id: "fixture-worker-14".into() },
        ];
        for index in 0..8 {
            use mast_engine::pr_ticket_cache::{ChatLinkStatus, ChecksStatus, DiffStat, PrStatus, TicketStatus};
            let titles = ["Validate the release", "Implement tracker rows", "Update the cache", "Review accessibility",
                "Add filtering", "Verify light themes", "Inspect status facts", "Test the expanded view"];
            let url = format!("https://github.com/example/project/pull/{}", index + 20);
            let pr = (index < 4 || index == 7).then(|| PrStatus {
                additions: Some(312), deletions: Some(48),
                number: index + 20, url: Some(url), state: if index == 2 || index == 3 { "MERGED" } else { "OPEN" }.into(),
                is_draft: false, review_decision: (index == 0).then(|| "APPROVED".into()), reviewers: Vec::new(),
                checks: Some(if index == 1 { ChecksStatus::Failing } else { ChecksStatus::Passing }),
                title: None, branch: Some(format!("tracker/task-{index}")), head_sha: Some("fixture-head".into()), mergeable: String::new(),
                has_reviewer_requested: false, merged_at: None,
            });
            let status = ChatLinkStatus {
                pr, ticket: Some(TicketStatus { identifier: format!("EX-{}", index + 100), title: Some(titles[index as usize].into()), url: None, status: None }),
                diff_stat: Some(DiffStat { files_changed: 1, lines_added: 4, lines_removed: 2 }),
                ..Default::default()
            };
            self.chat_header_links.insert(format!("fixture-worker-{index}"), ChatHeaderLinkCache {
                checkout: None, status: Some(status), fetched_at: Instant::now(),
            });
        }
        self.active_chat = "fixture-chat".into();
        let key = self.panel_key(cx);
        self.panels.update(&key, |panel| {
            panel.changes_open = true;
            panel.right_active = RightSurface::Team;
        });
    }

    #[cfg(feature = "snapshot")]
    pub(crate) fn fixture_snapshot_tracker_expanded(&mut self, cx: &mut Context<Self>) {
        self.fixture_snapshot_team(cx);
        self.right_pane_expanded = true;
    }
    /// The selected chat's workers. Link statuses (PR checks, diff stats) come
    /// from the header-link cache; missing ones are requested and the tab
    /// repaints when they land.
    pub(super) fn team_facts(&mut self, chat_id: &str, cx: &mut Context<Self>) -> std::sync::Arc<TeamFacts> {
        let state = self.state.read(cx);
        if self.tracker.chat_scope.as_ref().is_none_or(|(id, revision, _, _)| id != chat_id || *revision != state.board_status_revision) {
            let chat = state.chats.iter().find(|chat| chat.id == chat_id);
            self.tracker.chat_scope = Some((chat_id.to_owned(), state.board_status_revision,
                chat.and_then(|chat| chat.space_id.clone()), chat.is_some_and(|chat| chat.archived)));
        }
        let chat_space = self.tracker.chat_scope.as_ref().and_then(|(_, _, space, _)| space.clone());
        let snapshot = chat_space.as_deref().and_then(|id| state.work_outline(id).snapshot().cloned());
        let account_revision = snapshot.as_deref().map(|snapshot| state.linear.revision(crate::linear::store::account_scope(snapshot))).unwrap_or_default();
        let key = (chat_id.to_owned(), snapshot.as_ref().map(|s| s.revision),
            state.board_status_revision, self.board_links_revision, account_revision);
        if self.tracker.key.as_ref() == Some(&key) {
            return self.tracker.facts.clone();
        }
        let mut facts = {
            let state = self.state.read(cx);
            let Some(chat) = state.chats.iter().find(|chat| chat.id == chat_id) else {
                return std::sync::Arc::new(TeamFacts::default());
            };
            let now = chrono::Utc::now();
            let indicators = state.board_indicators(now);
            let snapshot = chat
                .space_id
                .as_deref()
                .and_then(|id| state.work_outline(id).snapshot().cloned());
            let links = self
                .chat_header_links
                .iter()
                .filter_map(|(id, entry)| Some((id.clone(), entry.status.clone()?)))
                .collect();
            team(&TeamInput {
                chat_id,
                chats: &state.chats,
                indicators: &indicators,
                snapshot: snapshot.as_deref(),
                links: &links,
                now,
            })
        };
        let all_workers = facts.clone();
        let (mut other_chats, mut additional_chats) = tracker_select_rows(&mut facts, snapshot.as_deref(), chat_id);
        let state = self.state.read(cx);
        let mut metadata = std::collections::HashMap::new();
        for chat in &state.chats {
            let harness = chat.config.as_ref().map(|config| crate::managed_accounts::harness_name(config.harness).to_owned()).unwrap_or_default();
            let model = chat.actual_model.clone().or_else(|| chat.config.as_ref().and_then(|config| config.model.clone()));
            metadata.insert(chat.id.clone(), (chat.title.clone().unwrap_or_else(|| "New session".into()), harness, model));
        }
        let mut requests = std::collections::HashMap::new();
        let todos = state.sessions.iter().filter_map(|session| session.todo_progress.map(|progress| (session.chat_id.clone(), progress))).collect();
        let stalled = state.sessions.iter().filter_map(|session| {
            if session.status == mast_proto::SessionStatus::AwaitingInput {
                Some((session.chat_id.clone(), ("waiting for input".to_owned(), false)))
            } else {
                session.last_turn_tool_failures.as_ref().map(|failure| (session.chat_id.clone(), (format!("{} tool calls failed", failure.count), true)))
            }
        }).collect();
        if let Some(snapshot) = &snapshot {
            for request in &snapshot.user_action_requests {
                if request.is_open() && !request.deduped
                    && request.effective_kind() != mast_proto::work_outline_user_actions::UserActionKind::Notice
                    && request.priority != Some(mast_proto::work_outline_user_actions::UserActionPriority::Fyi) {
                    requests.entry(request.chat_id.clone()).or_insert_with(|| crate::team::activity_line(&request.text).unwrap_or_default());
                }
            }
        }
        for session in &state.sessions {
            if session.status == mast_proto::SessionStatus::AwaitingInput
                && let Some(question) = session.awaiting_input.as_ref().and_then(|input| input.questions.first())
                && !question.question.trim().is_empty()
            {
                requests.entry(session.chat_id.clone()).or_insert_with(|| crate::team::activity_line(&question.question).unwrap_or_default());
            }
        }
        let items = snapshot.as_ref().and_then(|snapshot| {
            let root = crate::chat_work_context::assigned_item(snapshot, chat_id)?;
            let live_chats: std::collections::HashSet<_> = state.chats.iter().filter(|chat| !chat.archived).map(|chat| chat.id.as_str()).collect();
            let without_chats: std::collections::HashSet<_> = snapshot.items.iter()
                .filter(|item| !item.chat_ids.iter().any(|id| live_chats.contains(id.as_str())))
                .map(|item| item.id.as_str()).collect();
            let leaves: std::collections::HashSet<_> = snapshot.items.iter().filter(|item| item.child_ids.is_empty()).map(|item| item.id.as_str()).collect();
            let indicators = state.board_indicators(chrono::Utc::now());
            let links = self.chat_header_links.iter().filter_map(|(id, entry)| Some((id.clone(), entry.status.clone()?))).collect();
            let input = TeamInput { chat_id, chats: &state.chats, indicators: &indicators, snapshot: Some(snapshot), links: &links, now: chrono::Utc::now() };
            Some(crate::board::board(snapshot, Some(root), &input).into_iter()
                .filter(|item| item.id != root && leaves.contains(item.id.as_str()))
                .map(|item| { let unassigned = without_chats.contains(item.id.as_str()); (item, unassigned) })
                .collect::<Vec<_>>())
        }).unwrap_or_default();
        let project = snapshot.as_deref().filter(|snapshot| crate::linear::space_enabled(Some(snapshot))).and_then(|snapshot| {
            let root = crate::chat_work_context::assigned_item(snapshot, chat_id)?;
            let project = snapshot.linear_project_reference(root)?;
            Some(((crate::linear::store::account_scope(snapshot).to_owned(), project.external_id.clone(), account_revision), project.cached_title.clone()))
        });
        let project_link_context = snapshot.as_deref().filter(|snapshot| crate::linear::space_enabled(Some(snapshot))).and_then(|snapshot| {
            let root = crate::chat_work_context::assigned_item(snapshot, chat_id)?;
            if snapshot.item(root).is_none_or(|item| item.archived_at.is_some()) || snapshot.linear_project_reference(root).is_some() { return None; }
            Some((snapshot.space_id.clone(), root.to_owned(), crate::linear::store::account_scope(snapshot).to_owned(), account_revision))
        });
        let mut empty: Vec<_> = items.iter().filter(|(_, unassigned)| *unassigned).map(|(item, _)| item.clone()).collect();
        let mut completion = std::collections::HashMap::new();
        if let Some(snapshot) = snapshot.as_deref()
            && let Some(root) = crate::chat_work_context::assigned_item(snapshot, chat_id)
            && let Some((key, _)) = &project
            && let Some(issues) = self.tracker.linear_projects.get(key)
        {
            (facts, other_chats, additional_chats, empty, completion) = tracker_reconcile_project(snapshot, root, &items, &all_workers, issues);
        }
        let issue_states: std::collections::HashMap<_, _> = project.as_ref().and_then(|(key, _)| self.tracker.linear_projects.get(key)).into_iter().flatten()
            .map(|issue| (issue.identifier.to_ascii_uppercase(), issue.state.state_type.is_closed())).collect();
        let mut local_cleanup = Vec::new();
        if let (Some(snapshot), Some(issues)) = (snapshot.as_deref(), project.as_ref().and_then(|(key, _)| self.tracker.linear_projects.get(key))) {
            let linked: std::collections::HashSet<_> = snapshot.external_references.iter().filter(|reference|
                reference.provider == mast_proto::linear::LINEAR_PROVIDER && reference.reference_type == mast_proto::linear::LINEAR_ISSUE_REFERENCE)
                .map(|reference| reference.work_item_id.as_str()).collect();
            for (item, _) in &items {
                if linked.contains(item.id.as_str()) { continue; }
                let identifier = crate::tracker_tickets::title_issue_identifier(&item.title);
                if identifier.is_some_and(|id| issue_states.contains_key(id)) { continue; }
                if let Some(identifier) = identifier {
                    local_cleanup.push(TrackerCleanup::LinkIssue { item_id: item.id.clone(), identifier: identifier.into() });
                }
                for issue in issues.iter().filter(|issue| crate::tracker_tickets::titles_match(&item.title, &issue.title)) {
                    local_cleanup.push(TrackerCleanup::PossibleDuplicate { item_id: item.id.clone(), identifier: issue.identifier.clone() });
                }
            }
        }
        let closed_issues: std::collections::HashSet<_> = issue_states.iter().filter(|(_, closed)| **closed).map(|(id, _)| id.clone()).collect();
        let mut issue_prs = std::collections::HashMap::<String, Vec<mast_engine::pr_ticket_cache::PrStatus>>::new();
        let mut pending_items = std::collections::HashSet::new();
        let mut pending_issues = std::collections::HashSet::new();
        let mut completed_local_items = std::collections::HashSet::new();
        let mut completed_local_issues = std::collections::HashSet::new();
        let local_ids: std::collections::HashSet<_> = items.iter().map(|(item, _)| item.id.as_str()).collect();
        if let Some(snapshot) = snapshot.as_deref() {
            completed_local_items.extend(snapshot.items.iter().filter(|item| local_ids.contains(item.id.as_str())
                && item.effective_status == mast_proto::WorkItemEffectiveStatus::Completed).map(|item| item.id.clone()));
            for reference in snapshot.external_references.iter().filter(|reference| reference.provider == mast_proto::pr_review_sync::GITHUB_PROVIDER
                && reference.reference_type == mast_proto::pr_review_sync::GITHUB_PULL_REQUEST_REFERENCE && local_ids.contains(reference.work_item_id.as_str())) {
                let expected = crate::chat_work_context::parse_pr_url(&reference.url).map(|pr| pr.external_id());
                let known = self.tracker.health_item_prs.get(&reference.work_item_id).is_some_and(|prs| prs.iter().any(|pr|
                    pr.url.as_deref().and_then(crate::chat_work_context::parse_pr_url).map(|pr| pr.external_id())
                        .as_deref().zip(expected.as_deref()).is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))));
                if !known { pending_items.insert(reference.work_item_id.clone()); }
            }
            let mut issue_refs = std::collections::HashMap::<&str, &mast_proto::WorkItemExternalReferenceView>::new();
            for reference in snapshot.external_references.iter().filter(|reference| reference.provider == mast_proto::linear::LINEAR_PROVIDER
                && reference.reference_type == mast_proto::linear::LINEAR_ISSUE_REFERENCE && local_ids.contains(reference.work_item_id.as_str())) {
                issue_refs.entry(&reference.work_item_id).and_modify(|current| {
                    if (reference.created_at, &reference.id) < (current.created_at, &current.id) { *current = reference; }
                }).or_insert(reference);
            }
            for reference in issue_refs.into_values() {
                if completed_local_items.contains(&reference.work_item_id) { completed_local_issues.insert(reference.external_id.to_ascii_uppercase()); }
                if pending_items.contains(&reference.work_item_id) { pending_issues.insert(reference.external_id.to_ascii_uppercase()); }
                if let Some(prs) = self.tracker.health_item_prs.get(&reference.work_item_id) {
                    issue_prs.entry(reference.external_id.to_ascii_uppercase()).or_default().extend(prs.iter().cloned());
                }
            }
        }
        let mut unassigned_groups = std::collections::HashMap::<(TrackerFilter, LifecycleStage), Vec<crate::board::BoardItem>>::new();
        let mut unassigned_rows = std::collections::HashMap::new();
        let mut unassigned_prs = std::collections::HashMap::new();
        let mut unassigned_pending = Vec::new();
        for item in empty {
            let issue = item.ticket.as_ref().map(|id| id.to_ascii_uppercase());
            if pending_items.contains(&item.id) || issue.as_ref().is_some_and(|id| pending_issues.contains(id)) {
                unassigned_pending.push(item);
                continue;
            }
            let linear_done = issue.as_ref().and_then(|id| issue_states.get(id)).copied();
            let prs = issue.as_ref().and_then(|id| issue_prs.get(id)).or_else(|| self.tracker.health_item_prs.get(&item.id)).map(Vec::as_slice).unwrap_or_default();
            let item_done = completed_local_items.contains(&item.id)
                || issue.as_ref().is_some_and(|id| completed_local_issues.contains(id));
            let row = unassigned_lifecycle(item_done, linear_done, prs);
            if let Some(pr) = prs.iter().find(|pr| pr.state.eq_ignore_ascii_case("open")).or_else(|| prs.first()) {
                unassigned_prs.insert(item.id.clone(), pr.clone());
            }
            for filter in [TrackerFilter::All, TrackerFilter::NeedsYou, TrackerFilter::Stage(row.stage)] {
                if tracker_matches(&row, filter) { unassigned_groups.entry((filter, row.stage)).or_default().push(item.clone()); }
            }
            unassigned_rows.insert(item.id.clone(), row);
        }
        let not_started = unassigned_groups.get(&(TrackerFilter::All, LifecycleStage::NotStarted)).cloned().unwrap_or_default();
        let completed_items = unassigned_groups.get(&(TrackerFilter::All, LifecycleStage::Done)).cloned().unwrap_or_default();
        let linear_completed_items = completed_items.iter().filter(|item| item.ticket.as_ref().is_some_and(|id| closed_issues.contains(&id.to_ascii_uppercase())))
            .map(|item| item.id.clone()).collect();
        let now = chrono::Utc::now();
        let live_chat_ids: std::collections::HashSet<_> = state.chats.iter().filter(|chat| !chat.archived).map(|chat| chat.id.as_str()).collect();
        let mut review_ranges = std::collections::HashMap::<(String, String), Vec<&str>>::new();
        for review in state.chats.iter().filter(|chat| !chat.archived && chat.space_id == chat_space) {
            for range in state.chat_ref_targets(&review.id).iter().filter(|range|
                range.kind == mast_proto::RefDiffKind::Branch && range.id.starts_with("review-")) {
                review_ranges.entry((range.path.clone(), range.head_branch.clone())).or_default().push(&review.id);
            }
        }
        let mut lifecycle_rows: std::collections::HashMap<_, _> = facts.workers.iter().map(|worker| {
            let mut workers = vec![worker];
            workers.extend(additional_chats.get(&worker.chat_id).into_iter().flatten());
            let (completed, linear_completed) = completion.get(&worker.chat_id).copied().unwrap_or_else(||
                (worker.work_item_title.as_ref().map(|_| worker.state == crate::team::WorkerState::Done && !worker.archived), None));
            let branch_review = snapshot.as_ref().is_some_and(|snapshot| workers.iter().any(|chat| {
                self.branch_review_targets.get(&chat.chat_id).is_some_and(|target| {
                    matches!(target, mast_proto::ReviewTarget::Branch { branch: Some(branch), .. } if chat.branch.as_deref() == Some(branch.as_str()))
                        && self.branch_review_chats.get(&format!("{}\0{}", snapshot.space_id, target.state_key()))
                            .is_some_and(|review| live_chat_ids.contains(review.as_str()))
                }) || state.selected_execution_member(&chat.chat_id).is_some_and(|member| {
                    member.kind != crate::work_outline::execution::MemberKind::Ref
                        && member.branch.as_deref().zip(chat.branch.as_deref()).is_some_and(|(a, b)| a == b)
                        && member.branch.as_ref().is_some_and(|branch| review_ranges.get(&(member.checkout_path, branch.clone()))
                            .is_some_and(|reviews| reviews.iter().any(|review| *review != chat.chat_id.as_str())))
                })
            }));
            (worker.chat_id.clone(), crate::team::ticket_lifecycle(&workers, completed, linear_completed, branch_review, now))
        }).collect();
        if let Some(snapshot) = snapshot.as_deref() {
            for worker in &facts.workers {
                let chats: Vec<_> = std::iter::once(worker).chain(additional_chats.get(&worker.chat_id).into_iter().flatten()).collect();
                if !chats.iter().flat_map(|chat| &chat.prs).any(|pr| pr.state.eq_ignore_ascii_case("merged")) { continue; }
                let chat_ids: std::collections::HashSet<_> = chats.iter().map(|chat| chat.chat_id.as_str()).collect();
                let relevant: Vec<_> = snapshot.user_action_requests.iter().filter(|request| chat_ids.contains(request.chat_id.as_str())
                    && request.is_unresolved() && request.effective_kind() != mast_proto::work_outline_user_actions::UserActionKind::Notice
                    && request.priority != Some(mast_proto::work_outline_user_actions::UserActionPriority::Fyi)).collect();
                let stale: Vec<_> = relevant.iter().copied().filter(|request| stale_review_request(request)).collect();
                for request in &stale {
                    local_cleanup.push(TrackerCleanup::StaleRequest { request_id: request.id.clone() });
                    requests.remove(&request.chat_id);
                }
                for request in relevant.iter().copied().filter(|request| !stale_review_request(request)) {
                    requests.entry(request.chat_id.clone()).or_insert_with(|| crate::team::activity_line(&request.text).unwrap_or_default());
                }
                let awaiting = state.sessions.iter().any(|session| chat_ids.contains(session.chat_id.as_str())
                    && session.status == mast_proto::SessionStatus::AwaitingInput);
                let changes_requested = chats.iter().flat_map(|chat| &chat.prs).any(|pr| pr.state.eq_ignore_ascii_case("open") && pr.review == Some("changes requested"));
                if let Some(row) = lifecycle_rows.get_mut(&worker.chat_id) {
                    row.needs_you = awaiting || changes_requested || relevant.iter().any(|request| !stale_review_request(request));
                }
            }
        }
        let mut grouped_rows = std::collections::HashMap::<_, Vec<usize>>::new();
        let mut stale_rows = std::collections::HashMap::<_, Vec<usize>>::new();
        for (index, worker) in facts.workers.iter().enumerate().filter(|(_, worker)| !worker.archived) {
            let row = &lifecycle_rows[&worker.chat_id];
            for filter in [TrackerFilter::All, TrackerFilter::NeedsYou].into_iter().chain(LifecycleStage::ALL.map(TrackerFilter::Stage)) {
                if tracker_matches(row, filter) {
                    if row.stale { stale_rows.entry(filter).or_default().push(index); }
                    else { grouped_rows.entry((filter, row.stage)).or_default().push(index); }
                }
            }
        }
        let mut counts = [0; 6];
        let mut needs_you = 0;
        for worker in facts.live() {
            let row = &lifecycle_rows[&worker.chat_id];
            counts[row.stage.index()] += 1;
            needs_you += usize::from(row.needs_you);
        }
        for row in unassigned_rows.values() {
            counts[row.stage.index()] += 1;
            needs_you += usize::from(row.needs_you);
        }
        let ids: Vec<_> = facts.workers.iter().map(|worker| worker.chat_id.clone()).collect();
        self.tracker.link_ids = ids.clone();
        let review_targets: std::collections::HashMap<_, _> = facts.live().filter_map(|worker| {
            if let Some(pr) = worker.prs.iter().find(|pr| pr.state.eq_ignore_ascii_case("open")
                && pr.branch.as_deref().zip(worker.branch.as_deref()).is_some_and(|(pr_branch, branch)| pr_branch == branch)) {
                let target = crate::chat_work_context::parse_pr_url(&pr.url)?;
                return Some((worker.chat_id.clone(), (format!("pr:{}", target.external_id().to_lowercase()), pr.head_sha.clone())));
            }
            if !worker.prs.is_empty() { return None; }
            let member = state.selected_execution_member(&worker.chat_id)?;
            if !member.branch.as_deref().zip(worker.branch.as_deref()).is_some_and(|(member_branch, worker_branch)| member_branch == worker_branch) { return None; }
            let target = self.branch_review_targets.get(&worker.chat_id).filter(|target| {
                matches!(target, mast_proto::ReviewTarget::Branch { checkout_path: Some(path), branch: Some(branch), .. }
                    if *path == member.checkout_path && Some(branch.as_str()) == member.branch.as_deref())
            }).map(mast_proto::ReviewTarget::state_key).or_else(|| {
                let chat = state.chats.iter().find(|chat| chat.id == worker.chat_id)?;
                Some(format!("diff:{}:{}", chat.device_id, member.checkout_path))
            })?;
            Some((worker.chat_id.clone(), (target, member.head_sha)))
        }).collect();
        let visible_targets: std::collections::HashSet<_> = review_targets.values().cloned().collect();
        self.tracker.review_targets = review_targets;
        // Review marks may have changed on another surface or device since the
        // previous board revision. Re-read them once for this revision.
        self.tracker.review_progress.clear();
        let review_targets: Vec<_> = visible_targets.into_iter().collect();
        self.tracker.review_generation = self.tracker.review_generation.wrapping_add(1);
        let generation = self.tracker.review_generation;
        self.tracker.review_task = None;
        self.tracker.key = Some(key);
        let facts = std::sync::Arc::new(facts);
        self.tracker.facts = facts.clone();
        self.tracker.all_workers = std::sync::Arc::new(all_workers);
        self.tracker.not_started = not_started;
        self.tracker.completed_items = completed_items;
        self.tracker.unassigned_groups = unassigned_groups;
        self.tracker.unassigned_rows = unassigned_rows;
        self.tracker.unassigned_prs = unassigned_prs;
        self.tracker.unassigned_pending = unassigned_pending;
        self.tracker.linear_completed_items = linear_completed_items;
        self.tracker.other_chats = other_chats;
        self.tracker.additional_chats = additional_chats;
        self.tracker.lifecycle_rows = lifecycle_rows;
        self.tracker.local_cleanup = local_cleanup;
        self.tracker.linear_project_linked = project.is_some();
        self.tracker.can_link_linear_project = project_link_context.is_some();
        self.tracker.source_observation = snapshot.as_deref().map(|snapshot| TrackerSourceObservation {
            chat_id: chat_id.to_owned(), space_id: snapshot.space_id.clone(), snapshot_revision: snapshot.revision,
            account: crate::linear::store::account_scope(snapshot).to_owned(), account_revision,
            project_id: project.as_ref().map(|(key, _)| key.1.clone()),
        }).or_else(|| chat_space.map(|space_id| TrackerSourceObservation { chat_id: chat_id.to_owned(), space_id,
            snapshot_revision: u64::MAX, account: String::new(), account_revision: 0, project_id: None }));
        if self.tracker.project_picker.is_some() && self.tracker.project_picker != project_link_context {
            self.tracker.project_picker = None;
            self.tracker.project_choices = None;
            self.tracker.project_picker_error = None;
        }
        self.tracker.available_project_context = project_link_context;
        self.tracker.source_label = project.as_ref().map(|(key, title)| {
            let name = title.as_deref().map_or_else(|| "Linear project".to_owned(), |title| format!("Linear project · {title}"));
            self.tracker.linear_projects.get(key).map_or(name.clone(), |issues| {
                let local = facts.live().filter(|worker| worker.ticket.is_none()).count()
                    + self.tracker.unassigned_groups.iter().filter(|((filter, _), _)| *filter == TrackerFilter::All)
                        .flat_map(|(_, items)| items).filter(|item| item.ticket.is_none()).count()
                    + self.tracker.unassigned_pending.iter().filter(|item| item.ticket.is_none()).count();
                let outside = |id: &str| !issue_states.contains_key(&id.to_ascii_uppercase());
                let linked = facts.live().filter(|worker| worker.ticket.as_ref().is_some_and(|ticket| outside(&ticket.identifier))).count()
                    + self.tracker.unassigned_groups.iter().filter(|((filter, _), _)| *filter == TrackerFilter::All)
                        .flat_map(|(_, items)| items).chain(self.tracker.unassigned_pending.iter())
                        .filter(|item| item.ticket.as_deref().is_some_and(outside)).count();
                let suffix = if linked == 0 { String::new() } else { format!(", {linked} linked outside this page") };
                format!("{name} · {} tickets, plus {local} local{suffix}", issues.len())
            })
        }).unwrap_or_else(|| "Local tickets".into());
        self.tracker.source_notice = project.as_ref().and_then(|(key, _)| self.tracker.linear_errors.get(key).map(|error| error.message.clone())
            .or_else(|| self.tracker.linear_loading.contains(key).then(|| "Loading Linear tickets…".into())));
        if let Some((key, _)) = &project && self.tracker.linear_more.get(key) == Some(&true) {
            let shown = self.tracker.linear_projects.get(key).map_or(0, Vec::len);
            let more = format!("Showing {shown} Linear tickets; more not loaded.");
            self.tracker.source_notice = Some(self.tracker.source_notice.take().map_or(more.clone(), |notice| format!("{notice} {more}")));
        }
        self.tracker.grouped_rows = grouped_rows;
        self.tracker.stale_rows = stale_rows;
        self.tracker.counts = counts;
        self.tracker.needs_you = needs_you;
        self.tracker.metadata = metadata;
        self.tracker.requests = requests;
        self.tracker.stalled = stalled;
        self.tracker.todos = todos;
        #[cfg(test)] { self.tracker.rebuilds += 1; }
        let shell = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| {
                let due = shell.board_links_due_all(&ids, cx);
                shell.board_links_owed.extend(due);
                shell.top_up_board_links(cx);
                if shell.tracker.review_generation != generation { return; }
                shell.tracker.review_task = Some(cx.spawn(async move |shell, cx| {
                    for (target, head) in review_targets {
                        let tasks = shell.update(cx, |shell, cx| {
                            if shell.tracker.review_generation != generation { return None; }
                            Some(shell.state.update(cx, |state, cx| {
                                let review = state.chat_diff_rpc(mast_rpc::methods::GET_DIFF_REVIEW_STATE, serde_json::json!({"target": target, "readOnly": true}), cx);
                                let drafts = state.chat_diff_rpc(mast_rpc::methods::GET_REVIEW_DRAFTS, serde_json::json!({"target": target}), cx);
                                (review, drafts)
                            }))
                        }).ok().flatten();
                        let Some((review, drafts)) = tasks else { return; };
                        let review = review.await.ok().and_then(|value| serde_json::from_value::<mast_proto::DiffReviewStateReply>(value).ok());
                        let drafts = drafts.await.ok().and_then(|value| serde_json::from_value::<Vec<mast_proto::ReviewDraft>>(value).ok());
                        if let (Some(review), Some(drafts)) = (review, drafts) {
                            let viewed = reviewed_files_at_head(review.state, head.as_deref());
                            let _ = shell.update(cx, |shell, cx| {
                                if shell.tracker.review_generation == generation && (viewed > 0 || !drafts.is_empty()) {
                                    shell.tracker.review_progress.insert((target, head), (viewed, drafts.len()));
                                    cx.notify();
                                }
                            });
                        }
                    }
                }));
            });
        });
        facts
    }

    /// ⌥⌘T / `t`: open the Team tab; nothing happens without workers.
    pub(crate) fn open_team_tab(&mut self, cx: &mut Context<Self>) {
        if self.active_chat.is_empty() {
            return;
        }
        let chat_id = self.active_chat.clone();
        let facts = self.team_facts(&chat_id, cx);
        if !self.settings.team_tab_visibility.visible(!facts.is_empty() || !self.tracker.other_chats.is_empty() || !self.tracker.unassigned_rows.is_empty() || !self.tracker.unassigned_pending.is_empty() || self.tracker.linear_project_linked || self.tracker.can_link_linear_project) {
            return;
        }
        self.leave_overview_for_chat(cx);
        self.set_surfaces_open(true, cx);
        self.set_right_active(RightSurface::Team, cx);
        cx.notify();
    }

    /// The focus target of the Item and Team tabs' bodies.
    pub(super) fn team_panel_focus(&mut self, cx: &mut Context<Self>) -> FocusHandle {
        self.team_panel_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone()
    }

    /// The sticky chip after "Item": present only while the chat has workers.
    pub(super) fn render_team_tab_chip(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let chat_id = self.active_chat.clone();
        let team = self.team_facts(&chat_id, cx);
        if !self.settings.team_tab_visibility.visible(!team.is_empty() || !self.tracker.other_chats.is_empty() || !self.tracker.unassigned_rows.is_empty() || !self.tracker.unassigned_pending.is_empty() || self.tracker.linear_project_linked || self.tracker.can_link_linear_project) {
            return None;
        }
        let theme = Theme::of(cx).clone();
        let active = self.resolved_right_active(cx) == RightSurface::Team;
        let waiting = team.waiting() > 0;
        Some(
            crate::components::shell_parts::panel_tab(&theme, "right-surface-team-tab", "Tracker", format!("Tracker {}", self.tracker.counts.iter().sum::<usize>()), active)
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.set_right_active(RightSurface::Team, cx);
                }))
                .when(waiting, |el| {
                    el.child(
                        div()
                            .debug_selector(|| "right-surface-team-dot".into())
                            .child(crate::status_style::indicator_for_theme(crate::status_style::Status::Waiting, &theme)),
                    )
                })
                .into_any_element(),
        )
    }

    fn render_team_summary(team: &TeamFacts, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let parts = team.roll_up_parts();
        if parts.is_empty() { return div().child("No active workers").into_any_element(); }
        let mut summary = crate::components::linear_parts::team_summary(theme);
        for (index, (status, label)) in parts.into_iter().enumerate() {
            if index > 0 { summary = summary.child("·"); }
            let indicator = crate::status_style::animated_indicator(status, theme.appearance, format!("team-summary-{index}"), cx.entity_id(), cx);
            summary = summary.child(indicator).child(SharedString::from(label));
        }
        summary.into_any_element()
    }

    /// The Team tab's body: roll-up line, then one row per worker.
    pub(super) fn render_team_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let chat_id = self.active_chat.clone();
        let team = self.team_facts(&chat_id, cx);
        let all_workers = self.tracker.all_workers.clone();
        let focus = self.team_panel_focus(cx);
        let mut col = crate::components::linear_parts::team_column(&theme, false);
        let mut source = div().flex().items_center().gap(px(Theme::SPACE_SM))
            .child(crate::components::shell_parts::tracker_metadata(&theme, self.tracker.source_label.clone()));
        if self.tracker.linear_project_linked {
            source = source.child(team_button(&theme, "tracker-refresh", "Refresh".into(), false)
                .on_click(cx.listener(|this, _, _, cx| {
                    let chat_id = this.active_chat.clone();
                    this.load_tracker_project(&chat_id, true, cx);
                })));
        } else if self.tracker.can_link_linear_project {
            source = source.child(team_button(&theme, "tracker-link-project", "Link Linear project".into(), false)
                .on_click(cx.listener(|this, _, _, cx| this.open_tracker_project_picker(cx))));
        }
        col = col.child(source);
        if self.tracker.project_picker.is_some() && self.tracker.project_picker == self.tracker.available_project_context {
            let mut picker = div().flex().flex_col().gap(px(Theme::SPACE_XS));
            if self.tracker.project_linking { picker = picker.child(crate::components::shell_parts::tracker_metadata(&theme, "Linking…".into())); }
            if let Some(error) = &self.tracker.project_picker_error {
                picker = picker.child(crate::components::shell_parts::tracker_metadata(&theme, error.clone()));
            } else if let Some(projects) = &self.tracker.project_choices {
                if projects.is_empty() { picker = picker.child(crate::components::shell_parts::tracker_metadata(&theme, "No projects found".into())); }
                if !projects.is_empty() {
                    let rows = projects.clone();
                    let entity = cx.entity();
                    let row_theme = theme.clone();
                    picker = picker.child(gpui::uniform_list("tracker-project-picker", projects.len(), move |range, _, app| {
                        entity.update(app, |_, cx| range.filter_map(|index| rows.get(index)).map(|project| {
                            let selected = project.clone();
                            team_button(&row_theme, format!("tracker-project-{}", project.id), project.name.clone(), false)
                                .h(px(28.0)).on_click(cx.listener(move |this, _, _, cx| this.link_tracker_project(selected.clone(), cx)))
                                .into_any_element()
                        }).collect::<Vec<_>>())
                    }).w_full().h(px(projects.len().min(7) as f32 * 28.0)));
                }
            } else {
                picker = picker.child(crate::components::shell_parts::tracker_metadata(&theme, "Loading projects…".into()));
            }
            col = col.child(picker);
        }
        if let Some(notice) = &self.tracker.source_notice {
            col = col.child(crate::components::shell_parts::tracker_metadata(&theme, notice.clone()));
        }
        if self.tracker.chat_scope.as_ref().is_some_and(|(_, _, _, archived)| *archived) {
            col = col.child(
                crate::components::shell_parts::team_notice(&theme, "team-coordinator-archived", "Coordinator archived — hand off or unarchive"),
            );
        }
        if team.is_empty() && self.tracker.other_chats.is_empty() && self.tracker.unassigned_rows.is_empty() && self.tracker.unassigned_pending.is_empty() {
            col = col.child(
                crate::components::shell_parts::team_notice(&theme, "team-empty", "No tickets yet."),
            );
        } else {
            let reports = self.wakes.statuses.get(&chat_id).map(crate::orchestrator_wakes::latest_reports).unwrap_or_default();
            let unseen = self.wakes.statuses.get(&chat_id).map(crate::orchestrator_wakes::unseen_by_worker).unwrap_or_default();
            self.ensure_wake_status(&chat_id, cx);
            let colors = [theme.border_strong, theme.accent, theme.accent, theme.warning, theme.success, theme.text_faint];
            let mut bar = div().debug_selector(|| "tracker-stage-bar".into()).flex().w_full().h(px(5.0)).gap(px(2.0));
            for (count, color) in self.tracker.counts.into_iter().zip(colors) {
                if count > 0 {
                    bar = bar.child(div().h_full().flex_grow(count as f32).flex_basis(px(0.0)).rounded_full().bg(color));
                }
            }
            col = col.child(bar);
            let mut rollup = div().debug_selector(|| "team-rollup".into()).flex().flex_wrap().items_center().gap(px(Theme::SPACE_SM));
            let filters = LifecycleStage::ALL.into_iter().map(|stage| (TrackerFilter::Stage(stage), stage.label(), self.tracker.counts[stage.index()]))
                .chain(std::iter::once((TrackerFilter::NeedsYou, "need you", self.tracker.needs_you)));
            for (filter, label, count) in filters {
                let selected = self.tracker.filter == filter;
                rollup = rollup.child(Self::chip(&theme, format!("tracker-filter-{filter:?}"), format!("{count} {label}"), if selected { ChipTone::Link } else { ChipTone::Dim })
                    .cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                        this.tracker.filter = if this.tracker.filter == filter { TrackerFilter::All } else { filter };
                        cx.notify();
                    })));
            }
            if !self.tracker.unassigned_pending.is_empty() {
                rollup = rollup.child(Self::chip(&theme, "tracker-pr-pending".into(), format!("{} PR details unavailable", self.tracker.unassigned_pending.len()), ChipTone::Dim)
                    .debug_selector(|| "tracker-pr-pending".into()));
            }
            rollup = rollup.child(Self::chip(&theme, "tracker-cleanup-toggle".into(), format!("{} cleanup", self.tracker.health_entries.len() + self.tracker.local_cleanup.len()), ChipTone::Dim)
                .debug_selector(|| "tracker-cleanup-toggle".into())
                .cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                    let id = this.active_chat.clone();
                    if !this.tracker.health_expanded.remove(&id) { this.tracker.health_expanded.insert(id); }
                    cx.notify();
                })));
            let mut settings = div().relative().child(Self::chip(&theme, "tracker-wake-settings".into(), "Wake settings".into(), ChipTone::Dim)
                .cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                    this.tracker.wake_settings_open = !this.tracker.wake_settings_open;
                    cx.notify();
                })));
            if self.tracker.wake_settings_open {
                let menu = crate::popover::popover_card(&theme).w(px(300.0)).p(px(Theme::SPACE_SM))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| { this.tracker.wake_settings_open = false; cx.notify(); }))
                    .child(self.render_team_wake_header(&chat_id, &theme, cx));
                settings = settings.child(crate::popover::anchored_menu_below_end("tracker-wake-menu", menu.into_any_element(), None));
            }
            rollup = rollup.child(settings);
            col = col.child(rollup);
            if self.tracker.health_expanded.contains(&chat_id) { col = col.child(self.render_tracker_health(&theme, cx)); }
            col = col.child(self.render_team_archive_block(&chat_id, &all_workers, &theme, cx));
            col = col.children(self.render_team_archived(&chat_id, &all_workers, &theme, cx));
            for group in [LifecycleStage::ReadyToMerge, LifecycleStage::PrReview, LifecycleStage::CodeReview,
                LifecycleStage::Building, LifecycleStage::NotStarted, LifecycleStage::Done] {
                let label = group.label();
                let matching = self.tracker.grouped_rows.get(&(self.tracker.filter, group)).map(Vec::as_slice).unwrap_or_default();
                let unassigned = self.tracker.unassigned_groups.get(&(self.tracker.filter, group)).map(Vec::as_slice).unwrap_or_default();
                if matching.is_empty() && unassigned.is_empty() { continue; }
                let group_count = matching.len() + unassigned.len();
                let collapsible = group == LifecycleStage::Done && group_count > 5;
                let heading_label = if collapsible { format!("{} {label}", if self.tracker.done_expanded { "▾" } else { "▸" }) } else { label.to_owned() };
                let heading = crate::components::shell_parts::tracker_group_heading(&theme, &heading_label, group_count);
                if collapsible {
                    col = col.child(div().id("tracker-done-toggle").debug_selector(|| "tracker-done-toggle".into()).cursor_pointer().child(heading)
                        .on_click(cx.listener(|this, _, _, cx| { this.tracker.done_expanded = !this.tracker.done_expanded; cx.notify(); })));
                    if !self.tracker.done_expanded { continue; }
                } else {
                    col = col.child(heading);
                }
                for &index in matching { let worker = &team.workers[index]; col = col.child(self.render_tracker_worker(worker, reports.get(&worker.chat_id), unseen.get(&worker.chat_id), &theme, cx)); }
                for item in unassigned { col = col.child(self.render_unassigned_tracker_item(item, group, &theme, cx)); }
            }
            if self.tracker.filter == TrackerFilter::All && !self.tracker.unassigned_pending.is_empty() {
                col = col.child(crate::components::shell_parts::tracker_group_heading(&theme, "PR details unavailable", self.tracker.unassigned_pending.len()));
                for item in &self.tracker.unassigned_pending {
                    col = col.child(crate::components::shell_parts::tracker_row(&theme, format!("tracker-item-{}", item.id))
                        .child(div().flex().items_center().gap(px(Theme::SPACE_SM))
                            .children(self.tracker.linear_project_linked.then(|| item.ticket.clone()).flatten().map(|ticket| crate::components::shell_parts::tracker_mono(&theme, ticket, theme.text_faint)))
                            .children((self.tracker.linear_project_linked && item.ticket.is_none()).then(|| crate::components::shell_parts::tracker_metadata(&theme, "local".into())))
                            .child(crate::components::shell_parts::worker_title(&theme, item.title.clone())))
                        .child(crate::components::shell_parts::tracker_metadata(&theme, "PR status unavailable".into())));
                }
            }
            let stale = self.tracker.stale_rows.get(&self.tracker.filter).map(Vec::as_slice).unwrap_or_default();
            if !stale.is_empty() {
                col = col.child(div().id("tracker-stale-toggle").cursor_pointer().child(format!("{} Stale ({})", if self.tracker.stale_expanded { "▾" } else { "▸" }, stale.len()))
                    .on_click(cx.listener(|this, _, _, cx| { this.tracker.stale_expanded = !this.tracker.stale_expanded; cx.notify(); })));
                if self.tracker.stale_expanded { for &index in stale { let worker = &team.workers[index]; col = col.child(self.render_tracker_worker(worker, reports.get(&worker.chat_id), unseen.get(&worker.chat_id), &theme, cx)); } }
            }
            if !self.tracker.other_chats.is_empty() && self.tracker.filter == TrackerFilter::All {
                col = col.child(div().id("tracker-other-toggle").cursor_pointer().child(format!("{} Other chats ({})", if self.tracker.other_expanded { "▾" } else { "▸" }, self.tracker.other_chats.len()))
                    .on_click(cx.listener(|this, _, _, cx| { this.tracker.other_expanded = !this.tracker.other_expanded; cx.notify(); })));
                if self.tracker.other_expanded { for worker in &self.tracker.other_chats { col = col.child(self.render_tracker_worker(worker, reports.get(&worker.chat_id), unseen.get(&worker.chat_id), &theme, cx)); } }
            }
        }
        self.panel_root("team-tab", focus, col, cx)
    }

    fn render_unassigned_tracker_item(&self, item: &crate::board::BoardItem, stage: LifecycleStage, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let lifecycle = self.tracker.unassigned_rows.get(&item.id);
        let color = if stage == LifecycleStage::Done || stage == LifecycleStage::ReadyToMerge { theme.success } else { theme.border };
        let mut header = div().flex().items_center().gap(px(Theme::SPACE_SM))
            .child(div().size(px(7.0)).rounded_full().bg(color))
            .children(self.tracker.linear_project_linked.then(|| item.ticket.clone()).flatten().map(|ticket| crate::components::shell_parts::tracker_mono(theme, ticket, theme.text_faint)))
            .children((self.tracker.linear_project_linked && item.ticket.is_none()).then(|| crate::components::shell_parts::tracker_metadata(theme, "local".into())))
            .child(crate::components::shell_parts::worker_title(theme, item.title.clone()));
        if lifecycle.is_some_and(|row| row.needs_you) { header = header.child(Self::chip(theme, format!("tracker-needs-you-{}", item.id), "Needs you".into(), ChipTone::Warn)); }
        if lifecycle.is_some_and(|row| row.linear_completed_with_open_pr) {
            header = header.child(crate::components::shell_parts::tracker_metadata(theme, "Linear says done".into()));
        } else if self.tracker.linear_completed_items.contains(&item.id) {
            header = header.child(crate::components::shell_parts::tracker_metadata(theme, "Linear completed".into()));
        }
        if lifecycle.is_some_and(|row| row.item_completed_with_open_pr) {
            header = header.child(crate::components::shell_parts::tracker_metadata(theme, "Item marked done".into()));
        }
        if lifecycle.is_some_and(|row| row.merged_with_open_ticket) {
            header = header.child(crate::components::shell_parts::tracker_metadata(theme, "Merged, ticket still open".into()));
        }
        let mut row = crate::components::shell_parts::tracker_row(theme, format!("tracker-item-{}", item.id))
            .child(header).child(tracker_stage_track(&item.id, stage, theme));
        if let Some(pr) = self.tracker.unassigned_prs.get(&item.id) {
            let url = pr.url.clone().unwrap_or_default();
            let mut label = format!("PR #{} · {}", pr.number, pr.state.to_lowercase());
            if let Some(review) = &pr.review_decision { label.push_str(&format!(" · {}", review.to_lowercase().replace('_', " "))); }
            let link = crate::components::shell_parts::tracker_pr_line(theme, format!("tracker-item-pr-{}", item.id))
                .child(crate::components::shell_parts::tracker_metadata(theme, label));
            row = row.child(link.on_click(cx.listener(move |this, _, window, cx| {
                if !url.is_empty() { this.open_web_reference(&url, window.modifiers().secondary(), window, cx); }
            })));
        }
        row.into_any_element()
    }

    fn render_tracker_health(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use mast_engine::work_outline_health::HealthEntry;
        let mut list = div().debug_selector(|| "tracker-cleanup".into()).flex().flex_col().gap(px(Theme::SPACE_SM));
        if self.tracker.health_incomplete {
            list = list.child(crate::components::shell_parts::tracker_metadata(theme, "Some PR, project or branch details are unavailable".into()));
        }
        if self.tracker.health_entries.is_empty() && self.tracker.local_cleanup.is_empty() {
            list = list.child(crate::components::shell_parts::tracker_metadata(theme, if self.tracker.health_incomplete { "No cleanup found in available details" } else { "No cleanup found" }.into()));
        }
        for (index, entry) in self.tracker.local_cleanup.iter().enumerate() {
            let mut row = div().flex().items_center().gap(px(Theme::SPACE_SM));
            match entry {
                TrackerCleanup::LinkIssue { item_id, identifier } => {
                    let item_id = item_id.clone();
                    let identifier = identifier.clone();
                    row = row.child(format!("Link {identifier}"));
                    row = row.child(crate::components::shell_parts::tracker_action(theme, format!("cleanup-link-issue-{index}"), "Link", false)
                        .on_click(cx.listener(move |this, _, _, cx| this.link_tracker_issue(item_id.clone(), identifier.clone(), cx))));
                }
                TrackerCleanup::PossibleDuplicate { item_id, identifier } => {
                    let title = self.tracker.health_titles.get(item_id).cloned().unwrap_or_default();
                    row = row.child(format!("{title} · Possible duplicate of {identifier}"));
                }
                TrackerCleanup::StaleRequest { request_id } => {
                    let request_id = request_id.clone();
                    row = row.child("Resolve stale request — PR merged")
                        .child(crate::components::shell_parts::tracker_action(theme, format!("cleanup-resolve-request-{index}"), "Resolve", false)
                            .debug_selector(move || format!("cleanup-resolve-request-{index}"))
                            .on_click(cx.listener(move |this, _, _, cx| this.resolve_tracker_stale_request(request_id.clone(), cx))));
                }
            }
            list = list.child(row);
        }
        for (index, entry) in self.tracker.health_entries.iter().enumerate() {
            let title = |id: &str| self.tracker.health_titles.get(id).cloned().unwrap_or_else(|| "Work item".into());
            let chat_title = |id: &str| self.tracker.metadata.get(id).map(|value| value.0.clone()).unwrap_or_else(|| "Chat".into());
            let mut row = div().flex().flex_wrap().items_center().gap(px(Theme::SPACE_SM));
            match entry {
                HealthEntry::SeveralLiveChats { item_id, chat_ids } => {
                    row = row.child(format!("{} · {} live chats", title(item_id), chat_ids.len()));
                    for (chat_index, chat_id) in chat_ids.iter().enumerate() {
                        let id = chat_id.clone();
                        row = row.child(crate::components::shell_parts::tracker_metadata(theme, chat_title(chat_id)))
                            .child(crate::components::shell_parts::tracker_action(theme, format!("cleanup-archive-{index}-{chat_index}"), "Archive", false)
                                .on_click(cx.listener(move |this, _, _, cx| { this.set_worker_archived(&id, true, cx); })));
                    }
                }
                HealthEntry::ChatOnContainer { item_id, chat_id } => {
                    row = row.child(format!("{} is on container {}", chat_title(chat_id), title(item_id)));
                    let id = chat_id.clone();
                    row = row.child(crate::components::shell_parts::tracker_action(theme, format!("cleanup-open-{index}"), "Open chat", false)
                        .on_click(cx.listener(move |this, _, _, cx| this.open_chat(id.clone(), cx))));
                }
                HealthEntry::MergedPrOnOpenItem { item_id, .. } => {
                    let reason = if self.tracker.health_completed.contains(item_id) { "merged PR, Linear ticket open" } else { "merged PR, item open" };
                    row = row.child(format!("{} · {reason}", title(item_id)));
                    if let Some(url) = self.tracker.health_issue_urls.get(item_id).filter(|_| self.tracker.health_completed.contains(item_id)) {
                        let url = url.clone();
                        row = row.child(crate::components::shell_parts::tracker_action(theme, format!("cleanup-open-linear-{index}"), "Open Linear", false)
                            .on_click(cx.listener(move |this, _, window, cx| this.open_web_reference(&url, window.modifiers().secondary(), window, cx))));
                    } else {
                        let id = item_id.clone();
                        row = row.child(crate::components::shell_parts::tracker_action(theme, format!("cleanup-complete-{index}"), "Mark completed", false)
                            .debug_selector(move || format!("cleanup-complete-{index}"))
                            .on_click(cx.listener(move |this, _, _, cx| this.complete_tracker_health_item(id.clone(), cx))));
                    }
                }
                HealthEntry::UnlinkedPr { url } => {
                    row = row.child(format!("Unlinked PR · {url}"));
                    let selected = url.clone();
                    row = row.child(crate::components::shell_parts::tracker_action(theme, format!("cleanup-link-{index}"), "Link PR", false)
                        .debug_selector(move || format!("cleanup-link-{index}"))
                        .on_click(cx.listener(move |this, _, _, cx| { this.tracker.health_link_url = Some(selected.clone()); cx.notify(); })));
                }
                HealthEntry::WorkerOnDefaultBranch { chat_id } | HealthEntry::StaleChat { chat_id } => {
                    let condition = if matches!(entry, HealthEntry::StaleChat { .. }) { "stale" } else { "on the default branch" };
                    row = row.child(format!("{} · {condition}", chat_title(chat_id)));
                    let id = chat_id.clone();
                    row = row.child(crate::components::shell_parts::tracker_action(theme, format!("cleanup-open-{index}"), "Open chat", false)
                        .on_click(cx.listener(move |this, _, _, cx| this.open_chat(id.clone(), cx))));
                }
            }
            list = list.child(row);
        }
        if let Some(url) = &self.tracker.health_link_url {
            let targets = self.tracker.health_targets.clone();
            let count = targets.len();
            let url = url.clone();
            let color = theme.text;
            let entity = cx.entity();
            list = list.child(crate::components::shell_parts::tracker_metadata(theme, "Choose an item for this PR".into()));
            list = list.child(gpui::uniform_list("tracker-health-targets", count, move |range, _, app| {
                entity.update(app, |_, cx| range.map(|index| {
                    let (id, title) = &targets[index];
                    let id = id.clone();
                    let url = url.clone();
                    div().id(SharedString::from(format!("tracker-health-target-{index}"))).h(px(28.0)).cursor_pointer()
                        .debug_selector(move || format!("tracker-health-target-{index}"))
                        .text_color(color).child(title.clone())
                        .on_click(cx.listener(move |this, _, _, cx| this.link_tracker_health_pr(id.clone(), url.clone(), cx))).into_any_element()
                }).collect::<Vec<_>>())
            }).h(px(count.min(7) as f32 * 28.0)).w_full());
        }
        list.into_any_element()
    }

    fn render_tracker_worker(&self, worker: &WorkerFacts, report: Option<&String>, unseen: Option<&Vec<String>>, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use crate::team::TrackerGroup;
        let id = worker.chat_id.clone();
        let open_id = id.clone();
        let menu_id = id.clone();
        let (_, harness, model) = self.tracker.metadata.get(&id).cloned().unwrap_or_default();
        let identity = model.filter(|model| !model.is_empty()).unwrap_or(harness);
        if !self.tracker.lifecycle_rows.contains_key(&id) {
            let title = self.tracker.metadata.get(&id).map(|metadata| metadata.0.clone()).unwrap_or_else(|| worker.title.clone());
            return crate::components::shell_parts::tracker_row(theme, format!("team-worker-{id}"))
                .child(div().flex().items_center().gap(px(Theme::SPACE_SM))
                    .child(crate::components::shell_parts::worker_title(theme, title))
                    .child(crate::components::shell_parts::tracker_metadata(theme, worker.state.label().into()))
                    .children((!identity.is_empty()).then(|| crate::components::shell_parts::tracker_metadata(theme, identity))))
                .on_click(cx.listener(move |this, _, _, cx| this.open_chat(open_id.clone(), cx)))
                .into_any_element();
        }
        let stage = self.tracker.lifecycle_rows.get(&id).map(|row| row.stage).unwrap_or(LifecycleStage::Building);
        let group = worker.tracker_group();
        let open_pr = worker.prs.iter().find(|pr| pr.state.eq_ignore_ascii_case("open") && !pr.url.is_empty()
            && pr.branch.as_deref().zip(worker.branch.as_deref()).is_some_and(|(pr_branch, branch)| pr_branch == branch));
        let status = if stage == LifecycleStage::ReadyToMerge { crate::status_style::Status::Ready } else { worker.state.into() };
        let status_color = crate::status_style::status_style(status, theme.appearance).color;
        let stalled = self.tracker.stalled.get(&id).cloned();
        let title = tracker_headline(worker.ticket.as_ref().filter(|_| self.tracker.linear_project_linked), worker.work_item_title.as_deref().unwrap_or("Unassigned work"));
        let mut header = div().flex().items_center().gap(px(Theme::SPACE_SM))
            .debug_selector({ let id = id.clone(); move || format!("tracker-heading-{id}") })
            .child(div().flex_none().size(px(7.0)).rounded_full().bg(status_color));
        if let Some(ticket) = worker.ticket.as_ref().filter(|_| self.tracker.linear_project_linked) {
            let url = ticket.url.clone();
            header = header.child(crate::components::shell_parts::tracker_mono(theme, ticket.identifier.clone(), theme.text_faint)
                .id(SharedString::from(format!("team-ticket-{id}"))).cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    if let Some(url) = &url { this.open_web_reference(url, window.modifiers().secondary(), window, cx); }
                })));
        } else if self.tracker.linear_project_linked {
            header = header.child(crate::components::shell_parts::tracker_metadata(theme, "local".into()));
        }
        header = header.child(crate::components::shell_parts::worker_title(theme, title));
        if let Some(chats) = self.tracker.additional_chats.get(&id) {
            let toggle_id = id.clone();
            header = header.child(div().id(SharedString::from(format!("tracker-chats-{id}"))).cursor_pointer()
                .child(crate::components::shell_parts::tracker_metadata(theme, format!("{} chats", chats.len() + 1)))
                .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); if !this.tracker.expanded_chats.remove(&toggle_id) { this.tracker.expanded_chats.insert(toggle_id.clone()); } cx.notify(); })));
        }
        if let Some(unseen) = unseen.filter(|unseen| !unseen.is_empty()) {
            let tip = SharedString::from(format!("Unseen by the orchestrator:\n{}", unseen.join("\n")));
            header = header.child(crate::components::shell_parts::worker_unseen_badge(theme, format!("team-worker-unseen-{id}"))
                .tooltip(move |_, cx| cx.new(|_| SurfaceTabTooltip { text: tip.clone() }).into()));
        }
        if self.tracker.lifecycle_rows.get(&id).is_some_and(|row| row.needs_you) {
            header = header.child(Self::chip(theme, format!("tracker-needs-you-{id}"), "Needs you".into(), ChipTone::Warn));
        }
        if let Some(row) = self.tracker.lifecycle_rows.get(&id) {
            if row.linear_completed_with_open_pr {
                header = header.child(crate::components::shell_parts::tracker_metadata(theme, "Linear says done".into()));
            }
            if row.item_completed_with_open_pr {
                header = header.child(crate::components::shell_parts::tracker_metadata(theme, "Item marked done".into()));
            }
            if row.linear_completed_without_pr {
                header = header.child(crate::components::shell_parts::tracker_metadata(theme, "Linear completed".into()));
            }
            if row.merged_with_open_ticket {
                header = header.child(crate::components::shell_parts::tracker_metadata(theme, "Merged, ticket still open".into()));
            }
        }
        let track = tracker_stage_track(&id, stage, theme);
        let mut progress = div().flex().items_center().gap(px(Theme::SPACE_SM)).child(track)
            .debug_selector({ let id = id.clone(); move || format!("tracker-progress-{id}") });
        if let Some(todo) = self.tracker.todos.get(&id) {
            progress = progress.child(crate::components::shell_parts::tracker_checklist(theme, *todo, tracker_todo_label(*todo)));
        }
        let mut identity_line = div().flex().items_center().gap(px(Theme::SPACE_SM)).min_w_0()
            .debug_selector({ let id = id.clone(); move || format!("tracker-identity-{id}") });
        let has_identity = !identity.is_empty();
        if has_identity { identity_line = identity_line.child(crate::components::shell_parts::tracker_metadata(theme, identity)); }
        if let Some(branch) = &worker.branch {
            if has_identity { identity_line = identity_line.child(crate::components::shell_parts::worker_metadata(theme, "·".into(), theme.text_faint)); }
            identity_line = identity_line.child(div().id(SharedString::from(format!("team-branch-{id}")))
                .debug_selector({ let id = id.clone(); move || format!("team-branch-{id}") })
                .flex().items_center().gap(px(Theme::SPACE_XS)).min_w_0().text_color(theme.text_faint)
                .child(crate::icons::icon(crate::icons::GIT_BRANCH).size(px(11.0)).text_color(theme.text_faint))
                .child(crate::components::shell_parts::tracker_mono(theme, branch.clone(), theme.text_faint)));
        }
        let mut row = crate::components::shell_parts::tracker_row(theme, format!("team-worker-{id}"))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                let closeable = crate::chat_closeout::is_closeout_candidate(this.state.read(cx).chats.iter()
                    .find(|chat| chat.id == menu_id).and_then(|chat| chat.cwd.as_deref()));
                this.chat_menu.open(ChatMenuState { chat_id: menu_id.clone(), position: event.position, page: ChatMenuPage::Root, closeable });
                this.load_branch_review_target(menu_id.clone(), cx);
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.open_chat(open_id.clone(), cx)))
            .child(header).child(progress).child(identity_line);
        if let Some(pr) = tracker_primary_pr(&worker.prs).filter(|pr| pr.number > 0 || !pr.url.is_empty()) {
            let index = 0;
            let url = pr.url.clone();
            let mut line = crate::components::shell_parts::tracker_pr_line(theme, format!("team-pr-{id}-{index}"))
                .child(crate::components::shell_parts::tracker_mono(theme, tracker_pr_reference(pr), theme.text_muted));
            if !pr.state.is_empty() { line = line.child(SharedString::from(format!(" {}", pr.state.to_lowercase()))); }
            if worker.prs.len() > 1 { line = line.child(SharedString::from(format!(" +{} more", worker.prs.len() - 1))); }
            if let Some(checks) = pr.checks {
                let (label, color) = match checks {
                    mast_engine::pr_ticket_cache::ChecksStatus::Passing => ("checks passing", theme.success),
                    mast_engine::pr_ticket_cache::ChecksStatus::Failing => ("checks failing", theme.danger),
                    mast_engine::pr_ticket_cache::ChecksStatus::Pending => ("checks running", theme.text_muted),
                };
                line = line.child(" · ").child(crate::components::shell_parts::worker_metadata(theme, label.into(), color));
            }
            if let Some(review) = pr.review { line = line.child(SharedString::from(format!(" · {review}"))); }
            if let (Some(added), Some(removed)) = (pr.additions, pr.deletions) {
                line = line.child(" · ").child(crate::components::shell_parts::worker_metadata(theme, format!("+{added}"), theme.success))
                    .child(crate::components::shell_parts::worker_metadata(theme, format!("−{removed}"), theme.danger));
            }
            if index == 0 && let Some((viewed, drafts)) = self.tracker.review_targets.get(&id).and_then(|target| self.tracker.review_progress.get(target)) {
                line = line.child(Self::chip(theme, format!("tracker-review-progress-{id}"), format!("Review {viewed} viewed · {drafts} drafts"), ChipTone::Dim));
            }
            row = row.child(line.on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                if !url.is_empty() { this.open_web_reference(&url, window.modifiers().secondary(), window, cx); }
            })));
        }
        if worker.prs.is_empty() && let Some((viewed, drafts)) = self.tracker.review_targets.get(&id).and_then(|target| self.tracker.review_progress.get(target)) {
            row = row.child(Self::chip(theme, format!("tracker-review-progress-{id}"), format!("Review {viewed} viewed · {drafts} drafts"), ChipTone::Dim));
        }
        let age = worker.last_active.map(|at| mast_proto::view::format_time_ago(at, chrono::Utc::now())).unwrap_or_default();
        let request = self.tracker.requests.get(&id);
        let activity = request.map(|request| if self.tracker.lifecycle_rows.get(&id).is_some_and(|row| row.needs_you) { format!("Asked: {request}") } else { request.clone() })
            .or_else(|| worker.activity.clone())
            .or_else(|| stalled.as_ref().map(|(label, _)| label.clone())).unwrap_or_default();
        let activity = [activity, age.clone()].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ");
        if let Some(report) = report.filter(|_| request.is_none()) {
            let expanded = self.wakes.expanded_reports.contains(&id);
            let toggle_id = id.clone();
            let summary = [crate::team::activity_line(report).unwrap_or_default(), age].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ");
            row = row.child(crate::components::shell_parts::worker_report_toggle(theme, format!("team-worker-report-toggle-{id}"), format!("team-worker-activity-{id}"), format!("{} Report: {summary}", if expanded { "▾" } else { "▸" }))
                .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.toggle_report_expanded(&toggle_id, cx); })));
            if expanded { row = row.child(crate::components::shell_parts::worker_activity(theme, format!("team-worker-report-{id}"), report.clone(), true)); }
        } else {
            row = row.child(crate::components::shell_parts::worker_activity(theme, format!("team-worker-activity-{id}"), activity, false)
                .when(stalled.is_some(), |el| el.debug_selector({ let id = id.clone(); move || format!("team-worker-stalled-{id}") })));
        }
        let mut actions = div().flex().items_center().gap(px(Theme::SPACE_SM))
            .debug_selector({ let id = id.clone(); move || format!("tracker-actions-{id}") });
        if self.tracker.lifecycle_rows.get(&id).is_some_and(|row| row.needs_you)
            && (group == TrackerGroup::NeedsYou || stage == LifecycleStage::Done) {
            let answer_id = id.clone();
            actions = actions.child(crate::components::shell_parts::tracker_action(theme, format!("tracker-answer-{id}"), "Answer", true)
                .debug_selector({ let id = id.clone(); move || format!("tracker-answer-{id}") })
                .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.open_chat(answer_id.clone(), cx); })));
        } else if matches!(stage, LifecycleStage::PrReview | LifecycleStage::ReadyToMerge) && open_pr.is_some() {
            let pr = open_pr.unwrap();
            let review_id = id.clone();
            let review_branch = worker.branch.clone();
            let review_url = pr.url.clone();
            let review_head = pr.head_sha.clone();
            let merge_url = pr.url.clone();
            actions = actions.child(crate::components::shell_parts::tracker_action(theme, format!("tracker-review-{id}"), "Review PR", true)
                .debug_selector({ let id = id.clone(); move || format!("tracker-review-{id}") })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.tracker.key = None;
                    this.tracker.review_progress.clear();
                    this.tracker.review_generation = this.tracker.review_generation.wrapping_add(1);
                    this.tracker.review_task = None;
                    this.open_tracker_review(review_id.clone(), review_branch.clone(), review_url.clone(), review_head.clone(), cx);
                })));
            if stage == LifecycleStage::ReadyToMerge {
                actions = actions.child(crate::components::shell_parts::tracker_action(theme, format!("tracker-merge-{id}"), "Merge…", false)
                    .debug_selector({ let id = id.clone(); move || format!("tracker-merge-{id}") })
                    .on_click(cx.listener(move |this, _, window, cx| { cx.stop_propagation(); this.open_web_reference(&merge_url, window.modifiers().secondary(), window, cx); })));
            }
        } else {
            let changes_id = id.clone();
            let changes_branch = worker.branch.clone();
            actions = actions.child(crate::components::shell_parts::tracker_action(theme, format!("team-worker-changes-{id}"), "Changes", false)
                .debug_selector({ let id = id.clone(); move || format!("team-worker-changes-{id}") })
                .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.open_request_diff(changes_id.clone(), changes_branch.clone(), cx); })));
        }
        row = row.child(actions);
        if let Some(verdict) = worker.archivable.as_ref().filter(|a| a.archivable) {
            row = row.child(crate::components::shell_parts::worker_archive_hint(theme, format!("team-archivable-{id}"), &verdict.reason));
        }
        if self.tracker.expanded_chats.contains(&id) && let Some(chats) = self.tracker.additional_chats.get(&id) {
            for chat in std::iter::once(worker).chain(chats.iter()) {
                let chat_id = chat.chat_id.clone();
                let title = self.tracker.metadata.get(&chat_id).map(|metadata| metadata.0.clone()).unwrap_or_else(|| chat.title.clone());
                let branch = chat.branch.clone();
                row = row.child(div().id(SharedString::from(format!("tracker-additional-chat-{chat_id}"))).pl(px(16.0)).flex().items_center().gap(px(Theme::SPACE_SM)).cursor_pointer()
                    .child(title)
                    .child(crate::components::shell_parts::tracker_metadata(theme, chat.state.label().into()))
                    .children(branch.map(|branch| crate::components::shell_parts::tracker_mono(theme, branch, theme.text_faint)))
                    .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.open_chat(chat_id.clone(), cx); })));
            }
        }
        row.into_any_element()
    }

    /// The scrolling, focusable root shared by the Item and Team tabs.
    pub(super) fn panel_root(
        &mut self,
        id: &'static str,
        focus: FocusHandle,
        body: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tracked = focus.clone();
        let theme = Theme::of(cx).clone();
        crate::components::shell_parts::panel_root(&theme, id)
            .track_focus(&focus)
            .key_context("TeamPanel")
            .on_action(
                cx.listener(move |this, _: &OpenTeamTabFromPanel, window, cx| {
                    if tracked.is_focused(window)
                        && this.resolved_right_active(cx) != RightSurface::Team
                    {
                        this.open_team_tab(cx);
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .child(body)
            .into_any_element()
    }

    /// The Item tab's block: the roll-up line, the first workers, and a link
    /// to the Team tab.
    pub(super) fn render_item_tab_team(
        &mut self,
        team: &TeamFacts,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut block = crate::components::linear_parts::team_column(theme, true)
            .debug_selector(|| "chat-item-team".into())
            .child(
                crate::components::shell_parts::item_team_rollup(theme)
                    .child(Self::render_team_summary(team, theme, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.open_team_tab(cx))),
            );
        for worker in team.live().take(ITEM_TAB_WORKERS) {
            block = block.child(self.render_worker_row(worker, true, theme, cx));
        }
        if team.live().count() > ITEM_TAB_WORKERS {
            block = block.child(
                crate::components::shell_parts::item_team_all(theme, team.live().count())
                    .on_click(cx.listener(|this, _, _, cx| this.open_team_tab(cx))),
            );
        }
        block.into_any_element()
    }

    fn chip(theme: &Theme, id: String, label: String, tone: ChipTone) -> gpui::Stateful<gpui::Div> {
        let (border, text) = chip_palette(theme, tone);
        crate::components::shell_parts::panel_chip(theme, id, label, border, text)
    }

    fn pr_tone(pr: &WorkerPr) -> ChipTone {
        use mast_engine::pr_ticket_cache::ChecksStatus;
        match (pr.checks, pr.review) {
            (Some(ChecksStatus::Failing), _) | (_, Some("changes requested")) => ChipTone::Warn,
            _ => ChipTone::Link,
        }
    }

    /// One worker. `compact` (the Item tab) is title, state and age plus the
    /// activity line; the Team tab adds chips, diff stats and the
    /// Ready-to-merge jump.
    fn render_worker_row(
        &mut self,
        worker: &WorkerFacts,
        compact: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let prefix = if compact {
            "chat-item-worker"
        } else {
            "team-worker"
        };
        let id = worker.chat_id.clone();
        let selector = format!("{prefix}-{id}");
        let now = chrono::Utc::now();
        let age = worker
            .last_active
            .map(|at| mast_proto::view::format_time_ago(at, now))
            .unwrap_or_default();
        let status = crate::status_style::status_style(worker.state.into(), theme.appearance);
        let indicator = crate::components::shell_parts::activity_indicator(worker.state.into(), format!("{prefix}-status-{id}"), theme, cx.entity_id(), cx);
        let open_id = id.clone();
        let changes_id = id.clone();
        let changes_branch = worker.branch.clone();
        let changes_selector = format!("{prefix}-changes-{id}");
        let menu_id = id.clone();
        let unseen = self.unseen_for_worker(&id);
        let stalled = self.state.read(cx).session_for(&id).and_then(|session| {
            if session.status == mast_proto::SessionStatus::AwaitingInput {
                Some(("waiting for input".to_owned(), crate::status_style::status_style(crate::status_style::Status::Waiting, theme.appearance).color))
            } else {
                session
                    .last_turn_tool_failures
                    .as_ref()
                    .map(|failure| (format!("{} tool calls failed", failure.count), theme.danger))
            }
        });
        let mut row = crate::components::shell_parts::worker_row(theme, selector)
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                let closeable = crate::chat_closeout::is_closeout_candidate(this.state.read(cx).chats.iter()
                    .find(|chat| chat.id == menu_id).and_then(|chat| chat.cwd.as_deref()));
                this.chat_menu.open(ChatMenuState { chat_id: menu_id.clone(), position: event.position, page: ChatMenuPage::Root, closeable });
                this.load_branch_review_target(menu_id.clone(), cx);
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.open_chat(open_id.clone(), cx)))
            .child(
                crate::components::linear_parts::team_worker_header(theme)
                    .child(div().flex_none().child(indicator))
                    .child(
                        crate::components::shell_parts::worker_title(theme, worker.title.clone()),
                    )
                    .children(stalled.map(|(label, color)| {
                        crate::components::shell_parts::worker_metadata(theme, label, color)
                            .debug_selector({
                                let selector = format!("{prefix}-stalled-{id}");
                                move || selector.clone()
                            })
                    }))
                    .children((!unseen.is_empty()).then(|| {
                        let tip = SharedString::from(format!(
                            "Unseen by the orchestrator:\n{}",
                            unseen.join("\n")
                        ));
                        crate::components::shell_parts::worker_unseen_badge(theme, format!("{prefix}-unseen-{id}"))
                            .tooltip(move |_, cx| {
                                cx.new(|_| SurfaceTabTooltip { text: tip.clone() }).into()
                            })
                    }))
                    .child(
                        crate::components::shell_parts::worker_metadata(theme, worker.state.label().to_owned(), status.color),
                    )
                    .child(
                        crate::components::shell_parts::worker_metadata(theme, age, theme.text_faint),
                    )
                    .child(
                        crate::components::shell_parts::worker_action_link(theme, crate::icons::FILE_CODE, "Changes")
                            .id(SharedString::from(changes_selector.clone()))
                            .debug_selector(move || changes_selector.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_request_diff(changes_id.clone(), changes_branch.clone(), cx);
                            })),
                    ),
            );
        if let Some(report) = self.latest_report_for_worker(&id) {
            // The worker's own report is its activity line; click for the full text.
            let expanded = self.wakes.expanded_reports.contains(&id);
            let line = crate::team::activity_line(&report).unwrap_or_default();
            let toggle_id = id.clone();
            row = row.child(
                crate::components::shell_parts::worker_report_toggle(theme, format!("{prefix}-report-toggle-{id}"), format!("{prefix}-activity-{id}"), format!(
                        "{} Report: {line}",
                        if expanded { "\u{25be}" } else { "\u{25b8}" }
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_report_expanded(&toggle_id, cx);
                    })),
            );
            if expanded {
                row = row.child(
                    crate::components::shell_parts::worker_activity(theme, format!("{prefix}-report-{id}"), report, true),
                );
            }
        } else if let Some(activity) = &worker.activity {
            row = row.child(
                crate::components::shell_parts::worker_activity(theme, format!("{prefix}-activity-{id}"), activity.clone(), false),
            );
        }
        if !compact {
            let mut chips = crate::components::linear_parts::team_worker_metadata(theme, true);
            let mut any = false;
            if let Some(branch) = &worker.branch {
                any = true;
                chips = chips.child(Self::chip(
                    theme,
                    format!("team-branch-{id}"),
                    format!("\u{2387} {branch}"),
                    ChipTone::Dim,
                ));
            }
            for (ix, pr) in worker.prs.iter().enumerate() {
                any = true;
                let url = pr.url.clone();
                chips = chips.child(
                    Self::chip(
                        theme,
                        format!("team-pr-{id}-{ix}"),
                        pr.label(),
                        Self::pr_tone(pr),
                    )
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if !url.is_empty() {
                            let external = window.modifiers().secondary();
                            this.open_web_reference(&url, external, window, cx)
                        }
                    })),
                );
            }
            if let Some(ticket) = &worker.ticket {
                any = true;
                let url = ticket.url.clone();
                chips = chips.child(
                    Self::chip(
                        theme,
                        format!("team-ticket-{id}"),
                        ticket.identifier.clone(),
                        ChipTone::Link,
                    )
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if let Some(url) = &url {
                            let external = window.modifiers().secondary();
                            this.open_web_reference(url, external, window, cx)
                        }
                    })),
                );
            }
            if let Some((added, removed)) = worker.diff {
                any = true;
                chips = chips.child(
                    crate::components::shell_parts::worker_diff(theme, format!("team-diff-{id}"), added, removed),
                );
            }
            if any {
                row = row.child(chips);
            }
        }
        if !compact && let Some(verdict) = worker.archivable.as_ref().filter(|a| a.archivable) {
            row = row.child(
                crate::components::shell_parts::worker_archive_hint(theme, format!("team-archivable-{id}"), &verdict.reason),
            );
        }
        if let Some(ready) = &worker.ready {
            let text = ready.text.clone();
            let mut references = {
                let state = self.state.read(cx);
                state
                    .work_outline(&ready.space_id)
                    .snapshot()
                    .and_then(|snapshot| snapshot.user_action_request(&ready.request_id))
                    .map(|request| {
                        request
                            .links
                            .iter()
                            .filter_map(|link| match &link.target {
                                mast_proto::work_outline_user_actions::RequestLinkTarget::Pr {
                                    url,
                                }
                                | mast_proto::work_outline_user_actions::RequestLinkTarget::Url {
                                    url,
                                } => crate::work_outline::required_actions::parse_handoff_pr(url),
                                _ => None,
                            })
                            .chain(
                                request
                                    .facts
                                    .iter()
                                    .filter(|fact| {
                                        fact.label.eq_ignore_ascii_case("pr")
                                            || fact.label.eq_ignore_ascii_case("pull request")
                                    })
                                    .filter_map(|fact| {
                                        crate::work_outline::required_actions::parse_handoff_pr(
                                            &fact.value,
                                        )
                                    }),
                            )
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            if worker.branch.is_some() {
                references.extend(worker.prs.iter().filter_map(|pr| {
                    crate::work_outline::required_actions::parse_handoff_pr(&pr.url)
                }));
            }
            for pr in references {
                crate::github::store::request_status(&self.state, &pr, false, cx);
            }
            let lifecycle = {
                let state = self.state.read(cx);
                state
                    .work_outline(&ready.space_id)
                    .snapshot()
                    .and_then(|snapshot| snapshot.user_action_request(&ready.request_id))
                    .and_then(|request| {
                        let explicit_pr = request.links.iter().any(|link| match &link.target {
                            mast_proto::work_outline_user_actions::RequestLinkTarget::Pr {
                                url,
                            }
                            | mast_proto::work_outline_user_actions::RequestLinkTarget::Url {
                                url,
                            } => crate::work_outline::required_actions::parse_handoff_pr(url)
                                .is_some(),
                            _ => false,
                        }) || request.facts.iter().any(|fact| {
                            (fact.label.eq_ignore_ascii_case("pr")
                                || fact.label.eq_ignore_ascii_case("pull request"))
                                && crate::work_outline::required_actions::parse_handoff_pr(
                                    &fact.value,
                                )
                                .is_some()
                        });
                        let worker_prs: std::collections::HashSet<_> = worker
                            .prs
                            .iter()
                            .filter_map(|pr| {
                                crate::work_outline::required_actions::parse_handoff_pr(&pr.url)
                            })
                            .map(|pr| pr.key())
                            .collect();
                        crate::work_outline::required_actions::handoff_lifecycle_parts(
                            request.effective_kind(),
                            &request.links,
                            &request.facts,
                            state.github.ready_statuses().filter(|status| {
                                explicit_pr || worker_prs.contains(&status.pr_ref.key())
                            }),
                        )
                    })
            };
            let ready_color = crate::status_style::status_style(crate::status_style::Status::Ready, theme.appearance).color;
            row = row.child(
                crate::components::linear_parts::team_worker_metadata(theme, false)
                    .child(
                        crate::components::shell_parts::worker_ready_label(text, ready_color),
                    )
                    .when_some(lifecycle, |el, lifecycle| {
                        el.child(Self::chip(
                            theme,
                            format!("team-handoff-lifecycle-{id}"),
                            lifecycle.to_owned(),
                            ChipTone::Dim,
                        ))
                    })
                    .child(
                        crate::components::shell_parts::worker_jump_button(theme, format!("{prefix}-jump-{id}"), ready_color)
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_item_tab(cx);
                            })),
                    ),
            );
        }
        row.into_any_element()
    }
}

/// Small text button used by the Team tab's archive controls.
impl Shell {
    /// "Archive finished (N)", the N >= 3 suggestion, and the confirmation.
    fn render_team_archive_block(
        &mut self,
        chat_id: &str,
        team: &TeamFacts,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let finished = team.finished();
        let n = finished.len();
        let block = crate::components::linear_parts::team_archive_block(theme);
        if n == 0 {
            self.wakes.archive_confirm.remove(chat_id);
            return block.into_any_element();
        }
        if self.wakes.archive_confirm.contains(chat_id) {
            let mut confirm = crate::components::shell_parts::archive_confirmation(theme, n);
            for worker in &finished {
                let selector = format!("team-archive-confirm-row-{}", worker.chat_id);
                let reason = worker
                    .archivable
                    .as_ref()
                    .map(|a| a.reason.clone())
                    .unwrap_or_default();
                confirm = confirm.child(
                    crate::components::shell_parts::archive_confirmation_worker(theme, selector, &worker.title, &reason),
                );
            }
            let go_chat = chat_id.to_owned();
            let cancel_chat = chat_id.to_owned();
            confirm = confirm
                .child(
                    crate::components::shell_parts::archive_confirmation_note(theme),
                )
                .child(
                    crate::components::linear_parts::team_archive_actions(theme, true)
                        .child(
                            team_button(
                                theme,
                                "team-archive-confirm-go",
                                format!("Archive {n}"),
                                true,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.archive_finished_workers(&go_chat, cx),
                            )),
                        )
                        .child(
                            team_button(
                                theme,
                                "team-archive-confirm-cancel",
                                "Cancel".into(),
                                false,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.wakes.archive_confirm.remove(&cancel_chat);
                                    cx.notify();
                                },
                            )),
                        ),
                );
            return block.child(confirm).into_any_element();
        }
        let ask_chat = chat_id.to_owned();
        let mut line = crate::components::linear_parts::team_archive_actions(theme, false);
        if n >= SUGGEST_ARCHIVE_AT {
            line = line.child(
                crate::components::shell_parts::archive_suggestion(theme, n),
            );
        } else {
            line = line.child(div().flex_1());
        }
        line = line.child(
            team_button(
                theme,
                "team-archive-finished",
                format!("Archive finished ({n})"),
                false,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.wakes.archive_confirm.insert(ask_chat.clone());
                cx.notify();
            })),
        );
        block.child(line).into_any_element()
    }

    /// "Show archived (N)" and, when on, the archived workers with Unarchive.
    fn render_team_archived(
        &mut self,
        chat_id: &str,
        team: &TeamFacts,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let count = team.archived().count();
        if count == 0 {
            return Vec::new();
        }
        let shown = self.wakes.show_archived.contains(chat_id);
        let toggle_chat = chat_id.to_owned();
        let mut out = vec![
            crate::components::shell_parts::archived_worker_toggle(theme, shown, count)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.wakes.show_archived.remove(&toggle_chat) {
                        this.wakes.show_archived.insert(toggle_chat.clone());
                    }
                    cx.notify();
                }))
                .into_any_element(),
        ];
        if shown {
            for worker in team.archived() {
                let id = worker.chat_id.clone();
                let selector = format!("team-archived-{id}");
                let restore_id = id.clone();
                let button_id = format!("team-unarchive-{id}");
                out.push(
                    crate::components::shell_parts::archived_worker_row(theme, selector)
                        .child(
                            crate::components::shell_parts::archived_worker_title(theme, worker.title.clone()),
                        )
                        .child(
                            team_button(theme, button_id, "Unarchive".into(), false).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.set_worker_archived(&restore_id, false, cx)
                                }),
                            ),
                        )
                        .into_any_element(),
                );
            }
        }
        out
    }

    /// Confirmed "Archive finished": archive every worker the rule rates
    /// finished right now (re-evaluated, so a chat that started working since
    /// the dialog opened is skipped).
    fn archive_finished_workers(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.wakes.archive_confirm.remove(chat_id);
        self.team_facts(chat_id, cx);
        let ids: Vec<String> = self.tracker.all_workers
            .finished()
            .iter()
            .map(|w| w.chat_id.clone())
            .filter(|id| *id != self.active_chat)
            .collect();
        for id in ids {
            self.set_worker_archived(&id, true, cx);
        }
        cx.notify();
    }

    /// One `setChatArchived`; failures show in the sidebar notice.
    fn set_worker_archived(&mut self, worker_id: &str, archived: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.sidebar_notice = Some("Engine not connected".into());
            cx.notify();
            return;
        };
        let id = worker_id.to_owned();
        cx.spawn(async move |this, cx| {
            let failed = engine
                .client()
                .call(
                    mast_rpc::methods::MUTATE,
                    serde_json::json!({ "op": "setChatArchived", "chatId": id, "archived": archived }),
                )
                .await
                .is_err();
            if failed {
                let _ = this.update(cx, |this, cx| {
                    this.sidebar_notice = Some("Could not change that chat's archive state. Try again.".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod board_link_tests {
    use super::*;

    #[gpui::test]
    fn board_link_requests_skip_fresh_entries(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
            crate::history::init(Default::default(), Default::default(), Default::default(), Default::default(), cx);
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
        });
        let state = cx.new(|_| AppState::new());
        let shell = cx.new(|cx| Shell::new(state, crate::EngineBootConfig { data_dir:dir.path().into(), ipc_port:0, edge_url:String::new(), edge_token:None, org_id:None, workos_client_id:None, default_harness:mast_proto::HarnessId::Mock }, cx));
        shell.update(cx, |shell, cx| {
            shell.chat_header_links.insert("fresh".into(), ChatHeaderLinkCache { checkout:None, status:None, fetched_at:Instant::now() });
            shell.chat_header_links_pending.insert("pending".into());
            assert_eq!(shell.board_links_due(&["fresh".into(), "pending".into(), "missing".into()], cx), vec!["missing"]);
            let many: Vec<_> = (0..100).map(|i| format!("worker-{i}")).collect();
            assert_eq!(shell.board_links_due(&many, cx).len(), 7);
        });
    }
}
