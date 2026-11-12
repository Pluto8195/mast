//! Offscreen Metal snapshots for test fixtures. No native window is created.
use std::{path::PathBuf, sync::Arc};

use gpui::{App, AppContext, Entity, HeadlessAppContext, Pixels, Render, Size, Window};

use crate::{
    theme::{Appearance, Theme},
    ui_style::{self, UiLayout, UiStyleId},
};

/// Render a fixture into `$CARGO_TARGET_DIR/snapshots/<name>.png`.
///
/// The builder may install fixture-specific globals and construct the root view.
/// Names are single path components so snapshots cannot escape the output directory.
/// A Metal device and access to macOS services are required.
pub fn render_to_png<V: Render + 'static>(
    name: &str,
    size: Size<Pixels>,
    style: UiStyleId,
    appearance: Appearance,
    build_view: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<PathBuf> {
    render_to_png_with_layout(name, size, style, UiLayout::Minimal, appearance, build_view)
}

/// Render a fixture with an independently selected layout.
pub fn render_to_png_with_layout<V: Render + 'static>(
    name: &str,
    size: Size<Pixels>,
    style: UiStyleId,
    layout: UiLayout,
    appearance: Appearance,
    build_view: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "invalid snapshot name"
    );
    let native = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        native.text_system(),
        Arc::new(crate::icons::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_base::init(cx);
        ui_style::init_with_layout(style, layout, cx);
        Theme::install(appearance, cx);
    });
    let window = cx.open_window(size, build_view)?;
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())?;
    // GPUI animation elements use wall-clock time; settle entrance animations.
    std::thread::sleep(std::time::Duration::from_millis(800));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())?;
    let image = cx.capture_screenshot(window.into())?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let directory = target.join("snapshots");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{name}.png"));
    image.save(&path)?;
    Ok(path)
}

/// Construct the walkthrough test fixture using the production view.
pub fn walkthrough_fixture(cx: &mut App) -> Entity<impl Render + use<>> {
    crate::changes::walkthrough::snapshot_fixture(cx)
}

/// Construct the walkthrough with the populated diff used by the preview.
pub fn populated_walkthrough_fixture(cx: &mut App) -> Entity<impl Render + use<>> {
    crate::changes::walkthrough::snapshot_fixture_with_changes(true, cx)
}

/// Construct the Changes pane with its file tree expanded or collapsed.
pub fn changes_fixture(collapsed: bool, cx: &mut App) -> Entity<crate::changes::Changes> {
    let changes = crate::changes::snapshot_populated_changes(cx);
    if collapsed {
        changes.update(cx, |changes, cx| changes.collapse_snapshot_file_tree(cx));
    }
    changes
}

