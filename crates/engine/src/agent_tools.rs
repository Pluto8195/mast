//! Agent tools (Work Outline Milestone 4): engine-minted per-Run capability
//! tokens and the allow-listed agent RPC surface behind the injected
//! `zeron mcp` server (wire contract: `mast_proto::agent_tools`).
//!
//! - [`AgentTokens`]: when `SessionsEngine` spawns a Run of a Chat in a Space
//!   on a harness that can inject MCP servers, it mints a random token bound
//!   to `(chatId, runId, spaceId, deviceId)`. The [`AgentTokenLease`] lives
//!   exactly as long as the Run's driver task and revokes the token on drop,
//!   whatever path ends the Run. Tokens are memory-only (an engine restart
//!   revokes every token, and the Runs die with it); only a SHA-256 of each
//!   token is kept. An absolute expiry ([`DEFAULT_TOKEN_TTL`]) backs up the
//!   lease; the Run's driver renews it on every heartbeat, so only a hung
//!   driver (one that keeps its lease but stops running) lets it lapse.
//! - [`AgentToolsService`]: `AgentAuthenticate` / `AgentToolCall`. Every call
//!   authenticates by token lookup (plus the Chat row's Space), never by
//!   materializing the outline; the originating Chat is the token's, never a
//!   client-supplied id. Mutations run through
//!   `WorkOutlineService::mutate_as` with `agent_actor(chatId)`, which
//!   enforces the shared agent policy (`authorize_agent`) under the registry
//!   lock, and derive their `operationId` from `(token Chat, idempotencyKey)`,
//!   so a retry with the same key dedupes across Runs and processes.
//!
//! The policy (contract section 8): read the Chat's item and effective
//! instructions; create a child under the Chat's item or a descendant;
//! associate only the calling Chat; set status `active` / `completed` on the
//! Chat's item or a descendant (completing an item with open requests is
//! `openUserActionRequests`, never auto-resolved); create requests on the
//! Chat's item with the Chat as origin; resolve requests the Chat or its own
//! child Chats raised;
//! propose instructions for the item, its parent, or the Space root.
//! Everything else is `unauthorized`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use mast_doc::SessionCommandPayload;
use mast_harness::AgentToolsInjection;
use mast_proto::agent_tools::{self, AgentOrigin, AgentToolCallParams, AgentToolErrorCode, tools};
use mast_proto::work_outline::{derive_id, roles};
use mast_proto::{
    AssignChatToWorkItemParams, ChatDiffTargetSource, CreateUserActionRequestParams,
    CreateWorkItemParams, InstructionProposalOrigin, InstructionProposalTarget, ProposalAction,
    ProposeWorkItemInstructionParams, ResolveUserActionRequestParams, RunRequest, SandboxLevel,
    UpdateWorkItemParams, UserActionRequestOrigin, WorkItemChanges, WorkItemStatus,
    WorkOutlineError, WorkOutlineMutateReply, WorkOutlineMutation, WorkOutlinePlacement,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod archive;

use crate::diff_targets::DiffTargetError;
use crate::doc_host::DocHost;
use crate::sessions::SessionsEngine;
use crate::work_outline::WorkOutlineService;
use crate::workspace_host::WorkspaceHost;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A refused agent call: a gate failure ([`AgentToolErrorCode`]) or the
/// underlying Work Outline operation's error, passed through unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentToolError {
    Refused {
        code: AgentToolErrorCode,
        message: String,
    },
    WorkOutline(WorkOutlineError),
    /// A planning artifact tool failure (`artifact/<code>: …`).
    Artifact(mast_proto::ArtifactError),
}

impl AgentToolError {
    pub fn refused(code: AgentToolErrorCode, message: impl Into<String>) -> Self {
        Self::Refused {
            code,
            message: message.into(),
        }
    }

    fn unauthenticated() -> Self {
        Self::refused(
            AgentToolErrorCode::Unauthenticated,
            "this agent's Mast token is missing, unknown, or expired (it is only valid while \
             the Run that received it is running)",
        )
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::refused(AgentToolErrorCode::Invalid, message)
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Refused { code, .. } => code.as_str(),
            Self::WorkOutline(error) => error.code.as_str(),
            Self::Artifact(error) => error.code.as_str(),
        }
    }

    /// `agent-tools/<code>: …` or `work-outline/<code>: …`.
    pub fn to_wire_message(&self) -> String {
        match self {
            Self::Refused { code, message } => agent_tools::error_wire_message(*code, message),
            Self::WorkOutline(error) => error.to_wire_message(),
            Self::Artifact(error) => error.to_wire_message(),
        }
    }
}

impl From<mast_proto::ArtifactError> for AgentToolError {
    fn from(error: mast_proto::ArtifactError) -> Self {
        Self::Artifact(error)
    }
}

impl From<WorkOutlineError> for AgentToolError {
    fn from(error: WorkOutlineError) -> Self {
        Self::WorkOutline(error)
    }
}

impl std::fmt::Display for AgentToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_wire_message())
    }
}

impl std::error::Error for AgentToolError {}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

/// What one live token authorizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentGrant {
    pub chat_id: String,
    pub run_id: String,
    pub space_id: String,
    pub device_id: String,
    /// Absolute backstop; `None` = valid until the Run's lease drops.
    pub expires_at: Option<Instant>,
    /// What [`AgentTokenLease::renew`] extends `expires_at` by.
    pub ttl: Option<Duration>,
}

impl AgentGrant {
    pub fn origin(&self) -> AgentOrigin {
        AgentOrigin {
            chat_id: self.chat_id.clone(),
            run_id: self.run_id.clone(),
            space_id: self.space_id.clone(),
            device_id: self.device_id.clone(),
            admin_space_id: None,
        }
    }
}

type TokenKey = [u8; 32];

fn token_key(token: &str) -> TokenKey {
    Sha256::digest(token.as_bytes()).into()
}

/// Absolute token lifetime for engine-minted tokens, renewed while the Run's
/// driver is alive (generous: a renewal lands every heartbeat).
pub const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// The engine's live per-Run tokens (one registry per engine).
#[derive(Default)]
pub struct AgentTokens {
    grants: Mutex<HashMap<TokenKey, AgentGrant>>,
    /// User-granted admin chats (device-local; RPC-written only).
    admin: crate::admin::AdminGrants,
}

impl AgentTokens {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The admin grant store (shared by every RPC surface of this engine).
    pub fn admin(&self) -> &crate::admin::AdminGrants {
        &self.admin
    }

    /// Mint a token for one Run. Valid until the returned lease drops or
    /// [`DEFAULT_TOKEN_TTL`] passes without a [`AgentTokenLease::renew`].
    pub fn mint(
        self: &Arc<Self>,
        chat_id: &str,
        run_id: &str,
        space_id: &str,
        device_id: &str,
    ) -> AgentTokenLease {
        self.mint_with_ttl(
            chat_id,
            run_id,
            space_id,
            device_id,
            Some(DEFAULT_TOKEN_TTL),
        )
    }

    /// [`Self::mint`] with an absolute expiry as well.
    pub fn mint_with_ttl(
        self: &Arc<Self>,
        chat_id: &str,
        run_id: &str,
        space_id: &str,
        device_id: &str,
        ttl: Option<Duration>,
    ) -> AgentTokenLease {
        // Two v4 UUIDs: 244 random bits from the OS generator.
        let token = format!(
            "zat_{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let key = token_key(&token);
        lock(&self.grants).insert(
            key,
            AgentGrant {
                chat_id: chat_id.to_owned(),
                run_id: run_id.to_owned(),
                space_id: space_id.to_owned(),
                device_id: device_id.to_owned(),
                expires_at: ttl.map(|ttl| Instant::now() + ttl),
                ttl,
            },
        );
        AgentTokenLease {
            tokens: Arc::downgrade(self),
            key,
            token,
        }
    }

    /// The grant of a live token. Unknown, revoked, and expired tokens (an
    /// expired one is dropped here) are `unauthenticated`.
    pub fn authenticate(&self, token: &str) -> Result<AgentGrant, AgentToolError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(AgentToolError::unauthenticated());
        }
        let key = token_key(token);
        let mut grants = lock(&self.grants);
        let grant = grants
            .get(&key)
            .cloned()
            .ok_or_else(AgentToolError::unauthenticated)?;
        if grant.expires_at.is_some_and(|at| Instant::now() >= at) {
            grants.remove(&key);
            return Err(AgentToolError::unauthenticated());
        }
        Ok(grant)
    }

    /// Number of live tokens (diagnostics and tests).
    pub fn live_count(&self) -> usize {
        lock(&self.grants).len()
    }

    fn revoke(&self, key: &TokenKey) {
        lock(&self.grants).remove(key);
    }

    fn renew(&self, key: &TokenKey) {
        let mut grants = lock(&self.grants);
        if let Some(grant) = grants.get_mut(key)
            && let (Some(ttl), Some(at)) = (grant.ttl, grant.expires_at)
            // An expired token stays expired (authenticate drops it).
            && Instant::now() < at
        {
            grant.expires_at = Some(Instant::now() + ttl);
        }
    }
}

/// Owns one token for one Run; dropping it revokes the token.
pub struct AgentTokenLease {
    tokens: Weak<AgentTokens>,
    key: TokenKey,
    token: String,
}

impl AgentTokenLease {
    /// The secret handed to the Run's MCP server. Never log it.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Push the absolute expiry out by the token's TTL (the Run's driver
    /// calls this on every heartbeat). A lapsed token is not revived.
    pub fn renew(&self) {
        if let Some(tokens) = self.tokens.upgrade() {
            tokens.renew(&self.key);
        }
    }
}

impl std::fmt::Debug for AgentTokenLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentTokenLease(..)")
    }
}

impl Drop for AgentTokenLease {
    fn drop(&mut self) {
        if let Some(tokens) = self.tokens.upgrade() {
            tokens.revoke(&self.key);
        }
    }
}

// ---------------------------------------------------------------------------
// Injection
// ---------------------------------------------------------------------------

/// The loopback IPC port this process's engine actually bound (0 = none).
static BOUND_IPC_PORT: AtomicU16 = AtomicU16::new(0);

/// Record the port `serve_ipc` bound, so injected servers dial this engine
/// rather than whatever `MAST_IPC_PORT` / the default names. One engine
/// serves IPC per process; the latest bind wins.
pub fn note_bound_ipc_port(port: u16) {
    BOUND_IPC_PORT.store(port, Ordering::Relaxed);
}

/// The port recorded by [`note_bound_ipc_port`], if any.
pub fn bound_ipc_port() -> Option<u16> {
    match BOUND_IPC_PORT.load(Ordering::Relaxed) {
        0 => None,
        port => Some(port),
    }
}

/// How a Run reaches this engine's agent surface: the `zeron mcp` command
/// and the loopback IPC port it dials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentToolsLaunch {
    pub command: PathBuf,
    pub args: Vec<String>,
    /// `None`: the port this process's engine bound ([`bound_ipc_port`]),
    /// read at injection time.
    pub ipc_port: Option<u16>,
}

impl AgentToolsLaunch {
    /// The running `zeron` binary's `mcp` subcommand, dialing the IPC port
    /// the engine actually bound. `None` when the executable path is unknown.
    pub fn from_current_exe() -> Option<Self> {
        let command = std::env::current_exe().ok()?;
        let mast = command.with_file_name("mast");
        let command = if command.file_name().is_some_and(|name| name == "zeron") && mast.is_file() {
            mast
        } else {
            command
        };
        Some(Self {
            command,
            args: vec!["mcp".into()],
            ipc_port: None,
        })
    }

