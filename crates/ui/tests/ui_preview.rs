#[path = "ui_preview/matrix.rs"]
mod matrix;
#[cfg(all(target_os = "macos", feature = "snapshot"))]
#[path = "ui_preview/render.rs"]
mod render;
#[cfg(all(target_os = "macos", feature = "snapshot"))]
#[path = "ui_preview/warm_renderer.rs"]
mod warm_renderer;

#[cfg(all(target_os = "macos", feature = "snapshot"))]
fn snapshot<V: gpui::Render + 'static>(
    name: &str,
    size: gpui::Size<gpui::Pixels>,
    style: mast_ui::ui_style::UiStyleId,
    layout: mast_ui::ui_style::UiLayout,
    appearance: mast_ui::theme::Appearance,
    build: impl FnOnce(&mut gpui::Window, &mut gpui::App) -> gpui::Entity<V>,
) -> anyhow::Result<std::path::PathBuf> {
    if std::env::var_os("MAST_PREVIEW_WARM").is_some() {
        warm_renderer::render(name, size, style, layout, appearance, build)
    } else {
        mast_ui::snapshot::render_to_png_with_layout(name, size, style, layout, appearance, build)
    }
}

#[cfg(all(target_os = "macos", feature = "snapshot"))]
#[test]
#[ignore = "requires macOS Metal and access to macOS services"]
fn render_ui_preview() -> anyhow::Result<()> {
    if std::env::var_os("MAST_PREVIEW_SERVE").is_some() {
        use std::io::{BufRead, Write};
        for line in std::io::stdin().lock().lines() {
            let response = (|| -> anyhow::Result<_> {
                let request: serde_json::Value = serde_json::from_str(&line?)?;
                let manifest = render_preview(Some(&request))?;
                Ok(serde_json::json!({"manifest": manifest}))
            })()
            .unwrap_or_else(|error| serde_json::json!({"error": format!("{error:#}")}));
            println!("MAST_PREVIEW_RESPONSE={response}");
            std::io::stdout().flush()?;
        }
        return Ok(());
    }
    render_preview(None).map(|_| ())
}