/// Construct the offline shell used by the chat preview and gallery.
pub fn chat_fixture(data_dir: PathBuf, cx: &mut App) -> Entity<crate::shell::Shell> {
    let codex = std::env::var("MAST_PREVIEW_HARNESS").is_ok_and(|value| value == "codex");
    let (harness, model) = if codex { ("codex", "gpt-6-astra") } else { ("claude-code", "Claude Opus 5.5 — Extended reasoning") };
    let state = cx.new(|_| {
        let mut state = crate::state::AppState::new();
        state.connection = mast_proto::view::ConnectionStatus::Ready;
        state.workspace_scope = Some(mast_proto::WorkspaceScope::Local);
        state.local_device_id = Some("local".into());
        state.selected_chat = Some("fixture-chat".into());
        state.selected_space = Some("fixture-space".into());
        state.auto_selected = true;
        state.chats_synced = true;
        state.spaces_synced = true;
        state.spaces = vec![serde_json::from_value(serde_json::json!({"id":"fixture-space","deviceId":"local","path":"/fixture/project","createdAt":"2026-09-08T00:00:00Z"})).unwrap()];
        state.chats = [
            ("fixture-chat", "Review the workspace"),
            ("fixture-design", "Design review"),
            ("fixture-tests", "Verify the changes"),
        ].into_iter().map(|(id, title)| serde_json::from_value(serde_json::json!({"id":id,"deviceId":"local","spaceId":"fixture-space","title":title,"archived":false,"createdAt":"2026-09-08T00:00:00Z","config":{"harness":harness,"model":model,"reasoning":"high","sandbox":"workspace-write"}})).unwrap()).collect();
        preview_chat_outline(&mut state);
        state
    });
    state.update(cx, |state, cx| {
        state.receive_transcript_frame(mast_doc::TranscriptFrame::Reset { reset: serde_json::from_value(serde_json::json!([
            {"id":"user","role":"user","parts":[{"id":"text","kind":"text","text":"Review the sidebar spacing."}],"createdAt":1788900000000_i64,"deviceId":"local"},
            {"id":"assistant","role":"assistant","parts":[{"id":"text","kind":"text","text":"I inspected the layout.\n\nThe controls now align with their captions.\n\n- The model name stays visible.\n- The attachment has one border.\n\n```rust\nfn validate_layout() {\n    assert!(controls_align());\n}\n```"},{"id":"tool","kind":"tool","call":{"kind":"exec","command":"cargo test"},"resolved":true,"output":"Tests passed"},{"id":"tool2","kind":"tool","call":{"kind":"exec","command":"git diff --stat"},"resolved":true,"output":"Three files changed"},{"id":"next","kind":"text","text":"I checked the resulting changes. The transcript keeps this longer explanation within a comfortable reading width, even when the window becomes narrow.\n\n- The first list item has enough text to wrap onto another line while keeping its continuation aligned with the beginning of the item.\n- The second item shares the same indentation."},{"id":"tool3","kind":"tool","call":{"kind":"exec","command":"cargo check"},"resolved":true,"output":"Check passed"},{"id":"tool4","kind":"tool","call":{"kind":"exec","command":"cargo fmt --check"},"resolved":true,"output":"Formatting passed"}],"createdAt":1788900001000_i64,"deviceId":"local","status":"complete"}
        ])).unwrap() }, cx).unwrap();
        if std::env::var_os("MAST_PREVIEW_RETRY").is_some() {
            state.sessions.push(serde_json::from_value(serde_json::json!({
                "chatId": "fixture-chat", "deviceId": "local", "status": "errored",
                "updatedAt": "2026-09-08T00:00:00Z"
            })).unwrap());
            state.receive_transcript_frame(mast_doc::TranscriptFrame::Reset { reset: serde_json::from_value(serde_json::json!([
                {"id":"user","role":"user","parts":[{"id":"text","kind":"text","text":"Review the sidebar spacing."}],"createdAt":1788900000000_i64,"deviceId":"local"},
                {"id":"failure","role":"assistant","parts":[{"id":"error","kind":"error","message":"The service is overloaded. Please try again."}],"createdAt":1788900001000_i64,"deviceId":"local","status":"complete"}
            ])).unwrap() }, cx).unwrap();
        }
    });
    let boot = crate::EngineBootConfig {
        data_dir,
        ipc_port: 0,
        edge_url: String::new(),
        edge_token: None,
        org_id: None,
        workos_client_id: None,
        default_harness: mast_proto::HarnessId::Mock,
    };
    let shell = cx.new(|cx| {
        let mut shell = crate::shell::Shell::new(state, boot, cx);
        shell.fixture_snapshot_ready();
        shell.fixture_snapshot_attachment(
            include_bytes!("../assets/dock/mast-icon-ready.png").to_vec(),
            cx,
        );
        shell
    });
    shell.update(cx, |shell, cx| {
        shell.fixture_snapshot_item_panel(cx);
        shell.fixture_snapshot_expand_tool_group(cx);
        if std::env::var("MAST_OPEN_DIALOG").is_ok_and(|value| value == "model") {
            shell.fixture_snapshot_model_catalog(cx);
        }
    });
    shell
}

/// Construct the chat fixture with its hand-off dialog open.
pub fn handoff_fixture(data_dir: PathBuf, cx: &mut App) -> Entity<crate::shell::Shell> {
    let shell = chat_fixture(data_dir, cx);
    shell.update(cx, |shell, cx| shell.fixture_snapshot_hand_off(cx));
    shell
}

/// Construct an artifact pane with local, in-memory fixture content.
pub fn artifact_fixture(window: &mut Window, cx: &mut App) -> Entity<impl Render + use<>> {
    crate::artifacts::tab::snapshot_fixture(window, cx)
}

struct OutlineFixture {
    outline: Entity<crate::work_outline::outline_view::OutlineView>,
    details: Entity<crate::work_outline::details_panel::WorkItemDetails>,
    _shell: Entity<crate::shell::Shell>,
}

/// Offline Overview with its outline toolbar visible.
pub fn overview_fixture(cx: &mut App) -> Entity<crate::overview::Overview> {
    let state = cx.new(|_| {
        let mut state = crate::state::AppState::new();
        state.connection = mast_proto::view::ConnectionStatus::Ready;
        state.workspace_scope = Some(mast_proto::WorkspaceScope::Local);
        state.local_device_id = Some("local".into());
        state.spaces = vec![serde_json::from_value(serde_json::json!({"id":"fixture-space","deviceId":"local","path":"/fixture/project","createdAt":"2026-09-08T00:00:00Z"})).unwrap()];
        state
    });
    let transcript = cx.new(|cx| crate::transcript::Transcript::new(state.clone(), cx));
    let composer = cx.new(|cx| crate::composer::Composer::new(state.clone(), cx));
    let overview = cx.new(|cx| {
        crate::overview::Overview::new(
            state,
            gpui::WeakEntity::new_invalid(),
            transcript,
            composer,
            cx,
        )
    });
    overview.update(cx, |overview, cx| overview.show_snapshot_outline(cx));
    overview
}