    /// The injection for one Run holding `token`; `None` when this engine
    /// serves no IPC port (the server would reach another engine, or none).
    pub fn injection(&self, token: &str) -> Option<AgentToolsInjection> {
        let ipc_port = self.ipc_port.or_else(bound_ipc_port)?;
        Some(AgentToolsInjection {
            server_name: agent_tools::SERVER_NAME.into(),
            command: self.command.clone(),
            args: self.args.clone(),
            env: [
                (agent_tools::TOKEN_ENV.to_owned(), token.to_owned()),
                (agent_tools::IPC_PORT_ENV.to_owned(), ipc_port.to_string()),
                // Older `zeron mcp` binaries read the old names.
                (agent_tools::LEGACY_TOKEN_ENV.to_owned(), token.to_owned()),
                (
                    agent_tools::LEGACY_IPC_PORT_ENV.to_owned(),
                    ipc_port.to_string(),
                ),
            ]
            .into_iter()
            .collect(),
        })
    }
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Arguments every mutating tool shares.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyArgs {
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportFrictionArgs {
    kind: mast_proto::FrictionKind,
    summary: String,
    what_happened: String,
    workaround: Option<String>,
    suggestion: Option<String>,
    approval: Option<mast_proto::FrictionApproval>,
    severity: Option<mast_proto::FrictionSeverity>,
    #[serde(default)]
    links: Vec<mast_proto::RequestLink>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListFrictionArgs {
    kind: Option<mast_proto::FrictionKind>,
    status: Option<mast_proto::FrictionStatus>,
    since: Option<i64>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BindWorktreeArgs {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateChildArgs {
    title: String,
    #[serde(default)]
    description: Option<String>,
    /// Default: the Chat's own item.
    #[serde(default)]
    parent_work_item_id: Option<String>,
    /// `pending` (default) or `active`.
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    placement: Option<WorkOutlinePlacement>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssociateArgs {
    work_item_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetStatusArgs {
    /// Default: the Chat's own item.
    #[serde(default)]
    work_item_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    waiting_until: mast_proto::FieldPatch<i64>,
    #[serde(default)]
    waiting_reason: mast_proto::FieldPatch<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateUserActionRequestArgs {
    #[serde(default)]
    options: Vec<mast_proto::work_outline_user_actions::RequestOption>,
    #[serde(default)]
    links: Vec<mast_proto::RequestLink>,
    #[serde(default)]
    facts: Vec<mast_proto::RequestFact>,
    request: String,
    #[serde(default)]
    kind: Option<mast_proto::work_outline_user_actions::UserActionKind>,
    #[serde(default)]
    priority: Option<mast_proto::work_outline_user_actions::UserActionPriority>,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    dedupe_key: Option<String>,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    tool_call_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveUserActionRequestArgs {
    #[serde(default)]
    outcome: Option<String>,
    user_action_request_id: String,
    #[serde(default)]
    resolution: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnswerUserActionRequestArgs {
    request_id: String,
    user_message_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProposeInstructionArgs {
    /// `item` (default), `parent`, or `spaceRoot`.
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    container_work_item_id: Option<String>,
    #[serde(default)]
    rule_node_id: Option<String>,
    #[serde(default)]
    rule_node_path: Vec<String>,
    /// Required for `add` and `edit`.
    #[serde(default)]
    text: String,
    /// `add` (default), `edit`, or `remove`.
    #[serde(default)]
    action: Option<String>,
    /// Required for `edit` and `remove`.
    #[serde(default)]
    instruction_id: Option<String>,
    #[serde(default)]
    rationale: Option<String>,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    tool_call_id: Option<String>,
}

/// Only the prompt and attachments come from the caller; every other Run
/// setting is the child Chat's own (rebuilt engine-side).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueChildTurnArgs {
    chat_id: String,
    prompt: String,
    #[serde(default)]
    attachments: Vec<String>,
}

/// `work_outline_read_subtree` arguments.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadSubtreeArgs {
    /// Default: the Chat's own item.
    #[serde(default)]
    work_item_id: Option<String>,
    /// Levels of descendants below the item (default 4, at most 8).
    #[serde(default)]
    max_depth: Option<u32>,
    #[serde(default)]
    include_archived: bool,
    #[serde(default)]
    include_completed: bool,
}

/// The engine-internal `chat_launch_child_for_item` arguments: the work item
/// the child is started for, its first prompt, and its config. The MCP
/// server fills the config in from the caller's (sandbox capped); the engine
/// re-checks the caps.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LaunchChildArgs {
    work_item_id: String,
    prompt: String,
    #[serde(default)]
    config: Option<mast_proto::ChatConfig>,
}

/// Most items one `work_outline_read_subtree` call returns.
const MAX_SUBTREE_ITEMS: usize = 300;
/// Deepest `maxDepth` of `work_outline_read_subtree` (default 4).
const MAX_SUBTREE_DEPTH: u32 = 8;
const DEFAULT_SUBTREE_DEPTH: u32 = 4;
/// Most Chats / open requests listed per item in a subtree read.
const MAX_SUBTREE_ITEM_ROWS: usize = 25;
/// Longest `detail` a subtree read returns per request.
const MAX_SUBTREE_DETAIL_CHARS: usize = 1000;

fn sandbox_rank(level: SandboxLevel) -> u8 {
    match level {
        SandboxLevel::ReadOnly => 0,
        SandboxLevel::WorkspaceWrite => 1,
        SandboxLevel::DangerFullAccess => 2,
    }
}

/// A sandbox's reach once approval is counted: Auto runs full access unless
/// the chat is read-only.
fn effective_rank(level: SandboxLevel, auto_approve: bool) -> u8 {
    if auto_approve && level != SandboxLevel::ReadOnly {
        sandbox_rank(SandboxLevel::DangerFullAccess)
    } else {
        sandbox_rank(level)
    }
}

/// A child chat's config may never be wider than its parent's. Auto (full
/// access, never asks) needs an Auto parent; it forces full access in the
/// Codex harness (read-only is never widened), so an Auto child counts as
/// full access in the sandbox cap.
fn check_child_config(
    parent: Option<&mast_proto::ChatConfig>,
    child: &mast_proto::ChatConfig,
) -> Result<(), AgentToolError> {
    let parent_auto = parent.is_some_and(|c| c.auto_approve);
    let cap = parent
        .map(|c| c.sandbox)
        .unwrap_or(SandboxLevel::WorkspaceWrite);
    if child.auto_approve && !parent_auto {
        return Err(AgentToolError::refused(
            AgentToolErrorCode::Forbidden,
            "autoApprove refused: a child chat can only be Auto when the calling chat is on \
             Auto (never wider than its parent)",
        ));
    }
    if effective_rank(child.sandbox, child.auto_approve) > effective_rank(cap, parent_auto) {
        return Err(AgentToolError::refused(
            AgentToolErrorCode::Forbidden,
            "a child chat can never have a wider sandbox than its parent",
        ));
    }
    Ok(())
}

/// Whether `target` is `root` or one of its effective descendants.
fn in_subtree(snapshot: &mast_proto::WorkOutlineSnapshot, root: &str, target: &str) -> bool {
    let mut cursor = Some(target.to_owned());
    let mut steps = 0usize;
    while let Some(id) = cursor {
        if id == root {
            return true;
        }
        steps += 1;
        if steps > snapshot.items.len() {
            return false;
        }
        cursor = snapshot
            .item(&id)
            .and_then(|item| item.parent_work_item_id.clone());
    }
    false
}

/// The agent scope of a Chat on `root`: its item's subtree, widened to the
/// nearest project at or above it ("project scope", contract amendment).
fn in_agent_scope(snapshot: &mast_proto::WorkOutlineSnapshot, root: &str, target: &str) -> bool {
    in_subtree(snapshot, root, target)
        || snapshot
            .project_root(root)
            .is_some_and(|project| snapshot.is_within(&project.id, target))
}

fn resolve_id<'a>(id: &str, ids: impl Iterator<Item = &'a str>, kind: &str) -> Result<String, AgentToolError> {
    let id = id.trim();
    let ids: Vec<_> = ids.collect();
    if ids.contains(&id) { return Ok(id.to_owned()); }
    if id.chars().count() < 6 { return Err(AgentToolError::invalid(format!("{kind} id prefix must be at least 6 characters"))); }
    let matches: Vec<_> = ids.into_iter().filter(|candidate| candidate.starts_with(id)).collect();
    match matches.as_slice() {
        [matched] => Ok((*matched).to_owned()),
        [] => Err(AgentToolError::invalid(format!("{kind} not found"))),
        _ => Err(AgentToolError::invalid(format!("ambiguous: matches {}", matches.join(", ")))),
    }
}

/// Most attachments a child's first Turn may carry.
const MAX_CHILD_ATTACHMENTS: usize = 16;
/// Longest `read_current` waits for a Linear issue that is not cached.
const LINEAR_READ_TIMEOUT: Duration = Duration::from_secs(8);

fn parse_args<T: serde::de::DeserializeOwned>(arguments: &Value) -> Result<T, AgentToolError> {
    let arguments = if arguments.is_null() {
        json!({})
    } else {
        arguments.clone()
    };
    serde_json::from_value(arguments)
        .map_err(|e| AgentToolError::invalid(format!("invalid arguments: {e}")))
}

/// The authenticated caller. Holds no outline: reads that need one build it
/// themselves (only `read_current` materializes a snapshot).
struct CallContext<'a> {
    grant: AgentGrant,
    workspace: &'a WorkspaceHost,
    /// The Space this chat administers right now (a live user grant on this
    /// call), else `None`.
    admin_space: Option<String>,
    /// The chat's title, for attribution.
    chat_title: String,
}

impl CallContext<'_> {
    /// The work item the calling Chat is associated with (the same lookup
    /// the shared agent policy uses), if any.
    fn current_item_id(&self) -> Option<String> {
        self.workspace
            .chat_assigned_work_item(&self.grant.chat_id, &self.grant.space_id)
    }

    /// The Chat's item, or `unauthorized` naming what the agent tried.
    fn require_item(&self, what: &str) -> Result<String, AgentToolError> {
        self.current_item_id().ok_or_else(|| {
            WorkOutlineError::unauthorized(format!(
                "This Chat is not associated with a work item; associate it before {what}"
            ))
            .into()
        })
    }

    fn operation_id(&self, arguments: &Value) -> Result<String, AgentToolError> {
        let args: KeyArgs = parse_args(arguments)?;
        let key = agent_tools::validate_idempotency_key(args.idempotency_key.as_deref())
            .map_err(AgentToolError::invalid)?;
        Ok(agent_tools::operation_id(&self.grant.chat_id, key))
    }

    /// The operation id for the caller's `idempotencyKey`, or, when it is
    /// omitted, for the key `derive` builds (see `derived_idempotency_key`).
    /// Only naturally idempotent tools derive; the rest use `operation_id`.
    fn operation_id_or_derived(
        &self,
        arguments: &Value,
        derive: impl FnOnce() -> Result<String, AgentToolError>,
    ) -> Result<String, AgentToolError> {
        let args: KeyArgs = parse_args(arguments)?;
        if non_empty(args.idempotency_key).is_some() {
            return self.operation_id(arguments);
        }
        Ok(agent_tools::operation_id(&self.grant.chat_id, &derive()?))
    }

    /// A registry row's numeric field as text (`""` when absent): the state
    /// a derived key depends on, so a later call never replays an older one.
    fn row_stamp(&self, kind: &str, id: &str, field: &str) -> String {
        self.workspace
            .registry_row(kind, id)
            .and_then(|row| row.fields.get(field).and_then(Value::as_i64))
            .map(|stamp| stamp.to_string())
            .unwrap_or_default()
    }
}

/// An admin chat's resolution note carries "by admin chat <title> (granted
/// by Mikey)" so the origin chat can tell it from the user's own answer.
fn attributed_note(context: &CallContext, note: Option<String>) -> Option<String> {
    if context.admin_space.is_none() {
        return note;
    }
    let who = crate::admin::attribution(&context.chat_title);
    Some(match note {
        Some(note) => format!("{note} ({who})"),
        None => format!("Resolved {who}"),
    })
}

mod admin_tool;

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimChildArgs {
    inputs_hash: String,
}

/// Ledger keys binding a `create_chat` operation to its inputs (the same
/// scheme as Work Outline mutations, under their own prefix).
fn child_chat_ledger_keys(operation_id: &str, inputs_hash: &str) -> (String, String) {
    (
        format!("agent-child-chat:op:{operation_id}"),
        format!("agent-child-chat:in:{inputs_hash}:{operation_id}"),
    )
}

/// The engine's agent RPC surface (one per `EngineRpc`).
pub struct AgentToolsService {
    tokens: Arc<AgentTokens>,
    workspace: WorkspaceHost,
    work_outline: Arc<WorkOutlineService>,
    doc_host: Option<DocHost>,
    /// Live Chat status for `work_outline_read_subtree` (engine assembly).
    sessions: Mutex<Option<SessionsEngine>>,
    /// Serializes `create_chat` input claims (check-then-mark).
    child_claims: Mutex<()>,
    /// Linear integration: `read_current` adds the linked ticket in Spaces
    /// with Linear on (the key itself never reaches the agent).
    linear: Mutex<Option<crate::linear::LinearService>>,
    /// Planning artifacts (`artifact_*` tools).
    artifacts: Mutex<Option<crate::artifacts::ArtifactsService>>,
    /// Chat diff targets (`bind_chat_worktree`).
    diff_targets: Mutex<Option<crate::diff_targets::DiffTargets>>,
    /// PR state for the finished-worker rule.
    pr_ticket_cache: Mutex<Option<crate::pr_ticket_cache::PrTicketCache>>,
    /// Agent archives, for Recent changes / undo in the UI.
    archive_log: crate::archivable::AgentArchiveLog,
}

impl AgentToolsService {
    pub fn new(
        tokens: Arc<AgentTokens>,
        workspace: WorkspaceHost,
        work_outline: Arc<WorkOutlineService>,
        doc_host: Option<DocHost>,
    ) -> Self {
        Self {
            tokens,
            workspace,
            work_outline,
            doc_host,
            sessions: Mutex::new(None),
            child_claims: Mutex::new(()),
            linear: Mutex::new(None),
            artifacts: Mutex::new(None),
            pr_ticket_cache: Mutex::new(None),
            archive_log: crate::archivable::AgentArchiveLog::default(),
            diff_targets: Mutex::new(None),
        }
    }

    /// Attach the sessions engine (live Chat status in subtree reads).
    pub fn set_sessions(&self, sessions: SessionsEngine) {
        *lock(&self.sessions) = Some(sessions);
    }

    /// Attach the engine's artifact service (engine assembly).
    pub fn set_artifacts(&self, artifacts: crate::artifacts::ArtifactsService) {
        *lock(&self.artifacts) = Some(artifacts);
    }

    /// Attach the engine's diff targets service (engine assembly).
    pub fn set_diff_targets(&self, diff_targets: crate::diff_targets::DiffTargets) {
        *lock(&self.diff_targets) = Some(diff_targets);
    }

    /// The artifact service and the caller it acts for.
    fn artifact_call(
        &self,
        context: &CallContext,
    ) -> Result<
        (
            crate::artifacts::ArtifactsService,
            crate::artifacts::AgentCaller,
        ),
        AgentToolError,
    > {
        let service = lock(&self.artifacts).clone().ok_or_else(|| {
            AgentToolError::refused(
                AgentToolErrorCode::Unsupported,
                "planning artifacts are not available on this engine",
            )
        })?;
        let caller = crate::artifacts::AgentCaller {
            chat_id: context.grant.chat_id.clone(),
            run_id: context.grant.run_id.clone(),
            space_id: context.grant.space_id.clone(),
            device_id: context.grant.device_id.clone(),
            work_item_id: context.current_item_id(),
        };
        Ok((service, caller))
    }

    /// Attach the engine's Linear service (engine assembly).
    pub fn set_linear(&self, linear: crate::linear::LinearService) {
        *lock(&self.linear) = Some(linear);
    }

    /// The Linear issue `identifier` for an agent: the cached copy, else a
    /// fetch bounded by [`LINEAR_READ_TIMEOUT`] (this runs on a blocking
    /// thread; the fetch runs on the engine's runtime). `Err(code)` on
    /// failure.
    fn linear_issue_for_agent(
        &self,
        identifier: &str,
    ) -> Result<mast_proto::LinearIssueSummary, &'static str> {
        let Some(linear) = lock(&self.linear).clone() else {
            return Err("unavailable");
        };
        if let Some(issue) = linear.cached_issue(identifier) {
            return Ok(issue);
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return Err("unavailable");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let identifier = identifier.to_owned();
        handle.spawn(async move {
            let _ = tx.send(linear.issue(&identifier, false).await);
        });
        match rx.recv_timeout(LINEAR_READ_TIMEOUT) {
            Ok(Ok(issue)) => Ok(issue),
            Ok(Err(error)) => Err(error.code.as_str()),
            Err(_) => Err("network"),
        }
    }

    /// `linearIssue` / `linearProject` fields of `read_current` for `item`
    /// (nearest issue / project reference on its path), only in a Space with
    /// Linear on.
    fn linear_context(
        &self,
        snapshot: &mast_proto::WorkOutlineSnapshot,
        item_id: &str,
    ) -> Option<Value> {
        if !snapshot.space_settings.linear_enabled {
            return None;
        }
        let mut out = serde_json::Map::new();
        if let Some(reference) =
            snapshot.nearest_linear_reference(item_id, mast_proto::linear::LINEAR_ISSUE_REFERENCE)
        {
            match self.linear_issue_for_agent(&reference.external_id) {
                Ok(issue) => {
                    out.insert("linearIssue".into(), json!(issue));
                }
                Err(code) => {
                    out.insert(
                        "linearIssue".into(),
                        json!({
                            "identifier": reference.external_id,
                            "url": reference.url,
                            "unavailable": code,
                        }),
                    );
                }
            }
        }
        if let Some(reference) =
            snapshot.nearest_linear_reference(item_id, mast_proto::linear::LINEAR_PROJECT_REFERENCE)
        {
            out.insert(
                "linearProject".into(),
                json!(mast_proto::LinearProjectRef {
                    id: reference.external_id.clone(),
                    name: reference.cached_title.clone().unwrap_or_default(),
                    url: reference.url.clone(),
                }),
            );
        }
        (!out.is_empty()).then_some(Value::Object(out))
    }

    pub fn tokens(&self) -> &Arc<AgentTokens> {
        &self.tokens
    }

    /// `AgentAuthenticate`: who the token speaks for (token lookup and the
    /// Chat row only; no outline is materialized).
    pub fn authenticate(&self, token: &str) -> Result<AgentOrigin, AgentToolError> {
        let context = self.context(token)?;
        let mut origin = context.grant.origin();
        origin.admin_space_id = context.admin_space.clone();
        Ok(origin)
    }

    fn context(&self, token: &str) -> Result<CallContext<'_>, AgentToolError> {
        let grant = self.tokens.authenticate(token)?;
        // The Chat must still exist in the Space the token was minted for
        // (Space is write-once; a deleted Chat ends the grant's use).
        let chat = self
            .workspace
            .chat(&grant.chat_id)
            .ok()
            .flatten()
            .ok_or_else(AgentToolError::unauthenticated)?;
        if chat.space_id.as_deref() != Some(grant.space_id.as_str()) {
            return Err(AgentToolError::unauthenticated());
        }
        let admin_space = self
            .tokens
            .admin()
            .live(&grant.chat_id)
            .filter(|g| g.space_id == grant.space_id)
            .map(|g| g.space_id);
        Ok(CallContext {
            grant,
            workspace: &self.workspace,
            admin_space,
            chat_title: chat.title.clone().unwrap_or_default(),
        })
    }

    /// `AgentToolCall`: authenticate, authorize, and run one tool.
    ///
    /// Arguments are normalised first (guessed names such as `text` for
    /// `request` map to the real parameter), and an argument error carries
    /// the tool's expected parameters and an example call.
    pub fn call(&self, params: &AgentToolCallParams) -> Result<Value, AgentToolError> {
        let context = self.context(&params.token)?;
        let arguments = agent_tools::normalize_arguments(&params.tool, &params.arguments);
        self.dispatch(&context, &params.tool, &arguments)
            .map_err(|error| match error {
                AgentToolError::Refused {
                    code: AgentToolErrorCode::Invalid,
                    message,
                } => AgentToolError::invalid(agent_tools::with_usage(&params.tool, &message)),
                other => other,
            })
    }

    fn dispatch(
        &self,
        context: &CallContext,
        tool: &str,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        match tool {
            tools::READ_CURRENT => Ok(self.read_current(context)),
            tools::GET_USER_ACTION_REQUEST => self.get_user_action_request(context, arguments),
            tools::READ_SUBTREE => self.read_subtree(context, arguments),
            tools::CREATE_CHILD => self.create_child(context, arguments),
            tools::ASSOCIATE_CURRENT_CHAT => self.associate_current_chat(context, arguments),
            tools::SET_STATUS => self.set_status(context, arguments),
            tools::CREATE_USER_ACTION_REQUEST => {
                self.create_user_action_request(context, arguments)
            }
            tools::RESOLVE_USER_ACTION_REQUEST => {
                self.resolve_user_action_request(context, arguments)
            }
            tools::ANSWER_USER_ACTION_REQUEST => {
                self.answer_user_action_request(context, arguments)
            }
            tools::APPLY_CONTAINER_TEMPLATE => self.apply_container_template(context, arguments),
            tools::PROPOSE_INSTRUCTION => self.propose_instruction(context, arguments),
            tools::PROPOSE_APPROVAL_RULE => self.propose_approval_rule(context, arguments),
            tools::ADMIN => self.admin(context, arguments),
            tools::QUEUE_CHILD_CHAT_TURN => self.queue_child_chat_turn(context, arguments),
            tools::CLAIM_CHILD_CHAT => self.claim_child_chat(context, arguments),
            tools::PROJECT_SCOPE_CHATS => Ok(self.project_scope_chats(context)),
            tools::CHAT_ARCHIVE_INFO => self.chat_archive_info(context, arguments),
            tools::CHAT_ARCHIVE => self.chat_archive(context, arguments),
            tools::REVIEW_GET => {
                let (service, caller) = self.artifact_call(context)?;
                let mut result = service.agent_review_get(&caller, arguments)?;
                if let Some(review) = result.get("review") {
                    let repo = review.pointer("/pr/repo").and_then(Value::as_str);
                    let number = review.pointer("/pr/number").and_then(Value::as_u64);
                    let reviewed_head = review
                        .pointer("/pr/headSha")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    if let (Some(repo), Some(number), Some(reviewed_head)) =
                        (repo, number, reviewed_head)
                    {
                        let url = format!("https://github.com/{repo}/pull/{number}");
                        // The live PR cache wins only when it knows the head;
                        // otherwise keep the answer from the local PR ref.
                        let current = lock(&self.pr_ticket_cache).as_ref().and_then(|cache| {
                            cache
                                .status_for(None, None, Some(&url), None)
                                .pr
                                .and_then(|pr| pr.head_sha)
                        });
                        if let Some(current) = current {
                            result["stale"] =
                                (!current.eq_ignore_ascii_case(&reviewed_head)).into();
                            result["currentHeadSha"] = current.into();
                        }
                    }
                }
                Ok(result)
            }
            tools::REVIEW_PUBLISH => {
                let (service, caller) = self.artifact_call(context)?;
                Ok(service.agent_review_publish(&caller, arguments)?)
            }
            tools::ARTIFACT_PUBLISH => {
                let (service, caller) = self.artifact_call(context)?;
                Ok(service.agent_publish(&caller, arguments)?)
            }
            tools::ARTIFACT_GET_PLAN => {
                let (service, caller) = self.artifact_call(context)?;
                Ok(service.agent_get_plan(&caller)?)
            }
            tools::ARTIFACT_GET_DECISIONS => {
                let (service, caller) = self.artifact_call(context)?;
                Ok(service.agent_get_decisions(&caller, arguments)?)
            }
            tools::ARTIFACT_RENDER_CHECK => {
                let (service, caller) = self.artifact_call(context)?;
                Ok(service.agent_render_check(&caller, arguments)?)
            }
            tools::BIND_CHAT_WORKTREE => self.bind_chat_worktree(context, arguments),
            tools::REPORT_FRICTION => self.report_friction(context, arguments),
            tools::LIST_FRICTION => self.list_friction(context, arguments),
            tools::DISMISS_FRICTION => self.dismiss_friction(context, arguments),
            other => Err(AgentToolError::refused(
                AgentToolErrorCode::Unsupported,
                format!("unknown agent tool: {other}"),
            )),
        }
    }

    /// Apply one mutation as the token's Chat: `mutate_as` with the agent
    /// actor runs the shared policy under the registry lock, then the kill
    /// switch, the durable replay ledger, and one registry write.
    fn apply(
        &self,
        context: &CallContext,
        mutation: &WorkOutlineMutation,
    ) -> Result<WorkOutlineMutateReply, AgentToolError> {
        let actor = self.work_outline.agent_actor(&context.grant.chat_id, None);
        Ok(self
            .work_outline
            .mutate_as_admin(mutation, &actor, context.admin_space.as_deref())?)
    }

    fn resolve_work_item_id(&self, context: &CallContext, id: &str) -> Result<String, AgentToolError> {
        let snapshot = self.work_outline.snapshot(&context.grant.space_id);
        match resolve_id(id, snapshot.items.iter().map(|item| item.id.as_str()), "work item") {
            Err(AgentToolError::Refused { ref message, .. }) if message.contains("not found") => Ok(id.trim().to_owned()),
            result => result,
        }
    }

    fn resolve_request_id(&self, context: &CallContext, id: &str) -> Result<String, AgentToolError> {
        let snapshot = self.work_outline.snapshot(&context.grant.space_id);
        let root = context.current_item_id();
        let requests: Vec<_> = snapshot.user_action_requests.iter().filter(|request| {
            context.admin_space.as_deref() == Some(request.space_id.as_str())
                || request.chat_id == context.grant.chat_id
                || self.workspace.chat(&request.chat_id).ok().flatten().is_some_and(|chat| chat.parent_chat_id.as_deref() == Some(context.grant.chat_id.as_str()))
                || root.as_deref().is_some_and(|root| in_agent_scope(&snapshot, root, &request.work_item_id))
        }).collect();
        match resolve_id(id, snapshot.user_action_requests.iter().map(|request| request.id.as_str()), "request") {
            Ok(id) => Ok(id),
            Err(AgentToolError::Refused { ref message, .. }) if message.contains("not found") => {
                let open = requests.iter().filter(|request| request.is_open()).map(|request| request.id.as_str()).collect::<Vec<_>>().join(", ");
                Err(WorkOutlineError::new(mast_proto::work_outline::WorkOutlineErrorCode::NotFound, format!("request not found; open requests in your scope: {}", if open.is_empty() { "(none)" } else { &open })).into())
            }
            Err(error) => Err(error),
        }
    }

    // ---- tools ---------------------------------------------------------------

    fn report_friction(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let args: ReportFrictionArgs = parse_args(arguments)?;
        let chat = self
            .workspace
            .chat(&context.grant.chat_id)
            .map_err(|e| AgentToolError::invalid(e.to_string()))?
            .ok_or_else(|| AgentToolError::invalid("calling chat not found"))?;
        let harness_id = lock(&self.sessions)
            .as_ref()
            .and_then(|sessions| {
                sessions.run_harness(&context.grant.chat_id, &context.grant.run_id)
            })
            .or_else(|| chat.config.as_ref().map(|config| config.harness));
        let harness = harness_id
            .and_then(|id| serde_json::to_value(id).ok())
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".into());
        let now = chrono::Utc::now().timestamp_millis();
        let report = mast_proto::FrictionReport {
            id: String::new(),
            kind: args.kind,
            summary: args.summary,
            what_happened: args.what_happened,
            workaround: args.workaround,
            suggestion: args.suggestion,
            approval: args.approval,
            severity: args.severity,
            links: args.links,
            provenance: mast_proto::FrictionProvenance {
                chat_id: context.grant.chat_id.clone(),
                harness,
                model: chat
                    .actual_model
                    .or_else(|| chat.config.and_then(|c| c.model)),
                space_id: context.grant.space_id.clone(),
                work_item_id: context.current_item_id(),
                branch: chat.branch,
                created_at: now,
                source: mast_proto::FrictionSource::Agent, file_name: None,
            },
            fingerprint: String::new(),
            occurrences: 0,
            last_seen: now,
            status: mast_proto::FrictionStatus::Open,
            promoted_item_id: None,
            promoted_at: None,
        };
        let report = self
            .workspace
            .record_friction(report)
            .map_err(|e| AgentToolError::invalid(e.to_string()))?;
        Ok(json!({ "report": report }))
    }

    fn list_friction(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let args: ListFrictionArgs = parse_args(arguments)?;
        let mut reports = self.workspace.friction_reports(&context.grant.space_id);
        reports.retain(|report| {
            args.kind.is_none_or(|kind| report.kind == kind)
                && args.status.is_none_or(|status| report.status == status)
                && args.since.is_none_or(|since| report.last_seen >= since)
        });
        reports.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then_with(|| a.id.cmp(&b.id)));
        reports.truncate(args.limit.unwrap_or(100).min(500));
        let reports: Vec<Value> = reports.into_iter().map(|report| {
            let reason = self.workspace.registry_row(mast_doc::registry::friction::KIND_FRICTION_REPORTS, &report.id).and_then(|row| row.fields.get("dismissalReason").cloned());
            let mut value = json!(report);
            if let Some(reason) = reason { value["dismissalReason"] = reason; }
            value
        }).collect();
        Ok(json!({ "reports": reports }))
    }

    fn dismiss_friction(&self, context: &CallContext, arguments: &Value) -> Result<Value, AgentToolError> {
        let id = arguments.get("id").and_then(Value::as_str).unwrap_or("").trim();
        let reason = arguments.get("reason").and_then(Value::as_str).unwrap_or("").trim();
        if id.is_empty() || reason.is_empty() || reason.chars().count() > 1000 {
            return Err(AgentToolError::invalid("id and reason (at most 1000 characters) are required"));
        }
        let direct_children = self.workspace.read_chats().map_err(|error| AgentToolError::invalid(error.to_string()))?.iter().any(|chat| chat.space_id.as_deref() == Some(&context.grant.space_id) && chat.parent_chat_id.as_deref() == Some(&context.grant.chat_id));
        let snapshot = self.work_outline.snapshot(&context.grant.space_id);
        let project_workers = context.current_item_id().and_then(|item| snapshot.worker_destination_for(&item)).is_some_and(|destination| {
            snapshot.assignments.iter().any(|assignment| assignment.chat_id != context.grant.chat_id && assignment.work_item_id.as_deref().is_some_and(|item| snapshot.is_within(destination, item)))
        });
        let orchestrator = direct_children || project_workers;
        if context.admin_space.as_deref() != Some(&context.grant.space_id) && !orchestrator {
            return Err(WorkOutlineError::unauthorized("Only orchestrator or admin chats may dismiss friction").into());
        }
        let reports = self.workspace.friction_reports(&context.grant.space_id);
        let id = resolve_id(id, reports.iter().map(|report| report.id.as_str()), "friction report")?;
        let report = reports.iter().find(|report| report.id == id).expect("resolved report");
        let report = self.workspace.dismiss_friction(&context.grant.space_id, &report.fingerprint, reason).map_err(|error| AgentToolError::invalid(error.to_string()))?;
        Ok(json!({"report": report, "dismissalReason": reason}))
    }

    /// `bind_chat_worktree`: record a worktree as the calling Chat's diff
    /// target. Blocking git runs inline (this is already on a blocking thread).
    fn bind_chat_worktree(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let args: BindWorktreeArgs = parse_args(arguments)?;
        // Binding is a find-or-add plus select, so without a key it needs no
        // replay ledger: a repeat re-selects the same target (a derived key
        // would instead replay a stale selection after binding another path).
        let key: KeyArgs = parse_args(arguments)?;
        let operation_id = match non_empty(key.idempotency_key) {
            Some(_) => Some(context.operation_id(arguments)?),
            None => None,
        };
        let path = args.path.as_deref().map(str::trim).unwrap_or_default();
        if path.is_empty() {
            return Err(AgentToolError::invalid(
                "path is required: the absolute top-level folder of the worktree",
            ));
        }
        let diff_targets = lock(&self.diff_targets).clone().ok_or_else(|| {
            AgentToolError::refused(
                AgentToolErrorCode::Unsupported,
                "diff targets are not available",
            )
        })?;
        let chat_id = &context.grant.chat_id;
        let outcome = diff_targets
            .bind(
                chat_id,
                path,
                ChatDiffTargetSource::Agent,
                operation_id.as_deref(),
            )
            .map_err(|e| match e {
                DiffTargetError::NotLocal { .. } => {
                    AgentToolError::refused(AgentToolErrorCode::Forbidden, e.to_string())
                }
                other => AgentToolError::invalid(other.to_string()),
            })?;
        Ok(json!({
            "chatId": chat_id,
            "path": outcome.path,
            "branch": outcome.branch,
            "headSha": outcome.head_sha,
            "repoName": outcome.repo_name,
            "selected": outcome.selected,
            "replayed": outcome.replayed,
            "targets": outcome.reply.targets,
        }))
    }

    fn read_current(&self, context: &CallContext) -> Value {
        let unassociated = json!({
            "chatId": context.grant.chat_id,
            "spaceId": context.grant.space_id,
            "workItem": null,
            "effectiveInstructions": null,
            "openUserActionRequests": [],
            "note": "This Chat is not associated with a work item. Use \
                     work_outline_associate_current_chat to associate it with one of this \
                     Space's items before using the other Work Outline tools.",
        });
        // The shared read model: the item, its effective instructions, and
        // the open requests this Chat raised, from ONE snapshot (the path
        // and children below read the same one).
        let snapshot = self.work_outline.snapshot(&context.grant.space_id);
        let Ok(agent) = self.work_outline.agent_context_in(
            &context.grant.chat_id,
            &context.grant.space_id,
            &snapshot,
        ) else {
            return unassociated;
        };
        let item = &agent.work_item;
        let mut path = Vec::new();
        let mut cursor = Some(item.id.clone());
        while let Some(id) = cursor {
            let Some(node) = snapshot.item(&id) else {
                break;
            };
            path.push(json!({ "id": node.id, "title": node.title }));
            if path.len() > snapshot.items.len() {
                break;
            }
            cursor = node.parent_work_item_id.clone();
        }
        path.reverse();
        let children: Vec<Value> = item
            .child_ids
            .iter()
            .filter_map(|id| snapshot.item(id))
            .filter(|child| child.archived_at.is_none())
            .map(|child| {
                json!({
                    "id": child.id,
                    "title": child.title,
                    "status": child.status,
                    "effectiveStatus": child.effective_status,
                    "waitingUntil": child.waiting_until,
                    "waitingReason": child.waiting_reason,
                })
            })
            .collect();
        let linear = self.linear_context(&snapshot, &item.id);
        let mut reply = json!({
            "chatId": agent.chat_id,
            "spaceId": agent.space_id,
            "workItem": {
                "id": item.id,
                "title": item.title,
                "description": item.description,
                "status": item.status,
                "effectiveStatus": item.effective_status,
                "waitingUntil": item.waiting_until,
                "waitingReason": item.waiting_reason,
                "parentWorkItemId": item.parent_work_item_id,
                "archived": item.archived_at.is_some(),
                "path": path,
                "children": children,
            },
            "effectiveInstructions": agent.effective_instructions,
            "openUserActionRequests": agent.open_user_action_requests,
            "userActionRequests": snapshot.user_action_requests.iter().filter(|r| r.chat_id == context.grant.chat_id).collect::<Vec<_>>(),
            "note": "You may create children under this item or its descendants, set their \
                     status to active or completed, ask the user for an action on this item, \
                     resolve requests you or chats you created raised, and propose \
                     instructions for it, its parent, or the Space root.",
        });
        if let (Some(Value::Object(linear)), Some(object)) = (linear, reply.as_object_mut()) {
            object.extend(linear);
        }
        // Project templates: where workers this Chat creates go (set only
        // when its project has a live destination the Chat is not inside).
        if let Some(destination) = snapshot.worker_destination_for(&item.id)
            && let Some(object) = reply.as_object_mut()
        {
            object.insert("workerDestinationId".into(), json!(destination));
        }
        reply
    }

    fn create_child(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let args: CreateChildArgs = parse_args(arguments)?;
        let operation_id = context.operation_id(arguments)?;
        let status = match args.status.as_deref().map(str::trim) {
            None | Some("") | Some("pending") => None,
            Some("active") => Some(WorkItemStatus::Active),
            Some(other) => {
                return Err(AgentToolError::invalid(format!(
                    "status must be pending or active for a new item, not {other:?}"
                )));
            }
        };
        // No explicit parent: workers of a project go to its worker
        // destination, anything else under the Chat's own item.
        let mut to_destination = false;
        let parent = match non_empty(args.parent_work_item_id) {
            Some(id) => Some(self.resolve_work_item_id(context, &id)?),
            None => context.current_item_id().map(|current| {
                let snapshot = self.work_outline.snapshot(&context.grant.space_id);
                match snapshot.worker_destination_for(&current) {
                    Some(destination) => {
                        to_destination = true;
                        destination.to_owned()
                    }
                    None => current,
                }
            }),
        };
        let mutation = WorkOutlineMutation::CreateWorkItem(CreateWorkItemParams {
            space_id: context.grant.space_id.clone(),
            parent_work_item_id: parent.clone(),
            placement: args.placement.unwrap_or_default(),
            title: args.title,
            description: non_empty(args.description),
            status,
            operation_id,
        });
        let reply = self.apply(context, &mutation)?;
        Ok(json!({
            "workItemId": reply.work_item_id,
            "parentWorkItemId": parent,
            "resolvedToWorkerDestination": to_destination,
            "createdByChatId": context.grant.chat_id,
        }))
    }

    fn associate_current_chat(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let mut args: AssociateArgs = parse_args(arguments)?;
        args.work_item_id = self.resolve_work_item_id(context, &args.work_item_id)?;
        let operation_id = context.operation_id_or_derived(arguments, || {
            let assigned = context.row_stamp(
                mast_doc::work_outline::KIND_WORK_ITEM_CHAT_ASSIGNMENTS,
                &context.grant.chat_id,
                "assignedAt",
            );
            Ok(agent_tools::derived_idempotency_key(
                tools::ASSOCIATE_CURRENT_CHAT,
                &[&context.grant.chat_id, &args.work_item_id, &assigned],
            ))
        })?;
        let mutation = WorkOutlineMutation::AssignChatToWorkItem(AssignChatToWorkItemParams {
            chat_id: context.grant.chat_id.clone(),
            work_item_id: args.work_item_id.clone(),
            operation_id,
        });
        self.apply(context, &mutation)?;
        Ok(json!({ "chatId": context.grant.chat_id, "workItemId": args.work_item_id }))
    }

    fn apply_container_template(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let mut args: AssociateArgs = parse_args(arguments)?;
        args.work_item_id = self.resolve_work_item_id(context, &args.work_item_id)?;
        let target = args.work_item_id;
        let operation_id = context.operation_id_or_derived(arguments, || {
            let updated = context.row_stamp(
                mast_doc::work_outline::KIND_WORK_ITEMS,
                &target,
                "updatedAt",
            );
            Ok(agent_tools::derived_idempotency_key(
                tools::APPLY_CONTAINER_TEMPLATE,
                &[&target, &updated],
            ))
        })?;
        let mutation = WorkOutlineMutation::ApplyContainerTemplate(
            mast_proto::work_outline_container_templates::ApplyContainerTemplateParams {
                operation_id,
                space_id: context.grant.space_id.clone(),
                work_item_id: target.clone(),
            },
        );
        self.apply(context, &mutation)?;
        Ok(json!({ "workItemId": target, "applied": true }))
    }

    fn set_status(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let args: SetStatusArgs = parse_args(arguments)?;
        let status = match args.status.as_deref().map(str::trim) {
            Some("active") => Some(WorkItemStatus::Active),
            Some("completed") => Some(WorkItemStatus::Completed),
            None => None,
            Some(other) => {
                return Err(AgentToolError::invalid(format!(
                    "status must be active or completed, not {other:?}"
                )));
            }
        };
        let target = match non_empty(args.work_item_id) {
            Some(id) => self.resolve_work_item_id(context, &id)?,
            None => context.require_item("changing status")?,
        };
        if context.admin_space.is_none() {
            let own_item = context.require_item("changing status")?;
            let snapshot = self.work_outline.snapshot(&context.grant.space_id);
            if !in_subtree(&snapshot, &own_item, &target) {
                let allowed = snapshot.items.iter().filter(|item| in_subtree(&snapshot, &own_item, &item.id)).map(|item| format!("{} ({})", item.title.replace(['\n', '\r'], " "), item.id)).collect::<Vec<_>>().join(", ");
                let target_title = snapshot.item(&target).map(|item| item.title.replace(['\n', '\r'], " ")).unwrap_or_else(|| "unknown".into());
                let own_title = snapshot.item(&own_item).map(|item| item.title.replace(['\n', '\r'], " ")).unwrap_or_else(|| "unknown".into());
                return Err(WorkOutlineError::unauthorized(format!("Cannot change {target_title} ({target}); your item is {own_title} ({own_item}); allowed subtree: {allowed}")).into());
            }
        }
        let waiting_changed = !args.waiting_until.is_unchanged() || !args.waiting_reason.is_unchanged();
        if status.is_none() && !waiting_changed {
            return Err(AgentToolError::invalid("Provide status or waitingUntil"));
        }
        let operation_id = if waiting_changed { context.operation_id(arguments)? } else { context.operation_id_or_derived(arguments, || {
            let updated = context.row_stamp(
                mast_doc::work_outline::KIND_WORK_ITEMS,
                &target,
                "updatedAt",
            );
            Ok(agent_tools::derived_idempotency_key(
                tools::SET_STATUS,
                &[&target, args.status.as_deref().unwrap_or_default().trim(), &updated],
            ))
        })? };
        // `resolveOpenRequests` stays false: completing an item with open
        // requests fails (`openUserActionRequests`), never auto-resolves.
        let mutation = WorkOutlineMutation::UpdateWorkItem(UpdateWorkItemParams {
            work_item_id: target.clone(),
            changes: WorkItemChanges {
                status,
                waiting_until: args.waiting_until,
                waiting_reason: args.waiting_reason,
                ..WorkItemChanges::default()
            },
            operation_id,
        });
        self.apply(context, &mutation)?;
        Ok(json!({ "workItemId": target, "status": status }))
    }

    fn create_user_action_request(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        if let Some(links) = arguments.get("links").and_then(Value::as_array) {
            for (index, link) in links.iter().enumerate() {
                if serde_json::from_value::<mast_proto::RequestLink>(link.clone()).is_err() {
                    let label = link
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("<unlabeled>");
                    return Err(AgentToolError::invalid(format!(
                        "invalid link {index} '{label}'; allowed target types: artifact, file, diff, pr, chat, url"
                    )));
                }
            }
        }
        let mut args: CreateUserActionRequestArgs = parse_args(arguments)?;
        if args.request.trim().is_empty() {
            return Err(AgentToolError::invalid("request is empty"));
        }
        if args.links.len() > 8 || args.facts.len() > 8 {
            return Err(AgentToolError::invalid(
                "at most 8 links and 8 facts are allowed",
            ));
        }
        let mut warnings = Vec::new();
        let mut truncate = |value: &mut String, limit: usize, field: String| {
            if value.chars().count() > limit {
                *value = value.chars().take(limit - 1).chain(std::iter::once('…')).collect();
                warnings.push(format!("{field} truncated to {limit} characters"));
            }
        };
        for (index, fact) in args.facts.iter_mut().enumerate() {
            truncate(&mut fact.label, 40, format!("fact {index} label"));
            truncate(&mut fact.value, 120, format!("fact {index} value"));
        }
        for (index, link) in args.links.iter_mut().enumerate() {
            truncate(&mut link.label, 40, format!("link {index} label"));
        }
        let snapshot = self.work_outline.snapshot(&context.grant.space_id);
        for link in &args.links {
            use mast_proto::RequestLinkTarget;
            let valid = link.label.chars().count() <= 40
                && match &link.target {
                    RequestLinkTarget::Artifact { id } => {
                        self.workspace.artifact(id).is_some_and(|artifact| {
                            artifact.archived_at.is_none()
                                && artifact.space_id == context.grant.space_id
                                && (context.admin_space.as_deref()
                                    == Some(artifact.space_id.as_str())
                                    || artifact.chat_id.as_deref()
                                        == Some(context.grant.chat_id.as_str())
                                    || artifact
                                        .chat_id
                                        .as_deref()
                                        .and_then(|id| self.workspace.chat(id).ok().flatten())
                                        .is_some_and(|chat| {
                                            chat.parent_chat_id.as_deref()
                                                == Some(context.grant.chat_id.as_str())
                                        })
                                    || artifact.work_item_id.as_deref().is_some_and(|item| {
                                        context.current_item_id().is_some_and(|root| {
                                            in_agent_scope(&snapshot, &root, item)
                                        })
                                    }))
                        })
                    }
                    RequestLinkTarget::Chat { id } => {
                        self.workspace.chat(id).ok().flatten().is_some_and(|chat| {
                            chat.space_id.as_deref() == Some(context.grant.space_id.as_str())
                                && (context.admin_space.as_deref()
                                    == Some(context.grant.space_id.as_str())
                                    || id == &context.grant.chat_id
                                    || chat.parent_chat_id.as_deref()
                                        == Some(context.grant.chat_id.as_str())
                                    || context.current_item_id().is_some_and(|root| {
                                        snapshot.assignments.iter().any(|a| {
                                            a.chat_id == *id
                                                && a.work_item_id.as_deref().is_some_and(|item| {
                                                    in_agent_scope(&snapshot, &root, item)
                                                })
                                        })
                                    }))
                        })
                    }
                    RequestLinkTarget::File { path, line } => {
                        let path = std::path::Path::new(path);
                        !path
                            .components()
                            .any(|c| matches!(c, std::path::Component::ParentDir))
                            && *line != Some(0)
                            && self
                                .workspace
                                .chat(&context.grant.chat_id)
                                .ok()
                                .flatten()
                                .and_then(|chat| chat.cwd)
                                .is_some_and(|cwd| {
                                    let root = std::path::Path::new(&cwd);
                                    let candidate = if path.is_absolute() {
                                        path.to_path_buf()
                                    } else {
                                        root.join(path)
                                    };
                                    root.canonicalize().ok().is_some_and(|root| {
                                        candidate
                                            .ancestors()
                                            .find_map(|ancestor| ancestor.canonicalize().ok())
                                            .is_some_and(|ancestor| ancestor.starts_with(root))
                                    })
                                })
                    }
                    RequestLinkTarget::Diff { branch } => {
                        branch.as_ref().is_none_or(|b| !b.trim().is_empty())
                    }
                    RequestLinkTarget::Url { url } => reqwest::Url::parse(url)
                        .is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some()),
                    RequestLinkTarget::Pr { url } => reqwest::Url::parse(url).is_ok_and(|url| {
                        let parts: Vec<_> =
                            url.path_segments().map(|p| p.collect()).unwrap_or_default();
                        url.scheme() == "https"
                            && match url.host_str() {
                                Some("github.com") => {
                                    parts.len() == 4
                                        && !parts[0].is_empty()
                                        && !parts[1].is_empty()
                                        && parts[2] == "pull"
                                        && parts[3].parse::<u64>().is_ok_and(|n| n > 0)
                                }
                                Some("gitlab.com") => {
                                    parts.len() >= 5
                                        && parts[..parts.len() - 3].iter().all(|p| !p.is_empty())
                                        && parts[parts.len() - 3] == "-"
                                        && parts[parts.len() - 2] == "merge_requests"
                                        && parts[parts.len() - 1]
                                            .parse::<u64>()
                                            .is_ok_and(|n| n > 0)
                                }
                                _ => false,
                            }
                    }),
                };
            if !valid {
                return Err(AgentToolError::invalid(format!(
                    "invalid link '{}'; allowed target types: artifact, file, diff, pr, chat, url",
                    link.label
                )));
            }
        }
        // Requests are created on the Chat's own item, with the Chat as origin.
        let work_item_id = context.require_item("asking the user for an action")?;
        // A `dedupeKey` stands in for a missing `idempotencyKey`; without
        // either, a retry could duplicate the request.
        let operation_id = match non_empty(args.dedupe_key.clone()) {
            Some(dedupe_key) => context.operation_id_or_derived(arguments, || {
                // A repeat while the request is open replays it (same id);
                // once it is resolved, asking again is a new request.
                let resolved = self
                    .work_outline
                    .snapshot(&context.grant.space_id)
                    .user_action_requests
                    .iter()
                    .filter(|r| {
                        r.work_item_id == work_item_id
                            && r.dedupe_key.as_deref() == Some(dedupe_key.as_str())
                            && !r.deduped
                            && !r.is_open()
                    })
                    .count()
                    .to_string();
                let kind = serde_json::to_string(&args.kind).unwrap_or_default();
                let priority = serde_json::to_string(&args.priority).unwrap_or_default();
                let links = serde_json::to_string(&args.links).unwrap_or_default();
                let facts = serde_json::to_string(&args.facts).unwrap_or_default();
                let options = serde_json::to_string(&args.options).unwrap_or_default();
                let mut inputs = vec![
                    work_item_id.as_str(),
                    dedupe_key.as_str(),
                    args.request.as_str(),
                    args.detail.as_deref().unwrap_or_default(),
                    kind.as_str(),
                    priority.as_str(),
                    resolved.as_str(),
                ];
                // Preserve derived keys for requests created before links and facts existed.
                if !args.links.is_empty() || !args.facts.is_empty() {
                    inputs.extend([links.as_str(), facts.as_str()]);
                }
                if !args.options.is_empty() { inputs.push(options.as_str()); }
                Ok(agent_tools::derived_idempotency_key(
                    tools::CREATE_USER_ACTION_REQUEST,
                    &inputs,
                ))
            })?,
            None => context.operation_id(arguments)?,
        };
        let mutation =
            WorkOutlineMutation::CreateUserActionRequest(CreateUserActionRequestParams {
                options: args.options,
                links: args.links,
                facts: args.facts,
                kind: args.kind.or_else(|| {
                    args.dedupe_key
                        .as_deref()
                        .filter(|key| key.starts_with("artopt:"))
                        .map(|_| mast_proto::work_outline_user_actions::UserActionKind::Choose)
                }),
                priority: args.priority,
                work_item_id: work_item_id.clone(),
                origin: UserActionRequestOrigin {
                    kind: Default::default(),
                    chat_id: context.grant.chat_id.clone(),
                    message_id: non_empty(args.message_id),
                    tool_call_id: non_empty(args.tool_call_id),
                },
                text: args.request,
                info: non_empty(args.detail),
                dedupe_key: non_empty(args.dedupe_key),
                operation_id,
            });
        let reply = self.apply(context, &mutation)?;
        Ok(json!({
            "userActionRequestId": reply.user_action_request_id,
            "workItemId": work_item_id,
            "warnings": warnings,
        }))
    }

    fn get_user_action_request(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let id = arguments
            .get("requestId")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AgentToolError::invalid("requestId is required"))?;
        let id = self.resolve_request_id(context, id)?;
        let snapshot = self.work_outline.snapshot(&context.grant.space_id);
        let request = snapshot
            .user_action_requests
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| {
                WorkOutlineError::new(
                    mast_proto::work_outline::WorkOutlineErrorCode::NotFound,
                    "Request not found",
                )
            })?;
        let origin = self.workspace.chat(&request.chat_id).ok().flatten();
        let allowed = context.admin_space.as_deref() == Some(request.space_id.as_str())
            || request.chat_id == context.grant.chat_id
            || origin.as_ref().is_some_and(|c| {
                c.parent_chat_id.as_deref() == Some(context.grant.chat_id.as_str())
            })
            || context
                .current_item_id()
                .is_some_and(|id| in_agent_scope(&snapshot, &id, &request.work_item_id));
        if !allowed {
            return Err(WorkOutlineError::unauthorized("Request is outside your scope").into());
        }
        Ok(json!({"request": request}))
    }

