//! Work Outline user action requests and instruction proposals (Milestone 4):
//! `userActionRequests` (registry audit 7.6) and
//! `workItemInstructionProposals` (7.7), plus the narrow agent policy
//! (contract section 8).
//!
//! Mechanism layer, same write rules as the earlier kinds (create-if-absent
//! that never revives a tombstone, `update` ops without immutable fields,
//! clock observation, size gate, one batch per logical mutation):
//!
//! - requests: id `derive_id(operationId, "request")`; `spaceId`,
//!   `workItemId`, `chatId`, `messageId`, `toolCallId`, `text`, `info`,
//!   `dedupeKey`, `createdBy`, `createdAt`, `inputsHash` are written once by
//!   the create upsert, with `state: "open"`. Resolution is ONE op writing
//!   `state: "resolved"` and the ONE-field `resolution {by, at, note?}`; no
//!   op ever writes `open` again, and the reader treats a present resolution
//!   as resolved, so resolution is monotone. Resolving a request resolves
//!   every open duplicate of it (same `(workItemId, dedupeKey)`) in the same
//!   batch: duplicates are one logical request.
//! - proposals: id `derive_id(operationId, "proposal")`; `spaceId`, `target`,
//!   `text`, `proposedBy`, `sourceRef`, `createdAt`, `inputsHash` written
//!   once; `review` is the only LWW field. Approval writes the instruction
//!   `derive_id(proposalId, "instruction")` (with `sourceProposalId` and the
//!   source provenance) and the review in ONE batch; the reader lets an
//!   existing approval instruction win over a concurrent reject.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use mast_proto::work_outline::{WorkItemStatus, WorkOutlineActorKind, WorkOutlineMutation};
use mast_proto::work_outline_instructions::{
    InstructionScope, InstructionSource, WorkItemInstructionView,
};
use mast_proto::work_outline_user_actions::{
    ApproveWorkItemInstructionProposalParams, COMPLETION_RESOLUTION_NOTE,
    CreateUserActionRequestParams, InstructionProposalReview, InstructionProposalStatus,
    InstructionProposalTarget, ProposalAction, ProposalReviewState, ProposalTargetScope,
    ProposeWorkItemInstructionParams, RejectWorkItemInstructionProposalParams,
    ResolveUserActionRequestParams, UserActionAnswer, UserActionOriginKind, UserActionRequestState,
    UserActionRequestView, UserActionResolution, WorkItemInstructionProposalView,
    user_action_limits,
};
use serde_json::{Value, json};

use super::instructions::{NewInstruction, check_instruction_text, checked_source};
use super::materialize::{actor_field, decode_instruction, i64_field, space_of, str_field};
use super::*;

pub const KIND_USER_ACTION_REQUESTS: &str = "userActionRequests";
pub const KIND_WORK_ITEM_INSTRUCTION_PROPOSALS: &str = "workItemInstructionProposals";

fn unauthorized(message: impl Into<String>) -> WorkOutlineError {
    WorkOutlineError::unauthorized(message)
}

fn check_origin_id(what: &str, value: Option<&str>) -> Outcome<()> {
    if let Some(value) = value
        && (value.trim().is_empty()
            || value.chars().count() > user_action_limits::MAX_ORIGIN_ID_CHARS)
    {
        return Err(invalid(format!("Invalid origin {what} id")));
    }
    Ok(())
}

fn check_note(what: &str, note: Option<&str>) -> Outcome<()> {
    if let Some(note) = note {
        check_text(what, note, user_action_limits::MAX_NOTE_CHARS, false)?;
    }
    Ok(())
}

/// The first `max` Unicode scalar values of `text`.
fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn validate_request_content(p: &CreateUserActionRequestParams) -> Outcome<()> {
        check_text(
            "The requested action",
            &p.text,
            user_action_limits::MAX_REQUEST_TEXT_CHARS,
            true,
        )?;
        if let Some(info) = &p.info {
            check_text(
                "Supporting detail",
                info,
                user_action_limits::MAX_REQUEST_INFO_CHARS,
                false,
            )?;
        }
        if let Some(key) = &p.dedupe_key {
            check_text(
                "The deduplication key",
                key,
                user_action_limits::MAX_DEDUPE_KEY_CHARS,
                true,
            )?;
        }
        if p.links.len() > 8 || p.facts.len() > 8 {
            return Err(invalid("At most 8 links and 8 facts are allowed"));
        }
        if p.links.iter().any(|link| link.label.chars().count() > 40) {
            return Err(invalid("A link label exceeds 40 characters"));
        }
        if p.facts
            .iter()
            .any(|fact| fact.label.chars().count() > 40 || fact.value.chars().count() > 120)
        {
            return Err(invalid("A fact label or value exceeds its character limit"));
        }
        if p.options.iter().any(|option| {
            option.id.trim().is_empty()
                || option.id.chars().count() > 120
                || option.label.trim().is_empty()
                || option.label.chars().count() > 120
                || option
                    .detail
                    .as_ref()
                    .is_some_and(|detail| detail.chars().count() > 1000)
        }) {
            return Err(invalid(
                "Options require an id and label and must fit within their limits",
            ));
        }
        let mut option_ids = std::collections::HashSet::new();
        if p.options
            .iter()
            .any(|option| !option_ids.insert(&option.id))
        {
            return Err(invalid("Option ids must be unique"));
        }
    Ok(())
}

// ── lenient decoding (shared by the materializer and the mutators) ──────────

/// One decoded request plus the clock of its creation (the dedupe order).
#[derive(Debug, Clone)]
pub(crate) struct DecodedRequest {
    pub view: UserActionRequestView,
    pub created_clock: Option<String>,
}

/// Decode one `userActionRequests` row leniently (R7): only an undecidable
/// `spaceId` drops it. `deduped` and `originChatAvailable` are filled by
/// [`finish_requests`].
pub(crate) fn decode_request(row: &RegistryRow) -> Result<DecodedRequest, WorkOutlineRepair> {
    let space_id = space_of(row, KIND_USER_ACTION_REQUESTS)?;
    let resolution = row
        .fields
        .get("resolution")
        .filter(|v| v.is_object())
        .and_then(|v| serde_json::from_value::<UserActionResolution>(v.clone()).ok());
    let resolved =
        resolution.is_some() || row.fields.get("state").and_then(Value::as_str) == Some("resolved");
    Ok(DecodedRequest {
        view: UserActionRequestView {
            options: row
                .fields
                .get("options")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            links: row
                .fields
                .get("links")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            facts: row
                .fields
                .get("facts")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            kind: Some(
                row.fields
                    .get("kind")
                    .map(|v| serde_json::from_value(v.clone()).unwrap_or_default())
                    .unwrap_or_else(|| {
                        mast_proto::work_outline_user_actions::classify_legacy_request(
                            &str_field(row, "text").unwrap_or_default(),
                            str_field(row, "dedupeKey").as_deref(),
                        )
                    }),
            ),
            priority: row
                .fields
                .get("priority")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            id: row.id.clone(),
            space_id,
            work_item_id: str_field(row, "workItemId").unwrap_or_default(),
            chat_id: str_field(row, "chatId").unwrap_or_default(),
            origin_kind: match row.fields.get("originKind").and_then(Value::as_str) {
                Some("system") => UserActionOriginKind::System,
                _ => UserActionOriginKind::Chat,
            },
            message_id: str_field(row, "messageId").filter(|s| !s.is_empty()),
            tool_call_id: str_field(row, "toolCallId").filter(|s| !s.is_empty()),
            text: str_field(row, "text").unwrap_or_default(),
            info: str_field(row, "info").filter(|s| !s.is_empty()),
            dedupe_key: str_field(row, "dedupeKey").filter(|s| !s.is_empty()),
            created_by: actor_field(row, "createdBy"),
            created_at: i64_field(row, "createdAt").unwrap_or(0),
            state: if resolved {
                UserActionRequestState::Resolved
            } else {
                UserActionRequestState::Open
            },
            resolution,
            deduped: false,
            origin_chat_available: false,
        },
        created_clock: row.clocks.get("createdAt").cloned(),
    })
}

/// Duplicate-open collapse (registry audit 7.6): among OPEN requests with
/// equal `(workItemId, dedupeKey)`, the minimal `(creation clock, id)` is
/// canonical and the rest are `deduped`. Also stamps
/// `originChatAvailable` from `available_chat`.
pub(crate) fn finish_requests(
    requests: &mut [DecodedRequest],
    available_chat: impl Fn(&str) -> bool,
) {
    let mut canonical: BTreeMap<(String, String), (Option<String>, String)> = BTreeMap::new();
    for request in requests.iter() {
        let Some(key) = request.view.dedupe_key.as_ref() else {
            continue;
        };
        if !request.view.is_open() {
            continue;
        }
        let group = (request.view.work_item_id.clone(), key.clone());
        let candidate = (request.created_clock.clone(), request.view.id.clone());
        canonical
            .entry(group)
            .and_modify(|kept| {
                if candidate < *kept {
                    *kept = candidate.clone();
                }
            })
            .or_insert(candidate);
    }
    for request in requests.iter_mut() {
        request.view.origin_chat_available = request.view.origin_kind.is_chat()
            && !request.view.chat_id.is_empty()
            && available_chat(&request.view.chat_id);
        if let Some(key) = request.view.dedupe_key.as_ref()
            && request.view.is_open()
        {
            let group = (request.view.work_item_id.clone(), key.clone());
            request.view.deduped = canonical
                .get(&group)
                .is_some_and(|(_, id)| *id != request.view.id);
        }
    }
}

