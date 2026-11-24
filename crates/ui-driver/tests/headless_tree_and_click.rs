//! B3: a11y tree + bounds enumeration on the headless platform, and a
//! locator-driven click. Needs the zui test-support patch
//! (Pluto8195/zui branch mast/test-support, see MAST-PATCH.md there); runs
//! without a GPU (no renderer).
#![cfg(target_os = "macos")]
use std::sync::Arc;

// GPUI's platform state is process-wide; simultaneous headless contexts abort.
static HEADLESS_CONTEXT: std::sync::Mutex<()> = std::sync::Mutex::new(());

use gpui::{
    AnyWindowHandle, HeadlessAppContext, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PlatformInput, px, size,
};
use mast_ui::{
    theme::{Appearance, Theme},
    ui_style::{self, UiLayout, UiStyleId},
};

fn headless(data: &std::path::Path) -> (HeadlessAppContext, AnyWindowHandle) {
    headless_with(data, mast_ui::snapshot::chat_fixture)
}

fn headless_with(
    data: &std::path::Path,
    open: impl FnOnce(std::path::PathBuf, &mut gpui::App) -> gpui::Entity<mast_ui::shell::Shell>
    + 'static,
) -> (HeadlessAppContext, AnyWindowHandle) {
    let native = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        native.text_system(),
        Arc::new(mast_ui::icons::Assets),
        || None,
    );
    cx.set_a11y_enabled(true);
    let dd = data.to_path_buf();
    let window = {
        let dd2 = dd.clone();
        cx.update(move |cx| {
            gpui_base::init(cx);
            // Settle: animations jump to their end state.
            cx.set_reduce_motion(true);
            let mut s = mast_ui::settings::UiSettings::default();
            s.ui_style = UiStyleId::Default;
            s.ui_layout = UiLayout::Minimal;
            s.appearance = mast_ui::appearance::AppearanceMode::Dark;
            mast_ui::settings::init(s.clone(), &dd2, cx);
            let fonts = mast_ui::typography::register_fonts(cx);
            mast_ui::typography::init(
                s.ui_font_family.clone(),
                s.ui_font_size,
                s.terminal_font_family.clone(),
                s.terminal_font_size,
                s.code_font_family.clone(),
                s.code_font_size,
                fonts,
                cx,
            );
            mast_ui::history::init(
                s.git_history_columns,
                s.git_history_column_widths,
                s.git_history_column_order,
                s.git_history_author_display,
                cx,
            );
            mast_ui::composer::init(cx, s.composer_send_behavior);
            mast_ui::terminal::panel::init(cx);
            Theme::install_selection(
                Appearance::Dark,
                "default",
                mast_theme::AccentSelection::ThemeDefault,
                mast_theme::SurfacePreference::default(),
                cx,
            );
            let _ = ui_style::UiStyleId::Default;
        });
        cx.open_window(size(px(1200.), px(800.)), move |_, cx| open(dd, cx))
            .expect("open window")
    };
    (cx, window.into())
}

fn draw(cx: &mut HeadlessAppContext, h: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(h, |_, w, c| w.draw(c).clear()).unwrap();
}

#[test]
fn dump_tree_and_bounds() {
    let _guard = HEADLESS_CONTEXT.lock().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _g = rt.enter();
    let data = tempfile::tempdir().unwrap();
    let (mut cx, h) = headless(data.path());
    draw(&mut cx, h);
    draw(&mut cx, h);
    let (active, tree, bounds) = cx
        .update_window(h, |_, w, _| {
            (
                w.is_a11y_active(),
                w.debug_a11y_tree_json(),
                w.debug_bounds_all(),
            )
        })
        .unwrap();
    assert!(active && tree.is_some() && !bounds.is_empty());
    println!(
        "active={active} tree_len={:?} bounds={}",
        tree.as_ref().map(|t| t.len()),
        bounds.len()
    );
    if let Some(t) = &tree {
        std::fs::write(std::env::temp_dir().join("b3-tree.json"), t).unwrap();
    }
    for (k, b) in bounds.iter().take(5) {
        println!("bounds {k} {b:?}");
    }
}

/// Locator = a `debug_selector` name. Click the centre of its bounds with the
/// public `Window::dispatch_event`.
fn click_selector(cx: &mut HeadlessAppContext, h: AnyWindowHandle, selector: &str) {
    let b = cx
        .update_window(h, |_, w, _| w.debug_bounds_all())
        .unwrap()
        .into_iter()
        .find(|(k, _)| k == selector)
        .unwrap_or_else(|| panic!("no element with selector {selector}"))
        .1;
    let p = b.center();
    cx.update_window(h, |_, w, c| {
        let m = Modifiers::default();
        w.dispatch_event(
            PlatformInput::MouseMove(MouseMoveEvent {
                position: p,
                pressed_button: None,
                modifiers: m,
            }),
            c,
        );
        w.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: p,
                modifiers: m,
                click_count: 1,
                first_mouse: false,
            }),
            c,
        );
        w.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position: p,
                modifiers: m,
                click_count: 1,
            }),
            c,
        );
    })
    .unwrap();
    draw(cx, h);
}