    fn resolve_user_action_request(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let mut args: ResolveUserActionRequestArgs = parse_args(arguments)?;
        if args.user_action_request_id.trim().is_empty() {
            return Err(AgentToolError::invalid("userActionRequestId is empty"));
        }
        args.user_action_request_id = self.resolve_request_id(context, &args.user_action_request_id)?;
        // Resolving is terminal, so the request id alone identifies the change.
        let operation_id = context.operation_id_or_derived(arguments, || {
            Ok(agent_tools::derived_idempotency_key(
                tools::RESOLVE_USER_ACTION_REQUEST,
                &[args.user_action_request_id.trim()],
            ))
        })?;
        let mutation =
            WorkOutlineMutation::ResolveUserActionRequest(ResolveUserActionRequestParams {
                outcome: args.outcome,
                values: None,
                via: Some("agent".into()),
                user_action_request_id: args.user_action_request_id.clone(),
                note: attributed_note(context, non_empty(args.resolution)),
                answer: None,
                operation_id,
            });
        self.apply(context, &mutation)?;
        // `resolved` is read back from the row, never assumed from the call
        // succeeding: a mutation that wrote nothing must not report success.
        let resolved = self
            .workspace
            .registry_row(
                mast_doc::work_outline::KIND_USER_ACTION_REQUESTS,
                &args.user_action_request_id,
            )
            .and_then(|row| {
                row.fields
                    .get("state")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .is_some_and(|state| state == "resolved");
        if !resolved {
            return Err(WorkOutlineError::new(
                mast_proto::work_outline::WorkOutlineErrorCode::OperationConflict,
                "the request is still open after the resolve; nothing was changed",
            )
            .into());
        }
        Ok(json!({ "userActionRequestId": args.user_action_request_id, "resolved": true }))
    }

    /// `answer_user_action_request`: carry the human's own chat message to a
    /// request. The caller supplies only pointers; the engine reads the
    /// message from the caller's own transcript and requires it to be a
    /// user-role message the human typed (not an agent relay, a Mast system
    /// or wake message, or an assistant reply) created after the request.
    /// Then, like Required Actions' Reply & resolve, it delivers
    /// `Re: "<request>": <message>` to the request's origin Chat as a user
    /// message and resolves the request, recording the answer (text, source
    /// Chat and message) as the user's. Delivery comes first, so a failure
    /// leaves the request open; a replay of the same key never delivers twice.
    /// `chat_project_scope_chats`: the Chats on items inside the caller's
    /// project (the nearest project at or above its item; the item itself
    /// counts). Empty without a project. Read-only reach for `read_chat`.
    fn project_scope_chats(&self, context: &CallContext) -> Value {
        let chat_ids: Vec<String> = context
            .current_item_id()
            .and_then(|root| {
                let snapshot = self.work_outline.snapshot(&context.grant.space_id);
                let project = snapshot.project_root(&root)?.id.clone();
                Some(
                    snapshot
                        .assignments
                        .iter()
                        .filter(|a| {
                            a.work_item_id
                                .as_deref()
                                .is_some_and(|item| snapshot.is_within(&project, item))
                        })
                        .map(|a| a.chat_id.clone())
                        .collect(),
                )
            })
            .unwrap_or_default();
        json!({ "chatIds": chat_ids })
    }

    fn answer_user_action_request(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let args: AnswerUserActionRequestArgs = parse_args(arguments)?;
        let request_id = self.resolve_request_id(context, &args.request_id)?;
        let message_id = args.user_message_id.trim().to_owned();
        if request_id.is_empty() {
            return Err(AgentToolError::invalid("requestId is empty"));
        }
        if message_id.is_empty() {
            return Err(AgentToolError::invalid("userMessageId is empty"));
        }
        let operation_id = context.operation_id(arguments)?;
        let ledger_key = format!("answer-user-action:{operation_id}");
        let ledger = |e: crate::EngineError| AgentToolError::invalid(e.to_string());
        let caller = &context.grant.chat_id;
        let snapshot = self.work_outline.snapshot(&context.grant.space_id);
        let request = snapshot
            .user_action_requests
            .iter()
            .find(|r| r.id == request_id)
            .ok_or_else(|| {
                WorkOutlineError::new(
                    mast_proto::work_outline::WorkOutlineErrorCode::NotFound,
                    "Request not found",
                )
            })?;
        let delivered_before = self.workspace.ledger_holds(&ledger_key);
        if !request.is_open() && !delivered_before {
            return Err(AgentToolError::invalid(
                "the request is already resolved; nothing was changed",
            ));
        }
        // Reach: the request's origin Chat is the caller or one it created, or
        // the request sits in the caller's item subtree or project scope.
        let origin_chat = (!request.chat_id.is_empty())
            .then(|| self.workspace.chat(&request.chat_id).ok().flatten())
            .flatten();
        let admin = context.admin_space.as_deref() == Some(request.space_id.as_str());
        let own_item = match context.current_item_id() {
            Some(item) => item,
            None if admin => request.work_item_id.clone(),
            None => context.require_item("answering a request")?,
        };
        let reaches = admin
            || request.chat_id == *caller
            || origin_chat
                .as_ref()
                .is_some_and(|c| c.parent_chat_id.as_deref() == Some(caller.as_str()))
            || in_agent_scope(&snapshot, &own_item, &request.work_item_id);
        if !reaches {
            return Err(WorkOutlineError::unauthorized(
                "Agents may only answer requests of their own chat, a chat they created, or \
                 items in their own item's subtree or project",
            )
            .into());
        }
        // The message, from the caller's own transcript.
        let doc_host = self.doc_host.as_ref().ok_or_else(|| {
            AgentToolError::refused(AgentToolErrorCode::Unsupported, "no chat host")
        })?;
        let entries = doc_host
            .open(caller)
            .map_err(|e| AgentToolError::invalid(e.to_string()))?
            .doc()
            .read_entries()
            .map_err(|e| AgentToolError::invalid(e.to_string()))?;
        let entry = entries.iter().find(|e| e.id == message_id).ok_or_else(|| {
            AgentToolError::invalid(
                "userMessageId is not a message in this chat (read_chat on yourself lists ids)",
            )
        })?;
        let text = match entry.role {
            mast_doc::MessageRole::User => entry
                .parts
                .iter()
                .filter_map(|part| match part {
                    mast_doc::MessagePart::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
            mast_doc::MessageRole::Assistant => {
                return Err(AgentToolError::invalid(
                    "userMessageId is an assistant message; only a message the user typed counts",
                ));
            }
            mast_doc::MessageRole::System => {
                return Err(AgentToolError::invalid(
                    "userMessageId is a system message; only a message the user typed counts",
                ));
            }
        };
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err(AgentToolError::invalid("the user message has no text"));
        }
        if mast_proto::is_relayed_or_system_message(&text) {
            return Err(AgentToolError::invalid(
                "userMessageId is a message from an agent or Mast, not one the user typed; only \
                 their own words can answer a request. Ask them to reply in Required Actions.",
            ));
        }
        if entry.created_at <= request.created_at {
            return Err(AgentToolError::invalid(
                "the user message is older than the request, so it cannot answer it",
            ));
        }
        if text.chars().count() > mast_proto::user_action_limits::MAX_NOTE_CHARS {
            return Err(AgentToolError::invalid(format!(
                "the user message is longer than {} characters; ask the user to answer in \
                 Required Actions",
                mast_proto::user_action_limits::MAX_NOTE_CHARS
            )));
        }
        // Deliver as the user's reply, then record it.
        let mut delivered = false;
        if !delivered_before && let Some(origin) = origin_chat.as_ref() {
            doc_host
                .queue_message(
                    &origin.id,
                    &mast_proto::user_reply_message(&request.text, &text),
                    Vec::new(),
                )
                .map_err(|e| AgentToolError::invalid(e.to_string()))?;
            delivered = true;
        }
        self.workspace.ledger_mark(&ledger_key).map_err(ledger)?;
        let mutation =
            WorkOutlineMutation::ResolveUserActionRequest(ResolveUserActionRequestParams {
                outcome: Some("answered".into()),
                values: None,
                via: Some("chat".into()),
                user_action_request_id: request_id.clone(),
                note: Some(text.clone()),
                answer: Some(mast_proto::UserActionAnswer {
                    chat_id: caller.clone(),
                    message_id: message_id.clone(),
                    text,
                }),
                operation_id,
            });
        self.apply(context, &mutation)?;
        let resolved = self
            .workspace
            .registry_row(
                mast_doc::work_outline::KIND_USER_ACTION_REQUESTS,
                &request_id,
            )
            .and_then(|row| {
                row.fields
                    .get("state")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .is_some_and(|state| state == "resolved");
        if !resolved {
            return Err(WorkOutlineError::new(
                mast_proto::work_outline::WorkOutlineErrorCode::OperationConflict,
                "the request is still open after the answer; nothing was changed",
            )
            .into());
        }
        Ok(json!({
            "userActionRequestId": request_id,
            "resolved": true,
            "answeredBy": "user",
            "delivered": delivered || delivered_before,
            "sourceChatId": caller,
            "sourceMessageId": message_id,
        }))
    }

    fn propose_approval_rule(&self, context: &CallContext, arguments: &Value) -> Result<Value, AgentToolError> {
        let label = arguments.get("label").and_then(Value::as_str).unwrap_or("").trim();
        if label.is_empty() || label.chars().count() > 100 || label.chars().any(char::is_control) { return Err(AgentToolError::invalid("label must contain 1 to 100 printable characters")); }
        let prefix = arguments.get("argvPrefix").cloned().unwrap_or_else(|| json!([]));
        let Some(words) = prefix.as_array() else { return Err(AgentToolError::invalid("argvPrefix must be an array of strings")); };
        if words.iter().any(|word| word.as_str().is_none_or(str::is_empty)) { return Err(AgentToolError::invalid("argvPrefix must contain nonempty strings")); }
        let regex = arguments.get("commandRegex").and_then(Value::as_str).filter(|s| !s.is_empty());
        if words.is_empty() && regex.is_none() { return Err(AgentToolError::invalid("provide argvPrefix or commandRegex")); }
        let harness = arguments.get("harness").and_then(Value::as_str).unwrap_or("any");
        if !matches!(harness, "any" | "codex" | "claude-code") { return Err(AgentToolError::invalid("invalid harness")); }
        let cwd = arguments.get("cwdScope").and_then(Value::as_str);
        if cwd.is_some_and(|cwd| !std::path::Path::new(cwd).is_absolute()) { return Err(AgentToolError::invalid("cwdScope must be absolute")); }
        let rule = json!({"label": label, "harness": harness, "argvPrefix": prefix, "commandRegex": regex, "cwdScope": cwd});
        let rationale = arguments.get("rationale").and_then(Value::as_str).unwrap_or("");
        let detail = format!("Review this proposed rule. Accept rule enables it; agents cannot enable rules.\n\n{}\n\n{}", serde_json::to_string_pretty(&rule).unwrap_or_default(), rationale);
        let request = json!({"request": format!("Review proposed approval rule: {label}"), "detail": detail, "kind": "approve", "idempotencyKey": arguments.get("idempotencyKey")});
        self.create_user_action_request(context, &request)
    }

    fn propose_instruction(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let args: ProposeInstructionArgs = parse_args(arguments)?;
        let operation_id = context.operation_id(arguments)?;
        let action = match args.action.as_deref().map(str::trim).unwrap_or("add") {
            "add" | "" => ProposalAction::Add,
            "edit" => ProposalAction::Edit,
            "remove" => ProposalAction::Remove,
            other => {
                return Err(AgentToolError::invalid(format!(
                    "action must be add, edit, or remove, not {other:?}"
                )));
            }
        };
        let instruction_id = non_empty(args.instruction_id);
        match action {
            ProposalAction::Add => {
                if args.text.trim().is_empty() {
                    return Err(AgentToolError::invalid("text is empty"));
                }
                if instruction_id.is_some() {
                    return Err(AgentToolError::invalid(
                        "instructionId is only for action edit or remove",
                    ));
                }
            }
            ProposalAction::Edit | ProposalAction::Remove => {
                if instruction_id.is_none() {
                    return Err(AgentToolError::invalid(
                        "instructionId is required for action edit or remove (see work_outline_read_current)",
                    ));
                }
                if action == ProposalAction::Edit && args.text.trim().is_empty() {
                    return Err(AgentToolError::invalid("text is empty"));
                }
                if action == ProposalAction::Remove && !args.text.is_empty() {
                    return Err(AgentToolError::invalid(
                        "a remove carries no text; give the reason in rationale",
                    ));
                }
            }
        }
        let item = context.require_item("proposing instructions")?;
        let target = match args.target.as_deref().map(str::trim).unwrap_or("item") {
            "item" => InstructionProposalTarget::work_item(item),
            "spaceRoot" => InstructionProposalTarget::space_root(),
            "childTemplate" => InstructionProposalTarget {
                scope: mast_proto::work_outline_user_actions::ProposalTargetScope::ChildTemplate,
                work_item_id: Some(item.clone()),
                container_work_item_id: non_empty(args.container_work_item_id).map(|id| self.resolve_work_item_id(context, &id)).transpose()?,
                rule_node_id: non_empty(args.rule_node_id),
                rule_instruction_id: None,
                rule_node_path: args.rule_node_path,
            },
            "parent" => match self
                .workspace
                .effective_work_item_parent(&context.grant.space_id, &item)
            {
                Some(parent) => InstructionProposalTarget::work_item(parent),
                None => {
                    return Err(WorkOutlineError::unauthorized(
                        "This Chat's work item has no parent; propose for the item or the \
                         Space root",
                    )
                    .into());
                }
            },
            other => {
                return Err(AgentToolError::invalid(format!(
                    "target must be item, parent, spaceRoot, or childTemplate, not {other:?}"
                )));
            }
        };
        let mutation =
            WorkOutlineMutation::ProposeWorkItemInstruction(ProposeWorkItemInstructionParams {
                space_id: Some(context.grant.space_id.clone()),
                target: target.clone(),
                origin: InstructionProposalOrigin {
                    chat_id: Some(context.grant.chat_id.clone()),
                    message_id: non_empty(args.message_id),
                    tool_call_id: non_empty(args.tool_call_id),
                },
                text: args.text,
                action,
                instruction_id: instruction_id.clone(),
                rationale: non_empty(args.rationale),
                operation_id,
            });
        let reply = self.apply(context, &mutation)?;
        let snapshot = self.work_outline.snapshot(&context.grant.space_id);
        let resolved_target = snapshot.instruction_proposals.iter()
            .find(|proposal| Some(&proposal.id) == reply.proposal_id.as_ref())
            .map(|proposal| &proposal.target)
            .ok_or_else(|| AgentToolError::invalid("The proposed instruction could not be read back"))?;
        Ok(json!({
            "proposalId": reply.proposal_id,
            "action": action.as_str(),
            "instructionId": instruction_id,
            "target": resolved_target,
            "state": "pending",
            "note": "The user reviews the proposal; nothing changes until they accept it.",
        }))
    }

    /// Bind a `create_chat` idempotency key to its inputs: the first claim
    /// records the inputs hash durably; a later claim with the same hash is a
    /// replay, with another hash `operationConflict` (so a retry that changed
    /// its request never silently gets the old Chat back). A Chat created
    /// before claims existed adopts the hash of its first claim.
    fn claim_child_chat(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let operation_id = context.operation_id(arguments)?;
        let args: ClaimChildArgs = parse_args(arguments)?;
        let hash = args.inputs_hash.trim();
        if hash.is_empty() || hash.len() > 128 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(AgentToolError::invalid("inputsHash must be a hex digest"));
        }
        let (operation_key, inputs_key) = child_chat_ledger_keys(&operation_id, hash);
        let ledger = |e: crate::EngineError| AgentToolError::invalid(e.to_string());
        let _serial = lock(&self.child_claims);
        if self.workspace.ledger_holds(&operation_key) {
            if self.workspace.ledger_holds(&inputs_key) {
                return Ok(json!({ "replayed": true }));
            }
            return Err(WorkOutlineError::new(
                mast_proto::work_outline::WorkOutlineErrorCode::OperationConflict,
                "idempotency_key was already used to create a chat with different inputs; \
                 use a new key for a new chat",
            )
            .into());
        }
        // Inputs first: a crash between the two marks leaves the operation
        // unclaimed, and the retry claims both.
        self.workspace.ledger_mark(&inputs_key).map_err(ledger)?;
        self.workspace.ledger_mark(&operation_key).map_err(ledger)?;
        Ok(json!({ "replayed": false }))
    }

    /// Queue the first Turn of a child Chat created by the MCP `create_chat`
    /// tool. Every id derives from `(token Chat, idempotencyKey)`, so a retry
    /// (or a second call with the same key) never queues a second Turn; the
    /// Chat must be the one that key names and a child of the token's Chat.
    /// The Run request is rebuilt here from the child's stored config (the
    /// caller supplies only the prompt and attachments), so an agent can
    /// never widen the child's sandbox, approval mode, cwd, or harness.
    fn queue_child_chat_turn(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let operation_id = context.operation_id(arguments)?;
        let args: QueueChildTurnArgs = parse_args(arguments)?;
        let chat_id = derive_id(&operation_id, roles::CHAT);
        if args.chat_id != chat_id {
            return Err(AgentToolError::refused(
                AgentToolErrorCode::Forbidden,
                "chatId is not the Chat this idempotencyKey created",
            ));
        }
        let chat = self
            .workspace
            .chat(&chat_id)
            .ok()
            .flatten()
            .ok_or_else(|| AgentToolError::invalid("the child Chat does not exist yet"))?;
        if chat.parent_chat_id.as_deref() != Some(context.grant.chat_id.as_str()) {
            return Err(AgentToolError::refused(
                AgentToolErrorCode::Forbidden,
                "agents may only start Chats they created",
            ));
        }
        if args.prompt.trim().is_empty() {
            return Err(AgentToolError::invalid("prompt is empty"));
        }
        if args.attachments.len() > MAX_CHILD_ATTACHMENTS
            || args
                .attachments
                .iter()
                .any(|path| !std::path::Path::new(path).is_absolute())
        {
            return Err(AgentToolError::invalid(format!(
                "attachments must be at most {MAX_CHILD_ATTACHMENTS} absolute paths"
            )));
        }
        let space = chat
            .space_id
            .as_deref()
            .and_then(|id| self.workspace.space(id).ok().flatten());
        let request = child_run_request(&chat, space.as_ref(), args.prompt, args.attachments);
        let message_id = derive_id(&operation_id, roles::MESSAGE);
        let command_id = derive_id(&operation_id, roles::COMMAND);
        let payload = SessionCommandPayload::Run {
            request,
            message_id: message_id.clone(),
        };
        let doc_host = self.doc_host.as_ref().ok_or_else(|| {
            AgentToolError::refused(AgentToolErrorCode::Unsupported, "no chat host")
        })?;
        let queued = doc_host
            .queue_command_with_id(&chat_id, &command_id, payload)
            .map_err(|e| AgentToolError::invalid(e.to_string()))?;
        Ok(json!({
            "chatId": chat_id,
            "messageId": message_id,
            "commandId": command_id,
            "queued": queued,
        }))
    }
}

impl AgentToolsService {
    /// `work_outline_read_subtree`: the caller's item (or a descendant) with
    /// its descendants, each with its attached Chats and open user action
    /// requests, from ONE snapshot. Read-only; the target must be the
    /// caller's item or inside its subtree.
    fn read_subtree(
        &self,
        context: &CallContext,
        arguments: &Value,
    ) -> Result<Value, AgentToolError> {
        let args: ReadSubtreeArgs = parse_args(arguments)?;
        let target_arg = non_empty(args.work_item_id).map(|id| self.resolve_work_item_id(context, &id)).transpose()?;
        let root = match (context.admin_space.is_some(), context.current_item_id()) {
            (_, Some(item)) => item,
            (true, None) => target_arg.clone().ok_or_else(|| {
                AgentToolError::invalid("pass workItemId: this chat has no item of its own")
            })?,
            (false, None) => context.require_item("reading its subtree")?,
        };
        let target = target_arg.unwrap_or_else(|| root.clone());
        let max_depth = args
            .max_depth
            .unwrap_or(DEFAULT_SUBTREE_DEPTH)
            .min(MAX_SUBTREE_DEPTH);
        let snapshot = self.work_outline.snapshot(&context.grant.space_id);
        if snapshot.item(&target).is_none()
            || (context.admin_space.is_none() && !in_agent_scope(&snapshot, &root, &target))
        {
            return Err(WorkOutlineError::unauthorized(
                "Agents may only read their own work item, its descendants, and their project",
            )
            .into());
        }
        let sessions = lock(&self.sessions).clone();
        let mut requests: HashMap<&str, Vec<&mast_proto::UserActionRequestView>> = HashMap::new();
        for request in snapshot
            .user_action_requests
            .iter()
            .filter(|r| r.is_unresolved())
        {
            requests
                .entry(request.work_item_id.as_str())
                .or_default()
                .push(request);
        }
        let walk = SubtreeWalk {
            snapshot: &snapshot,
            workspace: &self.workspace,
            sessions: sessions.as_ref(),
            requests: &requests,
            include_archived: args.include_archived,
            include_completed: args.include_completed,
            max_depth,
        };
        let mut count = 0usize;
        let mut truncated = false;
        let tree = walk
            .node(&target, 0, &mut count, &mut truncated)
            .unwrap_or(Value::Null);
        Ok(json!({
            "chatId": context.grant.chat_id,
            "rootWorkItemId": root,
            "workItemId": target,
            "maxDepth": max_depth,
            "includeArchived": args.include_archived,
            "includeCompleted": args.include_completed,
            "itemCount": count,
            "truncated": truncated,
            "tree": tree,
        }))
    }

    /// `chat_launch_child_for_item` (engine-internal): authorize and shape
    /// the launch of a child Chat for a work item. `agent_tools` has no async
    /// launch service, so the RPC layer runs the returned params through the
    /// same `CreateChatForWorkItem` workflow the UI's Start Chat uses.
    ///
    /// The target must be the caller's item or a descendant (else
    /// `work-outline/unauthorized`), the child's sandbox may not exceed the
    /// caller's, an Auto child needs an Auto caller, and the new Chat records the
    /// caller as its parent. Ids derive from `(token Chat, idempotencyKey)`.
    pub fn prepare_child_launch(
        &self,
        params: &AgentToolCallParams,
    ) -> Result<mast_proto::CreateChatForWorkItemParams, AgentToolError> {
        let context = self.context(&params.token)?;
        let operation_id = context.operation_id(&params.arguments)?;
        let mut args: LaunchChildArgs = parse_args(&params.arguments)?;
        args.work_item_id = self.resolve_work_item_id(&context, &args.work_item_id)?;
        if args.prompt.trim().is_empty() {
            return Err(AgentToolError::invalid(
                "a prompt is required to start a chat for a work item",
            ));
        }
        let root = match context.current_item_id() {
            Some(item) => item,
            None if context.admin_space.is_some() => args.work_item_id.clone(),
            None => context.require_item("starting a chat for a work item")?,
        };
        let snapshot = self.work_outline.snapshot(&context.grant.space_id);
        if snapshot.item(&args.work_item_id).is_none()
            || (context.admin_space.is_none()
                && !in_agent_scope(&snapshot, &root, &args.work_item_id))
        {
            return Err(WorkOutlineError::unauthorized(
                "Agents may only start chats for their own work item, its descendants, and \
                 their project",
            )
            .into());
        }
        // The project itself stands for its worker destination.
        let mut args = args;
        if let Some(item) = snapshot.item(&args.work_item_id)
            && item.is_project
            && let Some(destination) = snapshot.worker_destination_for(&args.work_item_id)
            && !snapshot.is_within(destination, &root)
        {
            args.work_item_id = destination.to_owned();
        }
        let caller = self
            .workspace
            .chat(&context.grant.chat_id)
            .ok()
            .flatten()
            .ok_or_else(AgentToolError::unauthenticated)?;
        let config = args.config;
        if let Some(config) = &config {
            check_child_config(caller.config.as_ref(), config)?;
            if config.auto_approve {
                tracing::info!(
                    parent_chat_id = %context.grant.chat_id,
                    work_item_id = %args.work_item_id,
                    "agent started an Auto child chat"
                );
            }
        }
        Ok(mast_proto::CreateChatForWorkItemParams {
            work_item_id: args.work_item_id,
            initial_prompt: mast_proto::WorkItemPrompt {
                text: args.prompt,
                attachments: Vec::new(),
            },
            config,
            execution_override: None,
            worktree_choice: None,
            operation_id,
            parent_chat_id: Some(context.grant.chat_id.clone()),
            target_device_id: None,
            account_id: None,
        })
    }

    /// Authorize a successor launch using the caller's own item and configuration.
    pub fn prepare_orchestrator_successor(
        &self,
        params: &AgentToolCallParams,
    ) -> Result<(mast_proto::CreateChatForWorkItemParams, String, Option<mast_proto::HarnessId>, Option<String>, Option<mast_proto::ReasoningLevel>), AgentToolError> {
        let context = self.context(&params.token)?;
        let operation_id = context.operation_id(&params.arguments)?;
        let summary = params.arguments.get("summary").and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AgentToolError::invalid("summary is required"))?;
        let work_item_id = context.require_item("handing off orchestration")?;
        let caller = self.workspace.chat(&context.grant.chat_id).ok().flatten()
            .ok_or_else(AgentToolError::unauthenticated)?;
        if caller.archived {
            return Err(AgentToolError::invalid("an archived chat cannot hand off orchestration"));
        }
        let source = context.grant.chat_id.clone();
        #[derive(serde::Deserialize)]
        struct Selection {
            harness: Option<mast_proto::HarnessId>,
            model: Option<String>,
            reasoning: Option<mast_proto::ReasoningLevel>,
        }
        let selection: Selection = serde_json::from_value(params.arguments.clone())
            .map_err(|error| AgentToolError::invalid(format!("invalid hand-off selection: {error}")))?;
        Ok((mast_proto::CreateChatForWorkItemParams {
            work_item_id,
            initial_prompt: mast_proto::WorkItemPrompt { text: summary.to_owned(), attachments: Vec::new() },
            config: caller.config,
            execution_override: None,
            worktree_choice: None,
            operation_id,
            parent_chat_id: None,
            target_device_id: None,
            account_id: None,
        }, source, selection.harness, selection.model, selection.reasoning))
    }
}

/// One `work_outline_read_subtree` walk.
struct SubtreeWalk<'a> {
    snapshot: &'a mast_proto::WorkOutlineSnapshot,
    workspace: &'a WorkspaceHost,
    sessions: Option<&'a SessionsEngine>,
    requests: &'a HashMap<&'a str, Vec<&'a mast_proto::UserActionRequestView>>,
    include_archived: bool,
    include_completed: bool,
    max_depth: u32,
}

impl SubtreeWalk<'_> {
    /// Keep completed ancestors when their visible subtree contains work needing attention.
    fn has_attention(&self, item: &mast_proto::WorkItemView) -> bool {
        if self.requests.contains_key(item.id.as_str()) {
            return true;
        }
        if item.chat_ids.iter().any(|id| {
            self.workspace.chat(id).ok().flatten().is_some_and(|chat| {
                !chat.archived && self.sessions.and_then(|s| s.session_status(id)).is_some_and(|session| {
                    matches!(session.status, mast_proto::SessionStatus::Working | mast_proto::SessionStatus::AwaitingInput)
                })
            })
        }) {
            return true;
        }
        item.child_ids.iter().filter_map(|id| self.snapshot.item(id))
            .filter(|child| self.include_archived || child.archived_at.is_none())
            .any(|child| self.has_attention(child))
    }

    /// The nested node for `id` (depth 0 = the read's root), depth-first in
    /// outline order; `None` once [`MAX_SUBTREE_ITEMS`] is reached (which
    /// also sets `truncated`).
    fn node(&self, id: &str, depth: u32, count: &mut usize, truncated: &mut bool) -> Option<Value> {
        let item = self.snapshot.item(id)?;
        if *count >= MAX_SUBTREE_ITEMS {
            *truncated = true;
            return None;
        }
        *count += 1;
        let mut completed_omitted = 0usize;
        let visible_children: Vec<&str> = item
            .child_ids
            .iter()
            .filter_map(|child| self.snapshot.item(child))
            .filter(|child| self.include_archived || child.archived_at.is_none())
            .filter(|child| {
                let omit = !self.include_completed
                    && child.effective_status == mast_proto::WorkItemEffectiveStatus::Completed
                    && !self.has_attention(child);
                completed_omitted += usize::from(omit);
                !omit
            })
            .map(|child| child.id.as_str())
            .collect();
        let mut children = Vec::new();
        let mut omitted = 0usize;
        if depth < self.max_depth {
            for child in &visible_children {
                match self.node(child, depth + 1, count, truncated) {
                    Some(node) => children.push(node),
                    None => omitted += 1,
                }
            }
        } else {
            omitted = visible_children.len();
        }
        let chats = self.chats_of(item);
        let open = self.requests.get(id).map(Vec::as_slice).unwrap_or_default();
        let mut node = json!({
            "id": item.id,
            "title": item.title,
            "status": item.status,
            "effectiveStatus": item.effective_status,
            "waitingUntil": item.waiting_until,
            "waitingReason": item.waiting_reason,
            "chats": chats.0,
            "openUserActionRequests": open
                .iter()
                .take(MAX_SUBTREE_ITEM_ROWS)
                .map(|r| json!({
                    "id": r.id,
                    "links": r.links,
                    "facts": r.facts,
                    "options": r.options,
                    "text": r.text,
                    "detail": r.info.as_deref().map(|d| d.chars().take(MAX_SUBTREE_DETAIL_CHARS).collect::<String>()),
                    "detailTruncated": r.info.as_deref().is_some_and(|d| d.chars().count() > MAX_SUBTREE_DETAIL_CHARS),
                    "originChatId": r.chat_id,
                    "createdAt": r.created_at,
                }))
                .collect::<Vec<_>>(),
            "children": children,
            "completedOmitted": completed_omitted,
        });
        if item.archived_at.is_some() {
            node["archived"] = json!(true);
        }
        if chats.1 > 0 {
            node["chatsOmitted"] = json!(chats.1);
        }
        if open.len() > MAX_SUBTREE_ITEM_ROWS {
            node["openUserActionRequestsOmitted"] = json!(open.len() - MAX_SUBTREE_ITEM_ROWS);
        }
        if omitted > 0 {
            node["childrenOmitted"] = json!(omitted);
        }
        Some(node)
    }

    /// The item's attached Chats (id, title, status, lastMessageAt) and how
    /// many were left out.
    fn chats_of(&self, item: &mast_proto::WorkItemView) -> (Vec<Value>, usize) {
        let mut chats = Vec::new();
        let mut omitted = 0usize;
        for chat_id in &item.chat_ids {
            let Some(chat) = self.workspace.chat(chat_id).ok().flatten() else {
                continue;
            };
            if chat.archived && !self.include_archived {
                continue;
            }
            if chats.len() >= MAX_SUBTREE_ITEM_ROWS {
                omitted += 1;
                continue;
            }
            let session = self.sessions.and_then(|s| s.session_status(chat_id));
            let status = match &session {
                Some(session) => match session.status {
                    mast_proto::SessionStatus::Working => "working",
                    mast_proto::SessionStatus::AwaitingInput => "awaitingInput",
                    mast_proto::SessionStatus::Errored => "errored",
                    mast_proto::SessionStatus::Idle => "idle",
                },
                None => "idle",
            };
            let mut value = json!({
                "id": chat.id,
                "title": chat.title,
                "status": status,
                "awaitingInput": session.as_ref().and_then(|s| s.awaiting_input.as_ref()),
                "lastTurnToolFailures": session.as_ref().and_then(|s| s.last_turn_tool_failures.as_ref()),
                "lastMessageAt": chat.last_message_at,
            });
            if chat.archived {
                value["archived"] = json!(true);
            }
            chats.push(value);
        }
        (chats, omitted)
    }
}

/// The first Run of a child Chat, from the child's stored config and row
/// only: what a composer send in that Chat would queue (its harness, model,
/// sandbox, approval mode, and cwd; the host overwrites the cwd of an
/// execution-bound Chat and resolves instructions at dispatch).
fn child_run_request(
    chat: &mast_proto::Chat,
    space: Option<&mast_proto::Space>,
    prompt: String,
    attachments: Vec<String>,
) -> RunRequest {
    let config = chat.config.as_ref();
    RunRequest {
        prompt,
        harness: config.map(|c| c.harness),
        model: config.and_then(|c| c.model.clone()),
        reasoning: config.and_then(|c| c.reasoning),
        model_options: config.map(|c| c.model_options.clone()).unwrap_or_default(),
        cwd: chat
            .cwd
            .clone()
            .or_else(|| space.map(|s| s.path.clone()))
            .unwrap_or_else(|| "~".into()),
        sandbox: config
            .map(|c| c.sandbox)
            .unwrap_or(SandboxLevel::WorkspaceWrite),
        auto_approve: config.is_some_and(|c| c.auto_approve),
        resume: None,
        attachments,
        worktree: None,
        instructions: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_require_unique_prefixes() {
        let ids = ["abcdef111", "abcdef222", "123456789"];
        assert_eq!(resolve_id("123456", ids.into_iter(), "request").unwrap(), "123456789");
        let error = resolve_id("abcdef", ids.into_iter(), "request").unwrap_err().to_wire_message();
        assert!(error.contains("ambiguous: matches abcdef111, abcdef222"), "{error}");
        assert!(resolve_id("abcde", ids.into_iter(), "request").is_err());
    }

    fn arc() -> Arc<AgentTokens> {
        AgentTokens::new()
    }

    #[test]
    fn tokens_are_random_bound_and_revoked_with_their_lease() {
        let tokens = arc();
        let lease = tokens.mint("chat-1", "run-1", "space-1", "dev-1");
        let other = tokens.mint("chat-1", "run-2", "space-1", "dev-1");
        assert_ne!(lease.token(), other.token());
        assert!(lease.token().len() >= 64);
        let grant = tokens.authenticate(lease.token()).unwrap();
        assert_eq!(
            (
                grant.chat_id.as_str(),
                grant.run_id.as_str(),
                grant.space_id.as_str()
            ),
            ("chat-1", "run-1", "space-1")
        );
        let token = lease.token().to_owned();
        drop(lease);
        assert_eq!(
            tokens.authenticate(&token).unwrap_err().code(),
            "unauthenticated"
        );
        assert!(tokens.authenticate(other.token()).is_ok());
        assert_eq!(tokens.live_count(), 1);
        drop(other);
        assert_eq!(tokens.live_count(), 0);
    }

    #[test]
    fn forged_absent_and_expired_tokens_are_rejected() {
        let tokens = arc();
        let lease = tokens.mint("chat-1", "run-1", "space-1", "dev-1");
        for forged in ["", "   ", "zat_00000000", &format!("{}x", lease.token())] {
            let err = tokens.authenticate(forged).unwrap_err();
            assert_eq!(err.code(), "unauthenticated", "{forged:?}");
            assert!(
                err.to_wire_message()
                    .starts_with("agent-tools/unauthenticated: ")
            );
        }
        let short =
            tokens.mint_with_ttl("chat-1", "run-2", "space-1", "dev-1", Some(Duration::ZERO));
        assert!(tokens.authenticate(short.token()).is_err());
        let long = tokens.mint_with_ttl(
            "chat-1",
            "run-3",
            "space-1",
            "dev-1",
            Some(Duration::from_secs(3600)),
        );
        assert!(tokens.authenticate(long.token()).is_ok());
    }

    #[test]
    fn a_lease_outliving_its_registry_is_harmless() {
        let tokens = arc();
        let lease = tokens.mint("c", "r", "s", "d");
        drop(tokens);
        drop(lease);
    }

    #[test]
    fn injection_carries_the_token_in_env_only() {
        let launch = AgentToolsLaunch {
            command: PathBuf::from("/bin/zeron"),
            args: vec!["mcp".into()],
            ipc_port: Some(27655),
        };
        let injection = launch.injection("zat_secret").unwrap();
        assert_eq!(injection.server_name, "mast-agent");
        assert_eq!(injection.args, vec!["mcp".to_owned()]);
        assert_eq!(injection.env["MAST_AGENT_TOKEN"], "zat_secret");
        assert_eq!(injection.env["MAST_IPC_PORT"], "27655");
        // An older `zeron mcp` binary reads the old names; both are exported.
        assert_eq!(injection.env["ZERON_AGENT_TOKEN"], "zat_secret");
        assert_eq!(injection.env["ZERON_IPC_PORT"], "27655");
        assert!(mast_proto::agent_tools::is_server_name("zeron-agent"));
        assert!(mast_proto::agent_tools::is_server_name("mast-agent"));
        assert!(!mast_proto::agent_tools::is_server_name("zeron"));
        assert!(!injection.args.iter().any(|a| a.contains("zat_secret")));
        assert!(!injection.command.to_string_lossy().contains("zat_secret"));
    }

    #[test]
    fn default_launch_dials_the_bound_port_or_injects_nothing() {
        let launch = AgentToolsLaunch {
            command: PathBuf::from("/bin/zeron"),
            args: vec!["mcp".into()],
            ipc_port: None,
        };
        // Process-global: this is the only unit test that touches it.
        note_bound_ipc_port(0);
        assert!(
            launch.injection("zat_x").is_none(),
            "no bound port, no tools"
        );
        note_bound_ipc_port(41234);
        assert_eq!(
            launch.injection("zat_x").unwrap().env["MAST_IPC_PORT"],
            "41234"
        );
        note_bound_ipc_port(0);
    }

    #[test]
    fn minted_tokens_expire_absolutely_unless_renewed() {
        let tokens = arc();
        let lease = tokens.mint("c", "r", "s", "d");
        let grant = tokens.authenticate(lease.token()).unwrap();
        assert_eq!(grant.ttl, Some(DEFAULT_TOKEN_TTL));
        let at = grant.expires_at.expect("an absolute expiry");
        assert!(at > Instant::now() + DEFAULT_TOKEN_TTL - Duration::from_secs(60));

        // Renewal pushes a live token's expiry out by its TTL.
        let short = tokens.mint_with_ttl("c", "r2", "s", "d", Some(Duration::from_millis(200)));
        std::thread::sleep(Duration::from_millis(120));
        short.renew();
        std::thread::sleep(Duration::from_millis(120));
        assert!(
            tokens.authenticate(short.token()).is_ok(),
            "renewed in time"
        );
        // A driver that stops renewing loses the token though it holds the lease.
        std::thread::sleep(Duration::from_millis(250));
        assert!(tokens.authenticate(short.token()).is_err());
        short.renew();
        assert!(tokens.authenticate(short.token()).is_err(), "never revived");
    }

    fn cfg(sandbox: SandboxLevel, auto: bool) -> mast_proto::ChatConfig {
        serde_json::from_value(json!({
            "harness": "codex", "model": "m", "sandbox": sandbox, "autoApprove": auto,
        }))
        .unwrap()
    }

    #[test]
    fn auto_children_need_an_auto_parent() {
        use SandboxLevel::*;
        let auto_parent = cfg(WorkspaceWrite, true);
        assert!(check_child_config(Some(&auto_parent), &cfg(WorkspaceWrite, true)).is_ok());
        assert!(check_child_config(Some(&auto_parent), &cfg(DangerFullAccess, true)).is_ok());
        // An Ask parent is refused, even with a full-access sandbox.
        for parent in [cfg(WorkspaceWrite, false), cfg(DangerFullAccess, false)] {
            let err = check_child_config(Some(&parent), &cfg(WorkspaceWrite, true)).unwrap_err();
            assert!(
                format!("{err:?}").contains("autoApprove refused"),
                "{err:?}"
            );
        }
        assert!(check_child_config(None, &cfg(WorkspaceWrite, true)).is_err());
        // Omitted (Ask) stays allowed under either parent.
        assert!(check_child_config(Some(&auto_parent), &cfg(WorkspaceWrite, false)).is_ok());
        let ask = cfg(WorkspaceWrite, false);
        assert!(check_child_config(Some(&ask), &cfg(WorkspaceWrite, false)).is_ok());
        assert!(check_child_config(Some(&ask), &cfg(DangerFullAccess, false)).is_err());
        // Read-only children stay read-only and never count as full access.
        assert_eq!(effective_rank(ReadOnly, true), sandbox_rank(ReadOnly));
        let ro_auto = cfg(ReadOnly, true);
        assert!(check_child_config(Some(&ro_auto), &cfg(ReadOnly, true)).is_ok());
        assert!(check_child_config(Some(&ro_auto), &cfg(WorkspaceWrite, true)).is_err());
    }

    #[test]
    fn child_runs_take_only_prompt_and_attachments_from_the_caller() {
        let chat: mast_proto::Chat = serde_json::from_value(json!({
            "id": "child", "deviceId": "dev", "archived": false,
            "createdAt": "2026-01-01T00:00:00Z",
            "config": { "harness": "codex", "model": "m-1", "sandbox": "read-only",
                        "autoApprove": false },
        }))
        .unwrap();
        let space: mast_proto::Space = serde_json::from_value(json!({
            "id": "s", "deviceId": "dev", "path": "/repo", "createdAt": "2026-01-01T00:00:00Z",
        }))
        .unwrap();
        let request =
            child_run_request(&chat, Some(&space), "go".into(), vec!["/tmp/a.png".into()]);
        assert_eq!(request.prompt, "go");
        assert_eq!(request.attachments, vec!["/tmp/a.png".to_owned()]);
        assert_eq!(request.harness, Some(mast_proto::HarnessId::Codex));
        assert_eq!(request.model.as_deref(), Some("m-1"));
        assert_eq!(request.sandbox, SandboxLevel::ReadOnly);
        assert!(!request.auto_approve);
        assert_eq!(request.cwd, "/repo");
        assert!(request.worktree.is_none() && request.instructions.is_none());
    }
}
