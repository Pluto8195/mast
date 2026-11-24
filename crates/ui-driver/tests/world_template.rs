//! B2b: the small world materialized into real repos and engine stores, the
//! snapshot hash, the template and reset.
#![allow(clippy::disallowed_methods)]
use std::path::PathBuf;

use mast_ui_driver::environment::ScratchDirectories;
use mast_ui_driver::materialize::{self, main_repo, snapshot_hash, worktree_path};
use mast_ui_driver::template::Template;
use mast_ui_driver::world::SmallWorld;

static ISOLATED_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn engine_open_rejects_unguarded_process() {
    for spoofed in [false, true] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "unguarded_engine_open",
                "--nocapture",
            ])
            .env_clear();
        if spoofed {
            command
                .env("MAST_SOAK", "1")
                .env("MAST_SOAK_SHIM_DIR", "/nonexistent/mast-driver-shims")
                .env("PATH", "/usr/bin:/bin");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "unguarded engine rejection test failed"
        );
    }
}

#[test]
#[ignore = "launched with a cleared environment by engine_open_rejects_unguarded_process"]
fn unguarded_engine_open() {
    let dir = tempfile::tempdir().unwrap();
    let error = match materialize::open_core(dir.path()) {
        Ok(_) => panic!("unguarded engine unexpectedly started"),
        Err(error) => error,
    };
    assert!(error.contains("MAST_SOAK=1") || error.contains("soak guard"));
    assert!(!dir.path().join("profiles").exists());
}

#[test]
fn driver_requires_external_import_switch() {
    if run_isolated("driver_requires_external_import_switch", false) {
        return;
    }
    let output = std::process::Command::new("/usr/bin/env")
        .args(["-u", "MAST_DISABLE_EXTERNAL_IMPORT"])
        .arg(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "engine_open_without_import_switch"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "engine accepted a missing importer switch"
    );
}

#[test]
#[ignore = "launched inside the sandbox without the importer switch"]
fn engine_open_without_import_switch() {
    let scratch = ScratchDirectories::from_env().unwrap();
    let error = match materialize::open_core(scratch.data()) {
        Ok(_) => panic!("engine unexpectedly started without importer switch"),
        Err(error) => error,
    };
    assert!(error.contains("MAST_DISABLE_EXTERNAL_IMPORT=1"));
    assert!(!scratch.data().join("profiles").exists());
}

#[test]
fn driver_disables_external_session_import() {
    if run_isolated("driver_disables_external_session_import", false) {
        return;
    }
    assert_eq!(
        std::env::var("MAST_DISABLE_EXTERNAL_IMPORT").as_deref(),
        Ok("1")
    );
    let scratch = ScratchDirectories::create(&account()).unwrap();
    let runtime = rt();
    runtime.block_on(async {
        let core = materialize::open_core(scratch.data()).unwrap();
        let importer = &core.external_import;
        let disabled = |error: Option<mast_engine::EngineError>| {
            assert!(error.is_some_and(|error| {
                error
                    .to_string()
                    .contains("external session import is disabled")
            }));
        };
        disabled(importer.scan().err());
        disabled(importer.import("fixture", "fixture", scratch.root()).err());
        disabled(importer.sync("fixture").err());
        disabled(importer.sync_stale_imports().err());
        disabled(importer.repair_missing_timestamps().err());
        disabled(importer.repair_missing_config().err());
        disabled(importer.repair_missing_titles(&core.context_usage).err());
        disabled(importer.repair_junk_imports().err());
        disabled(importer.repair_missing_links(&core.context_usage).err());
        disabled(importer.transcript_path_for("fixture").err());
        disabled(importer.auto_adopt_external_sessions().err());
        disabled(importer.bulk_import_from_session_canvas(|_| {}).err());
        core.shutdown().await;
    });
    assert!(
        !scratch
            .data()
            .join("profiles/local/external_import_cursors")
            .exists()
    );
    finish(scratch);
}

