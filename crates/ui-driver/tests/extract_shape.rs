//! Explicitly invoked local shape extraction from a SQLite backup.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::os::unix::fs::{MetadataExt, PermissionsExt};

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

// A linked worktree can point its .git file at a different directory. Check
// that pointer before invoking git, so it cannot open the live Mast store.
fn safe_repository(path: &Path, live_stores: &[PathBuf]) -> bool {
    for ancestor in path.ancestors() {
        let dot_git = ancestor.join(".git");
        let Ok(metadata) = std::fs::symlink_metadata(&dot_git) else {
            continue;
        };
        let git_dir = if metadata.is_file() {
            let Ok(pointer) = std::fs::read_to_string(&dot_git) else {
                return false;
            };
            let Some(target) = pointer.trim().strip_prefix("gitdir: ") else {
                return false;
            };
            ancestor.join(target)
        } else {
            dot_git
        };
        let Ok(git_dir) = git_dir.canonicalize() else {
            return false;
        };
        if live_stores.iter().any(|store| git_dir.starts_with(store)) {
            return false;
        }
        if let Ok(common) = std::fs::read_to_string(git_dir.join("commondir")) {
            let Ok(common) = git_dir.join(common.trim()).canonicalize() else {
                return false;
            };
            if live_stores.iter().any(|store| common.starts_with(store)) {
                return false;
            }
        }
        return true;
    }
    false
}

fn account_home() -> Result<PathBuf, Box<dyn std::error::Error>> {
    // getpwuid_r reads the account database, independent of a caller's HOME.
    let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0u8; 16384];
    // SAFETY: record, result and the writable buffer remain valid for the call.
    let status = unsafe { libc::getpwuid_r(libc::getuid(), record.as_mut_ptr(), buffer.as_mut_ptr().cast(), buffer.len(), &mut result) };
    if status != 0 || result.is_null() { return Err("cannot resolve account home".into()); }
    // SAFETY: a successful lookup initialized record and returned its address.
    let record = unsafe { record.assume_init() };
    if record.pw_dir.is_null() { return Err("cannot resolve account home".into()); }
    // SAFETY: pw_dir points into the live buffer returned by getpwuid_r.
    let home = unsafe { std::ffi::CStr::from_ptr(record.pw_dir) }.to_str()?;
    Ok(PathBuf::from(home).canonicalize()?)
}

fn validate_output(output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let parent = output.parent().ok_or("output needs a parent directory")?.canonicalize()?;
    if output.file_name().is_none() || output.exists() { return Err("output must be a new file".into()); }
    for ancestor in parent.ancestors() {
        if ancestor.join(".git").exists() { return Err("output must be outside every git work tree".into()); }
    }
    let output = mast_harness::git_command().current_dir(&parent)
        .args(["rev-parse", "--is-inside-work-tree"]).env("LC_ALL", "C")
        .env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE").env_remove("GIT_CEILING_DIRECTORIES")
        .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null").output()?;
    if output.status.success() || !String::from_utf8_lossy(&output.stderr).contains("not a git repository") {
        return Err("output must be outside every git work tree".into());
    }
    Ok(())
}

fn write_shape_and_remove_source(output: &Path, source: &Path, shape: &WorldShape) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
    file.write_all(&serde_json::to_vec_pretty(shape)?)?;
    drop(file);
    std::fs::remove_file(source)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = source.as_os_str().to_os_string();
        sidecar.push(suffix);
        match std::fs::remove_file(sidecar) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn validate_source(source: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let metadata = std::fs::symlink_metadata(source)?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.uid() != unsafe { libc::getuid() } {
        return Err("source must be an owned, single-link regular file".into());
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = source.as_os_str().to_os_string();
        sidecar.push(suffix);
        if std::fs::symlink_metadata(sidecar).is_ok() {
            return Err("source backup must not have SQLite sidecars".into());
        }
    }
    let source = source.canonicalize()?;
    let parent = source.parent().ok_or("source needs a parent directory")?;
    let parent_metadata = std::fs::metadata(parent)?;
    if parent.parent() != Some(Path::new("/private/tmp"))
        || !parent.file_name().is_some_and(|name| name.to_string_lossy().starts_with("ui-driver-shape."))
        || parent_metadata.uid() != unsafe { libc::getuid() }
        || parent_metadata.permissions().mode() & 0o777 != 0o700 {
        return Err("source must be in an owned private shape directory".into());
    }
    Ok(source)
}

#[test]
fn extraction_paths_refuse_git_outputs_and_hard_linked_sources() {
    let root = tempfile::Builder::new().prefix("ui-driver-shape.").tempdir_in("/private/tmp").unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = root.path().join("docs.sqlite3");
    std::fs::write(&source, b"fixture").unwrap();
    assert!(validate_source(&source).is_ok(), "fixture source rejected: {:?}", validate_source(&source).err());
    let alias = root.path().join("alias");
    std::fs::hard_link(&source, &alias).unwrap();
    assert!(validate_source(&source).is_err());
    std::fs::remove_file(alias).unwrap();
    assert!(validate_output(&root.path().join("shape.json")).is_ok());
    let repo = root.path().join("repository");
    std::fs::create_dir(&repo).unwrap();
    let status = mast_harness::git_command().current_dir(&repo).args(["init", "-q"]).status().unwrap();
    assert!(status.success());
    assert!(validate_output(&repo.join("shape.json")).is_err());
    let shape = WorldShape::medium();
    let output = root.path().join("shape.json");
    write_shape_and_remove_source(&output, &source, &shape).unwrap();
    assert!(!source.exists());
    assert_eq!(shape, serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap());
}

#[test]
#[ignore = "reads a local backup and writes a private shape; invoke explicitly"]
fn extract_shape_from_backup() {
    extract().expect("shape extraction failed; source and output details are private");
}

fn extract() -> Result<(), Box<dyn std::error::Error>> {
    let source = validate_source(&PathBuf::from(std::env::var_os("MAST_SHAPE_SOURCE").ok_or("missing source")?))?;
    let output = PathBuf::from(std::env::var_os("MAST_SHAPE_OUTPUT").ok_or("missing output")?);
    validate_output(&output)?;
    let db = Connection::open_with_flags(&source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
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
    let home = account_home()?;
    let live_stores: Vec<_> = [".zeron", ".zeron-dev", ".mast", ".mast-dev", ".claude", ".codex"].into_iter().flat_map(|name| {
        let path = home.join(name);
        [Some(path.clone()), path.canonicalize().ok()].into_iter().flatten()
    }).collect();
    for path in candidates {
        if live_stores.iter().any(|store| path.starts_with(store)) {
            continue;
        }
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if live_stores.iter().any(|store| canonical.starts_with(store)) || !safe_repository(&canonical, &live_stores) {
            continue;
        }
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
    drop(db);
    write_shape_and_remove_source(&output, &source, &shape)?;
    Ok(())
}