/// Decode one `workItemInstructionProposals` row leniently. `approval`
/// looks up the approval instruction (`derive_id(id, "instruction")`) among
/// live instruction rows: when it exists the proposal reads approved (its
/// review synthesized from the instruction if a concurrent reject won the
/// register).
pub(crate) fn decode_proposal(
    row: &RegistryRow,
    approval: impl Fn(&str) -> Option<WorkItemInstructionView>,
) -> Result<WorkItemInstructionProposalView, WorkOutlineRepair> {
    let space_id = space_of(row, KIND_WORK_ITEM_INSTRUCTION_PROPOSALS)?;
    let stored: Option<InstructionProposalReview> = row
        .fields
        .get("review")
        .filter(|v| v.is_object())
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let instruction_id = derive_id(&row.id, roles::INSTRUCTION);
    let (review, status) = match approval(&instruction_id) {
        Some(instruction) => {
            let review = match stored {
                Some(review) if review.state == ProposalReviewState::Approved => review,
                _ => InstructionProposalReview {
                    state: ProposalReviewState::Approved,
                    by: instruction.created_by.clone().unwrap_or_default(),
                    at: instruction.created_at,
                    reason: None,
                    instruction_id: Some(instruction_id.clone()),
                },
            };
            (Some(review), InstructionProposalStatus::Approved)
        }
        None => {
            let status = match stored.as_ref().map(|r| r.state) {
                Some(ProposalReviewState::Approved) => InstructionProposalStatus::Approved,
                Some(ProposalReviewState::Rejected) => InstructionProposalStatus::Rejected,
                _ => InstructionProposalStatus::Pending,
            };
            (stored, status)
        }
    };
    Ok(WorkItemInstructionProposalView {
        id: row.id.clone(),
        space_id,
        target: row
            .fields
            .get("target")
            .filter(|v| v.is_object())
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(InstructionProposalTarget {
                scope: ProposalTargetScope::Unknown,
                ..Default::default()
            }),
        text: str_field(row, "text").unwrap_or_default(),
        proposed_by: actor_field(row, "proposedBy"),
        source_ref: row
            .fields
            .get("sourceRef")
            .filter(|v| v.is_object())
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        created_at: i64_field(row, "createdAt").unwrap_or(0),
        action: row
            .fields
            .get("action")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        instruction_id: str_field(row, "instructionId"),
        base_text: str_field(row, "baseText"),
        base_updated_at: i64_field(row, "baseUpdatedAt").unwrap_or(0),
        rationale: str_field(row, "rationale"),
        review,
        status,
    })
}

/// The materialized request and proposal lists of one Space.
pub(crate) struct MaterializedUserActions {
    /// Unresolved (open, canonical) requests per work item.
    pub unresolved_by_item: BTreeMap<String, u32>,
    pub requests: Vec<UserActionRequestView>,
    pub proposals: Vec<WorkItemInstructionProposalView>,
}

/// Step 9 for Milestone 4: decode, dedupe, and bound the lists. Every open
/// request (deduped ones flagged) and every pending proposal is listed; of
/// the rest only the newest `MAX_RECENT_*` by resolution / review time
/// (ties by id). Both lists are sorted `(createdAt, id)`. A malformed row is
/// reported only when its work item lives here.
pub(crate) fn materialize_user_actions(
    space_id: &str,
    inputs: &WorkOutlineInputs,
    has_item: impl Fn(&str) -> bool,
    repairs: &mut BTreeSet<WorkOutlineRepair>,
) -> MaterializedUserActions {
    let available: BTreeSet<&str> = inputs
        .chats
        .iter()
        .filter(|chat| !chat.archived)
        .map(|chat| chat.id.as_str())
        .collect();
    let mut decoded: Vec<DecodedRequest> = Vec::new();
    for row in &inputs.user_action_requests {
        match decode_request(row) {
            Ok(request) if request.view.space_id == space_id => decoded.push(request),
            Ok(_) => {}
            Err(repair) => {
                if str_field(row, "workItemId").is_some_and(|w| has_item(&w)) {
                    repairs.insert(repair);
                }
            }
        }
    }
    finish_requests(&mut decoded, |chat| available.contains(chat));
    let mut unresolved_by_item: BTreeMap<String, u32> = BTreeMap::new();
    let (mut open, mut resolved): (Vec<UserActionRequestView>, Vec<UserActionRequestView>) =
        decoded
            .into_iter()
            .map(|request| request.view)
            .partition(UserActionRequestView::is_open);
    for request in &open {
        if request.is_unresolved() {
            *unresolved_by_item
                .entry(request.work_item_id.clone())
                .or_default() += 1;
        }
    }
    resolved.sort_by(|a, b| {
        let at = |r: &UserActionRequestView| r.resolution.as_ref().map_or(0, |res| res.at);
        at(b).cmp(&at(a)).then_with(|| a.id.cmp(&b.id))
    });
    resolved.truncate(user_action_limits::MAX_RECENT_RESOLVED_REQUESTS);
    open.extend(resolved);
    open.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));

    let instruction_rows: HashMap<&str, &RegistryRow> = inputs
        .instructions
        .iter()
        .map(|row| (row.id.as_str(), row))
        .collect();
    let approval = |id: &str| {
        instruction_rows
            .get(id)
            .and_then(|row| decode_instruction(row).ok())
    };
    let mut pending: Vec<WorkItemInstructionProposalView> = Vec::new();
    let mut reviewed: Vec<WorkItemInstructionProposalView> = Vec::new();
    for row in &inputs.instruction_proposals {
        match decode_proposal(row, approval) {
            Ok(proposal) if proposal.space_id != space_id => {}
            Ok(proposal) if proposal.status == InstructionProposalStatus::Pending => {
                pending.push(proposal);
            }
            Ok(proposal) => reviewed.push(proposal),
            Err(repair) => {
                let target_here = row
                    .fields
                    .get("target")
                    .and_then(|t| t.get("workItemId"))
                    .and_then(Value::as_str)
                    .is_some_and(&has_item);
                if target_here {
                    repairs.insert(repair);
                }
            }
        }
    }
    reviewed.sort_by(|a, b| {
        let at = |p: &WorkItemInstructionProposalView| p.review.as_ref().map_or(0, |r| r.at);
        at(b).cmp(&at(a)).then_with(|| a.id.cmp(&b.id))
    });
    reviewed.truncate(user_action_limits::MAX_RECENT_REVIEWED_PROPOSALS);
    pending.extend(reviewed);
    pending.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    MaterializedUserActions {
        unresolved_by_item,
        requests: open,
        proposals: pending,
    }
}

impl RegistryDoc {
    // ── reads ───────────────────────────────────────────────────────────────

    /// Request and proposal rows of one Space (materializer inputs).
    pub(super) fn user_action_inputs(
        &self,
        space_id: &str,
    ) -> (Vec<RegistryRow>, Vec<RegistryRow>) {
        let of_space = |kind: &str| {
            self.overlay_rows(kind)
                .into_iter()
                .filter(|row| row.fields.get("spaceId").and_then(Value::as_str) == Some(space_id))
                .collect::<Vec<_>>()
        };
        (
            of_space(KIND_USER_ACTION_REQUESTS),
            of_space(KIND_WORK_ITEM_INSTRUCTION_PROPOSALS),
        )
    }

    fn chat_available(&self, chat_id: &str) -> bool {
        self.chat(chat_id)
            .ok()
            .flatten()
            .is_some_and(|chat| !chat.archived)
    }

    /// Every live request on `work_item_id`, deduped.
    fn requests_of_item(&self, work_item_id: &str) -> Vec<DecodedRequest> {
        let mut requests: Vec<DecodedRequest> = self
            .overlay_rows(KIND_USER_ACTION_REQUESTS)
            .iter()
            .filter(|row| {
                row.fields.get("workItemId").and_then(Value::as_str) == Some(work_item_id)
            })
            .filter_map(|row| decode_request(row).ok())
            .collect();
        finish_requests(&mut requests, |chat| self.chat_available(chat));
        requests
    }

    /// One live request, with its dedupe flag and origin availability.
    pub fn user_action_request(&self, id: &str) -> Option<UserActionRequestView> {
        let row = self.overlay_row(KIND_USER_ACTION_REQUESTS, id)?;
        let request = decode_request(&row).ok()?;
        self.requests_of_item(&request.view.work_item_id)
            .into_iter()
            .find(|r| r.view.id == id)
            .map(|r| r.view)
    }

    /// One live proposal with its effective status.
    pub fn instruction_proposal(&self, id: &str) -> Option<WorkItemInstructionProposalView> {
        let row = self.overlay_row(KIND_WORK_ITEM_INSTRUCTION_PROPOSALS, id)?;
        decode_proposal(&row, |instruction_id| {
            self.overlay_row(KIND_WORK_ITEM_INSTRUCTIONS, instruction_id)
                .and_then(|row| decode_instruction(&row).ok())
        })
        .ok()
    }

