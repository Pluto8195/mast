//! Work Outline user action requests and instruction proposals (Milestone 4):
//! wire types for the `userActionRequests` and `workItemInstructionProposals`
//! registry kinds, their `WorkOutlineMutate` operations, and the read-only
//! context an agent acting for a Chat may see.
//!
//! Binding references: `docs/internal/plans/work-outline/m0-contract.md` (sections 4
//! and 8, "Milestone 4 registry discoveries"), `m0-registry-persistence.md`
//! 7.6 and 7.7, and the specification's "Status and user actions" and
//! "Create instructions from Chats" sections. Wire conventions match
//! `work_outline.rs`: camelCase fields, defaulted non-identity fields,
//! lenient snapshot enums, strict request enums.
//!
//! Semantics the materializer guarantees (the doc crate implements them):
//! - a request is resolved iff its row carries a `resolution` (or a
//!   `resolved` state); resolution is monotone, there is no reopen;
//! - among OPEN requests with equal `(workItemId, dedupeKey)` the one with the
//!   smallest `(creation clock, id)` is canonical and the others are flagged
//!   `deduped`; only canonical open requests make an item
//!   `userActionRequired` and count in rollups;
//! - a proposal whose approval instruction (`derive_id(proposalId,
//!   "instruction")`) exists reads `approved`, whatever a concurrent reject
//!   wrote (approval has durable side effects); otherwise its `review`
//!   decides, and no review means `pending`.

use serde::{Deserialize, Serialize};

use crate::work_outline::{InstructionPlacement, WorkItemView, WorkOutlineActor};
use crate::work_outline_instructions::EffectiveInstructions;

/// Budgets of requests and proposals (registry audit 7.6 / 7.7).
pub mod user_action_limits {
    /// The requested action or question, in Unicode scalar values.
    pub const MAX_REQUEST_TEXT_CHARS: usize = 1_000;
    /// Optional supporting detail of a request.
    pub const MAX_REQUEST_INFO_CHARS: usize = 4_000;
    /// A request's deduplication key.
    pub const MAX_DEDUPE_KEY_CHARS: usize = 256;
    /// A resolution note or a rejection reason.
    pub const MAX_NOTE_CHARS: usize = 1_000;
    /// Origin Chat, message, and tool-call identifiers.
    pub const MAX_ORIGIN_ID_CHARS: usize = 256;
    /// Proposed instruction text (the instruction text limit).
    pub const MAX_PROPOSAL_TEXT_CHARS: usize =
        crate::work_outline::limits::MAX_INSTRUCTION_TEXT_CHARS;
    /// Resolved requests listed per Space in a snapshot (newest first by
    /// resolution time); every open request is always listed.
    pub const MAX_RECENT_RESOLVED_REQUESTS: usize = 50;
    /// Reviewed proposals listed per Space in a snapshot (newest first by
    /// review time); every pending proposal is always listed.
    pub const MAX_RECENT_REVIEWED_PROPOSALS: usize = 50;
}

/// The resolution note recorded for every open request when a work item is
/// completed with `resolveOpenRequests: true`.
pub const COMPLETION_RESOLUTION_NOTE: &str = "Resolved by completing the work item";

// --------------------------------------------------------------------------
// User action requests
// --------------------------------------------------------------------------

/// What the user is being asked to do. Unknown values decode as `ask`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UserActionKind {
    Review,
    Approve,
    Choose,
    Handoff,
    Notice,
    #[default]
    #[serde(other)]
    Ask,
}

/// Optional grouping override for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UserActionPriority {
    Blocking,
    Fyi,
    #[serde(other)]
    Normal,
}

/// Classify rows written before request kinds existed.
pub fn classify_legacy_request(text: &str, dedupe_key: Option<&str>) -> UserActionKind {
    let key = dedupe_key.unwrap_or_default();
    if key.starts_with("admin-bulk:") {
        UserActionKind::Approve
    } else if key.starts_with("pr-review:") {
        UserActionKind::Review
    } else if key.starts_with("artopt:") {
        UserActionKind::Choose
    } else if key.starts_with("artq:") {
        UserActionKind::Ask
    } else if text.trim_start().starts_with("Ready to merge") {
        UserActionKind::Handoff
    } else {
        UserActionKind::Ask
    }
}