/// Engine startup must always be guarded: otherwise background import can
/// inspect the account's real agent sessions while a fixture is being built.
fn run_isolated(name: &str, private_shape: bool) -> bool {
    if std::env::var_os("MAST_DRIVER_ROOT").is_some() {
        return false;
    }
    let _guard = ISOLATED_TEST.lock().unwrap();
    let home = account();
    let scratch = ScratchDirectories::create(&home).unwrap();
    if private_shape {
        let source = std::env::var_os("MAST_UI_DRIVER_SHAPE").expect("shape path required");
        std::fs::copy(source, scratch.root().join("shape.json")).unwrap();
    }
    let executable = std::env::current_exe().unwrap();
    let mut args = vec!["--exact", name, "--nocapture", "--test-threads=1"];
    if private_shape {
        args.push("--ignored");
    }
    let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    let output = scratch.run_isolated(&home, &executable, &args).unwrap();
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    finish(scratch);
    assert!(
        output.status.success(),
        "isolated world test failed: {}",
        output.status
    );
    true
}

fn account() -> PathBuf {
    // Never touched: only used to prove the scratch root does not overlap it.
    PathBuf::from(std::env::var_os("HOME").expect("HOME"))
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn finish(scratch: ScratchDirectories) {
    // The scratch root is a mktemp directory this test created.
    for attempt in 0..20 {
        match std::fs::remove_dir_all(scratch.root()) {
            Ok(()) => return,
            Err(error) if attempt < 19 && error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(error) => panic!("could not remove driver scratch: {error}"),
        }
    }
}

#[test]
fn medium_world_materializes() {
    if run_isolated("medium_world_materializes", false) {
        return;
    }
    let rt = rt();
    let scratch = ScratchDirectories::create(&account()).unwrap();
    let shape = mast_ui_driver::shape::WorldShape::medium();
    let world = SmallWorld::shaped(1, "medium", &shape).unwrap();
    let started = std::time::Instant::now();
    rt.block_on(async {
        materialize::materialize(&scratch, &world).await.unwrap();
        let core = materialize::open_core(scratch.data()).unwrap();
        let chats = core.workspace.read_chats().unwrap();
        assert_eq!(chats.len(), shape.chats);
        assert_eq!(
            chats.iter().filter(|chat| chat.archived).count(),
            shape.archived_chats
        );
        let links: usize = chats
            .iter()
            .map(|chat| core.workspace.chat_pr_links(&chat.id).unwrap().len())
            .sum();
        assert_eq!(links, shape.chat_pr_links);
        let service = mast_engine::work_outline::WorkOutlineService::new(core.workspace.clone());
        let snapshots: Vec<_> = world
            .spaces
            .iter()
            .map(|space| service.snapshot(&space.id))
            .collect();
        assert_eq!(
            snapshots
                .iter()
                .map(|snapshot| snapshot.items.len())
                .sum::<usize>(),
            shape.pending_items + shape.active_items + shape.completed_items
        );
        assert_eq!(
            snapshots
                .iter()
                .map(|snapshot| snapshot.external_references.len())
                .sum::<usize>(),
            shape.item_pr_refs
        );
        assert_eq!(
            snapshots
                .iter()
                .flat_map(|snapshot| &snapshot.user_action_requests)
                .filter(|request| request.is_open())
                .count(),
            shape.open_requests
        );
        assert_eq!(
            snapshots
                .iter()
                .flat_map(|snapshot| &snapshot.user_action_requests)
                .filter(|request| !request.is_open())
                .count(),
            shape.resolved_requests
        );
        assert_eq!(
            world
                .spaces
                .iter()
                .map(|space| core.workspace.friction_reports(&space.id).len())
                .sum::<usize>(),
            shape.friction_reports
        );
        core.shutdown().await;
    });
    eprintln!("medium materialization: {:?}", started.elapsed());
    finish(scratch);
}

#[test]
fn synthetic_scale_world_matches_shape_and_hash() {
    if run_isolated("synthetic_scale_world_matches_shape_and_hash", false) {
        return;
    }
    let mut shape = mast_ui_driver::shape::WorldShape::medium();
    shape.spaces = 3;
    shape.repositories = 2;
    shape.worktrees_per_repository_max = 3;
    shape.worktrees_per_repository_median = 3;
    shape.chats_per_space_max = 3;
    shape.chats_per_space_median = 2;
    shape.chats = 6;
    shape.archived_chats = 1;
    shape.pending_items = 3;
    shape.active_items = 3;
    shape.completed_items = 3;
    shape.outline_depth = 3;
    shape.chat_pr_links = 4;
    shape.item_pr_refs = 3;
    shape.open_requests = 2;
    shape.resolved_requests = 1;
    shape.friction_reports = 2;
    shape.messages_per_chat_median = 2;
    shape.messages_per_chat_p90 = 5;
    shape.branches_per_repository_max = 4;
    shape.branches_per_repository_median = 4;
    let runtime = rt();
    let hash_for = || {
        let world = SmallWorld::shaped(9, "scale", &shape).unwrap();
        let scratch = ScratchDirectories::create(&account()).unwrap();
        let hash = runtime.block_on(async {
            materialize::materialize(&scratch, &world).await.unwrap();
            let core = materialize::open_core(scratch.data()).unwrap();
            let service =
                mast_engine::work_outline::WorkOutlineService::new(core.workspace.clone());
            let snapshots: Vec<_> = world
                .spaces
                .iter()
                .map(|s| service.snapshot(&s.id))
                .collect();
            let chats = core.workspace.read_chats().unwrap();
            assert_eq!(chats.len(), shape.chats);
            assert_eq!(
                chats.iter().filter(|c| c.archived).count(),
                shape.archived_chats
            );
            assert_eq!(
                chats
                    .iter()
                    .map(|c| core.workspace.chat_pr_links(&c.id).unwrap().len())
                    .sum::<usize>(),
                shape.chat_pr_links
            );
            assert_eq!(snapshots.iter().map(|s| s.items.len()).sum::<usize>(), 9);
            for (status, expected) in [
                (mast_proto::WorkItemStatus::Pending, shape.pending_items),
                (mast_proto::WorkItemStatus::Active, shape.active_items),
                (mast_proto::WorkItemStatus::Completed, shape.completed_items),
            ] {
                assert_eq!(
                    snapshots
                        .iter()
                        .flat_map(|s| &s.items)
                        .filter(|item| item.status == status)
                        .count(),
                    expected
                );
            }
            assert_eq!(
                snapshots
                    .iter()
                    .flat_map(|s| &s.items)
                    .map(|item| item.depth as usize + 1)
                    .max()
                    .unwrap(),
                shape.outline_depth
            );
            assert_eq!(
                snapshots
                    .iter()
                    .map(|s| s.external_references.len())
                    .sum::<usize>(),
                shape.item_pr_refs
            );
            assert_eq!(
                snapshots
                    .iter()
                    .flat_map(|s| &s.user_action_requests)
                    .filter(|r| r.is_open())
                    .count(),
                shape.open_requests
            );
            assert_eq!(
                snapshots
                    .iter()
                    .flat_map(|s| &s.user_action_requests)
                    .filter(|r| !r.is_open())
                    .count(),
                shape.resolved_requests
            );
            assert_eq!(
                world
                    .spaces
                    .iter()
                    .map(|s| core.workspace.friction_reports(&s.id).len())
                    .sum::<usize>(),
                shape.friction_reports
            );
            let hash = snapshot_hash(&scratch, &world, &core).unwrap();
            core.shutdown().await;
            hash
        });
        let mut messages: Vec<_> = world.chats.iter().map(|chat| chat.entries.len()).collect();
        messages.sort_unstable();
        assert_eq!(messages[messages.len() / 2], shape.messages_per_chat_median);
        assert_eq!(
            messages[((messages.len() - 1) * 9).div_ceil(10)],
            shape.messages_per_chat_p90
        );
        for repo in 0..shape.repositories {
            let cwd = scratch.repositories().join(format!("repository-{repo}"));
            let worktrees = scratch
                .git(&cwd, &["worktree", "list", "--porcelain"], &[])
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&worktrees.stdout)
                    .lines()
                    .filter(|line| line.starts_with("worktree "))
                    .count(),
                shape.worktrees_per_repository_median
            );
            let branches = scratch
                .git(
                    &cwd,
                    &["for-each-ref", "--format=%(refname)", "refs/heads/"],
                    &[],
                )
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&branches.stdout).lines().count(),
                shape.branches_per_repository_median
            );
        }
        finish(scratch);
        hash
    };
    assert_eq!(hash_for(), hash_for());
}