#[cfg(all(target_os = "macos", feature = "snapshot"))]
fn render_preview(request: Option<&serde_json::Value>) -> anyhow::Result<std::path::PathBuf> {
    if let Some(request) = request {
        let object = request
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("request must be an object"))?;
        anyhow::ensure!(
            object
                .keys()
                .all(|key| matches!(key.as_str(), "screens" | "styles" | "layouts" | "themes")),
            "unknown request field"
        );
    }
    use image::{RgbaImage, imageops};
    use mast_ui::{
        theme::Appearance,
        ui_style::{UiLayout, UiStyleId},
    };
    use std::path::PathBuf;

    fn selection(
        request: Option<&serde_json::Value>,
        key: &str,
        variable: &str,
        defaults: &[&str],
    ) -> anyhow::Result<Vec<String>> {
        if let Some(value) = request.and_then(|request| request.get(key)) {
            return value
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("{key} must be an array"))?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| anyhow::anyhow!("{key} must contain strings"))
                })
                .collect();
        }
        Ok(std::env::var(variable)
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_else(|| defaults.iter().map(|value| (*value).to_owned()).collect()))
    }
    let screens = selection(
        request,
        "screens",
        "MAST_PREVIEW_SCREENS",
        &["chat", "details", "walkthrough", "appearance", "controls"],
    )?;
    let all_screens = [
        "chat",
        "overview",
        "linear",
        "linear-settings",
        "workflow-pack",
        "team",
        "changes-expanded",
        "changes-collapsed",
        "changes-focus-resize",
        "details",
        "walkthrough",
        "handoff",
        "appearance",
        "controls",
        "components",
        "browser",
    ];
    anyhow::ensure!(
        !screens.is_empty() && screens.iter().all(|s| all_screens.contains(&s.as_str())),
        "invalid or empty screen selection"
    );
    let style_names: Vec<String> = UiStyleId::ALL
        .iter()
        .map(|s| s.label().to_ascii_lowercase())
        .collect();
    let defaults: Vec<&str> = style_names.iter().map(String::as_str).collect();
    let styles = selection(request, "styles", "MAST_PREVIEW_STYLES", &defaults)?;
    anyhow::ensure!(
        !styles.is_empty() && styles.iter().all(|s| style_names.contains(s)),
        "invalid or empty style selection"
    );
    let layout_names: Vec<String> = UiLayout::ALL
        .iter()
        .map(|layout| layout.label().to_ascii_lowercase())
        .collect();
    let layout_defaults: Vec<&str> = layout_names.iter().map(String::as_str).collect();
    let layouts = selection(request, "layouts", "MAST_PREVIEW_LAYOUTS", &layout_defaults)?;
    anyhow::ensure!(
        !layouts.is_empty() && layouts.iter().all(|layout| layout_names.contains(layout)),
        "invalid or empty layout selection"
    );
    let mut themes = vec!["mast-light".to_string(), "mast-dark".to_string()];
    for theme in selection(request, "themes", "MAST_PREVIEW_THEMES", &[])? {
        if !themes.contains(&theme) {
            themes.push(theme);
        }
    }
    let registry = mast_theme::ThemeRegistry::builtin();
    for theme in &themes {
        anyhow::ensure!(registry.variant(theme).is_some(), "unknown theme: {theme}");
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let directory = target
        .join("ui-preview")
        .join(chrono::Local::now().format("%Y%m%d-%H%M%S-%3f").to_string());
    std::fs::create_dir_all(&directory)?;
    let data_dir = tempfile::tempdir()?;
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    let screen_refs: Vec<&str> = screens.iter().map(String::as_str).collect();
    let style_refs: Vec<&str> = styles.iter().map(String::as_str).collect();
    let layout_refs: Vec<&str> = layouts.iter().map(String::as_str).collect();
    let theme_refs: Vec<&str> = themes.iter().map(String::as_str).collect();
    let cells = matrix::matrix(&screen_refs, &style_refs, &layout_refs, &theme_refs);
    let (cell_width, cell_height) = matrix::cell_size(
        preview_width(),
        900,
        themes.len() as u32,
        (styles.len() * layouts.len()) as u32,
        4096,
    )
    .ok_or_else(|| anyhow::anyhow!("too many styles or themes for a contact sheet"))?;
    let mut sheets = Vec::new();
    let mut sheet_paths = Vec::new();
    for (screen_index, screen) in screens.iter().enumerate() {
        if screen == "components" {
            for (theme_index, theme) in themes.iter().enumerate() {
                let variant = registry.variant(theme).unwrap();
                let appearance = if variant.appearance == mast_theme::Appearance::Dark {
                    Appearance::Dark
                } else {
                    Appearance::Light
                };
                let path = render::render_component_matrix(
                    theme,
                    appearance,
                    data_dir.path().to_owned(),
                    &format!("preview-component-matrix-{theme_index}"),
                )?;
                let sheet = image::open(path)?.into_rgba8();
                let path = directory.join(format!("components-{theme}.png"));
                sheet.save(&path)?;
                println!("{}", path.display());
                sheet_paths.push(path);
                sheets.push(sheet);
            }
            continue;
        }
        let mut sheet = RgbaImage::new(
            cell_width * themes.len() as u32,
            cell_height * (styles.len() * layouts.len()) as u32,
        );
        for (index, cell) in cells
            .iter()
            .filter(|cell| cell.screen == screen.as_str())
            .enumerate()
        {
            let style = UiStyleId::ALL[style_names.iter().position(|s| s == cell.style).unwrap()];
            let layout = UiLayout::ALL[layout_names.iter().position(|s| s == cell.layout).unwrap()];
            let variant = registry.variant(cell.theme).unwrap();
            let appearance = match variant.appearance {
                mast_theme::Appearance::Light => Appearance::Light,
                mast_theme::Appearance::Dark => Appearance::Dark,
            };
            let path = if cell.screen == "browser" {
                render_browser(
                    style,
                    layout,
                    cell.theme,
                    appearance,
                    cell.label.clone(),
                    data_dir.path().to_owned(),
                    &format!("preview-{screen_index}-{index}"),
                )?
            } else {
                render::render(
                    cell.screen,
                    style,
                    layout,
                    cell.theme,
                    appearance,
                    cell.label.clone(),
                    data_dir.path().to_owned(),
                    &format!("preview-{screen_index}-{index}"),
                )?
            };
            let image = image::open(path)?.into_rgba8();
            let image = imageops::resize(
                &image,
                cell_width,
                cell_height,
                imageops::FilterType::Lanczos3,
            );
            imageops::replace(
                &mut sheet,
                &image,
                (index % themes.len()) as i64 * cell_width as i64,
                (index / themes.len()) as i64 * cell_height as i64,
            );
        }
        let path = directory.join(format!("{screen}.png"));
        sheet.save(&path)?;
        println!("{}", path.display());
        sheet_paths.push(path);
        sheets.push(sheet);
    }
    let columns = 2_u32.min(sheets.len() as u32);
    let rows = (sheets.len() as u32).div_ceil(columns);
    let max_width = sheets.iter().map(|s| s.width()).max().unwrap();
    let max_height = sheets.iter().map(|s| s.height()).max().unwrap();
    let (width, height) = matrix::cell_size(max_width, max_height, columns, rows, 4096)
        .ok_or_else(|| anyhow::anyhow!("index too large"))?;
    let mut index = RgbaImage::new(width * columns, height * rows);
    for (position, sheet) in sheets.iter().enumerate() {
        let thumbnail = imageops::resize(sheet, width, height, imageops::FilterType::Lanczos3);
        imageops::replace(
            &mut index,
            &thumbnail,
            (position as u32 % columns * width) as i64,
            (position as u32 / columns * height) as i64,
        );
    }
    let path = directory.join("index.png");
    index.save(&path)?;
    println!("{}", path.display());
    let manifest = serde_json::json!({
        "screens": screens,
        "styles": styles,
        "layouts": layouts,
        "themes": themes,
        "cellWidth": cell_width,
        "cellHeight": cell_height,
        "sheets": sheet_paths,
        "index": path,
    });
    let manifest_path = directory.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    println!("{}", manifest_path.display());
    Ok(manifest_path)
}