/// What raised a request. Lenient: unknown reads as `chat`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UserActionOriginKind {
    /// No Chat: the engine itself raised it (the PR review sync). Only the
    /// system actor may create such a request; `chatId` is empty and
    /// `originChatAvailable` is always `false`.
    System,
    /// A Chat (the agent or the user acting from it). Declared last because
    /// `#[serde(other)]` must sit on the last variant.
    #[default]
    #[serde(other)]
    Chat,
}

impl UserActionOriginKind {
    pub fn is_chat(&self) -> bool {
        *self == Self::Chat
    }
}

/// Where a request came from: the Chat (required for `kind: chat`, the
/// default) and, optionally, the message and tool call that raised it; or
/// `kind: system` with no Chat (system actor only).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserActionRequestOrigin {
    #[serde(default, skip_serializing_if = "UserActionOriginKind::is_chat")]
    pub kind: UserActionOriginKind,
    /// Empty for `kind: system`.
    #[serde(default)]
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Lenient: an unknown state reads as `open` (the materializer derives the
/// state from the presence of a resolution anyway).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UserActionRequestState {
    Resolved,
    /// Declared last because `#[serde(other)]` must sit on the last variant.
    #[default]
    #[serde(other)]
    Open,
}

/// Who resolved a request, when, and why. Written in ONE registry field in
/// the same op as `state: "resolved"`, so it never tears.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserActionResolution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(default)]
    pub by: WorkOutlineActor,
    #[serde(default)]
    pub at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Set when an agent carried the human's own chat message to this
    /// request (`answer_user_action_request`): the resolution is then the
    /// user's answer (`note` repeats its text), not the agent's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<UserActionAnswer>,
}

impl UserActionResolution {
    /// Whether this resolution is the user's answer, quoted from a chat
    /// message of theirs rather than typed into Required Actions.
    pub fn answered_by_user(&self) -> bool {
        self.answer.is_some()
    }
}

/// The user's chat message an agent pointed a request at. The text is the
/// message as the engine read it; the caller never supplies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserActionAnswer {
    /// The chat the message was typed in (the caller's own).
    pub chat_id: String,
    pub message_id: String,
    pub text: String,
}

/// Prefix Mast puts on a message one agent sends another (`send_message`).
pub const RELAYED_MESSAGE_PREFIX: &str = "[Message from Mast chat";
/// Prefix of Mast's own system messages (orchestrator wakes).
pub const SYSTEM_MESSAGE_PREFIX: &str = "[Mast system message";

/// Whether a user-role transcript message text is an agent relay or a Mast
/// system message rather than something the human typed.
pub fn is_relayed_or_system_message(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with(RELAYED_MESSAGE_PREFIX) || text.starts_with(SYSTEM_MESSAGE_PREFIX)
}

/// Longest quote of the request text in a reply message.
pub const REPLY_QUOTE_MAX_CHARS: usize = 120;

/// The message delivered to a request's origin Chat when the user answers:
/// `Re: "<request>": <note>`, the request quoted on one line.
pub fn user_reply_message(request_text: &str, note: &str) -> String {
    let collapsed = request_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let quote = if collapsed.chars().count() > REPLY_QUOTE_MAX_CHARS {
        let cut: String = collapsed.chars().take(REPLY_QUOTE_MAX_CHARS - 1).collect();
        format!("{}\u{2026}", cut.trim_end())
    } else {
        collapsed
    };
    format!("Re: \"{quote}\": {}", note.trim())
}

/// A labeled reference attached to a user action request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLink {
    pub label: String,
    pub target: RequestLinkTarget,
}

/// The destination of a request link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RequestLinkTarget {
    Artifact {
        id: String,
    },
    File {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line: Option<u32>,
    },
    Diff {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
    },
    Pr {
        url: String,
    },
    Chat {
        id: String,
    },
    Url {
        url: String,
    },
}

/// A labeled fact attached to a user action request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestFact {
    pub label: String,
    pub value: String,
}

