//! Synthetic busy session for the `AppState` observers: 250 chats, then 50
//! streaming updates to one chat that is not selected and 50 to the selected
//! chat. Each view entity is observed from the outside, so the counts are the
//! notifies (repaints requested) per view, whatever the view does internally.
//! Run with `--nocapture` to print the table.

use super::*;
use gpui::{AppContext, Entity, TestAppContext};
use std::{cell::Cell, rc::Rc};

type Counter = (&'static str, Rc<Cell<u64>>);

#[gpui::test]
fn working_heartbeat_notifies_transcript_without_notifying_sidebar(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        gpui_base::init(cx);
        settings::init(settings::UiSettings::default(), dir.path(), cx);
        cx.set_global(Theme::default());
        crate::history::init(Default::default(), Default::default(), Default::default(), Default::default(), cx);
        crate::app_menus::init(cx);
    });
    let now = Utc::now();
    let state = cx.new(|_| AppState::new());
    state.update(cx, |state, _| {
        state.chats = vec![chat_row(0)];
        state.selected_chat = Some("c0".into());
        state.sessions = vec![mast_proto::Session {
            chat_id: "c0".into(), device_id: "local".into(),
            status: mast_proto::SessionStatus::Working, started_at: Some(now), updated_at: now,
            last_completed_turn: None, awaiting_input: None, last_turn_tool_failures: None,
        }];
    });
    let shell = cx.new(|cx| Shell::new(state, EngineBootConfig {
        data_dir: dir.path().to_path_buf(), ipc_port: 0,
        edge_url: "http://127.0.0.1:1".into(), edge_token: None, org_id: None,
        workos_client_id: None, default_harness: mast_proto::HarnessId::Mock,
    }, cx));
    shell.update(cx, |shell, cx| shell.refresh_heartbeat(now, false, cx));
    cx.run_until_parked();
    let (sidebar, transcript) = shell.read_with(cx, |shell, _| (shell.sidebar_pane.clone(), shell.transcript.clone()));
    let counters = vec![watch(cx, "shell", &shell), watch(cx, "sidebar", &sidebar), watch(cx, "transcript", &transcript)];
    shell.update(cx, |shell, cx| shell.refresh_heartbeat(now, false, cx));
    cx.run_until_parked();
    assert_eq!(totals(&counters), vec![("shell", 0), ("sidebar", 0), ("transcript", 1)]);
    shell.update(cx, |shell, cx| {
        shell.running.open = true;
        shell.refresh_heartbeat(now, false, cx);
    });
    cx.run_until_parked();
    assert_eq!(totals(&counters), vec![("shell", 1), ("sidebar", 1), ("transcript", 2)]);
    shell.update(cx, |shell, cx| {
        shell.running.open = false;
        shell.refresh_heartbeat(now + chrono::TimeDelta::seconds(46), false, cx)
    });
    cx.run_until_parked();
    assert_eq!(totals(&counters), vec![("shell", 2), ("sidebar", 2), ("transcript", 3)]);
}

/// Set `MAST_BENCH_OPEN=1` to measure the open list; otherwise measure the closed disclosure.
#[gpui::test]
#[ignore]
fn sidebar_frame_cost(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        gpui_base::init(cx);
        settings::init(settings::UiSettings::default(), dir.path(), cx);
        cx.set_global(Theme::default());
        crate::history::init(Default::default(), Default::default(), Default::default(), Default::default(), cx);
        crate::composer::init(cx, Default::default());
        crate::terminal::panel::init(cx);
        crate::app_menus::init(cx);
    });
    let state = cx.new(|_| AppState::new());
    state.update(cx, |state, _| {
        state.chats = (0..400).map(chat_row).collect();
        state.sessions = (395..400).map(|index| mast_proto::Session {
            last_completed_turn: None,
            awaiting_input: None,
            last_turn_tool_failures: None,
            chat_id: format!("c{index}"),
            device_id: "local".into(),
            status: mast_proto::SessionStatus::Working,
            started_at: None,
            updated_at: Utc::now(),
        }).collect();
        state.chats_synced = true;
        state.selected_chat = Some("c0".into());
        state.no_project = true;
        state.data_dir = Some(dir.path().to_path_buf());
    });
    let boot = EngineBootConfig { data_dir: dir.path().to_path_buf(), ipc_port: 0, edge_url: "http://127.0.0.1:1".into(), edge_token: None, org_id: None, workos_client_id: None, default_harness: mast_proto::HarnessId::Mock };
    let (_, vcx) = cx.add_window_view(|_, cx| {
        let mut shell = Shell::new(state, boot, cx);
        shell.route = Route::Chat;
        shell.active_chat = "c0".into();
        shell.debug_gate = Some(GatePhase::Ready);
        shell.settings.sidebar_organization = SidebarOrganization::InOneList;
        shell.sessions_open = std::env::var_os("MAST_BENCH_OPEN").is_some();
        shell
    });
    vcx.update(|window, cx| window.draw(cx).clear());
    let start = std::time::Instant::now();
    for _ in 0..10 {
        vcx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
    }
    eprintln!("BENCH sidebar (open={}): 400 chats, 5 working, 10 frames: {:?}", std::env::var_os("MAST_BENCH_OPEN").is_some(), start.elapsed());
}

