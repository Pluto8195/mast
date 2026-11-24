//! Turn a [`SmallWorld`] into real git repos and real engine stores inside a
//! scratch root, and hash the result logically.
//!
//! Everything is created through the engine's own APIs (workspace host, work
//! outline mutations) and through real git run via the logging shim. Nothing is
//! written outside the scratch root.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mast_engine::{EngineCore, EngineProfile, HarnessRegistry};
use mast_proto::HarnessId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::environment::ScratchDirectories;
use crate::world::{SmallWorld, WorktreeState};

/// Where a world's repositories live under `repos/`.
pub fn main_repo(scratch: &ScratchDirectories, world: &SmallWorld) -> PathBuf {
    scratch.repositories().join(&world.spaces[0].repository)
}

pub fn worktree_path(scratch: &ScratchDirectories, path: &str) -> PathBuf {
    scratch.repositories().join("worktrees").join(path)
}

fn other(message: impl Into<String>) -> std::io::Error {
    std::io::Error::other(message.into())
}

fn git(
    scratch: &ScratchDirectories,
    cwd: &Path,
    args: &[&str],
    at: i64,
) -> std::io::Result<String> {
    let date = format!("@{} +0000", at / 1000);
    let output = scratch.git(
        cwd,
        args,
        &[("GIT_AUTHOR_DATE", &date), ("GIT_COMMITTER_DATE", &date)],
    )?;
    if !output.status.success() {
        return Err(other(format!(
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Create the repo, a few real commits, branches and linked worktrees (some
/// dirty, some deleted).
pub fn materialize_repos(scratch: &ScratchDirectories, world: &SmallWorld) -> std::io::Result<()> {
    let mut repositories: Vec<_> = world.spaces.iter().map(|s| s.repository.as_str()).collect();
    repositories.extend(world.worktrees.iter().map(|w| w.repository.as_str()));
    repositories.sort_unstable();
    repositories.dedup();
    for (repository_index, repository) in repositories.iter().enumerate() {
        let repo = scratch.repositories().join(repository);
        std::fs::create_dir_all(&repo)?;
        let hour = 3_600_000;
        let at = |step: i64| world.clock_ms - 100 * hour + step * hour;
        git(scratch, &repo, &["init", "-q", "-b", "main"], at(0))?;
        let commits = [
            ("README.md", "# Fixture project\n"),
            ("src/lib.rs", "pub fn answer() -> u32 {\n    42\n}\n"),
            ("src/layout.rs", "pub const WIDTH: u32 = 400;\n"),
            ("docs/notes.md", "Notes for the fixture world.\n"),
        ];
        for (step, (path, body)) in commits.iter().enumerate() {
            let file = repo.join(path);
            std::fs::create_dir_all(file.parent().ok_or_else(|| other("no parent"))?)?;
            std::fs::write(&file, body)?;
            git(scratch, &repo, &["add", path], at(step as i64 + 1))?;
            let message = format!("Add {path}");
            git(
                scratch,
                &repo,
                &["commit", "-q", "-m", &message],
                at(step as i64 + 1),
            )?;
        }
        for branch in if world.shape.is_none() {
            vec!["feature/search", "fix/stale-chat"]
        } else {
            vec![]
        } {
            git(scratch, &repo, &["branch", branch], at(10))?;
        }
        std::fs::create_dir_all(scratch.repositories().join("worktrees"))?;
        let target_branches = world.shape.as_ref().map(|shape| {
            if repository_index == 0 {
                shape.branches_per_repository_max
            } else {
                shape.branches_per_repository_median
            }
        });
        if target_branches == Some(0) {
            return Err(other("a repository needs at least one branch"));
        }
        let mut named_branches = 1usize;
        for (index, worktree) in world.worktrees.iter().enumerate() {
            if worktree.repository != *repository {
                continue;
            }
            if worktree.main {
                continue;
            }
            let path = worktree_path(scratch, &worktree.path);
            let path_text = path.to_str().ok_or_else(|| other("non-UTF-8 path"))?;
            if target_branches.is_some_and(|target| named_branches >= target) {
                git(
                    scratch,
                    &repo,
                    &["worktree", "add", "-q", "--detach", path_text],
                    at(20),
                )?;
            } else {
                git(
                    scratch,
                    &repo,
                    &["worktree", "add", "-q", "-b", &worktree.branch, path_text],
                    at(20),
                )?;
                named_branches += 1;
            }
            let step = 30 + index as i64;
            match worktree.state {
                WorktreeState::Clean => {}
                WorktreeState::Dirty => {
                    std::fs::write(path.join("README.md"), "# Fixture project\n\nEdited.\n")?;
                    std::fs::write(path.join("scratch.txt"), "untracked\n")?;
                }
                WorktreeState::Deleted => {
                    // The directory is inside the scratch root this code created.
                    std::fs::remove_dir_all(&path)?;
                }
            }
            let _ = step;
        }
        if let Some(shape) = &world.shape {
            let target = target_branches.expect("shaped world has branch counts");
            let _ = shape;
            for index in 0..target.saturating_sub(named_branches) {
                git(
                    scratch,
                    &repo,
                    &["branch", &format!("fixture-branch-{index}")],
                    at(10),
                )?;
            }
        }
    }
    Ok(())
}

pub fn open_core(data: &Path) -> Result<EngineCore, String> {
    crate::preflight()?;
    let scratch = ScratchDirectories::from_env().map_err(|e| e.to_string())?;
    let canonical = data.canonicalize().map_err(|e| e.to_string())?;
    if canonical == scratch.root() || !canonical.starts_with(scratch.root()) {
        return Err("driver engine data must be inside the sandbox scratch root".into());
    }
    let profile = EngineProfile::local(data).map_err(|e| e.to_string())?;
    EngineCore::assemble_with_profile(
        profile,
        Arc::new(HarnessRegistry::new()),
        HarnessId::Mock,
        None,
    )
    .map_err(|e| e.to_string())
}

async fn outline(client: &mast_rpc::RpcClient, params: Value) -> Result<Value, String> {
    client
        .call(mast_rpc::methods::WORK_OUTLINE_MUTATE, params)
        .await
        .map_err(|e| e.to_string())
}

/// Create spaces, chats, transcripts and the outline in the engine store under
/// `scratch.data()`. Returns after the store is flushed and the engine shut down.
pub async fn materialize_engine(
    scratch: &ScratchDirectories,
    world: &SmallWorld,
) -> Result<(), String> {
    let core = open_core(scratch.data())?;
    for (index, space) in world.spaces.iter().enumerate() {
        let mut repo = scratch.repositories().join(&space.repository);
        if world.shape.is_some() {
            repo = repo.join("spaces").join(index.to_string());
            std::fs::create_dir_all(&repo).map_err(|e| e.to_string())?;
        }
        core.workspace
            .create_space(
                &space.id,
                &core.device_id,
                &repo.to_string_lossy(),
                Some(space.name.clone()),
                true,
            )
            .map_err(|e| e.to_string())?;
    }
    for chat in &world.chats {
        let cwd = worktree_path(scratch, &chat.worktree);
        core.workspace
            .create_chat(
                &chat.id,
                Some(&chat.space_id),
                None,
                None,
                Some(cwd.to_string_lossy().into_owned()),
            )
            .map_err(|e| e.to_string())?;
        core.workspace
            .rename_chat(&chat.id, &chat.title)
            .map_err(|e| e.to_string())?;
        if chat.archived {
            core.workspace
                .set_chat_archived(&chat.id, true)
                .map_err(|e| e.to_string())?;
        }
        let handle = core.doc_host.open(&chat.id).map_err(|e| e.to_string())?;
        for entry in &chat.entries {
            let entry: mast_doc::SessionMessageEntry =
                serde_json::from_value(entry.clone()).map_err(|e| e.to_string())?;
            handle
                .doc()
                .push_message(&entry)
                .map_err(|e| e.to_string())?;
        }
    }
    let client = mast_rpc::memory_client(core.rpc_service());
    for item in &world.items {
        let mut params = json!({
            "op": "createWorkItem", "spaceId": item.space_id, "title": item.title,
            "status": item.status, "operationId": item.operation_id,
        });
        if let Some(parent) = item.parent {
            params["parentWorkItemId"] = json!(world.items[parent].id);
        }
        outline(&client, params).await?;
    }
    if let Some(shape) = &world.shape {
        if shape.chat_pr_links > 0 && world.chats.is_empty() {
            return Err("PR links need chats".into());
        }
        for index in 0..shape.chat_pr_links {
            core.workspace
                .add_chat_pr_link(
                    &world.chats[index % world.chats.len()].id,
                    &format!("https://github.com/fixture/repository/pull/{}", index + 1),
                    mast_proto::ChatLinkSource::Manual,
                )
                .map_err(|e| e.to_string())?;
        }
        if shape.item_pr_refs > 0 && world.items.is_empty() {
            return Err("PR references need items".into());
        }
        for index in 0..shape.item_pr_refs {
            outline(&client, json!({"op": "createWorkItemExternalReference",
                "workItemId": world.items[index % world.items.len()].id, "provider": "github",
                "referenceType": "pullRequest", "externalId": format!("fixture/repository#{}", index + 1),
                "url": format!("https://github.com/fixture/repository/pull/{}", index + 1),
                "operationId": format!("fixture-ref-{index}")})).await?;
        }
    }
    for (index, (chat, item)) in (if world.shape.is_none() {
        vec![(0usize, 3usize), (1, 4), (2, 8)]
    } else {
        vec![]
    })
    .into_iter()
    .enumerate()
    {
        outline(
            &client,
            json!({"op": "assignChatToWorkItem", "chatId": world.chats[chat].id,
                   "workItemId": world.items[item].id,
                   "operationId": format!("{}-assign-{index}", world.items[0].operation_id)}),
        )
        .await?;
    }
    if world.shape.is_some() {
        let mut items_by_space: std::collections::HashMap<&str, Vec<_>> =
            std::collections::HashMap::new();
        for item in &world.items {
            items_by_space.entry(&item.space_id).or_default().push(item);
        }
        let mut positions = std::collections::HashMap::new();
        for (index, chat) in world.chats.iter().enumerate() {
            let Some(items) = items_by_space.get(chat.space_id.as_str()) else {
                continue;
            };
            let position = positions.entry(&chat.space_id).or_insert(0usize);
            let item = items[*position % items.len()];
            *position += 1;
            outline(
                &client,
                json!({"op": "assignChatToWorkItem", "chatId": chat.id,
                "workItemId": item.id, "operationId": format!("fixture-assign-{index}")}),
            )
            .await?;
        }
    }
    for (index, request) in world.requests.iter().enumerate() {
        let operation = if world.shape.is_some() {
            format!("fixture-{:016x}-request-{index}", world.seed)
        } else {
            format!("{}-request-{index}", world.items[0].operation_id)
        };
        let created = outline(
            &client,
            json!({"op": "createUserActionRequest", "workItemId": world.items[request.item].id,
                   "origin": {"kind": "chat", "chatId": world.chats[request.chat].id},
                   "text": request.text, "info": request.detail,
                   "dedupeKey": operation, "operationId": operation}),
        )
        .await?;
        if request.answered {
            outline(
                &client,
                json!({"op": "resolveUserActionRequest",
                       "userActionRequestId": created["userActionRequestId"],
                       "note": "Use a new worktree.", "operationId": format!("{operation}-resolve")}),
            )
            .await?;
        }
    }
    if let Some(shape) = &world.shape {
        for index in 0..shape.friction_reports {
            let space = &world.spaces[index % world.spaces.len()];
            let report = serde_json::from_value(json!({
                "id": format!("friction-{index}"), "kind": "bug", "summary": format!("Fixture report {index}"),
                "whatHappened": "A synthetic issue occurred.", "workaround": null,
                "suggestion": null, "approval": null, "severity": null,
                "provenance": {"chatId": "fixture-chat", "harness": "mock", "model": null,
                    "spaceId": space.id, "workItemId": null, "branch": null,
                    "createdAt": world.clock_ms, "source": "agent"},
                "fingerprint": format!("fixture-friction-{index}"), "occurrences": 1,
                "lastSeen": world.clock_ms, "status": "open"
            })).map_err(|e| format!("invalid fixture friction: {e}"))?;
            core.workspace
                .record_friction(report)
                .map_err(|e| e.to_string())?;
        }
    }
    core.shutdown().await;
    Ok(())
}

/// Build the whole world: repos first (chats point at worktrees), then stores.
pub async fn materialize(scratch: &ScratchDirectories, world: &SmallWorld) -> Result<(), String> {
    materialize_repos(scratch, world).map_err(|e| e.to_string())?;
    if let Some(shape) = &world.shape {
        std::fs::write(
            scratch.data().join("driver-pr-count"),
            shape.chat_pr_links.max(shape.item_pr_refs).to_string(),
        )
        .map_err(|e| e.to_string())?;
    }
    materialize_engine(scratch, world).await
}

fn relative(root: &Path, path: &str) -> String {
    path.strip_prefix(root.to_str().unwrap_or_default())
        .unwrap_or(path)
        .to_owned()
}

/// Hash of the world's logical content: engine rows, transcripts, outline and
/// git refs/status. Paths are taken relative to the scratch root and wall-clock
/// stamps are excluded, so the same seed hashes the same in any scratch root.
pub fn snapshot_hash(
    scratch: &ScratchDirectories,
    world: &SmallWorld,
    core: &EngineCore,
) -> Result<String, String> {
    let lines = snapshot_lines(scratch, world, core)?;
    let mut hasher = Sha256::new();
    for line in &lines {
        hasher.update(line.as_bytes());
        hasher.update(*b"\n");
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// The logical lines [`snapshot_hash`] hashes (also useful to diff two worlds).
pub fn snapshot_lines(
    scratch: &ScratchDirectories,
    world: &SmallWorld,
    core: &EngineCore,
) -> Result<Vec<String>, String> {
    let root = scratch.root();
    let mut lines: Vec<String> = Vec::new();
    for space in &world.spaces {
        let row = core
            .workspace
            .space(&space.id)
            .map_err(|e| e.to_string())?
            .ok_or("space missing")?;
        lines.push(format!(
            "space {} {:?} {}",
            space.id,
            row.name,
            relative(root, &row.path)
        ));
    }
    let mut chats = core.workspace.read_chats().map_err(|e| e.to_string())?;
    chats.sort_by(|a, b| a.id.cmp(&b.id));
    for chat in chats {
        lines.push(format!(
            "chat {} {:?} {} archived={}",
            chat.id,
            chat.title,
            relative(root, &chat.cwd.clone().unwrap_or_default()),
            chat.archived
        ));
        let handle = core.doc_host.open(&chat.id).map_err(|e| e.to_string())?;
        for entry in handle.doc().read_entries().map_err(|e| e.to_string())? {
            lines.push(format!(
                "entry {}",
                serde_json::to_string(&entry).map_err(|e| e.to_string())?
            ));
        }
        for link in core
            .workspace
            .chat_pr_links(&chat.id)
            .map_err(|e| e.to_string())?
        {
            lines.push(format!(
                "chat-pr {} {} {:?}",
                chat.id, link.url, link.source
            ));
        }
    }
    let service = mast_engine::work_outline::WorkOutlineService::new(core.workspace.clone());
    for space in &world.spaces {
        let snapshot = service.snapshot(&space.id);
        for item in &snapshot.items {
            lines.push(format!(
                "item {} parent={:?} {:?} {:?}/{:?}",
                item.id, item.parent_work_item_id, item.title, item.status, item.effective_status
            ));
        }
        for assignment in &snapshot.assignments {
            // Not the whole view: it carries the device id and a wall-clock stamp.
            lines.push(format!(
                "assignment {} {:?} {:?}",
                assignment.chat_id, assignment.work_item_id, assignment.state
            ));
        }
        let mut requests = snapshot.user_action_requests.clone();
        requests.sort_by(|a, b| a.id.cmp(&b.id));
        for request in requests {
            lines.push(format!(
                "request {} {:?} open={}",
                request.id,
                request.text,
                request.is_open()
            ));
        }
        for reference in &snapshot.external_references {
            lines.push(format!(
                "reference {} {} {}",
                reference.work_item_id, reference.reference_type, reference.url
            ));
        }
    }
    let mut repositories: Vec<_> = world.spaces.iter().map(|s| s.repository.as_str()).collect();
    repositories.extend(world.worktrees.iter().map(|w| w.repository.as_str()));
    repositories.sort_unstable();
    repositories.dedup();
    for repository in repositories {
        let repo = scratch.repositories().join(repository);
        for (cwd, args) in [
            (
                &repo,
                vec!["for-each-ref", "--format=%(refname) %(objectname)"],
            ),
            (&repo, vec!["worktree", "list", "--porcelain"]),
        ] {
            let out = git(scratch, cwd, &args, world.clock_ms).map_err(|e| e.to_string())?;
            lines.push(out.replace(root.to_str().unwrap_or_default(), "<root>"));
        }
    }
    for worktree in &world.worktrees {
        let path = worktree_path(scratch, &worktree.path);
        if path.is_dir() {
            lines.push(format!(
                "status {} {}",
                worktree.path,
                git(scratch, &path, &["status", "--porcelain"], world.clock_ms)
                    .map_err(|e| e.to_string())?
            ));
        } else {
            lines.push(format!("status {} deleted", worktree.path));
        }
    }
    Ok(lines)
}

/// Read the world back out of the stores as UI rows (the first chat selected).
pub fn load_rows(
    world: &SmallWorld,
    core: &EngineCore,
) -> Result<mast_ui::snapshot::WorldRows, String> {
    let mut spaces = Vec::new();
    for space in &world.spaces {
        spaces.push(
            core.workspace
                .space(&space.id)
                .map_err(|e| e.to_string())?
                .ok_or("space missing")?,
        );
    }
    let mut chats = core.workspace.read_chats().map_err(|e| e.to_string())?;
    // Deterministic order: the world's own chat order.
    chats.sort_by_key(|chat| world.chats.iter().position(|c| c.id == chat.id));
    let selected = world.chats.first().map(|chat| chat.id.clone());
    let mut transcript = Vec::new();
    if let Some(id) = &selected {
        let handle = core.doc_host.open(id).map_err(|e| e.to_string())?;
        for entry in handle.doc().read_entries().map_err(|e| e.to_string())? {
            transcript.push(serde_json::to_value(entry).map_err(|e| e.to_string())?);
        }
    }
    let service = mast_engine::work_outline::WorkOutlineService::new(core.workspace.clone());
    Ok(mast_ui::snapshot::WorldRows {
        spaces,
        chats,
        outline: service.snapshot(&world.spaces[0].id),
        selected_chat: selected,
        transcript,
    })
}