/// A choice offered on a user action request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One materialized `userActionRequests` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserActionRequestView {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<RequestOption>,
    #[serde(default)]
    pub links: Vec<RequestLink>,
    #[serde(default)]
    pub facts: Vec<RequestFact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<UserActionKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<UserActionPriority>,
    pub id: String,
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub work_item_id: String,
    /// The originating Chat (kept when the Chat is archived or deleted);
    /// empty for a `system` origin.
    #[serde(default)]
    pub chat_id: String,
    /// `system` for a request the engine raised without a Chat (omitted for
    /// `chat`).
    #[serde(default, skip_serializing_if = "UserActionOriginKind::is_chat")]
    pub origin_kind: UserActionOriginKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// The concise requested action or question.
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<WorkOutlineActor>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub state: UserActionRequestState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<UserActionResolution>,
    /// An open request that duplicates an earlier open request with the same
    /// `(workItemId, dedupeKey)`; it never counts and resolves with it.
    #[serde(default)]
    pub deduped: bool,
    /// The origin Chat row is live and not archived. `false` means the
    /// request stays inspectable but cannot open its Chat.
    #[serde(default)]
    pub origin_chat_available: bool,
}

impl UserActionRequestView {
    pub fn effective_kind(&self) -> UserActionKind {
        self.kind
            .unwrap_or_else(|| classify_legacy_request(&self.text, self.dedupe_key.as_deref()))
    }

    pub fn is_open(&self) -> bool {
        self.state == UserActionRequestState::Open
    }

    /// Open and canonical: the requests that make an item
    /// `userActionRequired` and that count in rollups.
    pub fn is_unresolved(&self) -> bool {
        self.is_open() && !self.deduped
    }
}

/// `createUserActionRequest`. Reply: `userActionRequestId` =
/// `derive_id(operationId, "request")`. The origin Chat must exist and
/// belong to the item's Space; agents may only name their own Chat and
/// their assigned item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserActionRequestParams {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<RequestOption>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<RequestLink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<RequestFact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<UserActionKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<UserActionPriority>,
    pub work_item_id: String,
    pub origin: UserActionRequestOrigin,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    pub operation_id: String,
}

/// Replace the editable content of an open request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserActionRequestParams {
    pub user_action_request_id: String,
    pub request: CreateUserActionRequestParams,
}

/// `resolveUserActionRequest`: records an explicit resolution (also on every
/// open duplicate of the request). Resolving a resolved request is a no-op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveUserActionRequestParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    pub user_action_request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Agents only, set by the engine's `answer_user_action_request`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<UserActionAnswer>,
    pub operation_id: String,
}

// --------------------------------------------------------------------------
// Instruction proposals
// --------------------------------------------------------------------------

/// The scope a proposal targets. Lenient in snapshots (`unknown`); the
/// mutators reject `unknown`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProposalTargetScope {
    SpaceRoot,
    ChildTemplate,
    #[default]
    WorkItem,
    #[doc(hidden)]
    #[serde(other)]
    Unknown,
}

/// What a proposal asks for. `add` (the default, and what rows written before
/// edit/remove proposals read as) creates a new instruction; `edit` replaces
/// the text of an existing one; `remove` archives it. Lenient: unknown reads
/// as `add`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProposalAction {
    Edit,
    Remove,
    /// Declared last because `#[serde(other)]` must sit on the last variant.
    #[default]
    #[serde(other)]
    Add,
}

impl ProposalAction {
    pub fn is_add(&self) -> bool {
        *self == Self::Add
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Edit => "edit",
            Self::Remove => "remove",
        }
    }
}

/// `{scope: "spaceRoot"}` or `{scope: "workItem", workItemId}`. Immutable on
/// the row. For `edit` / `remove` it is the targeted instruction's scope.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionProposalTarget {
    #[serde(default)]
    pub scope: ProposalTargetScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_work_item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_instruction_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule_node_path: Vec<String>,
}

impl InstructionProposalTarget {
    pub fn space_root() -> Self {
        Self {
            scope: ProposalTargetScope::SpaceRoot,
            work_item_id: None,
            ..Default::default()
        }
    }

    pub fn work_item(work_item_id: impl Into<String>) -> Self {
        Self {
            scope: ProposalTargetScope::WorkItem,
            work_item_id: Some(work_item_id.into()),
            ..Default::default()
        }
    }
}

/// Provenance of a proposal: the proposing Chat and, optionally, the
/// message and tool call (the row's `sourceRef`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionProposalOrigin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Lenient: an unknown review state reads as `unknown` (the proposal then
/// reads `pending` unless its approval instruction exists).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProposalReviewState {
    Approved,
    Rejected,
    #[doc(hidden)]
    #[serde(other)]
    Unknown,
}