fn watch<T: 'static>(cx: &mut TestAppContext, name: &'static str, entity: &Entity<T>) -> Counter {
    let count = Rc::new(Cell::new(0));
    let seen = count.clone();
    cx.update(|cx| {
        cx.observe(entity, move |_, _| {
            if std::env::var("PERF_BT").is_ok_and(|v| v == name) && seen.get() == 3 {
                eprintln!("{}", std::backtrace::Backtrace::force_capture());
            }
            seen.set(seen.get() + 1)
        })
        .detach()
    });
    (name, count)
}

fn chat_row(index: usize) -> mast_proto::Chat {
    use chrono::TimeZone;
    mast_proto::Chat {
        id: format!("c{index}"),
        device_id: "local".into(),
        title: Some(format!("Chat {index}")),
        archived: false,
        cwd: Some(format!("/work/repo{}", index % 25)),
        branch: None,
        checkout_id: None,
        source_context: None,
        config: None,
        last_message_preview: None,
        last_message_at: Some(Utc.timestamp_opt(1_700_000_000 + index as i64, 0).unwrap()),
        created_at: Utc.timestamp_opt(1_600_000_000, 0).unwrap(),
        harness_session_id: None,
        harness_session_cwd: None,
        actual_model: None,
        parent_chat_id: None,
        space_id: None,
        last_seen_at: None,
        room_gen: None,
        linked_pr_url: None,
        linked_pr_source: None,
        linked_ticket_id: None,
        linked_ticket_source: None,
    }
}

#[gpui::test]
fn industrial_chat_shell_renders_light_and_dark(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        gpui_base::init(cx);
        settings::init(settings::UiSettings::default(), dir.path(), cx);
        crate::ui_style::init(crate::ui_style::UiStyleId::Industrial, cx);
        crate::appearance::init(crate::appearance::AppearanceMode::Light, Default::default(), Default::default(), Default::default(), cx);
        crate::history::init(Default::default(), Default::default(), Default::default(), Default::default(), cx);
        crate::composer::init(cx, Default::default());
        crate::terminal::panel::init(cx);
        crate::app_menus::init(cx);
    });
    let state = cx.new(|_| AppState::new());
    state.update(cx, |state, _| {
        state.chats = vec![chat_row(0)];
        state.chats_synced = true;
        state.selected_chat = Some("c0".into());
        state.no_project = true;
        state.data_dir = Some(dir.path().to_path_buf());
    });
    let boot = EngineBootConfig { data_dir: dir.path().to_path_buf(), ipc_port: 0, edge_url: "http://127.0.0.1:1".into(), edge_token: None, org_id: None, workos_client_id: None, default_harness: mast_proto::HarnessId::Mock };
    let (shell, vcx) = cx.add_window_view(|_, cx| {
        let mut shell = Shell::new(state.clone(), boot, cx);
        shell.route = Route::Chat;
        shell.active_chat = "c0".into();
        shell.debug_gate = Some(GatePhase::Ready);
        shell
    });
    for mode in [crate::appearance::AppearanceMode::Light, crate::appearance::AppearanceMode::Dark] {
        vcx.update(|window, cx| {
            crate::appearance::set_mode(mode, cx);
            window.refresh();
            window.draw(cx).clear();
        });
        assert!(vcx.debug_bounds("composer-surface").is_some());
    }
    let sidebar = shell.read_with(vcx, |shell, _| shell.sidebar_pane.clone());
    let before = sidebar.read_with(vcx, |pane, _| pane.renders);
    shell.update(vcx, |shell, cx| {
        shell.settings.sidebar_collapsed = true;
        shell.sidebar_tween = None;
        cx.notify();
    });
    for i in 0..5 {
        state.update(vcx, |state, cx| {
            state.chats[0].title = Some(format!("Changed while hidden {i}"));
            cx.notify();
        });
        vcx.run_until_parked();
        vcx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
    }
    assert_eq!(sidebar.read_with(vcx, |pane, _| pane.renders), before,
        "collapsed sidebar must not render even when its state changes");
    shell.update(vcx, |shell, cx| {
        shell.settings.sidebar_collapsed = false;
        cx.notify();
    });
    vcx.update(|window, cx| { window.refresh(); window.draw(cx).clear(); });
    assert!(sidebar.read_with(vcx, |pane, _| pane.renders) > before,
        "expanding must remount the sidebar with current state");
}

