//! Explicitly invoked local shape extraction from a SQLite backup.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use mast_doc::{REGISTRY_DOC_ID, RegistryDoc, SessionDoc};
use mast_proto::work_outline::WorkItemStatus;
use mast_ui_driver::shape::WorldShape;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

fn percentile(values: &mut [usize], numerator: usize, denominator: usize) -> usize {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[((values.len() - 1) * numerator).div_ceil(denominator)]
}

// Suppress all git output on failure: it may contain private paths or names.
fn git(path: &Path, args: &[&str]) -> Option<String> {
    let output = mast_harness::git_command()
        .current_dir(path)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
#[ignore = "reads a local backup and writes a private shape; invoke explicitly"]
fn extract_shape_from_backup() {
    extract().expect("shape extraction failed; source and output details are private");
}

fn extract() -> Result<(), Box<dyn std::error::Error>> {
    let source = PathBuf::from(std::env::var_os("MAST_SHAPE_SOURCE").ok_or("missing source")?)
        .canonicalize()?;
    if !source.starts_with("/private/tmp/ui-driver-shape") {
        return Err("source must be a backup in the shape scratch directory".into());
    }
    let output = PathBuf::from(std::env::var_os("MAST_SHAPE_OUTPUT").ok_or("missing output")?);
    let db = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let snapshot = |id: &str| -> rusqlite::Result<Option<Vec<u8>>> {
        db.query_row(
            "SELECT bytes FROM snapshots WHERE doc_id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()
    };
    let registry = RegistryDoc::from_bytes(
        &snapshot(REGISTRY_DOC_ID)?.ok_or("registry missing")?,
        "shape-reader",
    )?;
    let spaces = registry.read_spaces()?;
    let chats = registry.read_chats()?;
    let mut chats_per_space: Vec<usize> = spaces
        .iter()
        .map(|space| {
            chats
                .iter()
                .filter(|chat| chat.space_id.as_deref() == Some(&space.id))
                .count()
        })
        .collect();
    let mut message_counts = Vec::with_capacity(chats.len());
    for chat in &chats {
        let count = if let Some(bytes) = snapshot(&chat.id)? {
            let doc = loro::LoroDoc::new();
            doc.import(&bytes)?;
            SessionDoc::from_doc(doc).read_entries()?.len()
        } else {
            0
        };
        message_counts.push(count);
    }
    let mut candidates = BTreeSet::new();
    candidates.extend(spaces.iter().map(|s| PathBuf::from(&s.path)));
    candidates.extend(
        chats
            .iter()
            .filter_map(|chat| chat.cwd.as_ref().map(PathBuf::from)),
    );
    let mut repos = BTreeMap::new();
    for path in candidates {
        let Some(common) = git(
            &path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        ) else {
            continue;
        };
        let Ok(common) = PathBuf::from(common.trim()).canonicalize() else {
            continue;
        };
        if repos.contains_key(&common) {
            continue;
        }
        let worktrees = git(&path, &["worktree", "list", "--porcelain"])
            .ok_or("cannot count worktrees")?
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count();
        let branches = git(
            &path,
            &["for-each-ref", "--format=%(refname)", "refs/heads/"],
        )
        .ok_or("cannot count branches")?
        .lines()
        .count();
        repos.insert(common, (worktrees, branches));
    }
    let mut worktrees: Vec<_> = repos.values().map(|v| v.0).collect();
    let mut branches: Vec<_> = repos.values().map(|v| v.1).collect();
    let mut shape = WorldShape {
        spaces: spaces.len(),
        repositories: repos.len(),
        worktrees_per_repository_max: worktrees.iter().copied().max().unwrap_or(0),
        worktrees_per_repository_median: percentile(&mut worktrees, 1, 2),
        chats_per_space_max: chats_per_space.iter().copied().max().unwrap_or(0),
        chats_per_space_median: percentile(&mut chats_per_space, 1, 2),
        chats: chats.len(),
        archived_chats: chats.iter().filter(|c| c.archived).count(),
        pending_items: 0,
        active_items: 0,
        completed_items: 0,
        outline_depth: 0,
        chat_pr_links: chats
            .iter()
            .map(|chat| {
                let links = registry.chat_pr_links(&chat.id);
                let legacy = match (&chat.linked_pr_url, &chat.linked_pr_source) {
                    (Some(url), Some(_)) => {
                        !links.iter().any(|link| link.url.eq_ignore_ascii_case(url))
                    }
                    _ => false,
                };
                links.len() + usize::from(legacy)
            })
            .sum(),
        item_pr_refs: 0,
        open_requests: 0,
        resolved_requests: 0,
        friction_reports: 0,
        messages_per_chat_median: percentile(&mut message_counts, 1, 2),
        messages_per_chat_p90: percentile(&mut message_counts, 9, 10),
        branches_per_repository_median: percentile(&mut branches, 1, 2),
        branches_per_repository_max: branches.iter().copied().max().unwrap_or(0),
    };
    for space in &spaces {
        let outline = registry.work_outline_snapshot(&space.id, false);
        for item in &outline.items {
            match item.status {
                WorkItemStatus::Pending => shape.pending_items += 1,
                WorkItemStatus::Active => shape.active_items += 1,
                WorkItemStatus::Completed => shape.completed_items += 1,
            }
            shape.outline_depth = shape.outline_depth.max(item.depth as usize + 1);
        }
        shape.item_pr_refs += outline
            .external_references
            .iter()
            .filter(|r| r.provider == "github" && r.reference_type == "pullRequest")
            .count();
        shape.open_requests += outline
            .user_action_requests
            .iter()
            .filter(|r| r.is_open())
            .count();
        shape.resolved_requests += outline
            .user_action_requests
            .iter()
            .filter(|r| !r.is_open())
            .count();
        shape.friction_reports += registry.friction_reports(&space.id).len();
    }
    std::fs::write(output, serde_json::to_vec_pretty(&shape)?)?;
    Ok(())
}