    /// Resolution writes for every open request in `requests` (one op each).
    fn plan_resolutions<'a>(
        requests: impl IntoIterator<Item = &'a UserActionRequestView>,
        actor: &WorkOutlineActor,
        now_ms: i64,
        note: Option<&str>,
        answer: Option<&UserActionAnswer>,
        outcome: Option<&str>,
        values: Option<&serde_json::Value>,
        via: Option<&str>,
    ) -> Vec<Planned> {
        let mut resolution = json!({ "by": actor_row_value(actor), "at": now_ms });
        if let Some(note) = note {
            resolution["note"] = json!(note);
        }
        if let Some(answer) = answer {
            resolution["answer"] = json!({
                "chatId": answer.chat_id,
                "messageId": answer.message_id,
                "text": answer.text,
            });
        }
        if let Some(outcome) = outcome {
            resolution["outcome"] = json!(outcome);
        }
        if let Some(values) = values {
            resolution["values"] = values.clone();
        }
        if let Some(via) = via {
            resolution["via"] = json!(via);
        }
        requests
            .into_iter()
            .filter(|request| request.is_open())
            .map(|request| {
                Planned::update(
                    KIND_USER_ACTION_REQUESTS,
                    &request.id,
                    BTreeMap::from([
                        ("state".to_owned(), json!("resolved")),
                        ("resolution".to_owned(), resolution.clone()),
                    ]),
                )
            })
            .collect()
    }

    /// The completion gate of `updateWorkItem` (status `completed`): with
    /// open requests on the item, `resolve == false` is
    /// `openUserActionRequests` (count = unresolved, non-deduped requests);
    /// `resolve == true` plans an explicit resolution for every open request
    /// (duplicates included) to commit with the status change.
    pub(super) fn plan_completion_resolutions(
        &self,
        work_item_id: &str,
        resolve: bool,
        actor: &WorkOutlineActor,
        now_ms: i64,
    ) -> Outcome<Vec<Planned>> {
        let requests: Vec<UserActionRequestView> = self
            .requests_of_item(work_item_id)
            .into_iter()
            .map(|r| r.view)
            .filter(UserActionRequestView::is_open)
            .collect();
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        if !resolve {
            let count = requests.iter().filter(|r| r.is_unresolved()).count().max(1);
            return Err(WorkOutlineError::open_user_action_requests(count));
        }
        Ok(Self::plan_resolutions(
            &requests,
            actor,
            now_ms,
            Some(COMPLETION_RESOLUTION_NOTE),
            None,
            None,
            None,
            None,
        ))
    }

    // ── request mutations ──────────────────────────────────────────────────

    /// `createUserActionRequest`: id = `derive_id(operationId, "request")`;
    /// a replay returns the same id without writing, other inputs are
    /// `operationConflict`, a deleted id is never revived. The work item
    /// must be live and not archived; the origin Chat must exist and belong
    /// to the item's Space. A duplicate of an open request (same
    /// `dedupeKey`) is written and reads as `deduped`.
    pub fn create_user_action_request(
        &mut self,
        p: &CreateUserActionRequestParams,
        actor: &WorkOutlineActor,
        now_ms: i64,
    ) -> Outcome<String> {
        check_operation_id(&p.operation_id)?;
        check_id("work item", &p.work_item_id)?;
        validate_request_content(p)?;
        let system_origin = p.origin.kind == UserActionOriginKind::System;
        if system_origin {
            if actor.kind != WorkOutlineActorKind::System {
                return Err(unauthorized(
                    "Only the system may raise a request without an origin Chat",
                ));
            }
            if !p.origin.chat_id.is_empty()
                || p.origin.message_id.is_some()
                || p.origin.tool_call_id.is_some()
            {
                return Err(invalid("A system request has no origin Chat"));
            }
        } else {
            check_id("origin Chat", &p.origin.chat_id)?;
        }
        check_origin_id("message", p.origin.message_id.as_deref())?;
        check_origin_id("tool call", p.origin.tool_call_id.as_deref())?;
        let id = derive_id(&p.operation_id, roles::REQUEST);
        let hash = inputs_hash(p);
        match self.row_state(KIND_USER_ACTION_REQUESTS, &id) {
            RowState::Live => {
                let stored = self
                    .overlay_row(KIND_USER_ACTION_REQUESTS, &id)
                    .and_then(|row| row.fields.get("inputsHash").cloned());
                return if stored == Some(json!(hash)) {
                    Ok(id)
                } else {
                    Err(err(
                        WorkOutlineErrorCode::OperationConflict,
                        "operationId was already used for a different request",
                    ))
                };
            }
            RowState::Tombstoned => return Err(not_found("This request was deleted")),
            RowState::Absent => {}
        }
        let item = self.live_item(&p.work_item_id)?;
        self.require_space(&item.space_id)?;
        if item.archived() {
            return Err(err(
                WorkOutlineErrorCode::Archived,
                "The work item is archived",
            ));
        }
        if !system_origin {
            let chat = self
                .chat(&p.origin.chat_id)
                .ok()
                .flatten()
                .ok_or_else(|| not_found("The origin Chat was not found"))?;
            match chat.space_id.as_deref().filter(|s| !s.is_empty()) {
                None => {
                    return Err(err(
                        WorkOutlineErrorCode::Projectless,
                        "Chats outside a project cannot raise work item requests",
                    ));
                }
                Some(space) if space != item.space_id => {
                    return Err(err(
                        WorkOutlineErrorCode::CrossSpace,
                        "The origin Chat belongs to another Space",
                    ));
                }
                Some(_) => {}
            }
        }
        let mut set = BTreeMap::from([
            ("spaceId".to_owned(), json!(item.space_id)),
            ("workItemId".to_owned(), json!(p.work_item_id)),
            ("chatId".to_owned(), json!(p.origin.chat_id)),
            ("text".to_owned(), json!(p.text)),
            ("state".to_owned(), json!("open")),
            ("createdBy".to_owned(), actor_row_value(actor)),
            ("createdAt".to_owned(), json!(now_ms)),
            ("inputsHash".to_owned(), json!(hash)),
            ("v".to_owned(), json!(WORK_OUTLINE_ROW_VERSION)),
        ]);
        if !p.options.is_empty() {
            set.insert("options".to_owned(), json!(p.options));
        }
        set.insert("links".to_owned(), json!(p.links));
        set.insert("facts".to_owned(), json!(p.facts));
        set.insert("kind".to_owned(), json!(p.kind.unwrap_or_default()));
        if let Some(priority) = p.priority {
            set.insert("priority".to_owned(), json!(priority));
        }
        if system_origin {
            set.insert("originKind".to_owned(), json!("system"));
        }
        for (field, value) in [
            ("messageId", &p.origin.message_id),
            ("toolCallId", &p.origin.tool_call_id),
            ("info", &p.info),
            ("dedupeKey", &p.dedupe_key),
        ] {
            if let Some(value) = value {
                set.insert(field.to_owned(), json!(value));
            }
        }
        self.commit(
            vec![Planned::upsert(KIND_USER_ACTION_REQUESTS, &id, set)],
            &[(KIND_WORK_ITEMS, p.work_item_id.as_str())],
        )?;
        Ok(id)
    }

    /// Replace content without changing identity or resolution state.
    pub fn update_user_action_request(
        &mut self,
        p: &mast_proto::UpdateUserActionRequestParams,
    ) -> Outcome<()> {
        check_operation_id(&p.request.operation_id)?;
        validate_request_content(&p.request)?;
        let request = self.user_action_request(&p.user_action_request_id)
            .ok_or_else(|| not_found("Request not found"))?;
        if !request.is_open() || request.work_item_id != p.request.work_item_id
            || request.dedupe_key != p.request.dedupe_key {
            return Err(invalid("Request is resolved or does not match its key and work item"));
        }
        let set = BTreeMap::from([
            ("chatId".to_owned(), json!(p.request.origin.chat_id)),
            ("originKind".to_owned(), json!(p.request.origin.kind)),
            ("messageId".to_owned(), json!(p.request.origin.message_id)),
            ("toolCallId".to_owned(), json!(p.request.origin.tool_call_id)),
            ("text".to_owned(), json!(p.request.text)),
            ("info".to_owned(), json!(p.request.info)),
            ("options".to_owned(), json!(p.request.options)),
            ("links".to_owned(), json!(p.request.links)),
            ("facts".to_owned(), json!(p.request.facts)),
            ("kind".to_owned(), json!(p.request.kind.unwrap_or_default())),
            ("priority".to_owned(), json!(p.request.priority)),
        ]);
        self.commit(vec![Planned::update(KIND_USER_ACTION_REQUESTS, &request.id, set)],
            &[(KIND_WORK_ITEMS, request.work_item_id.as_str())])?;
        Ok(())
    }

    /// `resolveUserActionRequest`: one resolution op per open request of the
    /// dedupe group (the request and its open duplicates), one batch.
    /// Resolving a resolved request writes nothing (monotone, no reopen).
    pub fn resolve_user_action_request(
        &mut self,
        p: &ResolveUserActionRequestParams,
        actor: &WorkOutlineActor,
        now_ms: i64,
    ) -> Outcome<()> {
        check_id("request", &p.user_action_request_id)?;
        check_note("The resolution note", p.note.as_deref())?;
        if let Some(outcome) = p.outcome.as_deref() {
            if ![
                "approved",
                "declined",
                "changesRequested",
                "chosen",
                "answered",
                "done",
                "dismissed",
                "merged",
                "superseded",
            ]
            .contains(&outcome)
            {
                return Err(WorkOutlineError::new(
                    WorkOutlineErrorCode::Invalid,
                    "Invalid resolution outcome",
                ));
            }
            if actor.kind == WorkOutlineActorKind::Agent
                && !["done", "superseded", "declined"].contains(&outcome)
                && !(outcome == "answered" && p.answer.is_some())
            {
                return Err(WorkOutlineError::new(
                    WorkOutlineErrorCode::Invalid,
                    "Agents cannot record this outcome",
                ));
            }
        }
        if let Some(via) = p.via.as_deref()
            && (!["card", "chat", "agent", "prMerged"].contains(&via)
                || (via == "card" && actor.kind != WorkOutlineActorKind::User)
                || (via == "chat" && p.answer.is_none())
                || (via == "agent" && actor.kind != WorkOutlineActorKind::Agent)
                || (via == "prMerged" && actor.kind != WorkOutlineActorKind::System))
        {
            return Err(WorkOutlineError::new(
                WorkOutlineErrorCode::Invalid,
                "Invalid resolution provenance",
            ));
        }
        if p.values
            .as_ref()
            .is_some_and(|v| v.to_string().len() > 4000)
        {
            return Err(WorkOutlineError::new(
                WorkOutlineErrorCode::Invalid,
                "Resolution values are too large",
            ));
        }

        if let Some(answer) = &p.answer {
            // Only the engine's `answer_user_action_request` writes this, as
            // an agent actor; it verified the message before calling.
            if actor.kind != WorkOutlineActorKind::Agent {
                return Err(WorkOutlineError::new(
                    WorkOutlineErrorCode::Invalid,
                    "Only an agent carries the user's chat message as an answer",
                ));
            }
            check_id("answer chat", &answer.chat_id)?;
            check_id("answer message", &answer.message_id)?;
            check_text(
                "The answer",
                &answer.text,
                user_action_limits::MAX_NOTE_CHARS,
                true,
            )?;
        }
        let request = self
            .user_action_request(&p.user_action_request_id)
            .ok_or_else(|| not_found("Request not found"))?;
        self.require_space(&request.space_id)?;
        if !request.is_open() {
            return Ok(());
        }
        if p.via.as_deref() == Some("prMerged")
            && request.kind != Some(mast_proto::work_outline_user_actions::UserActionKind::Handoff)
        {
            return Err(WorkOutlineError::new(WorkOutlineErrorCode::Invalid, "Only handoffs may be resolved by a merged PR"));
        }
        // An agent resolves only the members of the group it may resolve: its
        // own Chat's and, for a coordinator, its direct children's (the same
        // rule `authorize_agent_mutation` applies to the target). A duplicate
        // another Chat raised is that Chat's request and stays open (the
        // user's resolve keeps whole-group behavior). The target is always in
        // the group: filtering it out by the caller's Chat id once made a
        // coordinator's resolve of its child's deduped request write nothing.
        let agent_chat = match actor.kind {
            WorkOutlineActorKind::Agent => Some(actor.chat_id.clone().unwrap_or_default()),
            _ => None,
        };
        let group: Vec<UserActionRequestView> = if p.via.as_deref() == Some("prMerged") {
            vec![request]
        } else { match &request.dedupe_key {
            None => vec![request],
            Some(key) => self
                .requests_of_item(&request.work_item_id)
                .into_iter()
                .map(|r| r.view)
                .filter(|r| r.is_open() && r.dedupe_key.as_ref() == Some(key))
                .filter(|r| {
                    r.id == p.user_action_request_id
                        || agent_chat
                            .as_ref()
                            .is_none_or(|chat| self.agent_may_resolve_request(chat, r))
                })
                .collect(),
        }};
        let plan = Self::plan_resolutions(
            &group,
            actor,
            now_ms,
            p.note.as_deref(),
            p.answer.as_ref(),
            p.outcome.as_deref(),
            p.values.as_ref(),
            p.via.as_deref(),
        );
        self.commit(plan, &[])
    }

    // ── proposal mutations ─────────────────────────────────────────────────

    /// `proposeWorkItemInstruction`: id = `derive_id(operationId,
    /// "proposal")`, written once (replay and conflict rules as requests).
    /// An agent's proposal without an origin Chat records the agent's Chat.
    pub fn propose_work_item_instruction(
        &mut self,
        p: &ProposeWorkItemInstructionParams,
        actor: &WorkOutlineActor,
        now_ms: i64,
    ) -> Outcome<String> {
        check_operation_id(&p.operation_id)?;
        check_note("The rationale", p.rationale.as_deref())?;
        let existing = match p.action {
            ProposalAction::Add => {
                if p.instruction_id.is_some() {
                    return Err(invalid("Only an edit or remove names an instruction"));
                }
                check_instruction_text(&p.text)?;
                None
            }
            ProposalAction::Edit | ProposalAction::Remove => {
                let instruction_id = p
                    .instruction_id
                    .as_deref()
                    .ok_or_else(|| invalid("An edit or remove proposal needs an instructionId"))?;
                let instruction = self.live_instruction(instruction_id)?;
                if instruction.archived() {
                    return Err(err(
                        WorkOutlineErrorCode::Archived,
                        "The instruction is archived",
                    ));
                }
                if p.action == ProposalAction::Edit {
                    check_instruction_text(&p.text)?;
                } else if !p.text.is_empty() {
                    return Err(invalid("A remove proposal carries no text (use rationale)"));
                }
                Some(instruction)
            }
        };
        if let Some(chat) = &p.origin.chat_id {
            check_id("origin Chat", chat)?;
        }
        check_origin_id("message", p.origin.message_id.as_deref())?;
        check_origin_id("tool call", p.origin.tool_call_id.as_deref())?;
        let id = derive_id(&p.operation_id, roles::PROPOSAL);
        let hash = inputs_hash(p);
        match self.row_state(KIND_WORK_ITEM_INSTRUCTION_PROPOSALS, &id) {
            RowState::Live => {
                let stored = self
                    .overlay_row(KIND_WORK_ITEM_INSTRUCTION_PROPOSALS, &id)
                    .and_then(|row| row.fields.get("inputsHash").cloned());
                return if stored == Some(json!(hash)) {
                    Ok(id)
                } else {
                    Err(err(
                        WorkOutlineErrorCode::OperationConflict,
                        "operationId was already used for a different proposal",
                    ))
                };
            }
            RowState::Tombstoned => return Err(not_found("This proposal was deleted")),
            RowState::Absent => {}
        }
        let mut origin = p.origin.clone();
        if origin.chat_id.is_none() && actor.kind == WorkOutlineActorKind::Agent {
            origin.chat_id = actor.chat_id.clone();
        }
        let origin_space = match &origin.chat_id {
            None => None,
            Some(chat_id) => {
                let chat = self
                    .chat(chat_id)
                    .ok()
                    .flatten()
                    .ok_or_else(|| not_found("The origin Chat was not found"))?;
                Some(chat.space_id.filter(|s| !s.is_empty()).ok_or_else(|| {
                    err(
                        WorkOutlineErrorCode::Projectless,
                        "Chats outside a project cannot propose instructions",
                    )
                })?)
            }
        };
        let (space_id, target) = if let Some(instruction) = &existing {
            let target = if let Some(rule_instruction_id) = &instruction.template_instruction_id {
                let item = instruction.work_item_id.as_deref().ok_or_else(|| invalid("Template instruction has no work item"))?;
                let (container, rule_node, followed_item) = self.followed_template_rule(&instruction.space_id, item)
                    .ok_or_else(|| invalid("The instruction no longer follows a child template"))?;
                if followed_item != item {
                    return Err(invalid("The instruction no longer follows a child template"));
                }
                let template = self.live_item(&container)?.container_template.ok_or_else(|| invalid("Child template not found"))?;
                if !template.root.find(&rule_node).is_some_and(|node| node.instructions.iter().any(|i| i.id == *rule_instruction_id)) {
                    return Err(invalid("The template instruction no longer exists"));
                }
                InstructionProposalTarget {
                    scope: ProposalTargetScope::ChildTemplate,
                    work_item_id: Some(item.to_owned()),
                    container_work_item_id: Some(container),
                    rule_node_id: Some(rule_node),
                    rule_instruction_id: Some(rule_instruction_id.clone()),
                    rule_node_path: Vec::new(),
                }
            } else { match &instruction.work_item_id {
                Some(id) => {
                    if self.live_item(id)?.archived() {
                        return Err(err(
                            WorkOutlineErrorCode::Archived,
                            "The work item is archived",
                        ));
                    }
                    InstructionProposalTarget::work_item(id)
                }
                None => InstructionProposalTarget::space_root(),
            }};
            (instruction.space_id.clone(), target)
        } else {
            match p.target.scope {
                ProposalTargetScope::WorkItem => {
                    let work_item_id = p
                        .target
                        .work_item_id
                        .as_deref()
                        .ok_or_else(|| invalid("A work-item proposal needs a workItemId"))?;
                    check_id("work item", work_item_id)?;
                    let item = self.live_item(work_item_id)?;
                    if item.archived() {
                        return Err(err(
                            WorkOutlineErrorCode::Archived,
                            "The work item is archived",
                        ));
                    }
                    if p.space_id.as_ref().is_some_and(|s| *s != item.space_id) {
                        return Err(err(
                            WorkOutlineErrorCode::CrossSpace,
                            "The work item belongs to another Space",
                        ));
                    }
                    (
                        item.space_id,
                        InstructionProposalTarget::work_item(work_item_id),
                    )
                }
                ProposalTargetScope::SpaceRoot => {
                    if p.target.work_item_id.is_some() {
                        return Err(invalid("A Space-root proposal cannot name a work item"));
                    }
                    let space = p
                        .space_id
                        .clone()
                        .or_else(|| origin_space.clone())
                        .ok_or_else(|| {
                            invalid("A Space-root proposal needs a spaceId or an origin Chat")
                        })?;
                    (space, InstructionProposalTarget::space_root())
                }
                ProposalTargetScope::ChildTemplate => {
                    let item_id = p.target.work_item_id.as_deref().ok_or_else(|| invalid("A child-template proposal needs a followed item"))?;
                    let item = self.live_item(item_id)?;
                    if item.archived() { return Err(err(WorkOutlineErrorCode::Archived, "The work item is archived")); }
                    if p.space_id.as_ref().is_some_and(|space| *space != item.space_id) {
                        return Err(err(WorkOutlineErrorCode::CrossSpace, "The work item belongs to another Space"));
                    }
                    let tree = self.outline_tree(&item.space_id);
                    let direct_template = tree.parent.get(item_id).and_then(|parent| parent.as_ref())
                        .and_then(|parent| tree.items.get(parent).and_then(|item| item.container_template.as_ref().map(|template| (parent.clone(), template.root.id.clone(), item_id.to_owned()))));
                    let (container, followed_rule, followed) = direct_template
                        .or_else(|| self.followed_template_rule(&item.space_id, item_id))
                        .ok_or_else(|| invalid("The item does not follow a child template and is not a direct child of a template container"))?;
                    if p.target.container_work_item_id.as_deref().is_some_and(|id| id != container) {
                        return Err(invalid("The item does not follow the requested template"));
                    }
                    if p.target.rule_node_id.is_some() && !p.target.rule_node_path.is_empty() {
                        return Err(invalid("Use either ruleNodeId or ruleNodePath"));
                    }
                    let template = self.live_item(&container)?.container_template.ok_or_else(|| invalid("Child template not found"))?;
                    let rule_node = if !p.target.rule_node_path.is_empty() {
                        let mut node = &template.root;
                        for title in &p.target.rule_node_path {
                            let mut matches = node.children.iter().filter(|child| child.title == *title);
                            node = matches.next().ok_or_else(|| invalid("Template rule path not found"))?;
                            if matches.next().is_some() { return Err(invalid("Template rule path is ambiguous")); }
                        }
                        node.id.clone()
                    } else { p.target.rule_node_id.clone().unwrap_or(followed_rule) };
                    if template.root.find(&rule_node).is_none() { return Err(invalid("Template rule not found")); }
                    let root_of = |id: &str| -> Option<String> {
                        let mut current = id.to_owned();
                        loop {
                            let parent = tree.parent.get(&current)?.as_ref()?;
                            if parent == &container { return Some(current); }
                            current = parent.clone();
                        }
                    };
                    let child_root = root_of(&followed).ok_or_else(|| invalid("The followed item left its container"))?;
                    let target_item = tree.items.values().find(|candidate| {
                        candidate.template_node_id.as_deref() == Some(rule_node.as_str())
                            && root_of(&candidate.id).as_deref() == Some(child_root.as_str())
                            && self.followed_template_rule(&item.space_id, &candidate.id).is_some_and(|(c, n, f)| c == container && n == rule_node && f == candidate.id)
                    }).or_else(|| (rule_node == template.root.id && child_root == item_id).then_some(&item))
                        .ok_or_else(|| invalid("This child does not follow the requested template rule"))?;
                    (item.space_id.clone(), InstructionProposalTarget {
                        scope: ProposalTargetScope::ChildTemplate,
                        work_item_id: Some(target_item.id.clone()),
                        container_work_item_id: Some(container),
                        rule_node_id: Some(rule_node),
                        rule_instruction_id: None,
                        rule_node_path: Vec::new(),
                    })
                }
                ProposalTargetScope::Unknown => {
                    return Err(invalid("Unknown proposal target scope"));
                }
            }
        };
        self.require_space(&space_id)?;
        if origin_space.as_ref().is_some_and(|s| *s != space_id) {
            return Err(err(
                WorkOutlineErrorCode::CrossSpace,
                "The origin Chat belongs to another Space",
            ));
        }
        let mut set = BTreeMap::from([
            ("spaceId".to_owned(), json!(space_id)),
            (
                "target".to_owned(),
                serde_json::to_value(&target).unwrap_or(Value::Null),
            ),
            ("text".to_owned(), json!(p.text)),
            ("proposedBy".to_owned(), actor_row_value(actor)),
            (
                "sourceRef".to_owned(),
                serde_json::to_value(&origin).unwrap_or(Value::Null),
            ),
            ("createdAt".to_owned(), json!(now_ms)),
            ("inputsHash".to_owned(), json!(hash)),
            ("v".to_owned(), json!(WORK_OUTLINE_ROW_VERSION)),
        ]);
        if let Some(instruction) = &existing {
            let base_text = if target.scope == ProposalTargetScope::ChildTemplate {
                let container = target.container_work_item_id.as_deref().ok_or_else(|| invalid("The proposal has no template container"))?;
                let node = target.rule_node_id.as_deref().ok_or_else(|| invalid("The proposal has no template rule"))?;
                let rule_id = target.rule_instruction_id.as_deref().ok_or_else(|| invalid("The proposal has no template instruction"))?;
                let template = self.live_item(container)?.container_template.ok_or_else(|| invalid("Child template not found"))?;
                template.root.find(node).and_then(|node| node.instructions.iter().find(|rule| rule.id == rule_id))
                    .ok_or_else(|| invalid("The template instruction no longer exists"))?.text.clone()
            } else { instruction.text.clone() };
            if p.action == ProposalAction::Edit && p.text == base_text {
                return Err(invalid("The proposed text equals the current text"));
            }
            set.insert("action".to_owned(), json!(p.action.as_str()));
            set.insert("instructionId".to_owned(), json!(target.rule_instruction_id.as_deref().unwrap_or(&instruction.id)));
            set.insert("baseText".to_owned(), json!(base_text));
            set.insert("baseUpdatedAt".to_owned(), json!(instruction.updated_at));
        }
        if let Some(rationale) = p.rationale.as_ref().filter(|r| !r.is_empty()) {
            set.insert("rationale".to_owned(), json!(rationale));
        }
        let extra: Vec<(&str, &str)> = target
            .work_item_id
            .as_deref()
            .map(|w| (KIND_WORK_ITEMS, w))
            .into_iter()
            .collect();
        self.commit(
            vec![Planned::upsert(
                KIND_WORK_ITEM_INSTRUCTION_PROPOSALS,
                &id,
                set,
            )],
            &extra,
        )?;
        Ok(id)
    }

    /// `approveWorkItemInstructionProposal`: ONE batch with the instruction
    /// `derive_id(proposalId, "instruction")` (text = `approvedText`,
    /// `sourceProposalId`, source `{chatId, messageId, excerpt = proposal
    /// text (first 1,500 characters), contentHash = sha256(excerpt)}`,
    /// placement, budgets) and the `review`. Already approved (the
    /// instruction exists): returns its id without writing. Approving a
    /// rejected proposal is allowed (approval wins, as on concurrent
    /// review).
    pub fn approve_work_item_instruction_proposal(
        &mut self,
        p: &ApproveWorkItemInstructionProposalParams,
        actor: &WorkOutlineActor,
        now_ms: i64,
    ) -> Outcome<String> {
        check_operation_id(&p.operation_id)?;
        check_id("proposal", &p.proposal_id)?;
        let proposal = self
            .instruction_proposal(&p.proposal_id)
            .ok_or_else(|| not_found("Proposal not found"))?;
        self.require_space(&proposal.space_id)?;
        if proposal.target.scope == ProposalTargetScope::ChildTemplate {
            return self.approve_template_instruction_proposal(p, &proposal, actor, now_ms);
        }
        if proposal.action != ProposalAction::Add {
            return self.approve_instruction_modification(p, &proposal, actor, now_ms);
        }
        check_instruction_text(&p.approved_text)?;
        let instruction_id = derive_id(&p.proposal_id, roles::INSTRUCTION);
        match self.row_state(KIND_WORK_ITEM_INSTRUCTIONS, &instruction_id) {
            RowState::Live => return Ok(instruction_id),
            RowState::Tombstoned => {
                return Err(not_found("The approved instruction was deleted"));
            }
            RowState::Absent => {}
        }
        let scope = match proposal.target.scope {
            ProposalTargetScope::SpaceRoot => InstructionScope::SpaceRoot,
            ProposalTargetScope::WorkItem => InstructionScope::WorkItem {
                work_item_id: proposal
                    .target
                    .work_item_id
                    .clone()
                    .ok_or_else(|| invalid("The proposal's target has no work item"))?,
            },
            ProposalTargetScope::ChildTemplate => return Err(invalid("Child-template proposal approval is not supported")),
            ProposalTargetScope::Unknown => {
                return Err(invalid("The proposal's target scope is not supported"));
            }
        };
        let work_item_id = self.instruction_scope_item(&proposal.space_id, &scope)?;
        let source = checked_source(Some(&InstructionSource {
            chat_id: proposal.source_ref.chat_id.clone(),
            message_id: proposal.source_ref.message_id.clone(),
            excerpt: Some(truncate_chars(
                &proposal.text,
                limits::MAX_INSTRUCTION_EXCERPT_CHARS,
            )),
            content_hash: None,
        }))?;
        let hash = inputs_hash(p);
        let mut plan = self.plan_new_instruction(
            NewInstruction {
                id: &instruction_id,
                space_id: &proposal.space_id,
                work_item_id: work_item_id.as_deref(),
                text: &p.approved_text,
                placement: p.placement.clone().unwrap_or_default(),
                source,
                source_proposal_id: Some(&p.proposal_id),
                inputs_hash: &hash,
                action: "Approving this proposal",
            },
            actor,
            now_ms,
        )?;
        plan.push(Planned::update(
            KIND_WORK_ITEM_INSTRUCTION_PROPOSALS,
            &p.proposal_id,
            BTreeMap::from([(
                "review".to_owned(),
                json!({
                    "state": "approved",
                    "by": actor_row_value(actor),
                    "at": now_ms,
                    "instructionId": instruction_id,
                }),
            )]),
        ));
        let extra: Vec<(&str, &str)> = work_item_id
            .as_deref()
            .map(|w| (KIND_WORK_ITEMS, w))
            .into_iter()
            .collect();
        self.commit(plan, &extra)?;
        Ok(instruction_id)
    }

    fn approve_template_instruction_proposal(
        &mut self,
        p: &ApproveWorkItemInstructionProposalParams,
        proposal: &WorkItemInstructionProposalView,
        actor: &WorkOutlineActor,
        now_ms: i64,
    ) -> Outcome<String> {
        use mast_proto::work_outline_container_templates::{RuleInstruction, RuleNode, SetContainerTemplateParams};
        fn find_mut<'a>(node: &'a mut RuleNode, id: &str) -> Option<&'a mut RuleNode> {
            if node.id == id { return Some(node); }
            node.children.iter_mut().find_map(|child| find_mut(child, id))
        }
        let result_id = if !p.project_only && proposal.action != ProposalAction::Add {
            proposal.target.rule_instruction_id.clone().ok_or_else(|| invalid("The proposal has no template instruction"))?
        } else {
            derive_id(&proposal.id, roles::INSTRUCTION)
        };
        if proposal.status == InstructionProposalStatus::Approved {
            return Ok(proposal.review.as_ref().and_then(|r| r.instruction_id.clone()).unwrap_or(result_id));
        }
        let target = &proposal.target;
        let container = target.container_work_item_id.as_deref().ok_or_else(|| invalid("The proposal has no template container"))?;
        let mut template = self.live_item(container)?.container_template.ok_or_else(|| invalid("The child template no longer exists"))?;
        let legacy_root = target.rule_node_id.is_none() && target.rule_node_path.is_empty()
            && proposal.action == ProposalAction::Add
            && target.work_item_id.as_deref().is_some_and(|id|
                self.outline_tree(&proposal.space_id).parent.get(id).and_then(|parent| parent.as_deref()) == Some(container));
        let node_id = target.rule_node_id.as_deref().or_else(|| legacy_root.then_some(template.root.id.as_str()))
            .ok_or_else(|| invalid("The proposal's template rule cannot be resolved"))?.to_owned();
        let root_rule = template.root.id == node_id;
        let node = find_mut(&mut template.root, &node_id).ok_or_else(|| invalid("The template rule no longer exists"))?;
        if proposal.action != ProposalAction::Add {
            let rule_id = target.rule_instruction_id.as_deref().ok_or_else(|| invalid("The proposal has no template instruction"))?;
            let current = node.instructions.iter().find(|i| i.id == rule_id).ok_or_else(|| invalid("The template instruction no longer exists"))?;
            if proposal.base_text.as_deref() != Some(current.text.as_str()) && !p.allow_stale {
                return Err(invalid("The template instruction changed after this proposal was made; apply anyway or dismiss it"));
            }
        }
        let mut plan = if p.project_only {
            let item_id = target.work_item_id.as_deref().ok_or_else(|| invalid("The proposal has no followed item"))?;
            let direct_root = proposal.action == ProposalAction::Add && root_rule
                && self.outline_tree(&proposal.space_id).parent.get(item_id).and_then(|parent| parent.as_deref()) == Some(container)
                && self.live_item(item_id).is_ok_and(|item| !item.archived());
            if !direct_root && !self.followed_template_rule(&proposal.space_id, item_id).is_some_and(|(c, n, i)| c == container && n == node_id && i == item_id) {
                return Err(invalid("The work item no longer follows this template rule"));
            }
            let source = checked_source(Some(&InstructionSource {
                chat_id: proposal.source_ref.chat_id.clone(),
                message_id: proposal.source_ref.message_id.clone(),
                excerpt: Some(truncate_chars(&proposal.text, limits::MAX_INSTRUCTION_EXCERPT_CHARS)),
                content_hash: None,
            }))?;
            self.plan_new_instruction(NewInstruction {
                id: &result_id, space_id: &proposal.space_id, work_item_id: Some(item_id),
                text: if proposal.action == ProposalAction::Remove { proposal.base_text.as_deref().unwrap_or_default() } else { &p.approved_text }, placement: p.placement.clone().unwrap_or_default(),
                source, source_proposal_id: Some(&proposal.id), inputs_hash: &inputs_hash(p),
                action: "Approving this proposal",
            }, actor, now_ms)?
        } else {
            match proposal.action {
                ProposalAction::Add => node.instructions.push(RuleInstruction { id: result_id.clone(), text: p.approved_text.clone() }),
                ProposalAction::Edit => node.instructions.iter_mut().find(|i| Some(i.id.as_str()) == target.rule_instruction_id.as_deref()).unwrap().text = p.approved_text.clone(),
                ProposalAction::Remove => node.instructions.retain(|i| Some(i.id.as_str()) != target.rule_instruction_id.as_deref()),
            }
            self.plan_container_template(&SetContainerTemplateParams {
                operation_id: p.operation_id.clone(), space_id: proposal.space_id.clone(),
                work_item_id: container.to_owned(), template: Some(template),
            }, actor, now_ms)?
        };
        plan.push(Planned::update(KIND_WORK_ITEM_INSTRUCTION_PROPOSALS, &proposal.id,
            BTreeMap::from([("review".to_owned(), json!({"state": "approved", "by": actor_row_value(actor), "at": now_ms, "instructionId": result_id}))])));
        self.commit(plan, &[(KIND_WORK_ITEMS, container)])?;
        Ok(result_id)
    }

    /// Approving an `edit` (a new text on the same instruction, so its
    /// history and drift tracking stay) or a `remove` (archive), with the
    /// review in the same batch. Reply: the targeted instruction's id. An
    /// already approved proposal returns it without writing. A proposal whose
    /// instruction changed since its base is refused unless `allowStale`.
    fn approve_instruction_modification(
        &mut self,
        p: &ApproveWorkItemInstructionProposalParams,
        proposal: &WorkItemInstructionProposalView,
        actor: &WorkOutlineActor,
        now_ms: i64,
    ) -> Outcome<String> {
        let instruction_id = proposal
            .instruction_id
            .clone()
            .ok_or_else(|| invalid("The proposal names no instruction"))?;
        if proposal.status == InstructionProposalStatus::Approved {
            return Ok(instruction_id);
        }
        let instruction = self.live_instruction(&instruction_id)?;
        if instruction.template_instruction_id.is_some() {
            return Err(invalid(
                "This instruction comes from the container's template. Edit the template, or add your own instruction",
            ));
        }
        let mut plan = Vec::new();
        let archived_already = instruction.archived();
        if proposal.is_stale_against(&instruction) && !archived_already && !p.allow_stale {
            return Err(invalid(
                "The instruction changed after this proposal was made; apply anyway or dismiss it",
            ));
        }
        match proposal.action {
            ProposalAction::Edit => {
                if let Some(update) =
                    self.plan_instruction_text_update(&instruction, &p.approved_text, now_ms)?
                {
                    plan.push(update);
                }
            }
            _ if archived_already => {}
            _ => plan.push(Planned::update(
                KIND_WORK_ITEM_INSTRUCTIONS,
                &instruction_id,
                BTreeMap::from([
                    ("archivedAt".to_owned(), json!(now_ms)),
                    ("archivedBy".to_owned(), actor_row_value(actor)),
                    ("updatedAt".to_owned(), json!(now_ms)),
                ]),
            )),
        }
        plan.push(Planned::update(
            KIND_WORK_ITEM_INSTRUCTION_PROPOSALS,
            &p.proposal_id,
            BTreeMap::from([(
                "review".to_owned(),
                json!({
                    "state": "approved",
                    "by": actor_row_value(actor),
                    "at": now_ms,
                    "instructionId": instruction_id,
                }),
            )]),
        ));
        let extra: Vec<(&str, &str)> = instruction
            .work_item_id
            .as_deref()
            .map(|w| (KIND_WORK_ITEMS, w))
            .into_iter()
            .collect();
        self.commit(plan, &extra)?;
        Ok(instruction_id)
    }

    /// `rejectWorkItemInstructionProposal`: writes the `review`. An approved
    /// proposal is `invalid` (the instruction exists; archive it instead); a
    /// rejected one is a no-op.
    pub fn reject_work_item_instruction_proposal(
        &mut self,
        p: &RejectWorkItemInstructionProposalParams,
        actor: &WorkOutlineActor,
        now_ms: i64,
    ) -> Outcome<()> {
        check_id("proposal", &p.proposal_id)?;
        check_note("The rejection reason", p.reason.as_deref())?;
        let proposal = self
            .instruction_proposal(&p.proposal_id)
            .ok_or_else(|| not_found("Proposal not found"))?;
        self.require_space(&proposal.space_id)?;
        let instruction_id = derive_id(&p.proposal_id, roles::INSTRUCTION);
        if proposal.status == InstructionProposalStatus::Approved
            || self.row_state(KIND_WORK_ITEM_INSTRUCTIONS, &instruction_id) != RowState::Absent
        {
            return Err(invalid(
                "This proposal was already approved; archive its instruction instead",
            ));
        }
        if proposal.status == InstructionProposalStatus::Rejected {
            return Ok(());
        }
        let mut review = json!({
            "state": "rejected",
            "by": actor_row_value(actor),
            "at": now_ms,
        });
        if let Some(reason) = &p.reason {
            review["reason"] = json!(reason);
        }
        self.commit(
            vec![Planned::update(
                KIND_WORK_ITEM_INSTRUCTION_PROPOSALS,
                &p.proposal_id,
                BTreeMap::from([("review".to_owned(), review)]),
            )],
            &[],
        )
    }

    // ── agent policy (contract section 8) ──────────────────────────────────

    /// The live work item Chat `chat_id` is associated with in `space_id`.
    pub fn chat_assigned_work_item(&self, chat_id: &str, space_id: &str) -> Option<String> {
        let row = self.overlay_row(KIND_WORK_ITEM_CHAT_ASSIGNMENTS, chat_id)?;
        if row.fields.get("spaceId").and_then(Value::as_str) != Some(space_id) {
            return None;
        }
        let item_id = row.fields.get("workItemId").and_then(Value::as_str)?;
        let item = self.live_item(item_id).ok()?;
        (item.space_id == space_id).then(|| item_id.to_owned())
    }

    pub fn chats_share_project(&self, space: &str, first: &str, second: &str) -> bool {
        let tree = self.outline_tree(space);
        let project = |chat: &str| {
            let mut cursor = self.chat_assigned_work_item(chat, space);
            let mut remaining = tree.items.len() + 1;
            while let Some(id) = cursor {
                if tree.items.get(&id).is_some_and(|item| item.is_project) {
                    return Some(id);
                }
                if remaining == 0 {
                    break;
                }
                remaining -= 1;
                cursor = tree.parent.get(&id).cloned().flatten();
            }
            None
        };
        project(first) == project(second)
    }

    /// The effective (repaired) parent of `work_item_id` in `space_id`
    /// (`None` for a root item or an unknown one).
    pub fn effective_work_item_parent(&self, space_id: &str, work_item_id: &str) -> Option<String> {
        self.outline_tree(space_id)
            .parent
            .get(work_item_id)
            .cloned()
            .flatten()
    }

    /// Whether an agent acting for Chat `agent_chat` may resolve a request
    /// raised by `origin`: its own Chat, or a Chat it created (a
    /// coordinator closes what its direct children raised, e.g. "Ready to
    /// merge"); never requests from Chats a person started or another agent
    /// created.
    fn agent_may_resolve_origin(&self, agent_chat: &str, origin: &str) -> bool {
        origin == agent_chat
            || self.admin_space.as_deref().is_some_and(|space| {
                self.chat(origin)
                    .ok()
                    .flatten()
                    .is_some_and(|c| c.space_id.as_deref() == Some(space))
            })
            || self
                .chat(origin)
                .ok()
                .flatten()
                .is_some_and(|c| c.parent_chat_id.as_deref() == Some(agent_chat))
    }

    /// Whether an agent acting for Chat `agent_chat` may resolve or answer
    /// `request`: [`Self::agent_may_resolve_origin`], or the request sits on
    /// an item inside the Chat's own item's subtree or project scope.
    pub(crate) fn agent_may_resolve_request(
        &self,
        agent_chat: &str,
        request: &UserActionRequestView,
    ) -> bool {
        if self.agent_may_resolve_origin(agent_chat, &request.chat_id) {
            return true;
        }
        // An admin chat (live user grant) reaches every request of its Space;
        // bulk-approval requests are filtered by the admin policy itself.
        if self.admin_space.as_deref() == Some(request.space_id.as_str()) {
            return true;
        }
        let Some(space) = self
            .chat(agent_chat)
            .ok()
            .flatten()
            .and_then(|c| c.space_id)
            .filter(|s| !s.is_empty())
        else {
            return false;
        };
        space == request.space_id
            && self
                .chat_assigned_work_item(agent_chat, &space)
                .is_some_and(|root| self.item_in_agent_reach(&space, &root, &request.work_item_id))
    }

    /// `item` is `root` or below it, or inside `root`'s nearest project (the
    /// item itself counts as a project).
    fn item_in_agent_reach(&self, space: &str, root: &str, item: &str) -> bool {
        let tree = self.outline_tree(space);
        if item == root || tree.descendants(root).iter().any(|id| id == item) {
            return true;
        }
        let mut cursor = Some(root.to_owned());
        let mut steps = 0usize;
        while let Some(id) = cursor {
            if tree.items.get(&id).is_some_and(|i| i.is_project) {
                return id == item || tree.descendants(&id).iter().any(|d| d == item);
            }
            steps += 1;
            if steps > tree.items.len() {
                break;
            }
            cursor = tree.parent.get(&id).cloned().flatten();
        }
        false
    }

    /// Whether the mutation in flight is an admin chat's (a live user grant).
    pub(super) fn admin_in_force(&self) -> bool {
        self.admin_space.is_some()
    }

    fn item_in_space(&self, item_id: &str, space: &str) -> bool {
        // Unknown ids fall through to the mutator's `notFound`.
        self.live_item(item_id)
            .map_or(true, |item| item.space_id == space)
    }

    /// Admin chats (contract amendment "admin chats"): within their granted
    /// Space only, outline changes anywhere (create, update including status
    /// and project settings, move, archive, restore), instruction add / edit /
    /// reorder / archive, and resolving any Space request except a bulk-change
    /// approval (only the user resolves those). `None` = not an admin rule, the
    /// normal agent policy decides (own-Chat association, own requests,
    /// proposals). Linear / PR-sync settings, execution configuration (accounts
    /// and sandbox), references, repair, templates and approvals stay denied.
    fn authorize_admin_mutation(
        &self,
        space: &str,
        mutation: &WorkOutlineMutation,
    ) -> Option<Outcome<()>> {
        let outside = || unauthorized("Admin chats may only change their own Space");
        let ok_if = |cond: bool| Some(if cond { Ok(()) } else { Err(outside()) });
        match mutation {
            WorkOutlineMutation::CreateWorkItem(p) => ok_if(
                p.space_id == space
                    && p.parent_work_item_id
                        .as_deref()
                        .is_none_or(|parent| self.item_in_space(parent, space)),
            ),
            WorkOutlineMutation::UpdateWorkItem(p) => {
                let c = &p.changes;
                if c.external_sync_state.is_some() || !c.template.is_unchanged() {
                    return Some(Err(unauthorized(
                        "Admin chats cannot change sync state or template links",
                    )));
                }
                if c.resolve_open_requests {
                    return Some(Err(unauthorized(
                        "Agents cannot resolve open user action requests by completing an item",
                    )));
                }
                if matches!(c.status, Some(WorkItemStatus::Pending)) {
                    return Some(Err(unauthorized(
                        "Agents may only mark a work item active or completed",
                    )));
                }
                ok_if(self.item_in_space(&p.work_item_id, space))
            }
            WorkOutlineMutation::MoveWorkItem(p) => ok_if(
                self.item_in_space(&p.work_item_id, space)
                    && p.destination_parent_work_item_id
                        .as_deref()
                        .is_none_or(|parent| self.item_in_space(parent, space)),
            ),
            WorkOutlineMutation::ArchiveWorkItem(p) => {
                ok_if(self.item_in_space(&p.work_item_id, space))
            }
            WorkOutlineMutation::RestoreWorkItem(p) => {
                ok_if(self.item_in_space(&p.work_item_id, space))
            }
            WorkOutlineMutation::CreateWorkItemInstruction(p) => ok_if(
                p.space_id == space
                    && match &p.scope {
                        InstructionScope::SpaceRoot => true,
                        InstructionScope::WorkItem { work_item_id } => {
                            self.item_in_space(work_item_id, space)
                        }
                    },
            ),
            WorkOutlineMutation::UpdateWorkItemInstruction(p) => {
                self.admin_instruction(&p.instruction_id, space)
            }
            WorkOutlineMutation::MoveWorkItemInstruction(p) => {
                self.admin_instruction(&p.instruction_id, space)
            }
            WorkOutlineMutation::ArchiveWorkItemInstruction(p) => {
                self.admin_instruction(&p.instruction_id, space)
            }
            WorkOutlineMutation::ResolveUserActionRequest(p) => {
                let row = self.overlay_row(KIND_USER_ACTION_REQUESTS, &p.user_action_request_id)?;
                if str_field(&row, "dedupeKey")
                    .is_some_and(|key| key.starts_with(mast_proto::ADMIN_BULK_DEDUPE_PREFIX))
                {
                    return Some(Err(unauthorized("Only the user can approve a bulk change")));
                }
                let item_ok = str_field(&row, "workItemId")
                    .is_none_or(|item| self.item_in_space(&item, space));
                ok_if(item_ok)
            }
            // Parity with the user's own menus, inside the Space: accept /
            // dismiss proposals, move chats between items, instruction
            // revisions, execution configuration, template save.
            WorkOutlineMutation::ApproveWorkItemInstructionProposal(p) => {
                self.admin_proposal(&p.proposal_id, space)
            }
            WorkOutlineMutation::RejectWorkItemInstructionProposal(p) => {
                self.admin_proposal(&p.proposal_id, space)
            }
            WorkOutlineMutation::AssignChatToWorkItem(p) => ok_if(
                self.chat_in_space(&p.chat_id, space) && self.item_in_space(&p.work_item_id, space),
            ),
            WorkOutlineMutation::RemoveChatFromWorkItem(p) => {
                ok_if(self.chat_in_space(&p.chat_id, space))
            }
            WorkOutlineMutation::ApplyInstructionRevision(p) => {
                ok_if(self.chat_in_space(&p.chat_id, space))
            }
            WorkOutlineMutation::KeepInstructionSession(p) => {
                ok_if(self.chat_in_space(&p.chat_id, space))
            }
            WorkOutlineMutation::CreateInstructionRevision(p) => {
                ok_if(self.item_in_space(&p.work_item_id, space))
            }
            WorkOutlineMutation::ConfigureExecution(p) => ok_if(match &p.scope {
                mast_proto::ExecutionScope::Space { space_id } => space_id == space,
                mast_proto::ExecutionScope::WorkItem { work_item_id } => {
                    self.item_in_space(work_item_id, space)
                }
            }),
            WorkOutlineMutation::ApplyContainerTemplate(p) => {
                ok_if(p.space_id == space && self.item_in_space(&p.work_item_id, space))
            }
            WorkOutlineMutation::SetContainerTemplate(p) => {
                ok_if(p.space_id == space && self.item_in_space(&p.work_item_id, space))
            }
            WorkOutlineMutation::SaveWorkItemTemplate(p) => {
                ok_if(p.space_id == space && self.item_in_space(&p.project_work_item_id, space))
            }
            // Linear / PR-sync settings, external references, repair and
            // template deletion stay with the user (and the normal agent
            // policy refuses them).
            _ => None,
        }
    }

    fn chat_in_space(&self, chat_id: &str, space: &str) -> bool {
        self.chat(chat_id)
            .ok()
            .flatten()
            .is_none_or(|c| c.space_id.as_deref() == Some(space))
    }

    fn admin_proposal(&self, proposal_id: &str, space: &str) -> Option<Outcome<()>> {
        let in_space = self
            .overlay_row(KIND_WORK_ITEM_INSTRUCTION_PROPOSALS, proposal_id)
            .and_then(|row| str_field(&row, "spaceId"))
            .is_none_or(|s| s == space);
        Some(if in_space {
            Ok(())
        } else {
            Err(unauthorized("Admin chats may only change their own Space"))
        })
    }

    fn admin_instruction(&self, instruction_id: &str, space: &str) -> Option<Outcome<()>> {
        match self.live_instruction(instruction_id) {
            Ok(view) => Some(if view.space_id == space {
                Ok(())
            } else {
                Err(unauthorized("Admin chats may only change their own Space"))
            }),
            // The mutator reports the missing instruction.
            Err(_) => Some(Ok(())),
        }
    }

    /// The narrow agent policy: `Ok` when an agent acting for Chat
    /// `chat_id` may perform `mutation`, else `unauthorized` (with the
    /// reason). Allowed, all within the Chat's Space:
    /// - `createWorkItem` under the Chat's assigned item or a descendant;
    /// - `assignChatToWorkItem` of this Chat only: any item of its Space while
    ///   unassociated, else only within its assigned item's subtree; never
    ///   for a Chat with a captured execution binding;
    /// - `updateWorkItem` that only sets status `active` / `completed` on the
    ///   assigned item or a descendant (never `resolveOpenRequests`);
    /// - `createUserActionRequest` on the assigned item with origin Chat =
    ///   this Chat, `resolveUserActionRequest` of requests this Chat or a Chat
    ///   it created (`parentChatId` = this Chat) raised;
    /// - `proposeWorkItemInstruction` targeting the assigned item, its
    ///   parent, or the Space root, with no other origin Chat.
    ///
    /// Everything else (move, archive, restore, remove, references, repair,
    /// instruction writes, revisions, applications, execution
    /// configuration, approve / reject) is denied. Mechanism checks (live
    /// rows, archived targets, open requests) still run in the mutator.
    pub fn authorize_agent_mutation(
        &self,
        chat_id: &str,
        mutation: &WorkOutlineMutation,
    ) -> Outcome<()> {
        let chat = self
            .chat(chat_id)
            .ok()
            .flatten()
            .ok_or_else(|| unauthorized("The agent's Chat no longer exists"))?;
        let space = chat.space_id.filter(|s| !s.is_empty()).ok_or_else(|| {
            unauthorized("Agents in Chats outside a project cannot change the Work Outline")
        })?;
        if self.admin_space.as_deref() == Some(space.as_str())
            && let Some(decision) = self.authorize_admin_mutation(&space, mutation)
        {
            return decision;
        }
        let assigned = self.chat_assigned_work_item(chat_id, &space);
        let require_assigned = || {
            assigned
                .clone()
                .ok_or_else(|| unauthorized("The agent's Chat is not associated with a work item"))
        };
        // The assigned item and its effective descendants.
        let in_scope = |work_item_id: &str| -> Outcome<()> {
            let root = require_assigned()?;
            if work_item_id == root
                || self
                    .outline_tree(&space)
                    .descendants(&root)
                    .iter()
                    .any(|id| id == work_item_id)
            {
                Ok(())
            } else {
                Err(unauthorized(
                    "Agents may only change their own work item and its descendants",
                ))
            }
        };
        // Project scope (contract amendment "project scope"): the nearest
        // project at or above the assigned item and everything inside it, so
        // a Chat on a project's Planning item can create into its Tickets
        // sibling. Without a project it is exactly `in_scope`.
        let in_project_scope = |work_item_id: &str| -> Outcome<()> {
            let root = require_assigned()?;
            let tree = self.outline_tree(&space);
            let mut project = None;
            let mut cursor = Some(root.clone());
            let mut steps = 0usize;
            while let Some(id) = cursor {
                if tree.items.get(&id).is_some_and(|item| item.is_project) {
                    project = Some(id);
                    break;
                }
                steps += 1;
                if steps > tree.items.len() {
                    break;
                }
                cursor = tree.parent.get(&id).cloned().flatten();
            }
            let Some(project) = project else {
                return in_scope(work_item_id);
            };
            if in_scope(work_item_id).is_ok()
                || work_item_id == project
                || tree
                    .descendants(&project)
                    .iter()
                    .any(|id| id == work_item_id)
            {
                Ok(())
            } else {
                Err(unauthorized(
                    "Agents may only create items inside their own project",
                ))
            }
        };
        match mutation {
            WorkOutlineMutation::CreateWorkItem(p) => {
                if p.space_id != space {
                    return Err(unauthorized(
                        "Agents may only create items in their own Space",
                    ));
                }
                let parent = p.parent_work_item_id.as_deref().ok_or_else(|| {
                    unauthorized("Agents may only create children of their own work item")
                })?;
                in_project_scope(parent)
            }
            WorkOutlineMutation::AssignChatToWorkItem(p) => {
                if p.chat_id != chat_id {
                    return Err(unauthorized("Agents may only associate their own Chat"));
                }
                // A Chat launched with a captured execution binding keeps the
                // item it was started for (re-associating with it is a no-op).
                if assigned.as_deref() != Some(p.work_item_id.as_str())
                    && self.row_state(KIND_CHAT_EXECUTION_BINDINGS, chat_id) == RowState::Live
                {
                    return Err(unauthorized(
                        "Agents cannot re-associate a Chat started for a work item",
                    ));
                }
                match self.live_item(&p.work_item_id) {
                    Ok(item) if item.space_id != space => Err(unauthorized(
                        "Agents may only associate their Chat with an item in its Space",
                    )),
                    // Unassociated: any item of its Space. Associated: only
                    // within its current item's subtree (never wider).
                    Ok(_) if assigned.is_none() => Ok(()),
                    Ok(_) => in_scope(&p.work_item_id).map_err(|_| {
                        unauthorized(
                            "Agents may only move their Chat to an item within its current \
                             work item",
                        )
                    }),
                    // Let the mutator report the missing item.
                    Err(_) => Ok(()),
                }
            }
            // Container templates: an agent may make items inside its scope
            // follow their parent's template; editing the rules stays denied.
            WorkOutlineMutation::ApplyContainerTemplate(p) => {
                if p.space_id != space {
                    return Err(unauthorized(
                        "Agents may only apply templates in their own Space",
                    ));
                }
                in_scope(&p.work_item_id)
            }
            WorkOutlineMutation::UpdateWorkItem(p) => {
                let changes = &p.changes;
                let status_only = changes.title.is_none()
                    && changes.description.is_unchanged()
                    && changes.inherit_parent_instructions.is_none()
                    && changes.external_sync_state.is_none()
                    && changes.is_project.is_none()
                    && changes.worker_destination.is_unchanged()
                    && changes.template.is_unchanged();
                if !status_only {
                    return Err(unauthorized("Agents may only change a work item's status"));
                }
                match changes.status {
                    Some(WorkItemStatus::Active | WorkItemStatus::Completed) => {}
                    None if !changes.waiting_until.is_unchanged()
                        || !changes.waiting_reason.is_unchanged() => {}
                    _ => {
                        return Err(unauthorized(
                            "Agents may only mark a work item active or completed",
                        ));
                    }
                }
                if changes.resolve_open_requests {
                    return Err(unauthorized(
                        "Agents cannot resolve open user action requests by completing an item",
                    ));
                }
                in_scope(&p.work_item_id)
            }
            WorkOutlineMutation::CreateUserActionRequest(p) => {
                if p.origin.chat_id != chat_id {
                    return Err(unauthorized(
                        "Agents may only raise requests from their own Chat",
                    ));
                }
                if require_assigned()? != p.work_item_id {
                    return Err(unauthorized(
                        "Agents may only raise requests on their own work item",
                    ));
                }
                Ok(())
            }
            WorkOutlineMutation::UpdateUserActionRequest(p) => {
                let request = self.user_action_request(&p.user_action_request_id)
                    .ok_or_else(|| not_found("Request not found"))?;
                if request.chat_id != chat_id || p.request.origin.chat_id != chat_id || request.work_item_id != require_assigned()? {
                    return Err(unauthorized("Agents may only update requests raised by their own Chat"));
                }
                Ok(())
            }
            WorkOutlineMutation::ResolveUserActionRequest(p) => {
                match self.user_action_request(&p.user_action_request_id) {
                    Some(request) if self.agent_may_resolve_request(chat_id, &request) => Ok(()),
                    Some(_) => Err(unauthorized(
                        "Agents may only resolve requests raised by their own Chat or a Chat it \
                         created, or requests on items in their own item's subtree or project",
                    )),
                    None => Err(not_found("Request not found")),
                }
            }
            WorkOutlineMutation::ProposeWorkItemInstruction(p) => {
                if p.origin.chat_id.as_deref().is_some_and(|c| c != chat_id) {
                    return Err(unauthorized("Agents may only propose from their own Chat"));
                }
                if p.action != ProposalAction::Add {
                    // Only instructions the agent can read (its item's
                    // effective set, Space root included). An unknown id is
                    // left to the mutator's `notFound`.
                    let Some(id) = p.instruction_id.as_deref() else {
                        return Ok(());
                    };
                    if self.live_instruction(id).is_err() {
                        return Ok(());
                    }
                    let own = require_assigned()?;
                    let reachable = self
                        .effective_instructions_of(&own)?
                        .entries
                        .iter()
                        .any(|e| e.instruction_id == id);
                    return if reachable {
                        Ok(())
                    } else {
                        Err(unauthorized(
                            "Agents may only propose changes to instructions they inherit",
                        ))
                    };
                }
                match p.target.scope {
                    ProposalTargetScope::SpaceRoot => {
                        if p.space_id.as_ref().is_some_and(|s| *s != space) {
                            return Err(unauthorized(
                                "Agents may only propose for their own Space root",
                            ));
                        }
                        Ok(())
                    }
                    ProposalTargetScope::WorkItem => {
                        let own = require_assigned()?;
                        let target = p.target.work_item_id.as_deref().unwrap_or_default();
                        let parent = self
                            .outline_tree(&space)
                            .parent
                            .get(&own)
                            .cloned()
                            .flatten();
                        if target == own || parent.as_deref() == Some(target) {
                            Ok(())
                        } else {
                            Err(unauthorized(
                                "Agents may only propose for their work item, its parent, or the Space root",
                            ))
                        }
                    }
                    ProposalTargetScope::Unknown => Err(invalid("Unknown proposal target scope")),
                    ProposalTargetScope::ChildTemplate => {
                        let own = require_assigned()?;
                        if p.target.work_item_id.as_deref() != Some(own.as_str()) {
                            return Err(unauthorized("Agents may only propose for the template followed by their own work item"));
                        }
                        Ok(())
                    }
                }
            }
            other => Err(unauthorized(format!(
                "Agents cannot {} (ask the user)",
                other.op_name()
            ))),
        }
    }
}

#[cfg(test)]
mod tests;