fn preview_width() -> u32 {
    std::env::var("MAST_PREVIEW_WIDTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|width| (320..=4096).contains(width))
        .unwrap_or(1440)
}

/// Compare Minimal-only contact sheets while excluding the preview's label bar.
#[cfg(all(target_os = "macos", feature = "snapshot"))]
#[test]
#[ignore = "requires MAST_BASELINE_PREVIEW and MAST_CURRENT_PREVIEW paths"]
fn compare_minimal_preview() -> anyhow::Result<()> {
    let baseline = std::path::PathBuf::from(std::env::var("MAST_BASELINE_PREVIEW")?);
    let current = std::path::PathBuf::from(std::env::var("MAST_CURRENT_PREVIEW")?);
    let screens = std::env::var("MAST_PREVIEW_COMPARE_SCREENS")
        .unwrap_or_else(|_| "controls,details".to_owned());
    let rows: u32 = std::env::var("MAST_PREVIEW_COMPARE_ROWS")
        .ok().map(|value| value.parse()).transpose()?.unwrap_or(1);
    anyhow::ensure!(rows > 0, "comparison row count must be positive");
    for screen in screens.split(',').filter(|screen| !screen.is_empty()) {
        let expected = image::open(baseline.join(format!("{screen}.png")))?.into_rgba8();
        let actual = image::open(current.join(format!("{screen}.png")))?.into_rgba8();
        anyhow::ensure!(
            expected.width() == actual.width() && expected.height() <= actual.height(),
            "{screen} dimensions differ"
        );
        let (width, sheet_height) = expected.dimensions();
        anyhow::ensure!(sheet_height % rows == 0, "{screen} rows do not divide the sheet");
        let height = sheet_height / rows;
        let expected = image::imageops::crop_imm(&expected, 0, 30, width, height - 30).to_image();
        let actual = image::imageops::crop_imm(&actual, 0, 30, width, height - 30).to_image();
        let diff = image_compare::rgba_hybrid_compare(&expected, &actual)?;
        println!("{screen} similarity: {}", diff.score);
        diff.image
            .to_color_map()
            .save(current.join(format!("{screen}-diff.png")))?;
        anyhow::ensure!(
            diff.score >= 0.999,
            "{screen} differs from Minimal baseline"
        );
    }
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "snapshot"))]
fn render_browser(
    style: mast_ui::ui_style::UiStyleId,
    layout: mast_ui::ui_style::UiLayout,
    theme_id: &str,
    appearance: mast_ui::theme::Appearance,
    label: String,
    data_dir: std::path::PathBuf,
    name: &str,
) -> anyhow::Result<std::path::PathBuf> {
    use gpui::{AppContext, Render, div, prelude::*, px, size};
    struct BrowserFrame {
        browser: gpui::Entity<mast_ui::browser::BrowserSurface>,
        label: String,
    }
    impl Render for BrowserFrame {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            let theme = mast_ui::theme::Theme::of(cx);
            div()
                .size_full()
                .flex()
                .flex_col()
                .bg(theme.surface)
                .text_color(theme.text)
                .child(
                    div()
                        .h(px(30.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .bg(theme.surface_raised)
                        .child(format!("browser   |   {}", self.label)),
                )
                .child(div().flex_1().min_h_0().child(self.browser.clone()))
        }
    }
    let theme_id = theme_id.to_owned();
    snapshot(
        name,
        size(px(1440.), px(900.)),
        style,
        layout,
        appearance,
        move |window, cx| {
            let mut settings = mast_ui::settings::UiSettings::default();
            settings.ui_style = style;
            settings.ui_layout = layout;
            mast_ui::settings::init(settings.clone(), &data_dir, cx);
            let fonts = mast_ui::typography::register_fonts(cx);
            mast_ui::typography::init(
                settings.ui_font_family.clone(),
                settings.ui_font_size,
                settings.terminal_font_family.clone(),
                settings.terminal_font_size,
                settings.code_font_family.clone(),
                settings.code_font_size,
                fonts,
                cx,
            );
            mast_ui::composer::init(cx, settings.composer_send_behavior);
            mast_ui::theme::Theme::install_selection(
                appearance,
                &theme_id,
                mast_theme::AccentSelection::ThemeDefault,
                mast_theme::SurfacePreference::default(),
                cx,
            );
            let browser = cx.new(|cx| {
                mast_ui::browser::BrowserSurface::new(
                    mast_ui::browser::BrowserContext::default(),
                    false,
                    window,
                    cx,
                )
            });
            cx.new(|_| BrowserFrame { browser, label })
        },
    )
}