#[test]
#[ignore = "requires a private shape file"]
fn private_scale_world_materializes() {
    if run_isolated("private_scale_world_materializes", true) {
        return;
    }
    let path = PathBuf::from(std::env::var_os("MAST_DRIVER_ROOT").unwrap()).join("shape.json");
    let shape: mast_ui_driver::shape::WorldShape =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let world = SmallWorld::shaped(1, "scale", &shape).unwrap();
    let scratch = ScratchDirectories::create(&account()).unwrap();
    let runtime = rt();
    let started = std::time::Instant::now();
    runtime
        .block_on(materialize::materialize(&scratch, &world))
        .unwrap();
    eprintln!("scale materialization: {:?}", started.elapsed());
    finish(scratch);
}

#[test]
fn same_seed_same_hash_different_seed_different_hash() {
    if run_isolated("same_seed_same_hash_different_seed_different_hash", false) {
        return;
    }
    let rt = rt();
    let hash_for = |seed: u64| {
        let scratch = ScratchDirectories::create(&account()).unwrap();
        let world = SmallWorld::small(seed);
        let hash = rt.block_on(async {
            materialize::materialize(&scratch, &world).await.unwrap();
            let core = materialize::open_core(scratch.data()).unwrap();
            let hash = snapshot_hash(&scratch, &world, &core).unwrap();
            if std::env::var_os("MAST_DRIVER_DEBUG_LINES").is_some() {
                for line in materialize::snapshot_lines(&scratch, &world, &core).unwrap() {
                    eprintln!("LINE {line}");
                }
            }
            core.shutdown().await;
            hash
        });
        // Real content: commits, three worktrees (one dirty, one deleted).
        assert!(main_repo(&scratch, &world).join(".git").is_dir());
        assert!(worktree_path(&scratch, "worktree-0").is_dir());
        assert!(
            worktree_path(&scratch, "worktree-1")
                .join("scratch.txt")
                .is_file()
        );
        assert!(!worktree_path(&scratch, "worktree-2").exists());
        let log = scratch
            .git(
                &main_repo(&scratch, &world),
                &["rev-list", "--count", "main"],
                &[],
            )
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "4");
        assert!(
            std::fs::read_to_string(scratch.git_log())
                .unwrap()
                .contains("git worktree add")
        );
        finish(scratch);
        hash
    };
    let first = hash_for(7);
    let again = hash_for(7);
    assert_eq!(first, again);
    assert_ne!(first, hash_for(8));
}

