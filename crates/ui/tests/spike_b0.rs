//! B0 spike (throwaway): S1 input + PNG on HeadlessAppContext, S2 a11y probe, S3 clock probe.
#![cfg(all(target_os = "macos", feature = "snapshot"))]
use std::sync::Arc;

use gpui::{
    AppContext, HeadlessAppContext, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, PlatformInput, point, px, size,
};
use mast_ui::{
    theme::{Appearance, Theme},
    ui_style::{self, UiLayout, UiStyleId},
};

fn out_dir() -> std::path::PathBuf {
    let d = std::env::var_os("SPIKE_OUT").map(Into::into).unwrap_or_else(|| std::env::temp_dir().join("spike-b0"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
#[ignore = "spike: needs Metal"]
fn spike_s1_s2_s3() -> anyhow::Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    let _g = rt.enter();
    let data = tempfile::tempdir()?;
    let native = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        native.text_system(),
        Arc::new(mast_ui::icons::Assets),
        gpui_platform::current_headless_renderer,
    );
    let dd = data.path().to_path_buf();
    let window = {
        let dd2 = dd.clone();
        cx.update(move |cx| {
            gpui_base::init(cx);
            if std::env::var_os("SPIKE_REDUCE").is_some() { cx.set_reduce_motion(true); }
            let mut s = mast_ui::settings::UiSettings::default();
            s.ui_style = UiStyleId::Default;
            s.ui_layout = UiLayout::Minimal;
            s.appearance = mast_ui::appearance::AppearanceMode::Dark;
            mast_ui::settings::init(s.clone(), &dd2, cx);
            let fonts = mast_ui::typography::register_fonts(cx);
            mast_ui::typography::init(
                s.ui_font_family.clone(), s.ui_font_size, s.terminal_font_family.clone(),
                s.terminal_font_size, s.code_font_family.clone(), s.code_font_size, fonts, cx,
            );
            mast_ui::history::init(
                s.git_history_columns, s.git_history_column_widths, s.git_history_column_order,
                s.git_history_author_display, cx,
            );
            mast_ui::composer::init(cx, s.composer_send_behavior);
            mast_ui::terminal::panel::init(cx);
            Theme::install_selection(
                Appearance::Dark, "default",
                mast_theme::AccentSelection::ThemeDefault,
                mast_theme::SurfacePreference::default(), cx,
            );
        });
        cx.open_window(size(px(1200.), px(800.)), move |_, cx| mast_ui::snapshot::chat_fixture(dd, cx))?
    };
    let h: gpui::AnyWindowHandle = window.into();
    let draw = |cx: &mut HeadlessAppContext| {
        cx.run_until_parked();
        cx.update_window(h, |_, w, c| w.draw(c).clear()).unwrap();
    };
    draw(&mut cx);
    let t0 = std::time::Instant::now();
    if std::env::var_os("SPIKE_REDUCE").is_none() { std::thread::sleep(std::time::Duration::from_millis(800)); }
    draw(&mut cx);
    let a = cx.capture_screenshot(h)?;
    a.save(out_dir().join("a-initial.png"))?;
    println!("S1 initial png {}x{}", a.width(), a.height());

    // S2: a11y tree on the test platform.
    let tree = cx.update_window(h, |_, w, _| (w.is_a11y_active(), w.debug_a11y_tree_json()))?;
    println!("S2 a11y active={} tree_len={:?}", tree.0, tree.1.as_ref().map(|t| t.len()));
    if let Some(t) = &tree.1 { std::fs::write(out_dir().join("a11y.json"), t)?; }

    // S1: injected key text into the focused composer, then a click on sidebar row.
    for ch in "hello driver".chars() {
        let k = ch.to_string();
        cx.update_window(h, |_, w, c| {
            w.dispatch_keystroke(Keystroke { modifiers: Modifiers::default(), key: k.clone(), key_char: Some(k) }, c);
        })?;
    }
    draw(&mut cx);
    let b = cx.capture_screenshot(h)?;
    b.save(out_dir().join("b-typed.png"))?;
    let click_at = std::env::var("SPIKE_CLICK").ok().and_then(|s| {
        let (x, y) = s.split_once(',')?; Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?))
    }).unwrap_or((100., 120.));
    let p = point(px(click_at.0), px(click_at.1));
    cx.update_window(h, |_, w, c| {
        w.dispatch_event(PlatformInput::MouseMove(gpui::MouseMoveEvent { position: p, pressed_button: None, modifiers: Modifiers::default() }), c);
        w.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: p, modifiers: Modifiers::default(), click_count: 1, first_mouse: false }), c);
        w.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: p, modifiers: Modifiers::default(), click_count: 1 }), c);
    })?;
    draw(&mut cx);
    let c = cx.capture_screenshot(h)?;
    c.save(out_dir().join("c-clicked.png"))?;
    let diff = |x: &image::RgbaImage, y: &image::RgbaImage| x.pixels().zip(y.pixels()).filter(|(p, q)| p != q).count();
    println!("S1 diff initial->typed {} px; typed->clicked {} px; elapsed {:?}", diff(&a, &b), diff(&b, &c), t0.elapsed());
    Ok(())
}