fn totals(counters: &[Counter]) -> Vec<(&'static str, u64)> {
    counters.iter().map(|(n, c)| (*n, c.get())).collect()
}

fn delta(
    before: &[(&'static str, u64)],
    after: &[(&'static str, u64)],
) -> Vec<(&'static str, u64)> {
    before
        .iter()
        .zip(after)
        .map(|((n, b), (_, a))| (*n, a - b))
        .collect()
}

#[gpui::test]
fn streaming_one_chat_among_many_notifies_few_views(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        settings::init(settings::UiSettings::default(), dir.path(), cx);
        crate::history::init(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            cx,
        );
        gpui_base::init(cx);
        cx.set_global(Theme::default());
        crate::app_menus::init(cx);
    });
    let state = cx.new(|_| AppState::new());
    let window = cx.add_window({
        let state = state.clone();
        let data_dir = dir.path().to_path_buf();
        move |_, cx| {
            Shell::new(
                state,
                EngineBootConfig {
                    data_dir,
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: mast_proto::HarnessId::Mock,
                },
                cx,
            )
        }
    });
    state.update(cx, |state, _| {
        state.chats = (0..250).map(chat_row).collect();
        state.chats_synced = true;
        state.selected_chat = Some("c0".into());
        state.data_dir = Some(dir.path().to_path_buf());
    });
    let shell = window.root(cx).unwrap();
    let (transcript, composer) =
        shell.read_with(cx, |s, _| (s.transcript.clone(), s.composer.clone()));
    let pickers = composer.read_with(cx, |c, _| c.pickers().clone());
    let weak = shell.downgrade();
    let chat_types = cx.new(|cx| {
        crate::chat_type_view::ChatTypeView::new(
            state.clone(),
            weak.clone(),
            crate::chat_type_view::ChatViewKind::PrReview,
            cx,
        )
    });
    let accounts = cx.new(|cx| crate::settings::accounts::AccountsPage::new(state.clone(), cx));
    let archived = cx.new(|cx| crate::settings::archived::ArchivedPage::new(state.clone(), cx));
    let devices = cx.new(|cx| crate::settings::devices::DevicesPage::new(state.clone(), cx));
    let linear =
        cx.new(|cx| crate::linear::settings_page::LinearSettingsPage::new(state.clone(), cx));
    let changes = cx.new(|cx| crate::changes::Changes::new(state.clone(), cx));
    let terminal = cx.new(|cx| crate::terminal::panel::TerminalPanel::new(state.clone(), cx));
    let files = cx
        .new(|cx| crate::files::FilesSurface::new_explorer(state.clone(), "c0".into(), false, cx));
    cx.run_until_parked();

    let counters = vec![
        watch(cx, "shell", &shell),
        watch(cx, "transcript", &transcript),
        watch(cx, "composer", &composer),
        watch(cx, "pickers", &pickers),
        watch(cx, "chat_type_view", &chat_types),
        watch(cx, "settings accounts", &accounts),
        watch(cx, "settings archived", &archived),
        watch(cx, "settings devices", &devices),
        watch(cx, "settings linear", &linear),
        watch(cx, "changes", &changes),
        watch(cx, "terminal panel", &terminal),
        watch(cx, "files", &files),
    ];
    let start = totals(&counters);

    // A: a live heartbeat for a chat nobody has open: its session row's
    // lease timestamp moves, no visible field does.
    for i in 0..50 {
        state.update(cx, |state, cx| {
            state.sessions = vec![mast_proto::Session {
                last_completed_turn: None,
                awaiting_input: None,
                last_turn_tool_failures: None,
                chat_id: "c1".into(),
                device_id: "local".into(),
                status: mast_proto::SessionStatus::Working,
                started_at: None,
                updated_at: Utc::now() + chrono::TimeDelta::milliseconds(i),
            }];
            cx.notify();
        });
        cx.run_until_parked();
    }
    let after_heartbeats = totals(&counters);

    // B: the same chat's row gets new preview text each time (a real row
    // change, visible in lists that show previews).
    for i in 0..50 {
        state.update(cx, |state, cx| {
            state.chats[1].last_message_preview = Some(format!("preview {i}"));
            cx.notify();
        });
        cx.run_until_parked();
    }
    let after_previews = totals(&counters);

    // C: streaming into the selected chat: its transcript changes, no
    // chat/space/session field does.
    for i in 0..50 {
        state.update(cx, |state, cx| {
            state.transcript.push(mast_doc::SessionMessageEntry {
                id: format!("m{i}"),
                role: mast_doc::MessageRole::Assistant,
                parts: vec![mast_doc::MessagePart::Text {
                    id: "t0".into(),
                    text: format!("token {i}"),
                }],
                created_at: i,
                device_id: "local".into(),
                status: None,
                continuation_of: None,
            });
            state.transcript_revision = state.transcript_revision.wrapping_add(1);
            cx.notify();
        });
        cx.run_until_parked();
    }
    let after_selected = totals(&counters);

    for i in 0..50 {
        state.update(cx, |state, cx| {
            state.chats[0].last_message_preview = Some(format!("selected preview {i}"));
            state.chats[0].last_seen_at = Some(Utc::now());
            cx.notify();
        });
        cx.run_until_parked();
    }
    let selected_previews = delta(&after_selected, &totals(&counters));
    for (name, count) in &selected_previews {
        if matches!(*name, "composer" | "pickers") {
            assert_eq!(
                *count, 0,
                "{name} must ignore preview and read-marker changes"
            );
        }
    }
    let before_config = totals(&counters);
    state.update(cx, |state, cx| {
        state.chats[0].actual_model = Some("updated-model".into());
        cx.notify();
    });
    cx.run_until_parked();
    for (name, count) in delta(&before_config, &totals(&counters)) {
        if matches!(name, "composer" | "pickers") {
            assert!(count > 0, "{name} must follow model changes");
        }
    }

    eprintln!("PERF notifies per view (50 updates each)");
    eprintln!(
        "{:<22}{:>12}{:>12}{:>12}",
        "view", "heartbeat", "preview", "selected"
    );
    let a = delta(&start, &after_heartbeats);
    let b = delta(&after_heartbeats, &after_previews);
    let c = delta(&after_previews, &after_selected);
    for (((name, a), (_, b)), (_, c)) in a.iter().zip(&b).zip(&c) {
        eprintln!("{name:<22}{a:>12}{b:>12}{c:>12}");
    }
}

/// UI initialization and a headless shell with a preloaded synthetic chat list.
/// This excludes engine bootstrap and registry reads; see performance-sweep.md.
#[gpui::test]
#[ignore = "benchmark: run with --ignored --nocapture"]
fn bench_shell_initialization(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let settings = crate::perf::measure("startup", "load settings", || {
        settings::UiSettings::load(dir.path())
    });
    cx.update(|cx| {
        gpui_base::init(cx);
        settings::init(settings.clone(), dir.path(), cx);
        let fonts = crate::perf::measure("startup", "register fonts", || {
            crate::typography::register_fonts(cx)
        });
        crate::typography::init(
            settings.ui_font_family.clone(),
            settings.ui_font_size,
            settings.terminal_font_family.clone(),
            settings.terminal_font_size,
            settings.code_font_family.clone(),
            settings.code_font_size,
            fonts,
            cx,
        );
        crate::perf::measure("startup", "load themes", || {
            crate::theme_library::init(dir.path(), cx)
        });
        crate::appearance::init(
            settings.appearance,
            settings.theme_selection.clone(),
            settings.accent,
            settings.surface,
            cx,
        );
        crate::history::init(
            settings.git_history_columns,
            settings.git_history_column_widths,
            settings.git_history_column_order,
            settings.git_history_author_display,
            cx,
        );
        crate::composer::init(cx, settings.composer_send_behavior);
        crate::terminal::panel::init(cx);
        crate::app_menus::init(cx);
    });
    let state = cx.new(|_| AppState::new());
    state.update(cx, |state, _| {
        state.chats = (0..250).map(chat_row).collect();
        state.chats_synced = true;
        state.data_dir = Some(dir.path().to_path_buf());
    });
    let boot = EngineBootConfig {
        data_dir: dir.path().to_path_buf(),
        ipc_port: 0,
        edge_url: "http://127.0.0.1:1".into(),
        edge_token: None,
        org_id: None,
        workos_client_id: None,
        default_harness: mast_proto::HarnessId::Mock,
    };
    let (_, vcx) = crate::perf::measure("startup", "construct shell window", || {
        cx.add_window_view(|_, cx| Shell::new(state.clone(), boot, cx))
    });
    crate::perf::measure("startup", "first shell draw", || {
        vcx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
    });
}

/// A profile-backed startup including registry loading and the first sidebar data.
#[gpui::test]
#[ignore = "benchmark: run with --ignored --nocapture"]
fn bench_profile_startup(cx: &mut TestAppContext) {
    crate::perf::require_offline_executables();
    use mast_proto::{
        CreateWorkItemParams, WorkOutlineActor, WorkOutlineActorKind, WorkOutlineMutation,
        WorkOutlinePlacement,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();
    let dir = tempfile::tempdir().unwrap();
    let profile = mast_engine::EngineProfile::development(dir.path(), "benchmark-org", "dev-user");
    let mut registry = mast_doc::RegistryDoc::new("benchmark-device");
    registry
        .upsert_space(&mast_proto::Space {
            id: "benchmark-space".into(),
            device_id: "benchmark-device".into(),
            path: dir.path().to_string_lossy().into_owned(),
            name: Some("Benchmark".into()),
            git_detected: false,
            git_checked_at: None,
            checkout_id: None,
            created_at: Utc::now(),
        })
        .unwrap();
    let mut chats: Vec<_> = (0..250).map(chat_row).collect();
    for chat in &mut chats {
        chat.device_id = "benchmark-device".into();
        chat.space_id = Some("benchmark-space".into());
        registry.upsert_chat(chat).unwrap();
    }
    let actor = WorkOutlineActor {
        kind: WorkOutlineActorKind::User,
        device_id: "benchmark-device".into(),
        chat_id: None,
        message_id: None,
    };
    for i in 0..300 {
        registry
            .apply_work_outline_mutation(
                &WorkOutlineMutation::CreateWorkItem(CreateWorkItemParams {
                    space_id: "benchmark-space".into(),
                    parent_work_item_id: None,
                    placement: WorkOutlinePlacement::Last,
                    title: format!("Benchmark item {i}"),
                    description: None,
                    status: None,
                    operation_id: format!("benchmark-item-{i}"),
                }),
                &actor,
                1_700_000_000_000 + i,
            )
            .unwrap();
    }
    // Accumulated pending updates make the serialized registry approximately
    // 50 MiB while preserving 250 visible chats and 300 visible work items.
    let padding = "x".repeat(8192);
    for i in 0..6000 {
        chats[0].last_message_preview = Some(format!("{i}:{padding}"));
        registry.upsert_chat(&chats[0]).unwrap();
    }
    chats[0].last_message_preview = None;
    registry.upsert_chat(&chats[0]).unwrap();
    let bytes = registry.to_bytes().unwrap();
    eprintln!("BENCH startup | registry bytes={}", bytes.len());
    assert!((45_000_000..65_000_000).contains(&bytes.len()));
    let store = mast_sync::DocsStore::open(profile.store_root()).unwrap();
    store
        .save_snapshot(mast_doc::REGISTRY_DOC_ID, &bytes)
        .unwrap();
    assert_eq!(
        store
            .load_snapshot(mast_doc::REGISTRY_DOC_ID)
            .unwrap()
            .unwrap()
            .len(),
        bytes.len()
    );
    drop((bytes, registry, store));
    let started = std::time::Instant::now();
    let settings_started = std::time::Instant::now();
    let settings = settings::UiSettings::load(dir.path());
    eprintln!("BENCH phase | settings_keymap | ms={:.3}", settings_started.elapsed().as_secs_f64() * 1000.0);
    cx.update(|cx| {
        gpui_tokio::init_from_handle(cx, runtime.handle().clone());
        gpui_base::init(cx);
        settings::init(settings.clone(), dir.path(), cx);
        let fonts = crate::typography::register_fonts(cx);
        crate::typography::init(
            settings.ui_font_family.clone(),
            settings.ui_font_size,
            settings.terminal_font_family.clone(),
            settings.terminal_font_size,
            settings.code_font_family.clone(),
            settings.code_font_size,
            fonts,
            cx,
        );
        crate::theme_library::init(dir.path(), cx);
        crate::appearance::init(
            settings.appearance,
            settings.theme_selection.clone(),
            settings.accent,
            settings.surface,
            cx,
        );
        crate::history::init(
            settings.git_history_columns,
            settings.git_history_column_widths,
            settings.git_history_column_order,
            settings.git_history_author_display,
            cx,
        );
        crate::composer::init(cx, settings.composer_send_behavior);
        crate::terminal::panel::init(cx);
        crate::app_menus::init(cx);
    });
    let state = cx.new(|_| AppState::new());
    let boot = EngineBootConfig {
        data_dir: dir.path().to_path_buf(),
        ipc_port: 0,
        edge_url: "http://127.0.0.1:1".into(),
        edge_token: None,
        org_id: Some("benchmark-org".into()),
        workos_client_id: None,
        default_harness: mast_proto::HarnessId::Mock,
    };
    cx.update(|cx| AppState::bootstrap(state.clone(), boot.clone(), cx));
    let (_, vcx) = cx.add_window_view(|_, cx| Shell::new(state.clone(), boot, cx));
    vcx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
    eprintln!(
        "BENCH startup | initial shell draw | ms={:.2}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    state.update(vcx, |state, cx| {
        state.declare_work_outline_visible(vec!["benchmark-space".into()], cx)
    });
    let readiness_started = std::time::Instant::now();
    let mut scheduler_time = std::time::Duration::ZERO;
    let mut connection_recorded = false;
    let mut chats_recorded = false;
    let mut outline_recorded = false;
    loop {
        let tokio_started = std::time::Instant::now();
        let tokio_wall = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros();
        runtime.block_on(tokio::task::yield_now());
        let tokio_elapsed = tokio_started.elapsed();
        if tokio_elapsed.as_millis() >= 1 {
            eprintln!("BENCH scheduler | tokio | start_us={tokio_wall} duration_us={}", tokio_elapsed.as_micros());
        }
        let scheduler_started = std::time::Instant::now();
        let scheduler_wall = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros();
        loop {
            let tick_started = std::time::Instant::now();
            let tick_wall = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros();
            let progressed = vcx.executor().tick();
            let tick_elapsed = tick_started.elapsed();
            if tick_elapsed.as_millis() >= 1 {
                eprintln!("BENCH scheduler | gpui_tick | start_us={tick_wall} duration_us={}", tick_elapsed.as_micros());
            }
            if !progressed { break; }
        }
        let scheduler_elapsed = scheduler_started.elapsed();
        if scheduler_elapsed.as_millis() >= 1 {
            eprintln!("BENCH scheduler | gpui | start_us={scheduler_wall} duration_us={}", scheduler_elapsed.as_micros());
        }
        scheduler_time += scheduler_elapsed;
        state.read_with(vcx, |state, _| {
            if !connection_recorded && matches!(state.connection, crate::state::ConnectionStatus::Ready) {
                eprintln!("BENCH milestone | engine_rpc_ready | ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
                eprintln!("BENCH event | ready_observed | us={}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros());
                connection_recorded = true;
            }
            if !chats_recorded && state.chats.len() == 250 {
                eprintln!("BENCH milestone | chats_loaded | ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
                eprintln!("BENCH event | chats_observed | us={}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros());
                chats_recorded = true;
            }
            if !outline_recorded && state.work_outline("benchmark-space").snapshot().is_some_and(|snapshot| snapshot.items.len() == 300) {
                eprintln!("BENCH milestone | outline_loaded | ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
                eprintln!("BENCH event | outline_observed | us={}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros());
                outline_recorded = true;
            }
        });
        if state.read_with(vcx, |state, _| {
            state.chats.len() == 250
                && state
                    .work_outline("benchmark-space")
                    .snapshot()
                    .is_some_and(|snapshot| snapshot.items.len() == 300)
        }) {
            break;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(120),
            "profile startup did not load sidebar data"
        );
        std::thread::yield_now();
    }
    eprintln!(
        "BENCH startup | readiness wait | ms={:.2} scheduler_ms={:.2}",
        readiness_started.elapsed().as_secs_f64() * 1000.0,
        scheduler_time.as_secs_f64() * 1000.0
    );
    let draw_started = std::time::Instant::now();
    vcx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
    eprintln!(
        "BENCH startup | forced draw alone | ms={:.2}",
        draw_started.elapsed().as_secs_f64() * 1000.0
    );
    eprintln!(
        "BENCH startup | loaded sidebar draw | ms={:.2}",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

/// Cost of one sidebar redraw with a long one-list session history, the case
/// any Shell notification pays while the sidebar is mounted.
#[gpui::test]
#[ignore = "benchmark: run with --ignored --nocapture"]
fn bench_sidebar_redraw_with_many_sessions(cx: &mut TestAppContext) {
    struct SidebarHost(Entity<Shell>);
    impl Render for SidebarHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.update(cx, |shell, cx| {
                div()
                    .w(px(320.0))
                    .h(px(900.0))
                    .child(shell.render_chat_sidebar(&Theme::default(), cx))
            })
        }
    }
    const SESSIONS: usize = 1_000;
    const REDRAWS: u32 = 30;
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        settings::init(settings::UiSettings::default(), dir.path(), cx);
        gpui_base::init(cx);
        cx.set_global(Theme::default());
        crate::app_menus::init(cx);
        crate::history::init(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            cx,
        );
    });
    let (host, vcx) = cx.add_window_view(|_, cx| {
        SidebarHost(cx.new(|cx| {
            let state = cx.new(|_| AppState::new());
            let mut shell = Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: mast_proto::HarnessId::Mock,
                },
                cx,
            );
            shell.settings.sidebar_organization = SidebarOrganization::InOneList;
            shell.settings.sidebar_compact = true;
            shell.state.update(cx, |state, _| {
                state.workspace_scope = Some(mast_proto::WorkspaceScope::Local);
                state.local_device_id = Some("local".into());
                state.chats = (0..SESSIONS).map(chat_row).collect();
                state.chats_synced = true;
            });
            shell
        }))
    });
    let shell = host.read_with(vcx, |host, _| host.0.clone());
    for _ in 0..3 {
        shell.update(vcx, |_, cx| cx.notify());
        vcx.run_until_parked();
    }
    let started = std::time::Instant::now();
    for _ in 0..REDRAWS {
        shell.update(vcx, |_, cx| cx.notify());
        vcx.run_until_parked();
    }
    eprintln!(
        "BENCH sidebar | {SESSIONS} sessions | ms_per_redraw={:.2}",
        started.elapsed().as_secs_f64() * 1000.0 / f64::from(REDRAWS)
    );
}
