//! Work Outline agent tools (Milestone 4) against a real offline engine:
//! - a Run of a Chat in a Space gets the Mast MCP injection with a token
//!   bound to that Chat and Run, revoked when the Run ends; a projectless
//!   Chat's Run gets plain `run()` and no token;
//! - `AgentToolCall` authenticates by token only (forged / absent / revoked
//!   tokens are refused) and enforces the agent policy per tool;
//! - mutations run as `Actor { kind: agent, chatId }` and dedupe by
//!   `(token Chat, idempotencyKey)` across Runs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::{Value, json};
use tokio::sync::Notify;

use mast_doc::SessionCommandPayload;
use mast_engine::agent_tools::AgentToolsLaunch;
use mast_engine::diff_targets::DiffTargets;
use mast_engine::{EngineCore, HarnessRegistry};
use mast_harness::{AgentToolsInjection, Harness, HarnessError, RunControls};
use mast_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SteeringMode, WorkOutlineActorKind, WorkOutlineSnapshot, derive_id, roles,
};
use mast_rpc::{RpcClient, methods};

const SPACE: &str = "space-1";
const CHAT: &str = "chat-agent";
const OTHER_CHAT: &str = "chat-other";
const PROJECTLESS: &str = "chat-projectless";

#[derive(Debug, Clone)]
enum Spawn {
    Plain {
        prompt: String,
    },
    WithTools {
        prompt: String,
        tools: AgentToolsInjection,
    },
}

/// Records how each Run was spawned and holds the Run open until released.
#[derive(Clone, Default)]
struct GatedHarness {
    spawns: Arc<Mutex<Vec<Spawn>>>,
    /// Every Run request as the harness received it.
    requests: Arc<Mutex<Vec<RunRequest>>>,
    release: Arc<Notify>,
    questions: Arc<Mutex<Vec<mast_proto::UserInputQuestion>>>,
    events: Arc<Mutex<Vec<AgentEvent>>>,
    fake_codex: bool,
    answers: Arc<Mutex<Vec<Vec<mast_proto::UserInputAnswer>>>>,
}

impl GatedHarness {
    fn stream(&self, request: &RunRequest) -> BoxStream<'static, Result<AgentEvent, HarnessError>> {
        let release = self.release.clone();
        let cwd = request.cwd.clone();
        let started = futures::stream::iter(vec![Ok(AgentEvent::SessionStarted {
            harness: HarnessId::Mock,
            model: "mock".into(),
            tools: vec![],
            cwd,
            session_id: "sess-agent".into(),
            assistant_message_id: "a-1".into(),
        })]);
        let done = futures::stream::once(async move {
            release.notified().await;
            Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: Some("sess-agent".into()),
            })
        });
        let events = futures::stream::iter(self.events.lock().unwrap().clone().into_iter().map(Ok));
        started.chain(events).chain(done).boxed()
    }
}

#[async_trait]
impl Harness for GatedHarness {
    fn id(&self) -> HarnessId {
        if self.fake_codex { HarnessId::Codex } else { HarnessId::Mock }
    }
    fn display_name(&self) -> &str {
        "Gated"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[ReasoningLevel::Medium]
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    fn supports_agent_tools(&self) -> bool {
        true
    }
    fn instruction_support(&self) -> mast_harness::InstructionSupport {
        mast_harness::InstructionSupport::Supported {
            channel: mast_harness::InstructionChannel::Test,
            replace_strategy: mast_harness::InstructionReplaceStrategy::RespawnResume,
            max_bytes: 64 * 1024,
        }
    }
    async fn run(
        &self,
        request: RunRequest,
        _controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.spawns.lock().unwrap().push(Spawn::Plain {
            prompt: request.prompt.clone(),
        });
        self.requests.lock().unwrap().push(request.clone());
        Ok(self.stream(&request))
    }
    async fn run_with_agent_tools(
        &self,
        request: RunRequest,
        controls: RunControls,
        tools: AgentToolsInjection,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        let questions = self.questions.lock().unwrap().clone();
        if !questions.is_empty() {
            let answer = (controls.request_input)(questions);
            let answers = self.answers.clone();
            tokio::spawn(async move { if let Ok(answer) = answer.await { answers.lock().unwrap().push(answer); } });
        }
        self.spawns.lock().unwrap().push(Spawn::WithTools {
            prompt: request.prompt.clone(),
            tools,
        });
        self.requests.lock().unwrap().push(request.clone());
        Ok(self.stream(&request))
    }
}

struct Rig {
    _tmp: tempfile::TempDir,
    core: EngineCore,
    client: RpcClient,
    harness: GatedHarness,
    space_path: String,
    /// The service behind `client` (its agent archive log lives here).
    service: Arc<mast_engine::EngineRpc>,
}

async fn rig() -> Rig {
    rig_for(false).await
}

async fn rig_for(fake_codex: bool) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let space_path = root.join("project");
    std::fs::create_dir_all(&space_path).unwrap();
    let harness = GatedHarness { fake_codex, ..Default::default() };
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(harness.clone()));
    let core = EngineCore::assemble(
        &root.join("data"),
        Arc::new(registry),
        HarnessId::Mock,
        None,
    )
    .expect("engine core assembles");
    // No IPC port is served here: name one explicitly (the default dials the
    // port the engine bound, and injects nothing without one).
    core.sessions.set_agent_tools_launch(Some(AgentToolsLaunch {
        command: "/opt/zeron/bin/zeron".into(),
        args: vec!["mcp".into()],
        ipc_port: Some(27999),
    }));
    let service = core.rpc_service();
    let client = mast_rpc::memory_client(service.clone());
    let space_path = space_path.to_string_lossy().into_owned();
    client
        .call(
            methods::MUTATE,
            json!({ "op": "createSpace", "spaceId": SPACE, "deviceId": core.device_id,
                    "path": space_path }),
        )
        .await
        .unwrap();
    for chat in [CHAT, OTHER_CHAT] {
        client
            .call(
                methods::MUTATE,
                json!({ "op": "createChat", "chatId": chat, "spaceId": SPACE,
                        "deviceId": core.device_id }),
            )
            .await
            .unwrap();
    }
    client
        .call(
            methods::MUTATE,
            json!({ "op": "createChat", "chatId": PROJECTLESS, "deviceId": core.device_id }),
        )
        .await
        .unwrap();
    Rig {
        _tmp: tmp,
        core,
        client,
        harness,
        space_path,
        service,
    }
}