/// The review decision (ONE registry field).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionProposalReview {
    pub state: ProposalReviewState,
    #[serde(default)]
    pub by: WorkOutlineActor,
    #[serde(default)]
    pub at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The created instruction (approval only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_id: Option<String>,
}

/// Lenient: unknown reads as `pending`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstructionProposalStatus {
    Approved,
    Rejected,
    /// Declared last because `#[serde(other)]` must sit on the last variant.
    #[default]
    #[serde(other)]
    Pending,
}

/// One materialized `workItemInstructionProposals` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemInstructionProposalView {
    pub id: String,
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub target: InstructionProposalTarget,
    /// The proposed text (immutable; the reviewer's final text lives on the
    /// created instruction).
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_by: Option<WorkOutlineActor>,
    #[serde(default)]
    pub source_ref: InstructionProposalOrigin,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub action: ProposalAction,
    /// The instruction an `edit` / `remove` targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_id: Option<String>,
    /// The targeted instruction's text and `updatedAt` when the proposal was
    /// made (its base revision; `edit` / `remove` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_text: Option<String>,
    #[serde(default)]
    pub base_updated_at: i64,
    /// Why (optional, short; all actions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// The effective review: when the approval instruction exists this is an
    /// approval (synthesized from the instruction when a concurrent reject
    /// won the register).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<InstructionProposalReview>,
    #[serde(default)]
    pub status: InstructionProposalStatus,
}

/// `proposeWorkItemInstruction`. Reply: `proposalId` =
/// `derive_id(operationId, "proposal")`. `spaceId` is optional: a
/// work-item target implies its Space, and a Space-root target falls back to
/// the origin Chat's Space; it is required for a Space-root target without
/// an origin Chat. Agents may target only their item, its parent, or the
/// Space root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposeWorkItemInstructionParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    /// Ignored for `edit` / `remove` (taken from the instruction).
    #[serde(default)]
    pub target: InstructionProposalTarget,
    #[serde(default)]
    pub origin: InstructionProposalOrigin,
    /// The proposed text; empty for `remove`.
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "ProposalAction::is_add")]
    pub action: ProposalAction,
    /// Required for `edit` / `remove`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    pub operation_id: String,
}

impl WorkItemInstructionProposalView {
    /// An `edit` / `remove` is stale when its instruction's text differs from
    /// the base it was proposed against, or the instruction was archived.
    pub fn is_stale_against(&self, instruction: &crate::WorkItemInstructionView) -> bool {
        instruction.archived() || self.base_text.as_deref() != Some(instruction.text.as_str())
    }
}

/// `approveWorkItemInstructionProposal`: for `add`, creates the instruction
/// `derive_id(proposalId, "instruction")` from `approvedText` in the
/// proposal's target scope, with `sourceProposalId` and source provenance
/// (`chatId`, `messageId`, `excerpt` = the proposed text, `contentHash`), in
/// the same batch as the review. Reply: `instructionId`. Approving an
/// approved proposal returns the same id without writing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveWorkItemInstructionProposalParams {
    pub proposal_id: String,
    /// Unused for `remove`.
    #[serde(default)]
    pub approved_text: String,
    /// Absent = last in the target scope (`add` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<InstructionPlacement>,
    /// Apply an `edit` / `remove` whose instruction changed since the
    /// proposal was made.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_stale: bool,
    /// Apply a child-template proposal as an own instruction on its origin item.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub project_only: bool,
    pub operation_id: String,
}

/// `rejectWorkItemInstructionProposal`. Rejecting an approved proposal is
/// `invalid`; rejecting a rejected one is a no-op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectWorkItemInstructionProposalParams {
    pub proposal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub operation_id: String,
}

// --------------------------------------------------------------------------
// Agent read context
// --------------------------------------------------------------------------

/// What an agent acting for a Chat may read (contract section 8): the
/// Chat's assigned work item, its effective instructions, and the open
/// requests that Chat raised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkItemContext {
    pub chat_id: String,
    pub space_id: String,
    pub work_item: WorkItemView,
    pub effective_instructions: EffectiveInstructions,
    #[serde(default)]
    pub open_user_action_requests: Vec<UserActionRequestView>,
}

#[cfg(test)]
mod tests;