/// Offline Overview with an in-memory Linear panel, without network requests.
pub fn linear_overview_fixture(cx: &mut App) -> Entity<crate::overview::Overview> {
    let overview = overview_fixture(cx);
    overview.update(cx, |overview, cx| overview.show_snapshot_linear(cx));
    overview
}

/// Linear settings with an in-memory disconnected status and no RPC.
pub fn linear_settings_fixture(cx: &mut App) -> Entity<impl Render + use<>> {
    let state = cx.new(|_| {
        let mut state = crate::state::AppState::new();
        state.linear.status = Some(Ok(mast_proto::linear::LinearConnectionStatus::default()));
        state
    });
    cx.new(|cx| crate::linear::settings_page::LinearSettingsPage::offline_fixture(state, cx))
}

/// Offline chat shell with a single in-memory worker.
pub fn team_fixture(data_dir: PathBuf, cx: &mut App) -> Entity<crate::shell::Shell> {
    let shell = chat_fixture(data_dir, cx);
    shell.update(cx, |shell, cx| shell.fixture_snapshot_team(cx));
    shell
}

impl gpui::Render for OutlineFixture {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{div, prelude::*, px};
        div()
            .size_full()
            .flex()
            .child(div().w(px(420.)).h_full().child(self.outline.clone()))
            .child(div().flex_1().h_full().child(self.details.clone()))
    }
}

/// Construct the existing Work Outline fixture with an item details panel.
fn outline_with_requests(space: &str) -> mast_proto::WorkOutlineSnapshot {
    use crate::work_outline::store::{fixture_ids, fixture_snapshot};
    use mast_proto::{UserActionRequestState, UserActionRequestView};

    let mut snapshot = fixture_snapshot(space);
    snapshot.external_references.clear();
    let item = snapshot
        .items
        .iter_mut()
        .find(|item| item.id == fixture_ids::API)
        .unwrap();
    item.chat_ids.push("fixture-chat".into());
    for (id, text) in [
        ("preview-approve", "Approve the proposed layout"),
        ("preview-review", "Review the implementation"),
    ] {
        snapshot.user_action_requests.push(UserActionRequestView {
            id: id.into(),
            space_id: space.into(),
            work_item_id: fixture_ids::API.into(),
            chat_id: "fixture-chat".into(),
            text: text.into(),
            state: UserActionRequestState::Open,
            options: Vec::new(),
            kind: None,
            priority: None,
            origin_kind: Default::default(),
            message_id: None,
            tool_call_id: None,
            info: None,
            links: Vec::new(),
            facts: Vec::new(),
            dedupe_key: None,
            created_by: None,
            created_at: 1_760_000_000_000,
            resolution: None,
            deduped: false,
            origin_chat_available: true,
        });
    }
    snapshot
}

pub fn preview_chat_outline(state: &mut crate::state::AppState) {
    state.set_work_outline_for_test(outline_with_requests("fixture-space"));
}

pub fn outline_fixture(data_dir: PathBuf, cx: &mut App) -> Entity<impl Render + use<>> {
    use crate::work_outline::{
        model::OutlineSpace,
        outline_view::{OutlineInputs, OutlineView},
        store::{WorkOutlineLoad, fixture_ids},
    };
    let state = cx.new(|_| {
        let mut state = crate::state::AppState::new();
        state.connection = mast_proto::view::ConnectionStatus::Ready;
        state.workspace_scope = Some(mast_proto::WorkspaceScope::Local);
        state.selected_space = Some("s1".into());
        state.set_work_outline_for_test(outline_with_requests("s1"));
        state
    });
    let boot = crate::EngineBootConfig {
        data_dir,
        ipc_port: 0,
        edge_url: String::new(),
        edge_token: None,
        org_id: None,
        workos_client_id: None,
        default_harness: mast_proto::HarnessId::Mock,
    };
    let shell = cx.new(|cx| crate::shell::Shell::new(state.clone(), boot, cx));
    let outline = cx.new(|cx| OutlineView::new(state.clone(), cx));
    outline.update(cx, |outline, cx| {
        outline.set_inputs(
            OutlineInputs {
                spaces: vec![OutlineSpace {
                    id: "s1".into(),
                    label: "Project".into(),
                    load: WorkOutlineLoad::Ready(Arc::new(outline_with_requests("s1"))),
                    hidden: false,
                }],
                ..Default::default()
            },
            cx,
        )
    });
    let details = cx.new(|cx| {
        crate::work_outline::details_panel::WorkItemDetails::new(
            state,
            shell.downgrade(),
            "s1".into(),
            fixture_ids::API.into(),
            cx,
        )
    });
    details.update(cx, |details, _| {
        details.set_data(
            Some(Arc::new(outline_with_requests("s1"))),
            Arc::new(Vec::new()),
        )
    });
    cx.new(|_| OutlineFixture {
        outline,
        details,
        _shell: shell,
    })
}