fn selectors(cx: &mut HeadlessAppContext, h: AnyWindowHandle) -> Vec<String> {
    cx.update_window(h, |_, w, _| w.debug_bounds_all())
        .unwrap()
        .into_iter()
        .map(|(k, _)| k)
        .collect()
}

#[test]
fn click_element_by_locator() {
    let _guard = HEADLESS_CONTEXT.lock().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _g = rt.enter();
    let data = tempfile::tempdir().unwrap();
    let (mut cx, h) = headless(data.path());
    draw(&mut cx, h);
    draw(&mut cx, h);
    let before = selectors(&mut cx, h);
    assert!(before.iter().any(|k| k == "chat-item-identifiers-toggle"));
    assert!(!before.iter().any(|k| k == "chat-item-id-chat"));
    click_selector(&mut cx, h, "chat-item-identifiers-toggle");
    let after = selectors(&mut cx, h);
    assert!(
        after.iter().any(|k| k == "chat-item-id-chat")
            && after.iter().any(|k| k == "chat-item-id-workspace"),
        "identifiers section did not expand: {after:?}"
    );

    // The tree names the same control: element id, source location, role, label.
    let tree = cx
        .update_window(h, |_, w, _| w.debug_a11y_tree_json())
        .unwrap()
        .expect("a11y tree");
    let v: serde_json::Value = serde_json::from_str(&tree).unwrap();
    let copy = v["nodes"]
        .as_object()
        .unwrap()
        .values()
        .find(|n| n["element_id"].as_str() == Some("Name(\"chat-identity-copy\")"))
        .expect("chat-identity-copy node");
    assert_eq!(copy["aria"]["role"], "Button");
    assert!(
        copy["source_location"]
            .as_str()
            .unwrap()
            .contains("shell_parts.rs")
    );
}

/// Tree dump with the world header first. Runs inside the driver sandbox only
/// (it boots the real in-process engine on the scratch data dir).
#[test]
#[ignore = "runs inside the driver sandbox; see world_tree_dump_runs_isolated"]
fn inner_world_tree_dump() {
    use mast_ui_driver::{environment::ScratchDirectories, template::Template, world::SmallWorld};
    let scratch = ScratchDirectories::from_env().expect("guarded driver environment");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let world = SmallWorld::small(5);
    let template = rt.block_on(Template::build(&scratch, &world)).unwrap();
    template.restore(&scratch).unwrap(); // reset: drop everything, re-clone
    // The engine opens the restored stores, the rows go into the UI, and the
    // engine shuts down before the window opens (one owner of the data dir).
    let rows = rt.block_on(async {
        let core = mast_ui_driver::materialize::open_core(scratch.data()).unwrap();
        let rows = mast_ui_driver::materialize::load_rows(&world, &core).unwrap();
        core.shutdown().await;
        rows
    });
    let _g = rt.enter();
    let (mut cx, h) = headless_with(scratch.data(), move |data, cx| {
        mast_ui::snapshot::world_shell(data, rows, cx)
    });
    draw(&mut cx, h);
    draw(&mut cx, h);
    let tree = cx
        .update_window(h, |_, w, _| w.debug_a11y_tree_json())
        .unwrap()
        .unwrap_or_default();
    let selectors = selectors(&mut cx, h);
    let dump = format!(
        "{}\n{tree}\nselectors: {}\n",
        template.header(),
        selectors.join(" ")
    );
    std::fs::write(scratch.root().join("tree.txt"), &dump).unwrap();
    // The selected chat came out of the restored stores: its titlebar controls render.
    assert!(
        tree.contains("rename-chat-titlebar"),
        "selected chat missing from the a11y tree"
    );
    assert!(!selectors.is_empty());
    drop(cx);
    drop(rt);
}

#[test]
#[ignore = "requires permission to apply a macOS sandbox"]
fn world_tree_dump_runs_isolated() {
    use mast_ui_driver::environment::ScratchDirectories;
    let account = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let scratch = ScratchDirectories::create(&account).unwrap();
    let exe = std::env::current_exe().unwrap();
    let args = [
        "--ignored",
        "--exact",
        "inner_world_tree_dump",
        "--test-threads=1",
    ];
    let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    let out = scratch.run_isolated(&account, &exe, &args).unwrap();
    let dump = std::fs::read_to_string(scratch.root().join("tree.txt"));
    let ok = out.status.success();
    if !ok {
        eprintln!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let dump = dump.unwrap_or_default();
    assert!(ok, "inner test failed");
    assert!(
        dump.starts_with("preset=small seed=5 template="),
        "{}",
        &dump[..dump.len().min(200)]
    );
    println!("{}", dump.lines().next().unwrap());
    std::fs::remove_dir_all(scratch.root()).unwrap();
}