async fn wait_for<F: FnMut() -> bool>(mut predicate: F, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

fn run_command(prompt: &str, cwd: &str) -> SessionCommandPayload {
    SessionCommandPayload::Run {
        request: RunRequest {
            prompt: prompt.into(),
            harness: Some(HarnessId::Mock),
            model: None,
            reasoning: None,
            model_options: Default::default(),
            cwd: cwd.into(),
            sandbox: SandboxLevel::WorkspaceWrite,
            auto_approve: true,
            resume: None,
            attachments: Vec::new(),
            worktree: None,
            instructions: None,
        },
        message_id: uuid::Uuid::new_v4().to_string(),
    }
}

async fn create_item(rig: &Rig, op: &str, title: &str, parent: Option<&str>) -> String {
    let mut params = json!({ "op": "createWorkItem", "spaceId": SPACE, "title": title,
                             "operationId": op });
    if let Some(parent) = parent {
        params["parentWorkItemId"] = json!(parent);
    }
    rig.client
        .call(methods::WORK_OUTLINE_MUTATE, params)
        .await
        .unwrap();
    derive_id(op, roles::ITEM)
}

async fn tool(rig: &Rig, token: &str, name: &str, arguments: Value) -> Result<Value, String> {
    rig.client
        .call(
            mast_proto::agent_tools::TOOL_CALL_METHOD,
            json!({ "token": token, "tool": name, "arguments": arguments }),
        )
        .await
        .map_err(|e| e.to_string())
}

fn snapshot(rig: &Rig) -> WorkOutlineSnapshot {
    let inputs = rig.core.workspace.work_outline_inputs(SPACE);
    mast_doc::materialize_work_outline(SPACE, &inputs)
}

#[tokio::test(flavor = "multi_thread")]
async fn space_chat_runs_get_a_run_scoped_token_and_projectless_runs_get_none() {
    let rig = rig().await;
    let tokens = rig.core.sessions.agent_tokens();

    rig.core
        .doc_host
        .queue_command(CHAT, run_command("hello", &rig.space_path))
        .unwrap();
    wait_for(
        || !rig.harness.spawns.lock().unwrap().is_empty(),
        "space run",
    )
    .await;
    let spawn = rig.harness.spawns.lock().unwrap()[0].clone();
    let Spawn::WithTools { prompt, tools } = spawn else {
        panic!("a Chat in a Space must get agent tools: {spawn:?}");
    };
    assert_eq!(prompt, "hello");
    assert_eq!(tools.server_name, "mast-agent");
    assert_eq!(tools.args, vec!["mcp".to_owned()]);
    let token = tools.env["MAST_AGENT_TOKEN"].clone();
    assert_eq!(tools.env["MAST_IPC_PORT"], "27999");
    assert_eq!(tokens.live_count(), 1);

    let origin = rig
        .client
        .call(
            mast_proto::agent_tools::AUTHENTICATE_METHOD,
            json!({ "token": token }),
        )
        .await
        .expect("a live Run's token authenticates");
    assert_eq!(origin["chatId"], CHAT);
    assert_eq!(origin["spaceId"], SPACE);
    assert_eq!(origin["deviceId"], rig.core.device_id);
    assert!(!origin["runId"].as_str().unwrap().is_empty());

    // The Run ends: its token is revoked with it.
    rig.harness.release.notify_one();
    wait_for(|| tokens.live_count() == 0, "token revocation").await;
    let err = rig
        .client
        .call(
            mast_proto::agent_tools::AUTHENTICATE_METHOD,
            json!({ "token": token }),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("agent-tools/unauthenticated"), "{err}");
    let err = tool(&rig, &token, "work_outline_read_current", json!({}))
        .await
        .unwrap_err();
    assert!(err.contains("agent-tools/unauthenticated"), "{err}");

    // A projectless Chat keeps today's behavior: plain run, no token.
    rig.core
        .doc_host
        .queue_command(PROJECTLESS, run_command("solo", "~"))
        .unwrap();
    wait_for(
        || rig.harness.spawns.lock().unwrap().len() == 2,
        "projectless run",
    )
    .await;
    let spawn = rig.harness.spawns.lock().unwrap()[1].clone();
    assert!(
        matches!(&spawn, Spawn::Plain { prompt } if prompt == "solo"),
        "{spawn:?}"
    );
    assert_eq!(tokens.live_count(), 0);
    rig.harness.release.notify_one();
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_tool_calls_are_recorded_on_the_completed_turn() {
    let rig = rig().await;
    rig.core.doc_host.queue_command(CHAT, run_command("hello", &rig.space_path)).unwrap();
    wait_for(|| !rig.harness.spawns.lock().unwrap().is_empty(), "run").await;
    let Spawn::WithTools { tools, .. } = rig.harness.spawns.lock().unwrap()[0].clone() else { panic!("missing tools") };
    let token = &tools.env["MAST_AGENT_TOKEN"];
    assert!(tool(&rig, token, "missing_tool", json!({})).await.is_err());
    assert!(tool(&rig, token, "missing_tool", json!({})).await.is_err());
    rig.harness.release.notify_one();
    wait_for(|| rig.core.sessions.session_status(CHAT).is_some_and(|s| s.status == mast_proto::SessionStatus::Idle), "completed turn").await;
    let session = rig.core.sessions.session_status(CHAT).unwrap();
    let failures = session.last_turn_tool_failures.unwrap();
    assert_eq!(failures.count, 2);
    assert_eq!(failures.tools, ["missing_tool"]);
    rig.core.doc_host.queue_command(CHAT, run_command("next", &rig.space_path)).unwrap();
    wait_for(|| rig.harness.spawns.lock().unwrap().len() == 2, "second run").await;
    rig.harness.release.notify_one();
    wait_for(|| rig.core.sessions.session_status(CHAT).is_some_and(|s| s.status == mast_proto::SessionStatus::Idle && s.last_turn_tool_failures.is_none()), "clean completed turn").await;
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forged_or_absent_tokens_reach_no_tool() {
    let rig = rig().await;
    let lease = rig
        .core
        .sessions
        .agent_tokens()
        .mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    for token in ["", "zat_forged", &format!("{}0", lease.token())] {
        let err = tool(&rig, token, "work_outline_read_current", json!({}))
            .await
            .unwrap_err();
        assert!(
            err.contains("agent-tools/unauthenticated"),
            "{token:?}: {err}"
        );
    }
    // A missing token field is a malformed call, never an anonymous one.
    assert!(
        rig.client
            .call(
                mast_proto::agent_tools::TOOL_CALL_METHOD,
                json!({ "tool": "work_outline_read_current" }),
            )
            .await
            .is_err()
    );
    // Unknown tools are refused even with a valid token.
    let err = tool(&rig, lease.token(), "delete_everything", json!({}))
        .await
        .unwrap_err();
    assert!(err.contains("agent-tools/unsupported"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn each_tool_is_allowed_or_denied_per_the_agent_policy() {
    let rig = rig().await;
    let root_a = create_item(&rig, "op-a", "Feature A", None).await;
    let root_b = create_item(&rig, "op-b", "Feature B", None).await;
    let a_child = create_item(&rig, "op-a1", "A step", Some(&root_a)).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();

    // Unassociated: reads show no item; subtree tools are refused.
    let read = tool(&rig, &token, "work_outline_read_current", json!({}))
        .await
        .unwrap();
    assert_eq!(read["workItem"], Value::Null);
    assert_eq!(read["chatId"], CHAT);
    let err = tool(
        &rig,
        &token,
        "work_outline_create_child",
        json!({ "title": "x", "parentWorkItemId": root_a, "idempotencyKey": "k0" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");

    // Associate ONLY the calling Chat.
    let associated = tool(
        &rig,
        &token,
        "work_outline_associate_current_chat",
        json!({ "workItemId": root_a, "idempotencyKey": "assoc" }),
    )
    .await
    .unwrap();
    assert_eq!(associated["chatId"], CHAT);
    let snap = snapshot(&rig);
    let assignment = snap.assignments.iter().find(|a| a.chat_id == CHAT).unwrap();
    assert_eq!(assignment.work_item_id.as_deref(), Some(root_a.as_str()));
    let actor = assignment.assigned_by.as_ref().unwrap();
    assert_eq!(actor.kind, WorkOutlineActorKind::Agent);
    assert_eq!(actor.chat_id.as_deref(), Some(CHAT));
    assert!(snap.assignments.iter().all(|a| a.chat_id != OTHER_CHAT));
    let service = rig.core.rpc_service();
    let err = service
        .work_outline()
        .authorize_agent(
            CHAT,
            &serde_json::from_value(json!({ "op": "assignChatToWorkItem",
                "chatId": OTHER_CHAT, "workItemId": root_a, "operationId": "x" }))
            .unwrap(),
        )
        .unwrap_err();
    assert_eq!(
        err.code,
        mast_proto::WorkOutlineErrorCode::Unauthorized,
        "{err:?}"
    );

    // Read: the item, its path, and effective instructions.
    let read = tool(&rig, &token, "work_outline_read_current", json!({}))
        .await
        .unwrap();
    assert_eq!(read["workItem"]["id"], root_a);
    assert_eq!(read["workItem"]["children"][0]["id"], a_child);
    assert_eq!(read["effectiveInstructions"]["workItemId"], root_a);

    // Create a child under the Chat's item (default parent) and under a
    // descendant; never under another subtree or at the Space root.
    let created = tool(
        &rig,
        &token,
        "work_outline_create_child",
        json!({ "title": "Agent step", "idempotencyKey": "child-1" }),
    )
    .await
    .unwrap();
    let child = created["workItemId"].as_str().unwrap().to_owned();
    assert_eq!(created["parentWorkItemId"], root_a);
    let view = snapshot(&rig).item(&child).cloned().unwrap();
    let creator = view.created_by.unwrap();
    assert_eq!(creator.kind, WorkOutlineActorKind::Agent);
    assert_eq!(creator.chat_id.as_deref(), Some(CHAT));
    tool(
        &rig,
        &token,
        "work_outline_create_child",
        json!({ "title": "Deeper", "parentWorkItemId": a_child, "idempotencyKey": "child-2",
                "status": "active" }),
    )
    .await
    .unwrap();
    let err = tool(
        &rig,
        &token,
        "work_outline_create_child",
        json!({ "title": "Elsewhere", "parentWorkItemId": root_b, "idempotencyKey": "child-3" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    let err = service
        .work_outline()
        .authorize_agent(
            CHAT,
            &serde_json::from_value(json!({ "op": "createWorkItem", "spaceId": SPACE,
                "title": "Root", "operationId": "x" }))
            .unwrap(),
        )
        .unwrap_err();
    assert_eq!(
        err.code,
        mast_proto::WorkOutlineErrorCode::Unauthorized,
        "{err:?}"
    );
    let err = tool(
        &rig,
        &token,
        "work_outline_create_child",
        json!({ "title": "Done already", "status": "completed", "idempotencyKey": "child-4" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("agent-tools/invalid"), "{err}");

    // Status: active/completed inside the subtree only.
    tool(&rig, &token, "work_outline_set_status", json!({
        "waitingUntil": 1800000000000_i64, "waitingReason": "Observe", "idempotencyKey": "wait-1"
    })).await.unwrap();
    assert_eq!(snapshot(&rig).item(&root_a).unwrap().waiting_until, Some(1800000000000));
    assert_eq!(snapshot(&rig).item(&root_a).unwrap().waiting_reason.as_deref(), Some("Observe"));
    assert!(tool(&rig, &token, "work_outline_set_status", json!({"waitingUntil": null})).await.is_err());
    tool(&rig, &token, "work_outline_set_status", json!({
        "waitingUntil": null, "waitingReason": null, "idempotencyKey": "wait-clear"
    })).await.unwrap();
    assert_eq!(snapshot(&rig).item(&root_a).unwrap().waiting_until, None);
    tool(
        &rig,
        &token,
        "work_outline_set_status",
        json!({ "status": "active", "idempotencyKey": "s1" }),
    )
    .await
    .unwrap();
    tool(
        &rig,
        &token,
        "work_outline_set_status",
        json!({ "workItemId": child, "status": "completed", "idempotencyKey": "s2" }),
    )
    .await
    .unwrap();
    let snap = snapshot(&rig);
    assert_eq!(
        snap.item(&root_a).unwrap().status,
        mast_proto::WorkItemStatus::Active
    );
    assert_eq!(
        snap.item(&child).unwrap().status,
        mast_proto::WorkItemStatus::Completed
    );
    let err = tool(
        &rig,
        &token,
        "work_outline_set_status",
        json!({ "workItemId": root_b, "status": "active", "idempotencyKey": "s3" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    let err = tool(
        &rig,
        &token,
        "work_outline_set_status",
        json!({ "status": "pending", "idempotencyKey": "s4" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("agent-tools/invalid"), "{err}");

    // Everything outside the allow-list is refused by the policy.
    for forbidden in [
        json!({ "op": "updateWorkItem", "workItemId": root_a,
                "changes": { "title": "Renamed" }, "operationId": "x" }),
        json!({ "op": "moveWorkItem", "workItemId": a_child, "operationId": "x" }),
        json!({ "op": "archiveWorkItem", "workItemId": a_child,
                "descendantArchivePolicy": { "type": "archiveDescendants" }, "operationId": "x" }),
        json!({ "op": "removeChatFromWorkItem", "chatId": CHAT, "operationId": "x" }),
        json!({ "op": "createWorkItemInstruction", "spaceId": SPACE,
                "scope": { "type": "spaceRoot" }, "text": "x", "operationId": "x" }),
        json!({ "op": "configureExecution", "scope": { "type": "workItem", "workItemId": root_a },
                "configuration": null, "operationId": "x" }),
    ] {
        let mutation = serde_json::from_value(forbidden.clone()).unwrap();
        let err = service
            .work_outline()
            .authorize_agent(CHAT, &mutation)
            .unwrap_err();
        assert_eq!(
            err.code,
            mast_proto::WorkOutlineErrorCode::Unauthorized,
            "{forbidden}"
        );
    }

    // Requests: created on the Chat's item with the Chat as origin; an
    // item with an open request cannot be completed (never auto-resolved);
    // only the originating Chat resolves it.
    let request = tool(
        &rig,
        &token,
        "work_outline_create_user_action_request",
        json!({ "request": "Approve the plan?", "detail": "Two options", "idempotencyKey": "r1" }),
    )
    .await
    .unwrap();
    let request_id = request["userActionRequestId"].as_str().unwrap().to_owned();
    assert_eq!(request["workItemId"], root_a);
    let read = tool(&rig, &token, "work_outline_read_current", json!({}))
        .await
        .unwrap();
    assert_eq!(read["workItem"]["effectiveStatus"], "userActionRequired");
    assert_eq!(read["openUserActionRequests"][0]["id"], request_id);
    let err = tool(
        &rig,
        &token,
        "work_outline_set_status",
        json!({ "status": "completed", "idempotencyKey": "s5" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/openUserActionRequests"), "{err}");
    let snap = snapshot(&rig);
    assert_eq!(
        snap.item(&root_a).unwrap().status,
        mast_proto::WorkItemStatus::Active
    );
    let other =
        rig.core
            .sessions
            .agent_tokens()
            .mint(OTHER_CHAT, "run-9", SPACE, &rig.core.device_id);
    let err = tool(
        &rig,
        other.token(),
        "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": request_id, "idempotencyKey": "r2" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    tool(
        &rig,
        &token,
        "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": request_id, "resolution": "User approved in chat",
                "idempotencyKey": "r2" }),
    )
    .await
    .unwrap();
    tool(
        &rig,
        &token,
        "work_outline_set_status",
        json!({ "status": "completed", "idempotencyKey": "s6" }),
    )
    .await
    .unwrap();

    // Proposals: for the item, its parent, or the Space root; pending until
    // the user reviews them (no instruction is written).
    let proposal = tool(
        &rig,
        &token,
        "work_outline_propose_instruction",
        json!({ "target": "spaceRoot", "text": "Prefer small PRs", "idempotencyKey": "r3" }),
    )
    .await
    .unwrap();
    assert!(proposal["proposalId"].is_string(), "{proposal}");
    assert!(snapshot(&rig).instructions.is_empty());
    let err = tool(
        &rig,
        &token,
        "work_outline_propose_instruction",
        json!({ "target": "parent", "text": "x", "idempotencyKey": "r4" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("no parent"), "{err}");
    for name in [
        "work_outline_create_user_action_request",
        "work_outline_resolve_user_action_request",
        "work_outline_propose_instruction",
    ] {
        let err = tool(&rig, &token, name, json!({})).await.unwrap_err();
        assert!(err.contains("agent-tools/invalid"), "{name}: {err}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn retries_with_one_key_dedupe_across_runs_and_conflict_on_new_inputs() {
    let rig = rig().await;
    let root = create_item(&rig, "op-root", "Root", None).await;
    let tokens = rig.core.sessions.agent_tokens();
    let first = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    tool(
        &rig,
        first.token(),
        "work_outline_associate_current_chat",
        json!({ "workItemId": root, "idempotencyKey": "assoc" }),
    )
    .await
    .unwrap();

    let err = tool(
        &rig,
        first.token(),
        "work_outline_create_child",
        json!({ "title": "Step" }),
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("agent-tools/invalid") && err.contains("idempotencyKey"),
        "{err}"
    );

    let args = json!({ "title": "Step", "idempotencyKey": "step-1" });
    let a = tool(
        &rig,
        first.token(),
        "work_outline_create_child",
        args.clone(),
    )
    .await
    .unwrap();
    let b = tool(
        &rig,
        first.token(),
        "work_outline_create_child",
        args.clone(),
    )
    .await
    .unwrap();
    assert_eq!(a["workItemId"], b["workItemId"]);
    let count = |rig: &Rig| snapshot(rig).items.len();
    assert_eq!(count(&rig), 2);

    // A later Run of the same Chat retrying the same key: same item.
    drop(first);
    let second = tokens.mint(CHAT, "run-2", SPACE, &rig.core.device_id);
    let c = tool(&rig, second.token(), "work_outline_create_child", args)
        .await
        .unwrap();
    assert_eq!(a["workItemId"], c["workItemId"]);
    assert_eq!(count(&rig), 2);

    // Same key, different inputs: a conflict, never a second item.
    let err = tool(
        &rig,
        second.token(),
        "work_outline_create_child",
        json!({ "title": "Other", "idempotencyKey": "step-1" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/operationConflict"), "{err}");
    assert_eq!(count(&rig), 2);

    // Another Chat using the same key gets its own operation.
    let other = tokens.mint(OTHER_CHAT, "run-3", SPACE, &rig.core.device_id);
    tool(
        &rig,
        other.token(),
        "work_outline_associate_current_chat",
        json!({ "workItemId": root, "idempotencyKey": "assoc" }),
    )
    .await
    .unwrap();
    let d = tool(
        &rig,
        other.token(),
        "work_outline_create_child",
        json!({ "title": "Step", "idempotencyKey": "step-1" }),
    )
    .await
    .unwrap();
    assert_ne!(a["workItemId"], d["workItemId"]);
    assert_eq!(count(&rig), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_child_first_turn_runs_with_the_child_chat_config_not_the_callers() {
    let rig = rig().await;
    let lease = rig
        .core
        .sessions
        .agent_tokens()
        .mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let key = "spawn-reviewer";
    let child = derive_id(
        &mast_proto::agent_tools::operation_id(CHAT, key),
        roles::CHAT,
    );
    rig.client
        .call(
            methods::MUTATE,
            json!({ "op": "createChat", "chatId": child, "spaceId": SPACE,
                    "deviceId": rig.core.device_id, "parentChatId": CHAT,
                    "config": { "harness": "mock", "model": null, "reasoning": null,
                                "sandbox": "read-only", "autoApprove": false } }),
        )
        .await
        .unwrap();
    // A caller trying to smuggle a wider Run: only the prompt survives.
    let queued = tool(
        &rig,
        lease.token(),
        "chat_queue_child_turn",
        json!({
            "chatId": child,
            "idempotencyKey": key,
            "prompt": "review the diff",
            "command": { "type": "run", "messageId": "",
                "request": { "prompt": "x", "cwd": "/", "sandbox": "danger-full-access",
                             "autoApprove": true } },
            "sandbox": "danger-full-access",
            "autoApprove": true,
            "cwd": "/",
        }),
    )
    .await
    .unwrap();
    assert_eq!(queued["chatId"], child);
    wait_for(
        || !rig.harness.requests.lock().unwrap().is_empty(),
        "child run",
    )
    .await;
    let request = rig.harness.requests.lock().unwrap()[0].clone();
    assert_eq!(request.prompt, "review the diff");
    assert_eq!(request.sandbox, SandboxLevel::ReadOnly);
    assert!(!request.auto_approve);
    assert_eq!(request.cwd, rig.space_path);
    assert!(request.worktree.is_none());

    // Not a child of the caller, or the wrong key: refused.
    let err = tool(
        &rig,
        lease.token(),
        "chat_queue_child_turn",
        json!({ "chatId": OTHER_CHAT, "idempotencyKey": key, "prompt": "hi" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("agent-tools/forbidden"), "{err}");
    let err = tool(
        &rig,
        lease.token(),
        "chat_queue_child_turn",
        json!({ "chatId": child, "idempotencyKey": key, "prompt": "   " }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("agent-tools/invalid"), "{err}");
    rig.harness.release.notify_one();
}

async fn add_instruction(rig: &Rig, op: &str, scope: Value, text: &str) {
    rig.client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "createWorkItemInstruction", "spaceId": SPACE, "scope": scope,
                    "text": text, "operationId": op }),
        )
        .await
        .unwrap();
}

async fn assign_chat(rig: &Rig, op: &str, chat: &str, item: &str) {
    rig.client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "assignChatToWorkItem", "chatId": chat, "workItemId": item,
                    "operationId": op }),
        )
        .await
        .unwrap();
}

fn launch_args(key: &str, item: &str, sandbox: &str) -> Value {
    json!({
        "idempotencyKey": key, "workItemId": item, "prompt": "go",
        "config": { "harness": "mock", "model": null, "reasoning": null,
                    "modelOptions": {}, "sandbox": sandbox, "autoApprove": false },
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn user_hand_off_rpc_transfers_workers_and_replays() {
    let rig = rig().await;
    let item = create_item(&rig, "user-handoff-item", "Coordinator", None).await;
    assign_chat(&rig, "user-handoff-assignment", CHAT, &item).await;
    rig.client.call(methods::MUTATE, json!({"op":"createChat", "chatId":"user-handoff-worker", "spaceId":SPACE,
        "deviceId":rig.core.device_id, "parentChatId":CHAT})).await.unwrap();
    let params = json!({"chatId": CHAT, "summary":"Continue coordinating", "operationId":"user-handoff"});
    let reply = rig.client.call(methods::HAND_OFF_ORCHESTRATOR, params.clone()).await.unwrap();
    let successor = reply["chatId"].as_str().unwrap();
    let retry = rig.client.call(methods::HAND_OFF_ORCHESTRATOR, params).await.unwrap();
    assert_eq!(retry["chatId"], successor);
    assert_eq!(rig.core.workspace.chat("user-handoff-worker").unwrap().unwrap().parent_chat_id.as_deref(), Some(successor));
    let source = rig.core.doc_host.open(CHAT).unwrap();
    let entries = source.doc().read_entries().unwrap();
    let marker = format!("[Mast hand-off: {successor}]");
    assert_eq!(entries.iter().filter(|entry| entry.parts.iter().any(|part| matches!(part, mast_doc::MessagePart::Text { text, .. } if text == &marker))).count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn hand_off_orchestrator_starts_successor_with_workers_and_actions() {
    let rig = rig().await;
    let item = create_item(&rig, "handoff-item", "Coordinator", None).await;
    assign_chat(&rig, "handoff-assignment", CHAT, &item).await;
    rig.client.call(methods::MUTATE, json!({"op":"createChat", "chatId":"handoff-worker", "spaceId":SPACE,
        "deviceId":rig.core.device_id, "parentChatId":CHAT})).await.unwrap();
    assign_chat(&rig, "handoff-worker-assignment", "handoff-worker", &item).await;
    let tokens = rig.core.sessions.agent_tokens();
    let worker = tokens.mint("handoff-worker", "worker-run", SPACE, &rig.core.device_id);
    tool(&rig, worker.token(), "work_outline_create_user_action_request", json!({
        "request":"Review the result", "idempotencyKey":"review-result"
    })).await.unwrap();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let args = json!({"summary":"Continue the release", "idempotencyKey":"handoff-one", "harness":"mock", "model":"mock-1", "reasoning":"medium"});
    let reply = tool(&rig, lease.token(), "hand_off_orchestrator", args.clone()).await.unwrap();
    let successor = reply["chatId"].as_str().unwrap();
    let config = rig.core.workspace.chat(successor).unwrap().unwrap().config.unwrap();
    assert_eq!(config.harness, HarnessId::Mock);
    assert_eq!(config.model.as_deref(), Some("mock-1"));
    assert_eq!(config.reasoning, Some(ReasoningLevel::Medium));
    let retry = tool(&rig, lease.token(), "hand_off_orchestrator", args).await.unwrap();
    assert_eq!(retry["chatId"], successor);
    assert!(tool(&rig, lease.token(), "hand_off_orchestrator", json!({
        "summary":"Changed summary", "idempotencyKey":"handoff-one"
    })).await.is_err());
    assert_eq!(rig.core.workspace.chat("handoff-worker").unwrap().unwrap().parent_chat_id.as_deref(), Some(successor));
    assert!(!rig.core.workspace.chat(CHAT).unwrap().unwrap().archived);
    wait_for(|| rig.harness.requests.lock().unwrap().iter().any(|r| r.prompt.contains("Continue the release")), "successor turn").await;
    let requests = rig.harness.requests.lock().unwrap();
    let prompt = &requests.iter().find(|r| r.prompt.contains("Continue the release")).unwrap().prompt;
    assert!(prompt.starts_with("[Mast system message, not from the user: coordinator hand-off.]\n"));
    assert!(prompt.contains("handoff-worker"));
    assert!(prompt.contains("Review the result"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_child_started_for_a_descendant_item_gets_that_items_instructions() {
    let rig = rig().await;
    let root_a = create_item(&rig, "op-a", "Coordinator", None).await;
    let root_b = create_item(&rig, "op-b", "Elsewhere", None).await;
    let a_child = create_item(&rig, "op-a1", "Step", Some(&root_a)).await;
    let a_grand = create_item(&rig, "op-a11", "Leaf", Some(&a_child)).await;
    let b_child = create_item(&rig, "op-b1", "Other step", Some(&root_b)).await;
    add_instruction(
        &rig,
        "ins-root",
        json!({ "type": "workItem", "workItemId": root_a }),
        "Coordinator rule.",
    )
    .await;
    add_instruction(
        &rig,
        "ins-leaf",
        json!({ "type": "workItem", "workItemId": a_grand }),
        "Leaf rule.",
    )
    .await;
    add_instruction(
        &rig,
        "ins-other",
        json!({ "type": "workItem", "workItemId": b_child }),
        "Other subtree rule.",
    )
    .await;
    assign_chat(&rig, "assign-caller", CHAT, &root_a).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();

    // The child is started FOR the descendant: attached to it, parented to
    // the caller, with the leaf's effective instructions (its own plus the
    // inherited ones), never the caller's item's alone.
    let key = "leaf-worker";
    let reply = tool(
        &rig,
        &token,
        "chat_launch_child_for_item",
        launch_args(key, &a_grand, "workspace-write"),
    )
    .await
    .unwrap();
    let child = derive_id(
        &mast_proto::agent_tools::operation_id(CHAT, key),
        roles::CHAT,
    );
    assert_eq!(reply["chatId"], child);
    let snap = snapshot(&rig);
    let assignment = snap
        .assignments
        .iter()
        .find(|a| a.chat_id == child)
        .unwrap();
    assert_eq!(assignment.work_item_id.as_deref(), Some(a_grand.as_str()));
    let row = rig.core.workspace.chat(&child).unwrap().unwrap();
    assert_eq!(row.parent_chat_id.as_deref(), Some(CHAT));
    // An agent-started child is Ask unless an Auto caller asks for Auto.
    assert!(!row.config.unwrap().auto_approve);
    let pin = rig
        .core
        .workspace
        .selected_instruction_revision(&child)
        .expect("the launch selected an instruction revision");
    let revision = rig
        .core
        .workspace
        .instruction_revision(&pin.revision_id)
        .unwrap();
    assert_eq!(revision.work_item_id, a_grand);
    let texts: Vec<&str> = revision
        .body
        .entries
        .iter()
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(texts, ["Coordinator rule.", "Leaf rule."]);
    wait_for(
        || !rig.harness.requests.lock().unwrap().is_empty(),
        "child run",
    )
    .await;
    let request = rig.harness.requests.lock().unwrap()[0].clone();
    assert_eq!(request.prompt, "go");
    let rendered = request.instructions.unwrap().envelope.rendered;
    assert!(rendered.contains("Leaf rule."), "{rendered}");
    assert!(!rendered.contains("Other subtree rule."), "{rendered}");

    // A retry with the same key and inputs is the same Chat, started once.
    let again = tool(
        &rig,
        &token,
        "chat_launch_child_for_item",
        launch_args(key, &a_grand, "workspace-write"),
    )
    .await
    .unwrap();
    assert_eq!(again["chatId"], child);
    assert_eq!(again["commandId"], reply["commandId"]);
    assert_eq!(
        snapshot(&rig)
            .assignments
            .iter()
            .filter(|a| a.chat_id == child)
            .count(),
        1
    );
    // The caller's own item is a valid target too.
    let own = tool(
        &rig,
        &token,
        "chat_launch_child_for_item",
        launch_args("own-item", &root_a, "read-only"),
    )
    .await
    .unwrap();
    let own_chat = own["chatId"].as_str().unwrap().to_owned();
    assert_eq!(
        snapshot(&rig)
            .assignments
            .iter()
            .find(|a| a.chat_id == own_chat)
            .and_then(|a| a.work_item_id.clone()),
        Some(root_a.clone())
    );

    // Outside the caller's subtree (another root, a sibling subtree's
    // descendant, an unknown id): unauthorized, nothing created.
    let chats_before = rig.core.workspace.read_chats().unwrap().len();
    for target in [root_b.as_str(), b_child.as_str(), "no-such-item"] {
        let err = tool(
            &rig,
            &token,
            "chat_launch_child_for_item",
            launch_args(&format!("out-{target}"), target, "workspace-write"),
        )
        .await
        .unwrap_err();
        assert!(err.contains("work-outline/unauthorized"), "{target}: {err}");
    }
    // Never a wider sandbox than the caller's, and a prompt is required.
    let err = tool(
        &rig,
        &token,
        "chat_launch_child_for_item",
        launch_args("wide", &a_grand, "danger-full-access"),
    )
    .await
    .unwrap_err();
    assert!(err.contains("agent-tools/forbidden"), "{err}");
    let mut empty = launch_args("empty", &a_grand, "workspace-write");
    empty["prompt"] = json!("  ");
    let err = tool(&rig, &token, "chat_launch_child_for_item", empty)
        .await
        .unwrap_err();
    assert!(err.contains("agent-tools/invalid"), "{err}");
    assert_eq!(rig.core.workspace.read_chats().unwrap().len(), chats_before);

    // The re-association rule is NOT relaxed: the started child keeps the
    // item it was started for.
    let child_lease = tokens.mint(&child, "run-2", SPACE, &rig.core.device_id);
    let err = tool(
        &rig,
        child_lease.token(),
        "work_outline_associate_current_chat",
        json!({ "workItemId": root_a, "idempotencyKey": "reassoc" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    assert!(err.contains("re-associate"), "{err}");

    // The coordinator closes what its own child raised; an unrelated Chat
    // still cannot.
    let ready = tool(
        &rig,
        child_lease.token(),
        "work_outline_create_user_action_request",
        json!({ "request": "Ready to merge: leaf-1", "idempotencyKey": "ready" }),
    )
    .await
    .unwrap();
    let ready_id = ready["userActionRequestId"].as_str().unwrap().to_owned();
    let other = tokens.mint(OTHER_CHAT, "run-3", SPACE, &rig.core.device_id);
    let err = tool(
        &rig,
        other.token(),
        "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": ready_id, "idempotencyKey": "close-other" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    tool(
        &rig,
        &token,
        "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": ready_id, "resolution": "Merged",
                "idempotencyKey": "close-parent" }),
    )
    .await
    .unwrap();
    assert!(
        snapshot(&rig)
            .user_action_requests
            .iter()
            .find(|r| r.id == ready_id)
            .is_some_and(|r| r.state == mast_proto::UserActionRequestState::Resolved),
        "the coordinator resolved its child's request"
    );

    // An unassociated caller has no subtree to start chats in.
    let err = tool(
        &rig,
        other.token(),
        "chat_launch_child_for_item",
        launch_args("nobody", &a_grand, "workspace-write"),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    rig.harness.release.notify_waiters();
}

#[tokio::test(flavor = "multi_thread")]
async fn read_subtree_lists_descendants_chats_and_requests_within_the_callers_subtree() {
    let rig = rig().await;
    let root_a = create_item(&rig, "op-a", "Coordinator", None).await;
    let root_b = create_item(&rig, "op-b", "Elsewhere", None).await;
    let a_child = create_item(&rig, "op-a1", "Step", Some(&root_a)).await;
    let a_grand = create_item(&rig, "op-a11", "Leaf", Some(&a_child)).await;
    let a_deep = create_item(&rig, "op-a111", "Deep", Some(&a_grand)).await;
    let a_archived = create_item(&rig, "op-a2", "Old", Some(&root_a)).await;
    rig.client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "archiveWorkItem", "workItemId": a_archived,
                    "descendantArchivePolicy": { "type": "archiveDescendants" },
                    "operationId": "arch" }),
        )
        .await
        .unwrap();
    assign_chat(&rig, "assign-caller", CHAT, &root_a).await;
    assign_chat(&rig, "assign-worker", OTHER_CHAT, &a_grand).await;
    rig.client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "createUserActionRequest", "workItemId": a_grand,
                    "origin": { "chatId": OTHER_CHAT }, "text": "Ready to merge",
                    "info": "branch leaf-1", "operationId": "req-1" }),
        )
        .await
        .unwrap();
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();

    // Default: the caller's item, depth 4 (so the grandchild is in), archived
    // items left out.
    let read = tool(&rig, &token, "work_outline_read_subtree", json!({}))
        .await
        .unwrap();
    assert_eq!(read["rootWorkItemId"], root_a);
    assert_eq!(read["truncated"], false);
    assert_eq!(read["itemCount"], 4);
    let tree = &read["tree"];
    assert_eq!(tree["id"], root_a);
    assert_eq!(tree["children"].as_array().unwrap().len(), 1);
    let step = &tree["children"][0];
    assert_eq!(step["id"], a_child);
    assert_eq!(
        step["effectiveStatus"],
        snapshot(&rig)
            .item(&a_child)
            .map(|i| json!(i.effective_status))
            .unwrap()
    );
    let leaf = &step["children"][0];
    assert_eq!(leaf["id"], a_grand);
    assert_eq!(leaf["chats"][0]["id"], OTHER_CHAT);
    assert_eq!(leaf["chats"][0]["status"], "idle");
    assert!(leaf["chats"][0].get("lastMessageAt").is_some());
    let request = &leaf["openUserActionRequests"][0];
    assert_eq!(request["text"], "Ready to merge");
    assert_eq!(request["detail"], "branch leaf-1");
    assert_eq!(request["originChatId"], OTHER_CHAT);
    assert!(request["id"].as_str().is_some() && request["createdAt"].is_number());
    assert_eq!(leaf["children"][0]["id"], a_deep);
    // The caller's own chat is listed on its own item.
    assert_eq!(tree["chats"][0]["id"], CHAT);

    // maxDepth limits how far down it goes and says what it left out.
    let shallow = tool(
        &rig,
        &token,
        "work_outline_read_subtree",
        json!({ "maxDepth": 1 }),
    )
    .await
    .unwrap();
    assert_eq!(shallow["itemCount"], 2);
    assert_eq!(shallow["tree"]["children"][0]["id"], a_child);
    assert_eq!(shallow["tree"]["children"][0]["children"], json!([]));
    assert_eq!(shallow["tree"]["children"][0]["childrenOmitted"], 1);
    // A descendant as the root of the read; maxDepth is capped at 8.
    let from_leaf = tool(
        &rig,
        &token,
        "work_outline_read_subtree",
        json!({ "workItemId": a_grand, "maxDepth": 99 }),
    )
    .await
    .unwrap();
    assert_eq!(from_leaf["maxDepth"], 8);
    assert_eq!(from_leaf["tree"]["id"], a_grand);
    assert_eq!(from_leaf["itemCount"], 2);
    // includeArchived adds the archived item.
    let with_archived = tool(
        &rig,
        &token,
        "work_outline_read_subtree",
        json!({ "includeArchived": true }),
    )
    .await
    .unwrap();
    assert_eq!(with_archived["itemCount"], 5);
    let archived = with_archived["tree"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == a_archived)
        .unwrap();
    assert_eq!(archived["archived"], true);

    // Outside the subtree (another root, an ancestor-side item, unknown) and
    // unassociated callers are refused.
    for target in [root_b.as_str(), "no-such-item"] {
        let err = tool(
            &rig,
            &token,
            "work_outline_read_subtree",
            json!({ "workItemId": target }),
        )
        .await
        .unwrap_err();
        assert!(err.contains("work-outline/unauthorized"), "{target}: {err}");
    }
    let worker = tokens.mint(OTHER_CHAT, "run-2", SPACE, &rig.core.device_id);
    let err = tool(
        &rig,
        worker.token(),
        "work_outline_read_subtree",
        json!({ "workItemId": root_a }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    // A leaf's chat reads its own subtree only, never upward.
    let err = tool(
        &rig,
        worker.token(),
        "work_outline_read_subtree",
        json!({ "workItemId": a_child }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    let own = tool(&rig, worker.token(), "work_outline_read_subtree", json!({}))
        .await
        .unwrap();
    assert_eq!(own["tree"]["id"], a_grand);
}

#[tokio::test(flavor = "multi_thread")]
async fn read_subtree_caps_its_output_and_says_so() {
    let rig = rig().await;
    let root = create_item(&rig, "op-root", "Big", None).await;
    for i in 0..310 {
        create_item(
            &rig,
            &format!("op-leaf-{i}"),
            &format!("Leaf {i}"),
            Some(&root),
        )
        .await;
    }
    assign_chat(&rig, "assign-caller", CHAT, &root).await;
    let lease = rig
        .core
        .sessions
        .agent_tokens()
        .mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let read = tool(&rig, lease.token(), "work_outline_read_subtree", json!({}))
        .await
        .unwrap();
    assert_eq!(read["truncated"], true);
    assert_eq!(read["itemCount"], 300);
    assert_eq!(read["tree"]["children"].as_array().unwrap().len(), 299);
    assert_eq!(read["tree"]["childrenOmitted"], 11);
}

#[tokio::test(flavor = "multi_thread")]
async fn read_subtree_omits_completed_branches_and_preserves_requests() {
    let rig = rig().await;
    let root = create_item(&rig, "compact-root", "Coordinator", None).await;
    for i in 0..170 {
        let item = create_item(&rig, &format!("compact-create-{i}"), &format!("Completed work item {i} with a descriptive title"), Some(&root)).await;
        update_item(&rig, &format!("compact-complete-{i}"), &item, json!({"status": "completed"})).await.unwrap();
    }
    let parent = create_item(&rig, "compact-parent", "Completed parent", Some(&root)).await;
    let leaf = create_item(&rig, "compact-leaf", "Request", Some(&parent)).await;
    update_item(&rig, "compact-parent-done", &parent, json!({"status": "completed"})).await.unwrap();
    rig.client.call(methods::WORK_OUTLINE_MUTATE, json!({"op": "createUserActionRequest", "workItemId": leaf, "origin": {"chatId": OTHER_CHAT}, "text": "Review", "operationId": "compact-request"})).await.unwrap();
    assign_chat(&rig, "compact-assign", CHAT, &root).await;
    let lease = rig.core.sessions.agent_tokens().mint(CHAT, "compact-run", SPACE, &rig.core.device_id);
    let compact = tool(&rig, lease.token(), "work_outline_read_subtree", json!({})).await.unwrap();
    let full = tool(&rig, lease.token(), "work_outline_read_subtree", json!({"includeCompleted": true})).await.unwrap();
    assert_eq!(compact["itemCount"], 3);
    assert_eq!(compact["tree"]["completedOmitted"], 170);
    assert_eq!(compact["tree"]["children"][0]["children"][0]["openUserActionRequests"][0]["text"], "Review");
    assert_eq!(full["itemCount"], 173);
    assert_eq!(full["tree"]["completedOmitted"], 0);
    let shallow = tool(&rig, lease.token(), "work_outline_read_subtree", json!({"maxDepth": 0})).await.unwrap();
    assert_eq!(shallow["tree"]["completedOmitted"], 170);
    assert_eq!(shallow["tree"]["childrenOmitted"], 1);
    let selected = tool(&rig, lease.token(), "work_outline_read_subtree", json!({"workItemId": parent})).await.unwrap();
    assert_eq!(selected["tree"]["id"], parent);
    let before = serde_json::to_vec(&full).unwrap().len();
    let after = serde_json::to_vec(&compact).unwrap().len();
    // Baseline for this 173-item fixture: 46,499 bytes before, 1,220 after.
    println!("subtree fixture: {before} bytes before, {after} bytes after");
    assert!(after * 4 < before, "before={before}, after={after}");
}

#[tokio::test(flavor = "multi_thread")]
async fn read_subtree_preserves_completed_items_with_working_chats() {
    let rig = rig().await;
    let root = create_item(&rig, "working-root", "Coordinator", None).await;
    let completed = create_item(&rig, "working-item", "Completed", Some(&root)).await;
    update_item(&rig, "working-complete", &completed, json!({"status": "completed"})).await.unwrap();
    assign_chat(&rig, "working-caller", CHAT, &root).await;
    assign_chat(&rig, "working-worker", OTHER_CHAT, &completed).await;
    rig.core.doc_host.queue_command(OTHER_CHAT, run_command("work", &rig.space_path)).unwrap();
    wait_for(|| rig.core.sessions.session_status(OTHER_CHAT).is_some_and(|s| s.status == mast_proto::SessionStatus::Working), "worker running").await;
    let lease = rig.core.sessions.agent_tokens().mint(CHAT, "working-run", SPACE, &rig.core.device_id);
    let read = tool(&rig, lease.token(), "work_outline_read_subtree", json!({})).await.unwrap();
    assert_eq!(read["tree"]["children"][0]["id"], completed);
    assert_eq!(read["tree"]["children"][0]["chats"][0]["status"], "working");
    rig.harness.release.notify_one();
}

// ---- bind_chat_worktree -----------------------------------------------------
//
// Real temp git repositories: a superproject with a `flagship` submodule, a
// linked worktree of the SUBMODULE in the superproject's gitignored
// `.worktrees/` folder (the cb4e2158 layout), and one of the root repository.

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Zeron Test",
            "-c",
            "user.email=test@zeron.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "protocol.file.allow=always",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn init_repo(path: &Path, file: &str) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q"]);
    std::fs::write(path.join(file), "x\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "init"]);
}

struct BindRig {
    _tmp: tempfile::TempDir,
    ws: PathBuf,
    sub_wt: PathBuf,
    root_wt: PathBuf,
    other: PathBuf,
    core: EngineCore,
    rpc: Arc<mast_engine::rpc::EngineRpc>,
    targets: DiffTargets,
    token: String,
    /// Keeps the token's Run alive.
    _lease: mast_engine::agent_tools::AgentTokenLease,
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn bind_rig(attach: bool) -> BindRig {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    init_repo(&root.join("sources/flagship"), "flagship.txt");
    let ws = root.join("ws");
    init_repo(&ws, "README");
    let source = root.join("sources/flagship");
    git(&ws, &["submodule", "add", "-q", &text(&source), "flagship"]);
    std::fs::write(ws.join(".gitignore"), ".worktrees/\n").unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-q", "-m", "members"]);
    let sub_wt = ws.join(".worktrees/flagship/eng-3012");
    std::fs::create_dir_all(sub_wt.parent().unwrap()).unwrap();
    git(
        &ws.join("flagship"),
        &[
            "worktree",
            "add",
            "-q",
            "../.worktrees/flagship/eng-3012",
            "-b",
            "feat-eng-3012",
        ],
    );
    let root_wt = ws.join(".worktrees/ws/root-feature");
    std::fs::create_dir_all(root_wt.parent().unwrap()).unwrap();
    git(
        &ws,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "root-feature",
            &text(&root_wt),
        ],
    );
    let other = root.join("other");
    init_repo(&other, "other.txt");

    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("device-id"), "dev-a").unwrap();
    let core = EngineCore::assemble(
        &data,
        Arc::new(HarnessRegistry::new()),
        HarnessId::Mock,
        None,
    )
    .expect("engine core assembles");
    core.workspace
        .create_space(SPACE, "dev-a", &text(&ws), None, true)
        .unwrap();
    core.workspace
        .create_chat(CHAT, Some(SPACE), Some("dev-a"), None, Some(text(&ws)))
        .unwrap();
    let rpc = core.rpc_service();
    let targets = DiffTargets::new(core.repos.clone(), core.workspace.clone(), "dev-a");
    if attach {
        rpc.agent_tools().set_diff_targets(targets.clone());
    }
    let lease = core
        .sessions
        .agent_tokens()
        .mint(CHAT, "run-1", SPACE, "dev-a");
    BindRig {
        token: lease.token().to_owned(),
        _lease: lease,
        sub_wt: std::fs::canonicalize(&sub_wt).unwrap(),
        root_wt: std::fs::canonicalize(&root_wt).unwrap(),
        _tmp: tmp,
        ws,
        other,
        core,
        rpc,
        targets,
    }
}

fn bind_call(
    rig: &BindRig,
    token: &str,
    arguments: Value,
) -> Result<Value, mast_engine::agent_tools::AgentToolError> {
    rig.rpc
        .agent_tools()
        .call(&mast_proto::agent_tools::AgentToolCallParams {
            token: token.to_owned(),
            tool: mast_proto::agent_tools::tools::BIND_CHAT_WORKTREE.to_owned(),
            arguments,
        })
}

fn bind_ok(rig: &BindRig, path: &Path, key: &str) -> Value {
    bind_call(
        rig,
        &rig.token,
        json!({ "path": text(path), "idempotencyKey": key }),
    )
    .unwrap_or_else(|e| panic!("bind {}: {e}", path.display()))
}

#[tokio::test(flavor = "multi_thread")]
async fn bind_chat_worktree_records_and_selects_a_submodule_worktree() {
    let rig = bind_rig(true);
    let reply = bind_ok(&rig, &rig.sub_wt, "bind-flagship");
    assert_eq!(reply["chatId"], CHAT);
    assert_eq!(reply["path"], text(&rig.sub_wt));
    assert_eq!(reply["branch"], "feat-eng-3012");
    assert_eq!(
        reply["headSha"],
        git(&rig.sub_wt, &["rev-parse", "HEAD"]).as_str()
    );
    assert_eq!(reply["repoName"], "flagship");
    assert_eq!(reply["selected"], true);
    assert_eq!(reply["replayed"], false);
    assert_eq!(
        reply["targets"],
        json!([{ "path": text(&rig.sub_wt), "source": "agent", "selected": true }])
    );
    // The row is persisted, agent-sourced, with the derived operation id.
    let stored = rig.targets.stored(CHAT);
    assert_eq!(stored.targets.len(), 1);
    assert!(stored.targets[0].selected);
    let row = rig.targets.rows().into_iter().next().unwrap();
    assert_eq!(row.chat_id, CHAT);
    assert_eq!(
        row.ops,
        vec![mast_proto::agent_tools::operation_id(CHAT, "bind-flagship")]
    );
    assert!(row.ops[0].len() <= 64);
}

#[tokio::test(flavor = "multi_thread")]
async fn bind_chat_worktree_retries_replay_and_new_keys_do_not_duplicate() {
    let rig = bind_rig(true);
    bind_ok(&rig, &rig.sub_wt, "k1");
    let before = *rig.targets.changes().borrow();
    let retry = bind_ok(&rig, &rig.sub_wt, "k1");
    assert_eq!(retry["replayed"], true);
    assert_eq!(retry["targets"].as_array().unwrap().len(), 1);
    assert_eq!(*rig.targets.changes().borrow(), before, "no second write");

    // Same key, different path: the key is already spent.
    let err = bind_call(
        &rig,
        &rig.token,
        json!({ "path": text(&rig.root_wt), "idempotencyKey": "k1" }),
    )
    .unwrap_err();
    assert_eq!(err.code(), "invalid");
    assert_eq!(rig.targets.stored(CHAT).targets.len(), 1);

    // A new key for the same path refreshes the selection, no duplicate.
    let again = bind_ok(&rig, &rig.sub_wt, "k2");
    assert_eq!(again["replayed"], false);
    assert_eq!(again["targets"].as_array().unwrap().len(), 1);
    assert_eq!(rig.targets.stored(CHAT).targets.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn bind_chat_worktree_binds_each_repository_family_and_selects_the_last() {
    let rig = bind_rig(true);
    bind_ok(&rig, &rig.sub_wt, "a");
    let reply = bind_ok(&rig, &rig.root_wt, "b");
    assert_eq!(reply["repoName"], "ws");
    assert_eq!(reply["branch"], "root-feature");
    assert_eq!(
        reply["targets"],
        json!([
            { "path": text(&rig.sub_wt), "source": "agent", "selected": false },
            { "path": text(&rig.root_wt), "source": "agent", "selected": true },
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bind_chat_worktree_refuses_paths_outside_the_chats_repositories() {
    let rig = bind_rig(true);
    for (key, path) in [
        ("outside", text(rig.ws.parent().unwrap())),
        ("other", text(&rig.other)),
        ("relative", "flagship".to_owned()),
        ("missing", text(&rig.ws.join("nope"))),
    ] {
        let err = bind_call(
            &rig,
            &rig.token,
            json!({ "path": path, "idempotencyKey": key }),
        )
        .unwrap_err();
        assert_eq!(err.code(), "invalid", "{key}: {err}");
    }
    let subdir = rig.sub_wt.join("src");
    std::fs::create_dir_all(&subdir).unwrap();
    let err = bind_call(
        &rig,
        &rig.token,
        json!({ "path": text(&subdir), "idempotencyKey": "subdir" }),
    )
    .unwrap_err();
    assert_eq!(err.code(), "invalid", "{err}");
    // A chatId argument is ignored: the token's chat is the only one bound.
    core_chat_other(&rig);
    let reply = bind_call(
        &rig,
        &rig.token,
        json!({ "path": text(&rig.sub_wt), "idempotencyKey": "forge", "chatId": OTHER_CHAT }),
    )
    .unwrap();
    assert_eq!(reply["chatId"], CHAT);
    assert!(rig.targets.stored(OTHER_CHAT).targets.is_empty());
    assert_eq!(rig.targets.stored(CHAT).targets.len(), 1);
}

fn core_chat_other(rig: &BindRig) {
    rig.core
        .workspace
        .create_chat(
            OTHER_CHAT,
            Some(SPACE),
            Some("dev-a"),
            None,
            Some(text(&rig.ws)),
        )
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn bind_chat_worktree_validates_arguments_and_authentication() {
    let rig = bind_rig(true);
    let path = text(&rig.sub_wt);
    for (arguments, what) in [
        (json!({ "idempotencyKey": "k" }), "missing path"),
        (json!({ "path": "", "idempotencyKey": "k" }), "empty path"),
        (json!({ "path": 7, "idempotencyKey": "k" }), "bad path type"),
        (json!(null), "no arguments"),
    ] {
        let err = bind_call(&rig, &rig.token, arguments).unwrap_err();
        assert_eq!(err.code(), "invalid", "{what}: {err}");
    }
    assert!(rig.targets.stored(CHAT).targets.is_empty());

    // A key is optional for a bind (blank counts as omitted).
    for key in [None, Some(""), Some("  ")] {
        let mut arguments = json!({ "path": path });
        if let Some(key) = key {
            arguments["idempotencyKey"] = json!(key);
        }
        bind_call(&rig, &rig.token, arguments)
            .unwrap_or_else(|e| panic!("bind with key {key:?}: {e}"));
    }

    let good = json!({ "path": path, "idempotencyKey": "k" });
    for token in ["", "zat_forged", &format!("{}0", rig.token)] {
        let err = bind_call(&rig, token, good.clone()).unwrap_err();
        assert_eq!(err.code(), "unauthenticated", "{token:?}");
    }
    assert_eq!(rig.targets.stored(CHAT).targets.len(), 1);
}

/// Replay of today's failure (Mikey's ENG-1655 review chat and the
/// pr-page-parity child): `bind_chat_worktree` without an `idempotencyKey`
/// binds, and rebinding a path later re-selects it even after another bind.
#[tokio::test(flavor = "multi_thread")]
async fn bind_chat_worktree_without_a_key_binds_and_rebinding_reselects() {
    let rig = bind_rig(true);
    let call = |path: &Path| {
        bind_call(&rig, &rig.token, json!({ "path": text(path) })).unwrap_or_else(|e| panic!("{e}"))
    };
    let selected = || -> Option<String> {
        rig.targets
            .stored(CHAT)
            .targets
            .into_iter()
            .find(|t| t.selected)
            .map(|t| t.path)
    };
    let first = call(&rig.sub_wt);
    assert_eq!(first["selected"], true);
    assert_eq!(first["replayed"], false);
    // The same call again is harmless: still one target.
    let again = call(&rig.sub_wt);
    assert_eq!(again["targets"].as_array().unwrap().len(), 1);
    assert_eq!(again["replayed"], false);
    // Bind another path, then the first again: the first is selected, not
    // replayed as a stale no-op.
    call(&rig.root_wt);
    assert_eq!(selected().as_deref(), Some(text(&rig.root_wt).as_str()));
    call(&rig.sub_wt);
    assert_eq!(selected().as_deref(), Some(text(&rig.sub_wt).as_str()));
    assert_eq!(rig.targets.stored(CHAT).targets.len(), 2);
    // No replay ledger entries were written for the keyless calls.
    assert!(rig.targets.rows().into_iter().all(|row| row.ops.is_empty()));

    // `snake_case` keys are accepted too.
    let reply = bind_call(
        &rig,
        &rig.token,
        json!({ "path": text(&rig.sub_wt), "idempotency_key": "snake" }),
    )
    .unwrap();
    assert_eq!(reply["replayed"], false);
    let row = rig.targets.rows().into_iter().next().unwrap();
    assert_eq!(
        row.ops,
        vec![mast_proto::agent_tools::operation_id(CHAT, "snake")]
    );

    // A missing path lists the parameters and an example.
    let err = bind_call(&rig, &rig.token, json!({}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("path is required"), "{err}");
    assert!(err.contains("Usage of bind_chat_worktree"), "{err}");
    assert!(err.contains("example:"), "{err}");
}

/// Engine assembly attaches the shared `DiffTargets` itself: a bind works
/// without any test-side `set_diff_targets`.
#[tokio::test(flavor = "multi_thread")]
async fn bind_chat_worktree_is_wired_on_the_assembled_engine() {
    let rig = bind_rig(false);
    let reply = bind_ok(&rig, &rig.sub_wt, "k");
    assert_eq!(reply["selected"], true);
    assert_eq!(reply["path"], text(&rig.sub_wt));
}

#[tokio::test(flavor = "multi_thread")]
async fn bind_chat_worktree_refuses_a_chat_hosted_on_another_device() {
    let rig = bind_rig(true);
    // A Space on another device: its chats are hosted there.
    rig.core
        .workspace
        .create_space("space-b", "dev-b", &text(&rig.ws), None, true)
        .unwrap();
    rig.core
        .workspace
        .create_chat(
            "chat-remote",
            Some("space-b"),
            Some("dev-b"),
            None,
            Some(text(&rig.ws)),
        )
        .unwrap();
    let lease = rig
        .core
        .sessions
        .agent_tokens()
        .mint("chat-remote", "run-2", "space-b", "dev-b");
    let err = bind_call(
        &rig,
        lease.token(),
        json!({ "path": text(&rig.sub_wt), "idempotencyKey": "k" }),
    )
    .unwrap_err();
    assert_eq!(err.code(), "forbidden", "{err}");
}

/// What a coordinator's resolve of its child's request must leave behind.
fn request_is_resolved(snapshot: &WorkOutlineSnapshot, id: &str) -> bool {
    snapshot
        .user_action_requests
        .iter()
        .find(|r| r.id == id)
        .is_some_and(|r| r.state == mast_proto::UserActionRequestState::Resolved)
}

fn open_request_ids(read: &Value) -> Vec<String> {
    fn walk(node: &Value, out: &mut Vec<String>) {
        for request in node["openUserActionRequests"]
            .as_array()
            .into_iter()
            .flatten()
        {
            out.push(request["id"].as_str().unwrap_or_default().to_owned());
        }
        for child in node["children"].as_array().into_iter().flatten() {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(&read["tree"], &mut out);
    out
}

/// A child's request is raised with a `dedupeKey` (the tool guidance tells
/// agents to pass one per topic). The coordinator's resolve used to select the
/// dedupe group by the CALLER's Chat id, so the child's request fell out of
/// its own group, nothing was written, and the reply still said `resolved`.
#[tokio::test(flavor = "multi_thread")]
async fn a_coordinator_resolving_its_childs_deduped_request_persists_across_reads_and_restart() {
    let rig = rig().await;
    let root = create_item(&rig, "op-root", "Coordinator", None).await;
    let leaf = create_item(&rig, "op-leaf", "Leaf", Some(&root)).await;
    assign_chat(&rig, "assign-caller", CHAT, &root).await;
    let tokens = rig.core.sessions.agent_tokens();
    let coordinator = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = coordinator.token().to_owned();

    let launched = tool(
        &rig,
        &token,
        "chat_launch_child_for_item",
        launch_args("leaf-worker", &leaf, "workspace-write"),
    )
    .await
    .unwrap();
    let child = launched["chatId"].as_str().unwrap().to_owned();
    let child_lease = tokens.mint(&child, "run-2", SPACE, &rig.core.device_id);
    let raised = tool(
        &rig,
        child_lease.token(),
        "work_outline_create_user_action_request",
        json!({ "request": "Ready to merge: leaf", "dedupeKey": "ready-leaf",
                "idempotencyKey": "ready" }),
    )
    .await
    .unwrap();
    let request_id = raised["userActionRequestId"].as_str().unwrap().to_owned();
    let open = |read: &Value| open_request_ids(read).contains(&request_id);
    let read = tool(&rig, &token, "work_outline_read_subtree", json!({}))
        .await
        .unwrap();
    assert!(open(&read), "the child's request starts open: {read}");

    // An unrelated Chat is refused loudly and the request stays open.
    let stranger = tokens.mint(OTHER_CHAT, "run-3", SPACE, &rig.core.device_id);
    let err = tool(
        &rig,
        stranger.token(),
        "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": request_id, "idempotencyKey": "close-other" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    assert!(!request_is_resolved(&snapshot(&rig), &request_id));

    let reply = tool(
        &rig,
        &token,
        "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": request_id, "resolution": "Merged",
                "idempotencyKey": "close-parent" }),
    )
    .await
    .unwrap();
    assert_eq!(reply["resolved"], true);

    // The reply is backed by the row, by the subtree read, and by the row's
    // own fields.
    assert!(request_is_resolved(&snapshot(&rig), &request_id));
    let read = tool(&rig, &token, "work_outline_read_subtree", json!({}))
        .await
        .unwrap();
    assert!(!open(&read), "still open after the resolve: {read}");
    let row = rig
        .core
        .workspace
        .registry_row("userActionRequests", &request_id)
        .expect("the request row exists");
    assert_eq!(row.fields.get("state"), Some(&json!("resolved")), "{row:?}");
    assert_eq!(rig.core.workspace.registry_pending_len(), 0);

    // Re-assembling the engine from the same profile keeps it resolved.
    rig.harness.release.notify_waiters();
    let Rig {
        _tmp, core, client, ..
    } = rig;
    drop(client);
    // A graceful restart flushes the debounced registry snapshot first.
    core.workspace.shutdown();
    drop(core);
    let root_dir = std::fs::canonicalize(_tmp.path()).unwrap();
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(GatedHarness::default()));
    let again = EngineCore::assemble(
        &root_dir.join("data"),
        Arc::new(registry),
        HarnessId::Mock,
        None,
    )
    .expect("engine core re-assembles from the same profile");
    let inputs = again.workspace.work_outline_inputs(SPACE);
    let after = mast_doc::materialize_work_outline(SPACE, &inputs);
    assert!(
        request_is_resolved(&after, &request_id),
        "the resolution did not survive a restart"
    );
}

// ---- forgiving calls ----------------------------------------------------------

/// Replays of the calls agents got wrong on their first try (Claude Code loads
/// MCP tools lazily, so they guess parameters): missing keys on naturally
/// idempotent tools and guessed field names now succeed; the create-style
/// tools name the missing field and show an example.
#[tokio::test(flavor = "multi_thread")]
async fn first_guess_calls_succeed_or_say_exactly_what_to_pass() {
    let rig = rig().await;
    let root = create_item(&rig, "op-root", "Feature", None).await;
    let lease = rig
        .core
        .sessions
        .agent_tokens()
        .mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();
    let call = |name: &'static str, arguments: Value| {
        let (rig, token) = (&rig, token.clone());
        async move { tool(rig, &token, name, arguments).await }
    };

    // associate_current_chat: no key, `workItem` for `workItemId`.
    call(
        "work_outline_associate_current_chat",
        json!({ "workItem": root }),
    )
    .await
    .unwrap();
    assert_eq!(
        snapshot(&rig)
            .assignments
            .iter()
            .find(|a| a.chat_id == CHAT)
            .and_then(|a| a.work_item_id.clone())
            .as_deref(),
        Some(root.as_str())
    );
    // The same call again is fine.
    call(
        "work_outline_associate_current_chat",
        json!({ "itemId": root }),
    )
    .await
    .unwrap();

    // work_outline_set_status: no key; completing, reopening and completing
    // again each take effect (a derived key must not replay a stale result).
    let status = |rig: &Rig| snapshot(rig).item(&root).unwrap().status;
    for (wanted, expected) in [
        ("completed", mast_proto::WorkItemStatus::Completed),
        ("active", mast_proto::WorkItemStatus::Active),
        ("completed", mast_proto::WorkItemStatus::Completed),
        ("completed", mast_proto::WorkItemStatus::Completed),
    ] {
        // Updates are stamped with the clock; keep the stamps apart.
        tokio::time::sleep(Duration::from_millis(3)).await;
        call("work_outline_set_status", json!({ "status": wanted }))
            .await
            .unwrap();
        assert_eq!(status(&rig), expected, "after {wanted}");
    }
    // Also by an explicit item and a snake_case key.
    call(
        "work_outline_set_status",
        json!({ "status": "active", "work_item_id": root, "idempotency_key": "reopen" }),
    )
    .await
    .unwrap();
    assert_eq!(status(&rig), mast_proto::WorkItemStatus::Active);

    // create_user_action_request: `text` for `request`, a dedupeKey as the key.
    let created = call(
        "work_outline_create_user_action_request",
        json!({ "text": "Which branch?", "detail": "main or release", "dedupeKey": "branch" }),
    )
    .await
    .unwrap();
    let request_id = created["userActionRequestId"].as_str().unwrap().to_owned();
    // Repeating the call returns the same open request, not a duplicate.
    let repeated = call(
        "work_outline_create_user_action_request",
        json!({ "title": "Which branch?", "detail": "main or release", "dedupeKey": "branch" }),
    )
    .await
    .unwrap();
    assert_eq!(repeated["userActionRequestId"], request_id.as_str());
    assert_eq!(
        snapshot(&rig)
            .user_action_requests
            .iter()
            .filter(|r| r.is_open() && !r.deduped)
            .count(),
        1
    );
    // `question` works as well, with an explicit key.
    let other = call(
        "work_outline_create_user_action_request",
        json!({ "question": "Ship it?", "idempotencyKey": "ship" }),
    )
    .await
    .unwrap();
    assert_ne!(other["userActionRequestId"], request_id.as_str());

    // resolve: `id` for `userActionRequestId`, no key; a retry is fine.
    for _ in 0..2 {
        call(
            "work_outline_resolve_user_action_request",
            json!({ "id": request_id, "resolution": "main" }),
        )
        .await
        .unwrap();
    }
    assert!(request_is_resolved(&snapshot(&rig), &request_id));
    call(
        "work_outline_resolve_user_action_request",
        json!({ "requestId": other["userActionRequestId"] }),
    )
    .await
    .unwrap();

    // The same dedupeKey after a resolve is a new request, not a replay.
    let reasked = call(
        "work_outline_create_user_action_request",
        json!({ "request": "Which branch?", "detail": "main or release", "dedupeKey": "branch" }),
    )
    .await
    .unwrap();
    assert_ne!(reasked["userActionRequestId"], request_id.as_str());
    assert!(!request_is_resolved(
        &snapshot(&rig),
        reasked["userActionRequestId"].as_str().unwrap()
    ));

    // Create-style tools keep their key but say exactly what is missing.
    let err = call(
        "work_outline_create_user_action_request",
        json!({ "request": "No key, no dedupeKey" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("idempotencyKey is required"), "{err}");
    assert!(err.contains("dedupeKey"), "{err}");
    assert!(
        err.contains("Usage of work_outline_create_user_action_request"),
        "{err}"
    );
    assert!(err.contains("example:"), "{err}");
    let err = call("work_outline_create_user_action_request", json!({}))
        .await
        .unwrap_err();
    assert!(err.contains("missing field `request`"), "{err}");
    assert!(err.contains("required: request"), "{err}");
    assert!(err.contains("optional: kind"), "{err}");
    assert!(err.contains("detail (context for the user)"), "{err}");
    assert!(err.contains("example: {\"request\""), "{err}");
    let err = call("work_outline_create_child", json!({ "title": "x" }))
        .await
        .unwrap_err();
    assert!(err.contains("idempotencyKey is required"), "{err}");
    assert!(err.contains("Usage of work_outline_create_child"), "{err}");
    let err = call(
        "work_outline_propose_instruction",
        json!({ "idempotencyKey": "k" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("text is empty"), "{err}");
    assert!(
        err.contains("Usage of work_outline_propose_instruction"),
        "{err}"
    );
    let err = call("work_outline_resolve_user_action_request", json!({}))
        .await
        .unwrap_err();
    assert!(err.contains("missing field `userActionRequestId`"), "{err}");
    assert!(err.contains("example:"), "{err}");
    let err = call("work_outline_set_status", json!({ "status": "done" }))
        .await
        .unwrap_err();
    assert!(err.contains("status must be active or completed"), "{err}");
    assert!(err.contains("Usage of work_outline_set_status"), "{err}");

    // `parentId` stands for `parentWorkItemId` on create_child; the key stays
    // required, and `idempotency_key` is accepted for it.
    let child = call(
        "work_outline_create_child",
        json!({ "title": "Child", "parentId": root, "idempotency_key": "child-1" }),
    )
    .await
    .unwrap();
    assert_eq!(child["parentWorkItemId"], root.as_str());
    // A retry with the snake_case key replays the same child.
    let retry = call(
        "work_outline_create_child",
        json!({ "title": "Child", "parent": root, "idempotencyKey": "child-1" }),
    )
    .await
    .unwrap();
    assert_eq!(retry["workItemId"], child["workItemId"]);
}

// ---- project scope and worker destination ------------------------------------

async fn update_item(rig: &Rig, op: &str, item: &str, changes: Value) -> Result<Value, String> {
    rig.client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "updateWorkItem", "workItemId": item, "changes": changes,
                    "operationId": op }),
        )
        .await
        .map_err(|e| e.to_string())
}

/// `Project` (isProject, destination = Tickets) with `Planning` and `Tickets`
/// children, a second project `Other` with its own `Other tickets`, and a
/// bystander root.
struct ProjectRig {
    project: String,
    planning: String,
    tickets: String,
    ticket: String,
    other: String,
    other_tickets: String,
    bystander: String,
}

async fn project_rig(rig: &Rig) -> ProjectRig {
    let project = create_item(rig, "p-proj", "Project", None).await;
    let planning = create_item(rig, "p-plan", "Planning", Some(&project)).await;
    let tickets = create_item(rig, "p-tix", "Tickets", Some(&project)).await;
    let ticket = create_item(rig, "p-t1", "Ticket 1", Some(&tickets)).await;
    let other = create_item(rig, "p-other", "Other", None).await;
    let other_tickets = create_item(rig, "p-otix", "Other tickets", Some(&other)).await;
    let bystander = create_item(rig, "p-by", "Bystander", None).await;
    update_item(rig, "p-mark", &project, json!({ "isProject": true }))
        .await
        .unwrap();
    update_item(
        rig,
        "p-dest",
        &project,
        json!({ "workerDestination": tickets }),
    )
    .await
    .unwrap();
    update_item(rig, "p-mark2", &other, json!({ "isProject": true }))
        .await
        .unwrap();
    ProjectRig {
        project,
        planning,
        tickets,
        ticket,
        other,
        other_tickets,
        bystander,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hand_off_refuses_different_projects_in_the_same_space() {
    let rig = rig().await;
    let projects = project_rig(&rig).await;
    assign_chat(&rig, "handoff-project-a", CHAT, &projects.planning).await;
    assign_chat(&rig, "handoff-project-b", OTHER_CHAT, &projects.other_tickets).await;
    let wakes = mast_engine::orchestrator_wakes::OrchestratorWakes::new(
        rig.core.sessions.clone(), rig.core.doc_host.clone(), rig.core.workspace.clone(), None, Arc::new(|| 1_000_000),
    );
    let error = wakes.hand_off_orchestrator(CHAT, OTHER_CHAT, CHAT).await.unwrap_err();
    assert!(error.contains("same project"), "{error}");
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn project_destination_worker_prompt_wakes_without_watcher_delay() {
    let rig = rig().await;
    let project = project_rig(&rig).await;
    assign_chat(&rig, "wake-orchestrator", CHAT, &project.planning).await;
    assign_chat(&rig, "wake-worker", OTHER_CHAT, &project.ticket).await;
    let wakes = mast_engine::orchestrator_wakes::OrchestratorWakes::new(
        rig.core.sessions.clone(), rig.core.doc_host.clone(), rig.core.workspace.clone(), None, Arc::new(|| 1_000_000),
    );
    wakes.ingest_worker_signal(mast_engine::sessions::WorkerSignal::Input {
        chat_id: OTHER_CHAT.into(), input: mast_proto::AwaitingInput { request_id: "r1".into(), questions: vec![mast_proto::UserInputQuestion {
            command_approval: None,
            allow_custom: false,
            id: "q1".into(), header: "Approval".into(), question: "Run build?".into(), options: vec![], multi_select: false,
        }] },
    }).await;
    let held = wakes.status(CHAT).held;
    assert_eq!(held.len(), 1);
    assert!(held[0].summary.contains("waiting for approval: Run build?"));
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_records_its_worker_destination_and_validates_it() {
    let rig = rig().await;
    let p = project_rig(&rig).await;
    let snap = snapshot(&rig);
    let project = snap.item(&p.project).unwrap();
    assert!(project.is_project);
    assert_eq!(project.worker_destination_id.as_deref(), Some(p.tickets.as_str()));
    assert!(!snap.item(&p.planning).unwrap().is_project);
    // A destination must be inside the project, and needs a project.
    for (op, item, dest) in [
        ("bad-1", &p.project, &p.other_tickets),
        ("bad-2", &p.project, &p.bystander),
        ("bad-3", &p.planning, &p.ticket),
    ] {
        let error = update_item(&rig, op, item, json!({ "workerDestination": dest }))
            .await
            .unwrap_err();
        assert!(error.contains("invalid"), "{error}");
    }
    // Unmarking keeps clearing possible; clearing works.
    update_item(&rig, "clear", &p.project, json!({ "workerDestination": null }))
        .await
        .unwrap();
    assert_eq!(snapshot(&rig).item(&p.project).unwrap().worker_destination_id, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_planning_chat_creates_workers_in_the_project_destination() {
    let rig = rig().await;
    let p = project_rig(&rig).await;
    assign_chat(&rig, "assign-plan", CHAT, &p.planning).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();

    // No parent: the project's destination, though Tickets is a sibling.
    let made = tool(
        &rig,
        &token,
        "work_outline_create_child",
        json!({ "title": "ENG-1", "idempotencyKey": "k1" }),
    )
    .await
    .unwrap();
    assert_eq!(made["parentWorkItemId"], p.tickets);
    assert_eq!(made["resolvedToWorkerDestination"], true);
    // read_current tells the chat (and create_chat) where workers go.
    let current = tool(&rig, &token, "work_outline_read_current", json!({}))
        .await
        .unwrap();
    assert_eq!(current["workerDestinationId"], p.tickets);
    // An explicit parent wins: the Planning item itself, a sibling, a ticket.
    for (key, parent) in [("k2", &p.planning), ("k3", &p.tickets), ("k4", &p.ticket)] {
        let made = tool(
            &rig,
            &token,
            "work_outline_create_child",
            json!({ "title": "x", "idempotencyKey": key, "parentWorkItemId": parent }),
        )
        .await
        .unwrap();
        assert_eq!(made["parentWorkItemId"], *parent);
        assert_eq!(made["resolvedToWorkerDestination"], false);
    }
    // The project scope ends at the project: nothing outside it is reachable.
    for (key, parent) in [
        ("k5", &p.other),
        ("k6", &p.other_tickets),
        ("k7", &p.bystander),
    ] {
        let error = tool(
            &rig,
            &token,
            "work_outline_create_child",
            json!({ "title": "x", "idempotencyKey": key, "parentWorkItemId": parent }),
        )
        .await
        .unwrap_err();
        assert!(error.contains("unauthorized"), "{key}: {error}");
    }
    // Reads follow the same scope; the project's siblings stay hidden.
    let read = tool(
        &rig,
        &token,
        "work_outline_read_subtree",
        json!({ "workItemId": p.tickets }),
    )
    .await
    .unwrap();
    assert_eq!(read["workItemId"], p.tickets);
    let error = tool(
        &rig,
        &token,
        "work_outline_read_subtree",
        json!({ "workItemId": p.other_tickets }),
    )
    .await
    .unwrap_err();
    assert!(error.contains("unauthorized"), "{error}");
    // Everything else keeps the own-item rule: no status change on a sibling.
    let error = tool(
        &rig,
        &token,
        "work_outline_set_status",
        json!({ "status": "active", "workItemId": p.tickets, "idempotencyKey": "s1" }),
    )
    .await
    .unwrap_err();
    assert!(error.contains("unauthorized"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_project_the_scope_is_still_own_item_and_descendants() {
    let rig = rig().await;
    let root = create_item(&rig, "n-root", "Coordinator", None).await;
    let step = create_item(&rig, "n-step", "Step", Some(&root)).await;
    let sibling = create_item(&rig, "n-sib", "Sibling", Some(&root)).await;
    assign_chat(&rig, "assign-step", CHAT, &step).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();
    let made = tool(
        &rig,
        &token,
        "work_outline_create_child",
        json!({ "title": "x", "idempotencyKey": "k1" }),
    )
    .await
    .unwrap();
    assert_eq!(made["parentWorkItemId"], step);
    assert_eq!(made["resolvedToWorkerDestination"], false);
    let error = tool(
        &rig,
        &token,
        "work_outline_create_child",
        json!({ "title": "x", "idempotencyKey": "k2", "parentWorkItemId": sibling }),
    )
    .await
    .unwrap_err();
    assert!(error.contains("unauthorized"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worker_inside_the_destination_keeps_its_sub_items_under_itself() {
    let rig = rig().await;
    let p = project_rig(&rig).await;
    assign_chat(&rig, "assign-worker", CHAT, &p.ticket).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();
    let made = tool(
        &rig,
        &token,
        "work_outline_create_child",
        json!({ "title": "sub", "idempotencyKey": "k1" }),
    )
    .await
    .unwrap();
    assert_eq!(made["parentWorkItemId"], p.ticket);
    assert_eq!(made["resolvedToWorkerDestination"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_chat_for_a_project_starts_in_its_destination() {
    let rig = rig().await;
    let p = project_rig(&rig).await;
    assign_chat(&rig, "assign-plan", CHAT, &p.planning).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();

    let key = "to-project";
    tool(
        &rig,
        &token,
        "chat_launch_child_for_item",
        launch_args(key, &p.project, "workspace-write"),
    )
    .await
    .unwrap();
    let child = derive_id(
        &mast_proto::agent_tools::operation_id(CHAT, key),
        roles::CHAT,
    );
    let snap = snapshot(&rig);
    let assignment = snap.assignments.iter().find(|a| a.chat_id == child).unwrap();
    assert_eq!(assignment.work_item_id.as_deref(), Some(p.tickets.as_str()));
    // A named item inside the project (the Planning item's sibling ticket) wins.
    tool(
        &rig,
        &token,
        "chat_launch_child_for_item",
        launch_args("to-ticket", &p.ticket, "workspace-write"),
    )
    .await
    .unwrap();
    let child = derive_id(
        &mast_proto::agent_tools::operation_id(CHAT, "to-ticket"),
        roles::CHAT,
    );
    let snap = snapshot(&rig);
    let assignment = snap.assignments.iter().find(|a| a.chat_id == child).unwrap();
    assert_eq!(assignment.work_item_id.as_deref(), Some(p.ticket.as_str()));
    // Outside the project: refused.
    for (key, item) in [("out-1", &p.other_tickets), ("out-2", &p.bystander)] {
        let error = tool(
            &rig,
            &token,
            "chat_launch_child_for_item",
            launch_args(key, item, "workspace-write"),
        )
        .await
        .unwrap_err();
        assert!(error.contains("unauthorized"), "{key}: {error}");
    }
}

// ---- project templates ---------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn saving_a_project_template_freezes_the_subtree_and_agents_cannot() {
    let rig = rig().await;
    let p = project_rig(&rig).await;
    add_instruction(&rig, "tpl-i1", json!({"type":"workItem","workItemId": p.project}), "Be careful").await;
    add_instruction(&rig, "tpl-i2", json!({"type":"workItem","workItemId": p.planning}), "Write a plan").await;
    add_instruction(&rig, "tpl-i3", json!({"type":"workItem","workItemId": p.ticket}), "ticket only").await;
    let reply = rig
        .client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "saveWorkItemTemplate", "spaceId": SPACE,
                    "projectWorkItemId": p.project, "name": "Standard", "operationId": "tpl-save" }),
        )
        .await
        .unwrap();
    let template_id = derive_id("tpl-save", roles::TEMPLATE);
    assert_eq!(reply["templateId"], json!(template_id));

    let snap = snapshot(&rig);
    assert_eq!(snap.templates.len(), 1);
    let template = &snap.templates[0];
    assert_eq!((template.name.as_str(), template.revision), ("Standard", 1));
    let body = &template.body;
    assert!(body.is_project);
    assert_eq!(body.instructions, vec!["Be careful"]);
    let titles: Vec<&str> = body.children.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["Planning", "Tickets"]);
    assert_eq!(body.children[0].instructions, vec!["Write a plan"]);
    assert!(body.children[1].is_worker_destination);
    assert!(body.children[1].children.is_empty(), "tickets are dropped");
    let item = snap.item(&p.project).unwrap();
    assert_eq!(item.template_id.as_deref(), Some(template_id.as_str()));
    assert_eq!(item.template_revision, Some(1));

    // A retry with the same key replays; a re-save bumps the revision.
    let replay = rig
        .client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "saveWorkItemTemplate", "spaceId": SPACE,
                    "projectWorkItemId": p.project, "name": "Standard", "operationId": "tpl-save" }),
        )
        .await
        .unwrap();
    assert_eq!(replay["templateId"], json!(template_id));
    rig.client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "saveWorkItemTemplate", "spaceId": SPACE, "projectWorkItemId": p.project,
                    "name": "Standard", "templateId": template_id, "operationId": "tpl-resave" }),
        )
        .await
        .unwrap();
    assert_eq!(snapshot(&rig).templates[0].revision, 2);

    // The WORK_OUTLINE_MUTATE path reports a duplicate name as invalid.
    let duplicate = rig
        .client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "saveWorkItemTemplate", "spaceId": SPACE, "projectWorkItemId": p.other,
                    "name": "standard", "operationId": "tpl-dup" }),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(duplicate.contains("invalid"), "{duplicate}");

    // Deleting tombstones it; the project keeps its link.
    rig.client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "deleteWorkItemTemplate", "templateId": template_id,
                    "operationId": "tpl-del" }),
        )
        .await
        .unwrap();
    let snap = snapshot(&rig);
    assert!(snap.templates.is_empty());
    assert_eq!(snap.item(&p.project).unwrap().template_id.as_deref(), Some(template_id.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn propose_edit_and_remove_target_instructions_the_chat_can_read() {
    let rig = rig().await;
    let root_a = create_item(&rig, "op-a", "Feature A", None).await;
    let root_b = create_item(&rig, "op-b", "Feature B", None).await;
    add_instruction(
        &rig,
        "ins-a",
        json!({ "type": "workItem", "workItemId": root_a }),
        "Run the linter.",
    )
    .await;
    add_instruction(
        &rig,
        "ins-b",
        json!({ "type": "workItem", "workItemId": root_b }),
        "Elsewhere rule.",
    )
    .await;
    assign_chat(&rig, "assign-caller", CHAT, &root_a).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();

    // read_current lists instruction ids with their text.
    let read = tool(&rig, &token, "work_outline_read_current", json!({}))
        .await
        .unwrap();
    let entry = &read["effectiveInstructions"]["entries"][0];
    assert_eq!(entry["text"], "Run the linter.");
    let id = entry["instructionId"].as_str().unwrap().to_owned();
    let foreign = snapshot(&rig)
        .instructions
        .iter()
        .find(|i| i.text == "Elsewhere rule.")
        .unwrap()
        .id
        .clone();

    let edit = tool(
        &rig,
        &token,
        "work_outline_propose_instruction",
        json!({ "action": "edit", "instructionId": id, "text": "Run the linter and tests.",
                "rationale": "CI runs both", "idempotencyKey": "e1" }),
    )
    .await
    .unwrap();
    assert_eq!(edit["action"], "edit");
    let proposal_id = edit["proposalId"].as_str().unwrap().to_owned();
    let snap = snapshot(&rig);
    let proposal = snap.instruction_proposal(&proposal_id).unwrap();
    assert_eq!(proposal.base_text.as_deref(), Some("Run the linter."));
    assert_eq!(proposal.rationale.as_deref(), Some("CI runs both"));
    assert_eq!(snap.instructions.len(), 2, "nothing changes until accepted");

    let remove = tool(
        &rig,
        &token,
        "work_outline_propose_instruction",
        json!({ "action": "remove", "instructionId": id, "rationale": "obsolete",
                "idempotencyKey": "r1" }),
    )
    .await
    .unwrap();
    assert_eq!(remove["action"], "remove");

    // Validation: unknown id, an id outside the chat's reach, missing id,
    // remove with text, edit without text.
    let err = tool(
        &rig,
        &token,
        "work_outline_propose_instruction",
        json!({ "action": "edit", "instructionId": "nope", "text": "x", "idempotencyKey": "v1" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("not found") || err.contains("notFound"), "{err}");
    let err = tool(
        &rig,
        &token,
        "work_outline_propose_instruction",
        json!({ "action": "remove", "instructionId": foreign, "idempotencyKey": "v2" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("nauthorized") || err.contains("inherit"), "{err}");
    for (key, args) in [
        ("v3", json!({ "action": "edit", "text": "x" })),
        ("v4", json!({ "action": "remove", "instructionId": id, "text": "x" })),
        ("v5", json!({ "action": "edit", "instructionId": id })),
        ("v6", json!({ "action": "bogus", "instructionId": id })),
    ] {
        let mut args = args;
        args["idempotencyKey"] = json!(key);
        let err = tool(&rig, &token, "work_outline_propose_instruction", args)
            .await
            .unwrap_err();
        assert!(err.contains("agent-tools/invalid"), "{key}: {err}");
    }

    // Accepting the edit rewrites the same instruction.
    rig.client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "approveWorkItemInstructionProposal", "proposalId": proposal_id,
                    "approvedText": "Run the linter and tests.", "operationId": "approve-1" }),
        )
        .await
        .unwrap();
    let snap = snapshot(&rig);
    let instruction = snap.instructions.iter().find(|i| i.id == id).unwrap();
    assert_eq!(instruction.text, "Run the linter and tests.");
    assert_eq!(snap.instructions.len(), 2);
}

// ---- answering a request with the user's own message -------------------------------

const OUTSIDER: &str = "chat-outsider";

struct AnswerRig {
    rig: Rig,
    p: ProjectRig,
    /// The orchestrator's token (CHAT, on the project item).
    orchestrator: String,
    /// The planner's token (OTHER_CHAT, on Planning).
    planner: String,
    /// A chat on another project's item.
    outsider: String,
    request: String,
    /// Tokens stop working when their lease drops.
    _leases: [mast_engine::agent_tools::AgentTokenLease; 3],
}

async fn answer_rig() -> AnswerRig {
    let rig = rig().await;
    let p = project_rig(&rig).await;
    rig.client
        .call(
            methods::MUTATE,
            json!({ "op": "createChat", "chatId": OUTSIDER, "spaceId": SPACE,
                    "deviceId": rig.core.device_id }),
        )
        .await
        .unwrap();
    assign_chat(&rig, "assign-orch", CHAT, &p.project).await;
    assign_chat(&rig, "assign-plan", OTHER_CHAT, &p.planning).await;
    assign_chat(&rig, "assign-out", OUTSIDER, &p.other_tickets).await;
    let tokens = rig.core.sessions.agent_tokens();
    let leases = [
        tokens.mint(CHAT, "run-o", SPACE, &rig.core.device_id),
        tokens.mint(OTHER_CHAT, "run-p", SPACE, &rig.core.device_id),
        tokens.mint(OUTSIDER, "run-x", SPACE, &rig.core.device_id),
    ];
    let [orchestrator, planner, outsider] = [0, 1, 2].map(|i| leases[i].token().to_owned());
    let raised = tool(
        &rig,
        &planner,
        "work_outline_create_user_action_request",
        json!({ "request": "Approve plan and create tickets?", "dedupeKey": "approve-plan",
                "idempotencyKey": "raise" }),
    )
    .await
    .unwrap();
    let request = raised["userActionRequestId"].as_str().unwrap().to_owned();
    AnswerRig {
        rig,
        p,
        orchestrator,
        planner,
        outsider,
        request,
        _leases: leases,
    }
}

fn request_created_at(rig: &Rig, request: &str) -> i64 {
    snapshot(rig)
        .user_action_requests
        .iter()
        .find(|r| r.id == request)
        .unwrap()
        .created_at
}

/// Write a transcript message into `chat`'s doc.
fn push_message(rig: &Rig, chat: &str, id: &str, role: mast_doc::MessageRole, text: &str, at: i64) {
    rig.core
        .doc_host
        .open(chat)
        .unwrap()
        .doc()
        .push_message(&mast_doc::SessionMessageEntry {
            id: id.into(),
            role,
            parts: vec![mast_doc::MessagePart::Text {
                id: format!("{id}-part"),
                text: text.into(),
            }],
            created_at: at,
            device_id: rig.core.device_id.clone(),
            status: None,
            continuation_of: None,
        })
        .unwrap();
}

/// Every text delivered to `chat` (queued rows and transcript messages).
fn delivered_texts(rig: &Rig, chat: &str) -> Vec<String> {
    let handle = rig.core.doc_host.open(chat).unwrap();
    let mut out: Vec<String> = handle
        .doc()
        .read_queue()
        .unwrap()
        .into_iter()
        .map(|q| q.text)
        .collect();
    // An idle chat drains the row straight into a Run command.
    for command in handle.doc().read_commands().unwrap() {
        if let SessionCommandPayload::Run { request, .. } = command.payload {
            out.push(request.prompt);
        }
    }
    for entry in handle.doc().read_entries().unwrap() {
        for part in entry.parts {
            if let mast_doc::MessagePart::Text { text, .. } = part {
                out.push(text);
            }
        }
    }
    out
}

/// Every Re: reply the planner received, whether still queued, drained into a
/// command, or already started as a Run (polled: an idle chat drains async).
async fn replies_to_planner(rig: &Rig) -> Vec<String> {
    let snapshot_of = || -> Vec<String> {
        // The same message shows in the doc and as the Run the harness got,
        // so take whichever view has more rather than adding them.
        let in_doc: Vec<String> = delivered_texts(rig, OTHER_CHAT)
            .into_iter()
            .filter(|t| t.starts_with("Re:"))
            .collect();
        let started: Vec<String> = rig
            .harness
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.prompt.clone())
            .filter(|t| t.starts_with("Re:"))
            .collect();
        if started.len() > in_doc.len() {
            started
        } else {
            in_doc
        }
    };
    for _ in 0..200 {
        if !snapshot_of().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // Let a duplicate delivery, if any, show up before counting.
    tokio::time::sleep(Duration::from_millis(300)).await;
    snapshot_of()
}

fn answer_args(request: &str, message: &str, key: &str) -> Value {
    json!({ "requestId": request, "userMessageId": message, "idempotencyKey": key })
}

const ANSWER: &str = "answer_user_action_request";

#[tokio::test(flavor = "multi_thread")]
async fn answering_with_the_users_own_message_delivers_it_and_resolves_as_theirs() {
    let a = answer_rig().await;
    let at = request_created_at(&a.rig, &a.request) + 1_000;
    push_message(
        &a.rig,
        CHAT,
        "u-1",
        mast_doc::MessageRole::User,
        "  make linear tickets  ",
        at,
    );
    let reply = tool(
        &a.rig,
        &a.orchestrator,
        ANSWER,
        answer_args(&a.request[..8], "u-1", "ans-1"),
    )
    .await
    .unwrap();
    assert_eq!(reply["resolved"], true);
    assert_eq!(reply["answeredBy"], "user");
    assert_eq!(reply["delivered"], true);
    assert_eq!(reply["sourceChatId"], CHAT);
    assert_eq!(reply["sourceMessageId"], "u-1");

    // The planner gets the user's reply, not a relay.
    let expected = "Re: \"Approve plan and create tickets?\": make linear tickets";
    let texts = replies_to_planner(&a.rig).await;
    assert_eq!(texts, [expected], "{texts:?}");

    // The row records the user's answer with its source.
    let snap = snapshot(&a.rig);
    let request = snap
        .user_action_requests
        .iter()
        .find(|r| r.id == a.request)
        .unwrap();
    let resolution = request.resolution.as_ref().unwrap();
    assert!(resolution.answered_by_user());
    assert_eq!(resolution.note.as_deref(), Some("make linear tickets"));
    let answer = resolution.answer.as_ref().unwrap();
    assert_eq!(answer.chat_id, CHAT);
    assert_eq!(answer.message_id, "u-1");
    assert_eq!(answer.text, "make linear tickets");
    assert!(request_is_resolved(&snap, &a.request));

    // A replay with the same key neither delivers twice nor fails.
    let again = tool(
        &a.rig,
        &a.orchestrator,
        ANSWER,
        answer_args(&a.request[..8], "u-1", "ans-1"),
    )
    .await
    .unwrap();
    assert_eq!(again["resolved"], true);
    assert_eq!(replies_to_planner(&a.rig).await, [expected]);
}

#[tokio::test(flavor = "multi_thread")]
async fn answering_refuses_anything_but_a_human_message_newer_than_the_request() {
    let a = answer_rig().await;
    let created = request_created_at(&a.rig, &a.request);
    let later = created + 1_000;
    use mast_doc::MessageRole::{Assistant, System, User};
    push_message(&a.rig, CHAT, "m-assistant", Assistant, "go ahead", later);
    push_message(&a.rig, CHAT, "m-system", System, "go ahead", later);
    push_message(
        &a.rig,
        CHAT,
        "m-relay",
        User,
        "[Message from Mast chat Orchestrator (abc). Reply to it with the Mast `send_message` tool, chat abc.]\n\nMichael, verbatim: make linear tickets",
        later,
    );
    push_message(
        &a.rig,
        CHAT,
        "m-wake",
        User,
        "[Mast system message, not from the user: worker updates for you, the orchestrator.]\nWorker updates:\n- x",
        later,
    );
    push_message(&a.rig, CHAT, "m-old", User, "make linear tickets", created - 1_000);
    push_message(&a.rig, CHAT, "m-same", User, "make linear tickets", created);
    push_message(&a.rig, OTHER_CHAT, "m-foreign", User, "make linear tickets", later);
    let long = "x".repeat(1_001);
    push_message(&a.rig, CHAT, "m-long", User, &long, later);
    let cases = [
        ("m-assistant", "assistant message"),
        ("m-system", "system message"),
        ("m-relay", "from an agent or Mast"),
        ("m-wake", "from an agent or Mast"),
        ("m-old", "older than the request"),
        ("m-same", "older than the request"),
        // Another chat's message is not in the caller's own transcript.
        ("m-foreign", "not a message in this chat"),
        ("nope", "not a message in this chat"),
        ("m-long", "longer than"),
    ];
    for (i, (message, needle)) in cases.iter().enumerate() {
        let err = tool(
            &a.rig,
            &a.orchestrator,
            ANSWER,
            answer_args(&a.request, message, &format!("bad-{i}")),
        )
        .await
        .unwrap_err();
        assert!(err.contains(needle), "{message}: {err}");
    }
    // Nothing was delivered or resolved by any of those.
    assert!(!request_is_resolved(&snapshot(&a.rig), &a.request));
    assert!(
        delivered_texts(&a.rig, OTHER_CHAT)
            .iter()
            .all(|t| !t.starts_with("Re:"))
    );
    // The caller cannot supply the text itself.
    let err = tool(
        &a.rig,
        &a.orchestrator,
        ANSWER,
        json!({ "requestId": a.request, "userMessageId": "m-old", "text": "go",
                "idempotencyKey": "extra" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("older than the request"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn answering_refuses_resolved_and_out_of_reach_requests() {
    let a = answer_rig().await;
    let later = request_created_at(&a.rig, &a.request) + 1_000;
    push_message(&a.rig, CHAT, "u-1", mast_doc::MessageRole::User, "go", later);
    push_message(&a.rig, OUTSIDER, "u-x", mast_doc::MessageRole::User, "go", later);
    // A chat in another project cannot answer this project's request.
    let err = tool(
        &a.rig,
        &a.outsider,
        ANSWER,
        answer_args(&a.request, "u-x", "out-1"),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    assert!(!request_is_resolved(&snapshot(&a.rig), &a.request));
    // An unknown request.
    let err = tool(
        &a.rig,
        &a.orchestrator,
        ANSWER,
        answer_args("no-such-request", "u-1", "k-missing"),
    )
    .await
    .unwrap_err();
    assert!(err.contains("not found") || err.contains("notFound"), "{err}");
    // The planner's own request can be answered by the planner's own user
    // message too (its own chat is in reach), but only once.
    tool(
        &a.rig,
        &a.orchestrator,
        ANSWER,
        answer_args(&a.request, "u-1", "ans-1"),
    )
    .await
    .unwrap();
    let err = tool(
        &a.rig,
        &a.orchestrator,
        ANSWER,
        answer_args(&a.request, "u-1", "ans-2"),
    )
    .await
    .unwrap_err();
    assert!(err.contains("already resolved"), "{err}");
    // Only one reply ever reached the planner.
    assert_eq!(replies_to_planner(&a.rig).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn project_scope_reaches_other_chats_requests_and_nothing_wider() {
    let a = answer_rig().await;
    // The orchestrator did not create the planner, yet resolves its request.
    let reach = tool(&a.rig, &a.orchestrator, "chat_project_scope_chats", json!({}))
        .await
        .unwrap();
    let ids: Vec<&str> = reach["chatIds"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(ids.contains(&OTHER_CHAT) && ids.contains(&CHAT), "{ids:?}");
    assert!(!ids.contains(&OUTSIDER), "{ids:?}");
    // The planner is under the project too, but a chat outside it sees only
    // its own project.
    let out = tool(&a.rig, &a.outsider, "chat_project_scope_chats", json!({}))
        .await
        .unwrap();
    let out_ids: Vec<&str> = out["chatIds"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(out_ids.contains(&OUTSIDER) && !out_ids.contains(&OTHER_CHAT), "{out_ids:?}");
    // The outsider cannot resolve the planner's request.
    let err = tool(
        &a.rig,
        &a.outsider,
        "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": a.request, "idempotencyKey": "x-resolve" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    assert!(!request_is_resolved(&snapshot(&a.rig), &a.request));
    // The orchestrator can.
    let done = tool(
        &a.rig,
        &a.orchestrator,
        "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": a.request, "resolution": "handled",
                "idempotencyKey": "o-resolve" }),
    )
    .await
    .unwrap();
    assert_eq!(done["resolved"], true);
    let snap = snapshot(&a.rig);
    let resolution = snap
        .user_action_requests
        .iter()
        .find(|r| r.id == a.request)
        .unwrap()
        .resolution
        .clone()
        .unwrap();
    // A plain resolve is the agent's own, never marked as the user's.
    assert!(!resolution.answered_by_user());
    assert_eq!(resolution.by.kind, WorkOutlineActorKind::Agent);
    // The planner's own scope without a project keeps the old rule: a chat on
    // a project's Tickets item reaches only inside the project it belongs to
    // (the bystander has no project at all).
    let _ = (&a.planner, &a.p.bystander);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_without_a_project_keeps_its_own_subtree_reach() {
    let rig = rig().await;
    let root = create_item(&rig, "np-root", "Plain", None).await;
    let leaf = create_item(&rig, "np-leaf", "Leaf", Some(&root)).await;
    let sibling = create_item(&rig, "np-sib", "Sibling", None).await;
    assign_chat(&rig, "np-a", CHAT, &leaf).await;
    assign_chat(&rig, "np-b", OTHER_CHAT, &sibling).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let other = tokens.mint(OTHER_CHAT, "run-2", SPACE, &rig.core.device_id);
    let raised = tool(
        &rig,
        other.token(),
        "work_outline_create_user_action_request",
        json!({ "request": "Q?", "idempotencyKey": "q" }),
    )
    .await
    .unwrap();
    let request = raised["userActionRequestId"].as_str().unwrap().to_owned();
    let reach = tool(&rig, lease.token(), "chat_project_scope_chats", json!({}))
        .await
        .unwrap();
    assert_eq!(reach["chatIds"], json!([]));
    let err = tool(
        &rig,
        lease.token(),
        "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": request, "idempotencyKey": "r" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
}

/// A coordinator on a root item resolves requests raised by chats another
/// coordinator created, on descendant items; never on items outside its subtree.
#[tokio::test]
async fn resolve_reaches_requests_on_descendant_items_but_not_outside_the_subtree() {
    let rig = rig().await;
    let root = create_item(&rig, "rs-root", "Root", None).await;
    let feature = create_item(&rig, "rs-feature", "Feature", Some(&root)).await;
    let outside = create_item(&rig, "rs-outside", "Outside", None).await;
    assign_chat(&rig, "rs-a", CHAT, &root).await;
    assign_chat(&rig, "rs-b", OTHER_CHAT, &outside).await;
    // Two chats created by chat B (not by A), one under A's subtree, one outside.
    for (chat, item, op) in [
        ("chat-rs-inside", &feature, "rs-c1"),
        ("chat-rs-outside", &outside, "rs-c2"),
    ] {
        rig.client
            .call(
                methods::MUTATE,
                json!({ "op": "createChat", "chatId": chat, "spaceId": SPACE,
                        "deviceId": rig.core.device_id, "parentChatId": OTHER_CHAT }),
            )
            .await
            .unwrap();
        assign_chat(&rig, op, chat, item).await;
    }
    let tokens = rig.core.sessions.agent_tokens();
    let a = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let mut ids = Vec::new();
    for (chat, key) in [("chat-rs-inside", "q-in"), ("chat-rs-outside", "q-out")] {
        let lease = tokens.mint(chat, "run-x", SPACE, &rig.core.device_id);
        let raised = tool(
            &rig,
            lease.token(),
            "work_outline_create_user_action_request",
            json!({ "request": "Q?", "idempotencyKey": key }),
        )
        .await
        .unwrap();
        ids.push(raised["userActionRequestId"].as_str().unwrap().to_owned());
    }
    let err = tool(
        &rig,
        a.token(),
        "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": ids[1], "idempotencyKey": "r-out" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("work-outline/unauthorized"), "{err}");
    assert!(!request_is_resolved(&snapshot(&rig), &ids[1]));
    tool(
        &rig,
        a.token(),
        "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": ids[0], "idempotencyKey": "r-in" }),
    )
    .await
    .unwrap();
    assert!(request_is_resolved(&snapshot(&rig), &ids[0]));
}

// ---- admin chats ---------------------------------------------------------------

const OTHER_SPACE: &str = "space-2";

/// A second Space with one item, for "never another Space" checks.
async fn other_space_item(rig: &Rig) -> String {
    let path = std::path::Path::new(&rig.space_path).join("other-space");
    std::fs::create_dir_all(&path).unwrap();
    rig.client
        .call(
            methods::MUTATE,
            json!({ "op": "createSpace", "spaceId": OTHER_SPACE, "deviceId": rig.core.device_id,
                    "path": path.to_string_lossy() }),
        )
        .await
        .unwrap();
    let op = "op-foreign";
    rig.client
        .call(
            methods::WORK_OUTLINE_MUTATE,
            json!({ "op": "createWorkItem", "spaceId": OTHER_SPACE, "title": "Foreign",
                    "operationId": op }),
        )
        .await
        .unwrap();
    derive_id(op, roles::ITEM)
}

async fn grant(rig: &Rig, chat: &str, secs: Option<u64>, skip_bulk: bool) -> Value {
    rig.client
        .call(
            methods::GRANT_ADMIN,
            json!({ "chatId": chat, "durationSecs": secs, "skipBulkConfirmation": skip_bulk }),
        )
        .await
        .unwrap()
}

fn title_of(rig: &Rig, id: &str) -> String {
    snapshot(rig).item(id).map(|i| i.title.clone()).unwrap_or_default()
}

async fn admin(rig: &Rig, token: &str, args: Value) -> Result<Value, String> {
    tool(rig, token, "work_outline_admin", args).await
}

async fn auth_origin(rig: &Rig, token: &str) -> Value {
    rig.client
        .call(mast_proto::agent_tools::AUTHENTICATE_METHOD, json!({ "token": token }))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_grants_expire_revoke_and_are_user_only() {
    let rig = rig().await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();
    assert!(auth_origin(&rig, &token).await.get("adminSpaceId").is_none());
    let err = admin(&rig, &token, json!({ "read": true })).await.unwrap_err();
    assert!(err.contains("not an admin chat"), "{err}");

    // A projectless chat cannot be admin.
    assert!(
        rig.client
            .call(methods::GRANT_ADMIN, json!({ "chatId": PROJECTLESS }))
            .await
            .is_err()
    );

    // No agent tool grants: unknown tool names are refused.
    for name in ["grant_admin", "work_outline_grant_admin"] {
        assert!(tool(&rig, &token, name, json!({ "chatId": OTHER_CHAT })).await.is_err());
    }

    let granted = grant(&rig, CHAT, Some(3600), false).await;
    assert_eq!(granted["spaceId"], SPACE);
    assert_eq!(auth_origin(&rig, &token).await["adminSpaceId"], SPACE);
    let listed = rig.client.call(methods::LIST_ADMIN_GRANTS, json!({})).await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(admin(&rig, &token, json!({ "read": true })).await.is_ok());

    // Expiry: the grant stops working at once.
    tokens.admin().advance_clock_ms(3_600_001);
    assert!(auth_origin(&rig, &token).await.get("adminSpaceId").is_none());
    assert!(admin(&rig, &token, json!({ "read": true })).await.is_err());
    let listed = rig.client.call(methods::LIST_ADMIN_GRANTS, json!({})).await.unwrap();
    assert!(listed.as_array().unwrap().is_empty());

    // Renew, then revoke.
    grant(&rig, CHAT, None, false).await;
    assert_eq!(auth_origin(&rig, &token).await["adminSpaceId"], SPACE);
    rig.client.call(methods::REVOKE_ADMIN, json!({ "chatId": CHAT })).await.unwrap();
    assert!(auth_origin(&rig, &token).await.get("adminSpaceId").is_none());
    assert!(admin(&rig, &token, json!({ "read": true })).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_changes_anything_in_its_space_but_nothing_outside_it() {
    let rig = rig().await;
    let foreign = other_space_item(&rig).await;
    let a = create_item(&rig, "op-a", "Alpha", None).await;
    let b = create_item(&rig, "op-b", "Beta", None).await;
    let child = create_item(&rig, "op-c", "Child", Some(&b)).await;
    assign_chat(&rig, "assign-admin", CHAT, &a).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();

    // Without a grant the same tool call is refused and Beta is untouched.
    assert!(admin(&rig, &token, json!({ "idempotencyKey": "k0",
        "steps": [{ "op": "rename", "itemId": b, "title": "Nope" }] })).await.is_err());
    // The ordinary tool still only reaches the chat's own subtree.
    assert!(tool(&rig, &token, "work_outline_set_status",
        json!({ "workItemId": b, "status": "active" })).await.is_err());

    grant(&rig, CHAT, Some(3600), false).await;
    let done = admin(&rig, &token, json!({ "idempotencyKey": "k1", "steps": [
        { "op": "rename", "itemId": &b[..8], "title": "Beta 2" },
        { "op": "status", "itemId": child, "status": "active" },
        { "op": "move", "itemId": child, "parentId": a },
        { "op": "create", "parentId": a, "title": "New", "ref": "n" },
        { "op": "instruction_add", "itemId": "$n", "text": "Be careful." },
        { "op": "project", "itemId": b, "isProject": true },
        { "op": "archive", "itemId": child },
    ]})).await.unwrap();
    assert_eq!(done["applied"], 7, "{done}");
    assert_eq!(title_of(&rig, &b), "Beta 2");
    let snap = snapshot(&rig);
    assert!(snap.item(&b).unwrap().is_project);
    assert!(snap.item(&child).unwrap().archived_at.is_some());
    assert!(snap.instructions.iter().any(|i| i.text == "Be careful."));

    // Widened existing tools: set_status anywhere in the Space.
    assert!(tool(&rig, &token, "work_outline_set_status",
        json!({ "workItemId": b, "status": "active" })).await.is_ok());

    // Other Space: refused for every op shape.
    for step in [
        json!({ "op": "rename", "itemId": foreign, "title": "Hacked" }),
        json!({ "op": "move", "itemId": a, "parentId": foreign }),
        json!({ "op": "create", "parentId": foreign, "title": "x" }),
        json!({ "op": "archive", "itemId": foreign }),
    ] {
        let err = admin(&rig, &token, json!({ "idempotencyKey": format!("f{step}"),
            "steps": [step] })).await.unwrap_err();
        assert!(err.contains("nauthorized") || err.contains("own Space"), "{err}");
    }
    let inputs = rig.core.workspace.work_outline_inputs(OTHER_SPACE);
    let other = mast_doc::materialize_work_outline(OTHER_SPACE, &inputs);
    assert_eq!(other.item(&foreign).unwrap().title, "Foreign");

    // Unsupported ops (Linear/GitHub, delete, grant) are not in the vocabulary.
    for op in ["linear_link", "github_comment", "delete_item", "grant_admin"] {
        let err = admin(&rig, &token, json!({ "idempotencyKey": format!("u{op}"),
            "steps": [{ "op": op }] })).await.unwrap_err();
        assert!(err.contains("unknown op"), "{err}");
    }

    // Revoke: the next call is refused.
    rig.client.call(methods::REVOKE_ADMIN, json!({ "chatId": CHAT })).await.unwrap();
    assert!(admin(&rig, &token, json!({ "idempotencyKey": "k9",
        "steps": [{ "op": "rename", "itemId": b, "title": "Late" }] })).await.is_err());
    assert_eq!(title_of(&rig, &b), "Beta 2");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_resolves_requests_and_accepts_proposals_with_its_own_attribution() {
    let rig = rig().await;
    let a = create_item(&rig, "op-a", "Alpha", None).await;
    let b = create_item(&rig, "op-b", "Beta", None).await;
    assign_chat(&rig, "assign-admin", CHAT, &a).await;
    assign_chat(&rig, "assign-worker", OTHER_CHAT, &b).await;
    add_instruction(&rig, "ins-b", json!({ "type": "workItem", "workItemId": b }), "Old rule.").await;
    let tokens = rig.core.sessions.agent_tokens();
    let admin_lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let worker_lease = tokens.mint(OTHER_CHAT, "run-2", SPACE, &rig.core.device_id);
    let raised = tool(&rig, worker_lease.token(), "work_outline_create_user_action_request",
        json!({ "request": "Which db?", "dedupeKey": "db", "idempotencyKey": "q" })).await.unwrap();
    let request_id = raised["userActionRequestId"].as_str().unwrap().to_owned();
    let rule = snapshot(&rig).instructions[0].id.clone();
    let proposal = tool(&rig, worker_lease.token(), "work_outline_propose_instruction",
        json!({ "action": "edit", "instructionId": rule, "text": "New rule.",
                "idempotencyKey": "p" })).await.unwrap();
    let proposal_id = proposal["proposalId"].as_str().unwrap().to_owned();

    // Not admin: another chat's request is off limits.
    assert!(tool(&rig, admin_lease.token(), "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": request_id, "idempotencyKey": "x" })).await.is_err());

    grant(&rig, CHAT, Some(3600), false).await;
    let done = admin(&rig, admin_lease.token(), json!({ "idempotencyKey": "k", "steps": [
        { "op": "resolve_request", "requestId": &request_id[..8], "note": "Postgres" },
        { "op": "proposal_accept", "proposalId": proposal_id },
    ]})).await.unwrap();
    assert_eq!(done["applied"], 2, "{done}");
    let snap = snapshot(&rig);
    let resolved = snap.user_action_requests.iter().find(|r| r.id == request_id).unwrap();
    let resolution = resolved.resolution.as_ref().unwrap();
    assert_eq!(resolution.by.kind, WorkOutlineActorKind::Agent, "never the user");
    assert_eq!(resolution.by.chat_id.as_deref(), Some(CHAT));
    assert!(resolution.note.as_deref().unwrap().contains("by admin chat"));
    assert_eq!(snap.instructions.iter().find(|i| i.id == rule).unwrap().text, "New rule.");
}

#[tokio::test(flavor = "multi_thread")]
async fn bulk_changes_need_the_users_own_approval_with_a_single_use_token() {
    let rig = rig().await;
    let a = create_item(&rig, "op-a", "Alpha", None).await;
    let mut ids = Vec::new();
    for n in 0..11 {
        ids.push(create_item(&rig, &format!("op-i{n}"), &format!("Item {n}"), None).await);
    }
    assign_chat(&rig, "assign-admin", CHAT, &a).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();
    grant(&rig, CHAT, Some(36000), false).await;
    let steps: Vec<Value> = ids
        .iter()
        .map(|id| json!({ "op": "rename", "itemId": id, "title": "Renamed" }))
        .collect();

    let preview = admin(&rig, &token, json!({ "idempotencyKey": "b1", "steps": steps })).await.unwrap();
    assert_eq!(preview["needsApproval"], true, "{preview}");
    assert_eq!(preview["count"], 11);
    assert!(ids.iter().all(|id| title_of(&rig, id) != "Renamed"), "nothing ran");
    let confirm = preview["confirmToken"].as_str().unwrap().to_owned();
    let request_id = preview["requestId"].as_str().unwrap().to_owned();
    let snap = snapshot(&rig);
    let request = snap.user_action_requests.iter().find(|r| r.id == request_id).unwrap();
    assert!(request.text.starts_with("Approve bulk change"), "{}", request.text);

    // Not yet approved.
    let with_token = |key: &str, token: &str| {
        json!({ "idempotencyKey": key, "steps": steps, "confirmToken": token })
    };
    let err = admin(&rig, &token, with_token("b2", &confirm)).await.unwrap_err();
    assert!(err.contains("not approved"), "{err}");
    // The agent (admin or not) cannot resolve it itself.
    let err = tool(&rig, &token, "work_outline_resolve_user_action_request",
        json!({ "userActionRequestId": request_id, "idempotencyKey": "self" })).await.unwrap_err();
    assert!(err.contains("nauthorized") || err.contains("approve"), "{err}");
    let err = admin(&rig, &token, json!({ "idempotencyKey": "b3",
        "steps": [{ "op": "resolve_request", "requestId": request_id }] })).await.unwrap_err();
    assert!(err.contains("nauthorized") || err.contains("approve"), "{err}");
    // Different steps than the user saw: refused.
    let mut other = steps.clone();
    other[0]["title"] = json!("Sneaky");
    let err = admin(&rig, &token, json!({ "idempotencyKey": "b4", "steps": other,
        "confirmToken": confirm })).await.unwrap_err();
    assert!(err.contains("differ"), "{err}");

    // The user resolves it.
    rig.client.call(methods::WORK_OUTLINE_MUTATE, json!({
        "op": "resolveUserActionRequest", "userActionRequestId": request_id,
        "outcome": "approved", "via": "card", "operationId": "user-approves" })).await.unwrap();
    let done = admin(&rig, &token, with_token("b5", &confirm)).await.unwrap();
    assert_eq!(done["applied"], 11, "{done}");
    assert!(ids.iter().all(|id| title_of(&rig, id) == "Renamed"));

    // One use only.
    let err = admin(&rig, &token, with_token("b6", &confirm)).await.unwrap_err();
    assert!(err.contains("already used"), "{err}");

    // Expiry of an unused token.
    let steps2: Vec<Value> = ids.iter().map(|id| json!({ "op": "rename", "itemId": id, "title": "Again" })).collect();
    let p2 = admin(&rig, &token, json!({ "idempotencyKey": "b7", "steps": steps2 })).await.unwrap();
    let t2 = p2["confirmToken"].as_str().unwrap().to_owned();
    tokens.admin().advance_clock_ms(31 * 60 * 1000);
    grant(&rig, CHAT, None, false).await;
    let err = admin(&rig, &token, json!({ "idempotencyKey": "b8", "steps": steps2,
        "confirmToken": t2 })).await.unwrap_err();
    assert!(err.contains("expired"), "{err}");

    // "Skip bulk confirmations" runs the sweep directly.
    grant(&rig, CHAT, None, true).await;
    let done = admin(&rig, &token, json!({ "idempotencyKey": "b9", "steps": steps2 })).await.unwrap();
    assert_eq!(done["applied"], 11);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_turn_is_audited_as_one_entry_and_undone_in_one_step() {
    let rig = rig().await;
    let a = create_item(&rig, "op-a", "Alpha", None).await;
    let b = create_item(&rig, "op-b", "Beta", None).await;
    assign_chat(&rig, "assign-admin", CHAT, &a).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-7", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();
    grant(&rig, CHAT, Some(3600), false).await;
    for (key, steps) in [
        ("t1", json!([{ "op": "rename", "itemId": b, "title": "Beta 2" }])),
        ("t2", json!([{ "op": "move", "itemId": b, "parentId": a },
                      { "op": "group", "title": "Box", "itemIds": [a] }])),
    ] {
        admin(&rig, &token, json!({ "idempotencyKey": key, "steps": steps })).await.unwrap();
    }
    let turns = rig.client.call(methods::ADMIN_AUDIT_LIST, json!({ "spaceId": SPACE })).await.unwrap();
    let turns = turns.as_array().unwrap();
    assert_eq!(turns.len(), 1, "both calls are one turn (one run): {turns:?}");
    assert_eq!(turns[0]["turnId"], "run-7");
    assert!(turns[0]["label"].as_str().unwrap().starts_with("by admin chat"));
    assert_eq!(turns[0]["actions"].as_array().unwrap().len(), 4);
    assert_eq!(snapshot(&rig).item(&b).unwrap().parent_work_item_id.as_deref(), Some(a.as_str()));

    let reply = rig.client.call(methods::ADMIN_UNDO_TURN, json!({ "turnId": "run-7" })).await.unwrap();
    assert_eq!(reply["undone"], 4, "{reply}");
    let snap = snapshot(&rig);
    assert_eq!(snap.item(&b).unwrap().title, "Beta");
    assert_eq!(snap.item(&b).unwrap().parent_work_item_id, None);
    assert_eq!(snap.item(&a).unwrap().parent_work_item_id, None);
    assert!(rig.client.call(methods::ADMIN_UNDO_TURN, json!({ "turnId": "run-7" })).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn child_template_proposal_resolves_an_unlinked_direct_child() {
    let rig = rig().await;
    let container = create_item(&rig, "proposal-container", "Projects", None).await;
    let child = create_item(&rig, "proposal-child", "Project", Some(&container)).await;
    assign_chat(&rig, "proposal-chat", CHAT, &child).await;
    let lease = rig.core.sessions.agent_tokens().mint(CHAT, "proposal-run", SPACE, &rig.core.device_id);
    let token = lease.token();
    let missing = tool(&rig, token, "work_outline_propose_instruction", json!({
        "target": "childTemplate", "text": "Rule", "idempotencyKey": "missing"
    })).await.unwrap_err();
    assert!(missing.contains("not a direct child of a template container"), "{missing}");
    grant(&rig, CHAT, Some(3600), false).await;
    admin(&rig, token, json!({ "idempotencyKey": "set-template", "steps": [
        { "op": "template_set", "itemId": container, "template": { "root": {} } }
    ]})).await.unwrap();
    let result = tool(&rig, token, "work_outline_propose_instruction", json!({
        "target": "childTemplate", "containerWorkItemId": container, "text": "Rule", "idempotencyKey": "proposal"
    })).await.unwrap();
    let snap = snapshot(&rig);
    let proposal = snap.instruction_proposals.iter().find(|p| Some(p.id.as_str()) == result["proposalId"].as_str()).unwrap();
    let root = &snap.item(&container).unwrap().container_template.as_ref().unwrap().root.id;
    assert_eq!(proposal.target.rule_node_id.as_deref(), Some(root.as_str()));
    assert_eq!(result["target"]["ruleNodeId"].as_str(), Some(root.as_str()));
    assert_eq!(proposal.target.work_item_id.as_deref(), Some(child.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_configures_a_container_template_and_applies_it_to_existing_items() {
    let rig = rig().await;
    let admin_item = create_item(&rig, "op-admin", "Admin work", None).await;
    let container = create_item(&rig, "op-box", "Projects", None).await;
    let p1 = create_item(&rig, "op-p1", "Alpha", Some(&container)).await;
    let p2 = create_item(&rig, "op-p2", "Beta", Some(&container)).await;
    assign_chat(&rig, "assign-admin", CHAT, &admin_item).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token().to_owned();
    let template = json!({ "root": { "children": [
        { "title": "Planning", "instructions": [{ "text": "Plan first." }] },
        { "title": "Tickets", "isWorkerDestination": true }
    ]}});

    // Non-admin: editing template rules (and applying them elsewhere) is refused.
    let err = admin(&rig, &token, json!({ "idempotencyKey": "t0",
        "steps": [{ "op": "template_set", "itemId": container, "template": template }] }))
        .await
        .unwrap_err();
    assert!(err.contains("not an admin chat"), "{err}");
    let err = tool(&rig, &token, "work_outline_apply_container_template",
        json!({ "workItemId": container })).await.unwrap_err();
    assert!(err.contains("nauthorized"), "{err}");
    assert!(snapshot(&rig).item(&container).unwrap().container_template.is_none());

    // Admin: configure, then apply to every existing child (two items).
    grant(&rig, CHAT, Some(3600), false).await;
    let done = admin(&rig, &token, json!({ "idempotencyKey": "t1", "steps": [
        { "op": "template_set", "itemId": container, "template": template },
        { "op": "template_apply", "itemId": container },
    ]})).await.unwrap();
    assert_eq!(done["applied"], 2, "{done}");
    let snap = snapshot(&rig);
    assert!(snap.item(&container).unwrap().container_template.is_some());
    for project in [&p1, &p2] {
        let titles: Vec<String> = snap
            .item(project)
            .unwrap()
            .child_ids
            .iter()
            .map(|c| snap.item(c).unwrap().title.clone())
            .collect();
        assert!(titles.contains(&"Planning".to_owned()) && titles.contains(&"Tickets".to_owned()), "{titles:?}");
    }

    // The template edit is undoable as part of the turn.
    let reply = rig.client.call(methods::ADMIN_UNDO_TURN, json!({ "turnId": "run-1" })).await.unwrap();
    assert_eq!(reply["skipped"].as_array().unwrap().len(), 1, "apply cannot be undone: {reply}");
    assert!(snapshot(&rig).item(&container).unwrap().container_template.is_none());
}

// ---- archiving finished workers ------------------------------------------------

const ARCHIVE_CHILD: &str = "chat-archive-child";
const ARCHIVE_OUTSIDER: &str = "chat-archive-outsider";

struct ArchiveRig {
    rig: Rig,
    p: ProjectRig,
    orchestrator: String,
    /// CHAT is the orchestrator on Planning; OTHER_CHAT works on a ticket.
    _lease: mast_engine::agent_tools::AgentTokenLease,
}

async fn archive_rig() -> ArchiveRig {
    let rig = rig().await;
    let p = project_rig(&rig).await;
    assign_chat(&rig, "ar-orch", CHAT, &p.planning).await;
    assign_chat(&rig, "ar-worker", OTHER_CHAT, &p.ticket).await;
    for (chat, parent) in [(ARCHIVE_CHILD, Some(CHAT)), (ARCHIVE_OUTSIDER, None)] {
        let mut params = json!({ "op": "createChat", "chatId": chat, "spaceId": SPACE,
                                 "deviceId": rig.core.device_id });
        if let Some(parent) = parent {
            params["parentChatId"] = json!(parent);
        }
        rig.client.call(methods::MUTATE, params).await.unwrap();
    }
    assign_chat(&rig, "ar-out", ARCHIVE_OUTSIDER, &p.other_tickets).await;
    let lease = rig
        .core
        .sessions
        .agent_tokens()
        .mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    ArchiveRig {
        orchestrator: lease.token().to_owned(),
        rig,
        p,
        _lease: lease,
    }
}

fn chat_archived(rig: &Rig, chat: &str) -> bool {
    rig.core.workspace.chat(chat).unwrap().unwrap().archived
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_info_rates_children_and_project_workers_only() {
    let a = archive_rig().await;
    let info = tool(&a.rig, &a.orchestrator, "chat_archive_info", json!({}))
        .await
        .unwrap();
    let chats = info["chats"].as_object().unwrap();
    assert!(chats.contains_key(OTHER_CHAT) && chats.contains_key(ARCHIVE_CHILD), "{chats:?}");
    // Not itself, nor a chat in another project.
    assert!(!chats.contains_key(CHAT) && !chats.contains_key(ARCHIVE_OUTSIDER), "{chats:?}");
    // The ticket is open: not finished yet.
    assert_eq!(chats[OTHER_CHAT]["archivable"], false);
    update_item(&a.rig, "ar-done", &a.p.ticket, json!({ "status": "completed" }))
        .await
        .unwrap();
    let info = tool(&a.rig, &a.orchestrator, "chat_archive_info", json!({ "chatIds": [OTHER_CHAT] }))
        .await
        .unwrap();
    assert_eq!(info["chats"][OTHER_CHAT]["archivable"], true);
    assert_eq!(info["chats"][OTHER_CHAT]["reason"], "its item is completed");
    assert_eq!(info["chats"].as_object().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unfinished_worker_is_refused_unless_forced_with_a_reason() {
    let a = archive_rig().await;
    let err = tool(&a.rig, &a.orchestrator, "chat_archive", json!({ "chatId": OTHER_CHAT }))
        .await
        .unwrap_err();
    assert!(err.contains("not archived") && err.contains("force"), "{err}");
    assert!(!chat_archived(&a.rig, OTHER_CHAT));
    let err = tool(
        &a.rig,
        &a.orchestrator,
        "chat_archive",
        json!({ "chatId": OTHER_CHAT, "force": true }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("short reason"), "{err}");
    assert!(!chat_archived(&a.rig, OTHER_CHAT));
    let done = tool(
        &a.rig,
        &a.orchestrator,
        "chat_archive",
        json!({ "chatId": OTHER_CHAT, "force": true, "reason": "superseded by ENG-9" }),
    )
    .await
    .unwrap();
    assert_eq!((done["archived"].clone(), done["forced"].clone()), (json!(true), json!(true)));
    assert!(chat_archived(&a.rig, OTHER_CHAT));
    // Logged, attributed to the orchestrator, with the reason.
    let log = a.rig.service.agent_tools().agent_archives_since(0);
    assert_eq!(log.len(), 1);
    assert_eq!(
        (log[0].chat_id.as_str(), log[0].orchestrator_chat_id.as_str(), log[0].reason.as_str(), log[0].forced),
        (OTHER_CHAT, CHAT, "superseded by ENG-9", true)
    );
    // Reversible: unarchive needs neither the rule nor a reason.
    tool(
        &a.rig,
        &a.orchestrator,
        "chat_archive",
        json!({ "chatId": OTHER_CHAT, "archived": false }),
    )
    .await
    .unwrap();
    assert!(!chat_archived(&a.rig, OTHER_CHAT));
    assert_eq!(a.rig.service.agent_tools().agent_archives_since(0).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_worker_archives_without_force_and_is_logged() {
    let a = archive_rig().await;
    update_item(&a.rig, "ar-done", &a.p.ticket, json!({ "status": "completed" }))
        .await
        .unwrap();
    let done = tool(&a.rig, &a.orchestrator, "chat_archive", json!({ "chatId": OTHER_CHAT }))
        .await
        .unwrap();
    assert_eq!(done["forced"], false);
    assert_eq!(done["reason"], "its item is completed");
    assert!(chat_archived(&a.rig, OTHER_CHAT));
    let log = a.rig.service.agent_tools().agent_archives_since(0);
    assert_eq!(log.len(), 1);
    assert!(!log[0].forced);
    assert_eq!(log[0].space_id, SPACE);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worker_with_an_open_required_action_is_never_finished() {
    let a = archive_rig().await;
    update_item(&a.rig, "ar-done", &a.p.ticket, json!({ "status": "completed" }))
        .await
        .unwrap();
    // The worker raised a question that nobody answered.
    let worker = a.rig.core.sessions.agent_tokens().mint(OTHER_CHAT, "run-w", SPACE, &a.rig.core.device_id);
    tool(
        &a.rig,
        worker.token(),
        "work_outline_create_user_action_request",
        json!({ "request": "Which colour?", "dedupeKey": "colour" }),
    )
    .await
    .unwrap();
    let err = tool(&a.rig, &a.orchestrator, "chat_archive", json!({ "chatId": OTHER_CHAT }))
        .await
        .unwrap_err();
    assert!(err.contains("open Required Action"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn archiving_reaches_children_and_never_the_orchestrator_or_other_projects() {
    let a = archive_rig().await;
    // A child, as today (no finished rule for the caller's own chats).
    tool(&a.rig, &a.orchestrator, "chat_archive", json!({ "chatId": ARCHIVE_CHILD }))
        .await
        .unwrap();
    assert!(chat_archived(&a.rig, ARCHIVE_CHILD));
    for target in [ARCHIVE_OUTSIDER, CHAT, PROJECTLESS] {
        let err = tool(
            &a.rig,
            &a.orchestrator,
            "chat_archive",
            json!({ "chatId": target, "force": true, "reason": "try" }),
        )
        .await
        .unwrap_err();
        assert!(err.contains("forbidden") || err.contains("no such chat"), "{target}: {err}");
        assert!(!chat_archived(&a.rig, target), "{target}");
    }
    // The outsider's own orchestrator-less token reaches nothing of ours.
    let outsider = a.rig.core.sessions.agent_tokens().mint(ARCHIVE_OUTSIDER, "run-o", SPACE, &a.rig.core.device_id);
    let err = tool(
        &a.rig,
        outsider.token(),
        "chat_archive",
        json!({ "chatId": OTHER_CHAT, "force": true, "reason": "try" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("forbidden"), "{err}");
    assert!(!chat_archived(&a.rig, OTHER_CHAT));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_chat_archives_any_chat_of_its_space_without_the_rule_and_it_is_logged() {
    let a = archive_rig().await;
    grant(&a.rig, CHAT, Some(3600), false).await;
    // An unfinished worker and a chat in another project: no rule, no force.
    for target in [OTHER_CHAT, ARCHIVE_OUTSIDER] {
        tool(&a.rig, &a.orchestrator, "chat_archive", json!({ "chatId": target }))
            .await
            .unwrap();
        assert!(chat_archived(&a.rig, target), "{target}");
    }
    let log = a.rig.service.agent_tools().agent_archives_since(0);
    assert_eq!(log.len(), 2);
    assert!(log.iter().all(|e| e.orchestrator_chat_id == CHAT && !e.forced));
    // Still never itself, nor a projectless chat of another Space.
    for target in [CHAT, PROJECTLESS] {
        assert!(tool(&a.rig, &a.orchestrator, "chat_archive", json!({ "chatId": target }))
            .await
            .is_err());
        assert!(!chat_archived(&a.rig, target), "{target}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn bulk_change_rejects_non_approval_resolutions() {
    for outcome in [
        None,
        Some("dismissed"),
        Some("declined"),
        Some("changesRequested"),
        Some("answered"),
    ] {
        let rig = rig().await;
        let item = create_item(&rig, "admin-item", "Admin", None).await;
        let mut ids = Vec::new();
        for n in 0..11 {
            ids.push(create_item(&rig, &format!("item-{n}"), &format!("Item {n}"), None).await);
        }
        assign_chat(&rig, "assign-admin", CHAT, &item).await;
        let tokens = rig.core.sessions.agent_tokens();
        let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
        let token = lease.token().to_owned();
        grant(&rig, CHAT, Some(36000), false).await;
        let steps: Vec<Value> = ids
            .iter()
            .map(|id| json!({"op":"rename", "itemId":id, "title":"Changed"}))
            .collect();
        let preview = admin(
            &rig,
            &token,
            json!({"idempotencyKey":"preview", "steps":steps}),
        )
        .await
        .unwrap();
        let request_id = preview["requestId"].as_str().unwrap();
        rig.client.call(methods::WORK_OUTLINE_MUTATE, json!({"op":"resolveUserActionRequest", "userActionRequestId":request_id, "outcome":outcome, "note":"no", "via":"card", "operationId":"user-declines"})).await.unwrap();
        let err = admin(&rig, &token, json!({"idempotencyKey":"apply", "steps":steps, "confirmToken":preview["confirmToken"]})).await.unwrap_err();
        assert!(err.contains("not approved"), "{outcome:?}: {err}");
        assert!(ids.iter().all(|id| title_of(&rig, id) != "Changed"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn typed_request_outcomes_are_readable_by_request_and_current() {
    let rig = rig().await;
    let item = create_item(&rig, "outcome-item", "Outcomes", None).await;
    assign_chat(&rig, "outcome-assign", CHAT, &item).await;
    let tokens = rig.core.sessions.agent_tokens();
    let lease = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let token = lease.token();
    for (index, (kind, outcome)) in [
        ("approve", "approved"),
        ("approve", "declined"),
        ("review", "approved"),
        ("review", "changesRequested"),
        ("handoff", "approved"),
        ("handoff", "changesRequested"),
        ("ask", "answered"),
        ("choose", "chosen"),
        ("notice", "dismissed"),
    ]
    .iter()
    .enumerate()
    {
        let raised = tool(&rig, token, "work_outline_create_user_action_request", json!({
            "request": format!("Request {index}"), "kind": kind, "idempotencyKey": format!("create-{index}")
        })).await.unwrap();
        let id = raised["userActionRequestId"].as_str().unwrap();
        rig.client.call(methods::WORK_OUTLINE_MUTATE, json!({"op":"resolveUserActionRequest", "userActionRequestId":id,
            "outcome":outcome, "values":{"choice":"a"}, "via":"card", "operationId":format!("resolve-{index}")})).await.unwrap();
        let result = tool(
            &rig,
            token,
            "get_user_action_request",
            json!({"requestId":id}),
        )
        .await
        .unwrap();
        assert_eq!(result["request"]["kind"], *kind);
        assert_eq!(result["request"]["resolution"]["outcome"], *outcome);
        assert_eq!(result["request"]["resolution"]["values"]["choice"], "a");
    }
    let current = tool(&rig, token, "work_outline_read_current", json!({}))
        .await
        .unwrap();
    assert_eq!(current["userActionRequests"].as_array().unwrap().len(), 9);
}

#[tokio::test(flavor = "multi_thread")]
async fn friction_provenance_deduplication_and_filters() {
    let rig = rig().await;
    let tokens = rig.core.sessions.agent_tokens();
    let first = tokens.mint(CHAT, "run-1", SPACE, &rig.core.device_id);
    let second = tokens.mint(OTHER_CHAT, "run-2", SPACE, &rig.core.device_id);
    rig.core.workspace.set_chat_actual_model(CHAT, "model-actual");
    let input = json!({"kind":"bug", "summary":"Tool failed", "whatHappened":"It failed"});
    let row = tool(&rig, first.token(), "report_friction", input.clone()).await.unwrap();
    assert_eq!(row["report"]["provenance"]["chatId"], CHAT);
    assert_eq!(row["report"]["provenance"]["spaceId"], SPACE);
    assert_eq!(row["report"]["provenance"]["source"], "agent");
    assert_eq!(row["report"]["provenance"]["model"], "model-actual");
    let next = tool(&rig, second.token(), "report_friction", input).await.unwrap();
    assert_eq!(next["report"]["id"], row["report"]["id"]);
    assert_eq!(next["report"]["occurrences"], 2);
    let list = tool(&rig, first.token(), "list_friction", json!({"kind":"bug", "status":"open", "since":0, "limit":1})).await.unwrap();
    assert_eq!(list["reports"].as_array().unwrap().len(), 1);
    assert_eq!(list["reports"][0]["occurrences"], 2);
    for filter in [json!({"kind":"docs"}), json!({"status":"dismissed"}), json!({"since":i64::MAX})] {
        let list = tool(&rig, first.token(), "list_friction", filter).await.unwrap();
        assert!(list["reports"].as_array().unwrap().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn friction_captures_live_run_harness() {
    let rig = rig().await;
    rig.core.doc_host.queue_command(CHAT, run_command("test", &rig.space_path)).unwrap();
    wait_for(|| !rig.harness.spawns.lock().unwrap().is_empty(), "live run").await;
    let Spawn::WithTools { tools, .. } = rig.harness.spawns.lock().unwrap()[0].clone() else { panic!("missing tools") };
    let row = tool(&rig, &tools.env["MAST_AGENT_TOKEN"], "report_friction", json!({"kind":"bug", "summary":"Failure", "whatHappened":"Failed"})).await.unwrap();
    assert_eq!(row["report"]["provenance"]["harness"], "mock");
    rig.harness.release.notify_one();
}

#[tokio::test(flavor = "multi_thread")]
async fn approval_rule_proposal_only_creates_a_review_request() {
    let rig = rig().await;
    let root = create_item(&rig, "proposal-root", "Root", None).await;
    let tokens = rig.core.sessions.agent_tokens();
    let token = tokens.mint(CHAT, "proposal-run", SPACE, &rig.core.device_id);
    tool(&rig, token.token(), "work_outline_associate_current_chat", json!({"workItemId": root, "idempotencyKey": "proposal-associate"})).await.unwrap();
    let result = tool(&rig, token.token(), "propose_approval_rule", json!({
        "label": "Rust tests", "argvPrefix": ["cargo", "test"], "harness": "codex", "idempotencyKey": "proposal-rule"
    })).await.unwrap();
    assert!(result["userActionRequestId"].is_string());
    let snapshot = snapshot(&rig);
    assert!(snapshot.space_settings.approval_rules.is_empty());
    assert!(snapshot.user_action_requests.iter().any(|request| request.text.contains("Rust tests")));
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn friction_captures_tool_failures_and_can_be_disabled() {
    let rig = rig().await;
    rig.core.doc_host.queue_command(CHAT, run_command("test", &rig.space_path)).unwrap();
    wait_for(|| !rig.harness.spawns.lock().unwrap().is_empty(), "run").await;
    let Spawn::WithTools { tools, .. } = rig.harness.spawns.lock().unwrap()[0].clone() else { panic!("missing tools") };
    let token = &tools.env["MAST_AGENT_TOKEN"];
    for _ in 0..2 {
        assert!(tool(&rig, token, "missing_tool", json!({})).await.is_err());
    }
    let rows = rig.core.workspace.friction_reports(SPACE);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].occurrences, 2);
    assert_eq!(rows[0].provenance.source, mast_proto::FrictionSource::Auto);
    assert!(tool(&rig, token, "token=fake-credential", json!({})).await.is_err());
    let rows = rig.core.workspace.friction_reports(SPACE);
    let serialized = serde_json::to_string(&rows).unwrap();
    assert!(!serialized.contains("fake-credential"));
    assert!(serialized.contains("[REDACTED]"));
    rig.core.sessions.set_auto_capture_friction(false);
    assert!(tool(&rig, token, "another_missing_tool", json!({})).await.is_err());
    assert_eq!(rig.core.workspace.friction_reports(SPACE).len(), 2);
    rig.harness.release.notify_one();
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn friction_captures_approval_and_answer() {
    let rig = rig().await;
    *rig.harness.questions.lock().unwrap() = vec![mast_proto::UserInputQuestion { command_approval: None, allow_custom: false, id: "approval".into(), header: "Approve command".into(), question: "Codex wants to run `echo token=fake-secret`. Allow it? Reason: sandbox".into(), options: vec!["Yes".into(), "No".into()], multi_select: false }];
    let mut signals = rig.core.sessions.watch_worker_signals();
    rig.core.doc_host.queue_command(CHAT, run_command("test", &rig.space_path)).unwrap();
    let input = loop {
        if let mast_engine::sessions::WorkerSignal::Input { input, .. } = tokio::time::timeout(Duration::from_secs(10), signals.recv()).await.unwrap().unwrap() { break input; }
    };
    let rows = rig.core.workspace.friction_reports(SPACE);
    assert_eq!(rows.len(), 1);
    let approval = rows[0].approval.as_ref().unwrap();
    assert_eq!(approval.command, "echo token=[REDACTED]");
    assert_eq!(approval.why, "sandbox");
    assert_eq!(approval.answer, None);
    assert!(rig.core.sessions.respond_input(CHAT, &input.request_id, vec![mast_proto::UserInputAnswer { question_id: "approval".into(), labels: vec!["No".into()] }]).unwrap());
    let rows = rig.core.workspace.friction_reports(SPACE);
    assert_eq!(rows[0].approval.as_ref().unwrap().answer, Some(false));
    assert_eq!(rows[0].occurrences, 1);
    rig.harness.release.notify_one();
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn command_rule_answers_fake_codex_without_worker_wake() {
    use mast_proto::work_outline::{ApprovalRule, ApprovalRuleHarness, SetApprovalRuleParams, WorkOutlineActor, WorkOutlineMutation};
    let rig = rig_for(true).await;
    let rule = ApprovalRule {
        id: "rule-1".into(), label: "Tests".into(), created_by: "user".into(),
        harness: ApprovalRuleHarness::Codex, enabled: true, argv_prefix: vec!["cargo".into(), "test".into()],
        command_regex: None, cwd_scope: None, require_isolated_home: false, hit_count: 0, last_used_at: None,
    };
    rig.core.workspace.work_outline_mutate(&WorkOutlineMutation::SetApprovalRule(SetApprovalRuleParams {
        space_id: SPACE.into(), rule, operation_id: "approval-rule".into(),
    }), &WorkOutlineActor { kind: WorkOutlineActorKind::User, device_id: rig.core.device_id.clone(), chat_id: None, message_id: None }, &[]).unwrap();
    *rig.harness.questions.lock().unwrap() = vec![mast_proto::UserInputQuestion {
        command_approval: Some(mast_proto::CommandApproval { command: json!(["cargo", "test", "--offline"]), cwd: Some(rig.space_path.clone()), reason: Some("sandbox".into()), sandbox_permissions: None }),
        allow_custom: false, id: "approval".into(), header: "Approve command".into(), question: "Codex wants to run `cargo test --offline`. Allow it? Reason: sandbox".into(),
        options: vec!["Yes".into(), "No".into()], multi_select: false,
    }];
    rig.core.sessions.set_auto_capture_friction(false);
    let mut signals = rig.core.sessions.watch_worker_signals();
    let mut command = run_command("test", &rig.space_path);
    if let SessionCommandPayload::Run { request, .. } = &mut command { request.harness = Some(HarnessId::Codex); request.auto_approve = false; }
    rig.core.doc_host.queue_command(CHAT, command).unwrap();
    wait_for(|| !rig.harness.answers.lock().unwrap().is_empty(), "automatic answer").await;
    assert_eq!(rig.harness.answers.lock().unwrap()[0][0].labels, ["Yes"]);
    assert!(signals.try_recv().is_err());
    let rows = rig.core.workspace.friction_reports(SPACE);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].summary.contains("Auto-approved by rule rule-1"));
    assert_eq!(rows[0].approval.as_ref().unwrap().answer, Some(true));
    let inputs = rig.core.workspace.work_outline_inputs(SPACE);
    let snapshot = mast_doc::materialize_work_outline(SPACE, &inputs);
    assert_eq!(snapshot.space_settings.approval_rules[0].hit_count, 1);
    assert!(snapshot.space_settings.approval_rules[0].last_used_at.is_some());
    rig.harness.release.notify_one();
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn friction_captures_rejected_server_requests() {
    let rig = rig().await;
    *rig.harness.events.lock().unwrap() = vec![AgentEvent::ServerRequestRejected { method: "method/token=fake-secret".into(), server_name: "example".into() }, AgentEvent::ServerRequestRejected { method: "method/token=fake-secret".into(), server_name: "example".into() }];
    rig.core.doc_host.queue_command(CHAT, run_command("test", &rig.space_path)).unwrap();
    wait_for(|| rig.core.workspace.friction_reports(SPACE).first().is_some_and(|r| r.occurrences == 2), "friction report").await;
    let rows = rig.core.workspace.friction_reports(SPACE);
    assert_eq!(rows[0].kind, mast_proto::FrictionKind::ToolFailure);
    assert!(!serde_json::to_string(&rows).unwrap().contains("fake-secret"));
    rig.harness.release.notify_one();
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn friction_captures_errored_turn() {
    let rig = rig().await;
    *rig.harness.events.lock().unwrap() = vec![AgentEvent::Error { message: "password=fake-secret".into() }, AgentEvent::Done { status: DoneStatus::Errored, result: None, error: None, session_id: None }];
    rig.core.doc_host.queue_command(CHAT, run_command("test", &rig.space_path)).unwrap();
    wait_for(|| !rig.core.workspace.friction_reports(SPACE).is_empty(), "friction report").await;
    let rows = rig.core.workspace.friction_reports(SPACE);
    assert_eq!(rows[0].kind, mast_proto::FrictionKind::Environment);
    assert_eq!(rows[0].what_happened, "password=[REDACTED]");
    rig.core.shutdown().await;
}

#[path = "../../../test_support/isolation.rs"]
mod test_isolation;

#[tokio::test(flavor = "multi_thread")]
async fn prefixes_and_truncated_request_fields_are_accepted() {
    let rig = rig().await;
    let item = create_item(&rig, "prefix-item", "Prefix item", None).await;
    let outsider = create_item(&rig, "outside-item", "Outside", None).await;
    assign_chat(&rig, "prefix-assignment", CHAT, &item).await;
    let lease = rig.core.sessions.agent_tokens().mint(CHAT, "prefix-run", SPACE, &rig.core.device_id);
    let token = lease.token();
    let created = tool(&rig, token, "work_outline_create_user_action_request", json!({
        "request": "Review", "dedupeKey": "prefix-test",
        "facts": [{"label": "é".repeat(41), "value": "🙂".repeat(121)}],
        "links": [{"label": "L".repeat(41), "target": {"type":"url", "url":"https://example.com"}}]
    })).await.unwrap();
    assert_eq!(created["warnings"].as_array().unwrap().len(), 3);
    let id = created["userActionRequestId"].as_str().unwrap();
    let fetched = tool(&rig, token, "get_user_action_request", json!({"requestId": &id[..8]})).await.unwrap();
    assert_eq!(fetched["request"]["facts"][0]["value"].as_str().unwrap().chars().count(), 120);
    assert!(fetched["request"]["facts"][0]["value"].as_str().unwrap().ends_with('…'));
    assert!(fetched["request"]["links"][0]["label"].as_str().unwrap().ends_with('…'));
    let missing = tool(&rig, token, "get_user_action_request", json!({"requestId":"0000000000000000"})).await.unwrap_err();
    assert!(missing.contains("open requests in your scope") && missing.contains(id), "{missing}");
    let denied = tool(&rig, token, "work_outline_set_status", json!({"workItemId": &outsider[..8], "status":"active"})).await.unwrap_err();
    assert!(denied.contains(&item) && denied.contains(&outsider) && denied.contains("allowed subtree"), "{denied}");
    tool(&rig, token, "work_outline_resolve_user_action_request", json!({"userActionRequestId": &id[..8]})).await.unwrap();
    let completed = tool(&rig, token, "work_outline_set_status", json!({"workItemId": &item[..8], "status":"completed"})).await.unwrap();
    assert_eq!(completed["workItemId"], item);
}

#[tokio::test(flavor = "multi_thread")]
async fn friction_dismissal_requires_orchestrator_and_records_reason() {
    let rig = rig().await;
    let lease = rig.core.sessions.agent_tokens().mint(CHAT, "dismiss-run", SPACE, &rig.core.device_id);
    let token = lease.token();
    let report = tool(&rig, token, "report_friction", json!({"kind":"bug","summary":"dismiss test","whatHappened":"failed"})).await.unwrap();
    let id = report["report"]["id"].as_str().unwrap();
    let args = json!({"id":id, "reason":"Fixed in branch example"});
    let denied = tool(&rig, token, "dismiss_friction", args.clone()).await.unwrap_err();
    assert!(denied.contains("unauthorized"), "{denied}");
    rig.client.call(methods::MUTATE, json!({"op":"createChat", "chatId":"dismiss-child", "spaceId":SPACE, "deviceId":rig.core.device_id, "parentChatId":CHAT})).await.unwrap();
    let result = tool(&rig, token, "dismiss_friction", args).await.unwrap();
    assert_eq!(result["report"]["status"], "dismissed");
    let list = tool(&rig, token, "list_friction", json!({"status":"dismissed"})).await.unwrap();
    assert_eq!(list["reports"][0]["dismissalReason"], "Fixed in branch example");
}

#[tokio::test(flavor = "multi_thread")]
async fn project_orchestrator_can_dismiss_friction() {
    let rig = rig().await;
    let project = project_rig(&rig).await;
    assign_chat(&rig, "dismiss-project-orch", CHAT, &project.project).await;
    assign_chat(&rig, "dismiss-project-worker", OTHER_CHAT, &project.ticket).await;
    let lease = rig.core.sessions.agent_tokens().mint(CHAT, "project-dismiss-run", SPACE, &rig.core.device_id);
    let row = tool(&rig, lease.token(), "report_friction", json!({"kind":"bug","summary":"project worker","whatHappened":"failed"})).await.unwrap();
    let id = row["report"]["id"].as_str().unwrap();
    let result = tool(&rig, lease.token(), "dismiss_friction", json!({"id":id,"reason":"Fixed in branch"})).await.unwrap();
    assert_eq!(result["report"]["status"], "dismissed");
}