#[test]
fn template_clones_and_reset_restores_it() {
    if run_isolated("template_clones_and_reset_restores_it", false) {
        return;
    }
    let rt = rt();
    let scratch = ScratchDirectories::create(&account()).unwrap();
    let world = SmallWorld::small(11);
    let template = rt.block_on(Template::build(&scratch, &world)).unwrap();
    assert!(
        template
            .header()
            .starts_with("preset=small seed=11 template=")
    );

    // Dirty the run: new commit, new file, and an engine write.
    let repo = main_repo(&scratch, &world);
    std::fs::write(repo.join("extra.txt"), "x").unwrap();
    scratch.git(&repo, &["add", "extra.txt"], &[]).unwrap();
    scratch
        .git(&repo, &["commit", "-q", "-m", "extra"], &[])
        .unwrap();
    rt.block_on(async {
        let core = materialize::open_core(scratch.data()).unwrap();
        core.workspace
            .rename_chat(&world.chats[0].id, "renamed")
            .unwrap();
        let changed = snapshot_hash(&scratch, &world, &core).unwrap();
        assert_ne!(changed, template.hash());
        core.shutdown().await;
    });

    template.restore(&scratch).unwrap();
    let again = rt.block_on(async {
        let core = materialize::open_core(scratch.data()).unwrap();
        let hash = snapshot_hash(&scratch, &world, &core).unwrap();
        core.shutdown().await;
        hash
    });
    assert_eq!(again, template.hash());
    assert!(!repo.join("extra.txt").exists());
    // A second restore after more changes is stable too.
    std::fs::write(repo.join("again.txt"), "y").unwrap();
    template.restore(&scratch).unwrap();
    assert!(!repo.join("again.txt").exists());
    finish(scratch);
}
