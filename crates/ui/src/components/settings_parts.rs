//! Presentation primitives for settings pages.

use crate::appearance::AppearanceMode;
use crate::theme::ink;
use crate::theme::{Appearance, Theme};
use gpui::{AnyElement, Hsla, SharedString, div, prelude::*, px};
use mast_theme::{AccentSelection, SurfacePreference, ThemeSelection};

pub(crate) fn settings_ghost_action(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0 * super::layout_spacing(theme)))
        .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(8.0)))
        .when(super::is_horizon(theme), |el| {
            el.rounded_full()
                .bg(theme.accent.opacity(0.12))
                .text_color(theme.accent)
        })
        .when(
            theme.ui_style.controls == crate::ui_style::ControlVariant::Key,
            |el| {
                el.bg(theme.surface_raised)
                    .border_1()
                    .border_b_2()
                    .border_color(theme.border)
            },
        )
        .px(px(10.0 * super::layout_spacing(theme)))
        .py(px(6.0 * super::layout_spacing(theme)))
        .text_size(crate::typography::ui_rems(
            12.0 + super::layout_type_adjustment(theme),
        ))
        .text_color(if super::is_horizon(theme) {
            theme.accent
        } else {
            theme.text_muted
        })
        .cursor_pointer()
}

pub(crate) fn settings_effective_surface(
    surface: mast_theme::SurfacePreference,
    style: crate::ui_style::UiStyle,
) -> mast_theme::SurfacePreference {
    match style.surface {
        crate::ui_style::SurfaceModel::ThemeDefault => surface,
        crate::ui_style::SurfaceModel::Opaque => mast_theme::SurfacePreference::Opaque,
        crate::ui_style::SurfaceModel::Glass => mast_theme::SurfacePreference::Frosted,
    }
}

/// Illustrates the spacing and guidance selected by a layout.
pub(crate) fn settings_layout_preview(
    theme: &Theme,
    layout: crate::ui_style::UiLayout,
) -> gpui::AnyElement {
    let (description, gap) = match layout {
        crate::ui_style::UiLayout::Minimal => ("Current spacing", 4.0),
        crate::ui_style::UiLayout::Compact => ("More on screen", 2.0),
        crate::ui_style::UiLayout::Guided => ("More labels and guidance", 6.0),
    };
    div()
        .size_full()
        .rounded(px(theme
            .ui_style
            .radius_roles()
            .panel
            .unwrap_or(crate::settings::widgets::OPTION_CARD_RADIUS)))
        .bg(theme.surface)
        .p(px(16.0))
        .flex()
        .flex_col()
        .gap(px(gap))
        .children((0..4).map(|_| {
            div()
                .h(px(12.0))
                .w_full()
                .rounded(px(3.0))
                .bg(theme.surface_raised)
        }))
        .child(
            div()
                .mt(px(8.0))
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted)
                .child(description),
        )
        .into_any_element()
}

pub(crate) fn settings_page_header(theme: &Theme, title: &str, count: Option<usize>) -> gpui::Div {
    let title = if theme.ui_style.design_roles.tracked_caps.unwrap_or_else(|| super::is_industrial(theme)) {
        crate::popover::tracked_upper(title)
    } else {
        title.to_owned()
    };
    div()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(px(10.0))
        .child(
            div()
                .text_size(crate::typography::ui_rems(
                    theme.ui_style.type_roles().title.unwrap_or(16.0)
                        + super::layout_type_adjustment(theme),
                ))
                .font_weight(theme.ui_style.design_roles.heading_weight.map(|weight| weight.weight()).unwrap_or(gpui::FontWeight::SEMIBOLD))
                .when_some(theme.ui_style.design_roles.heading_family, |el, family| el.font_family(match family { crate::style_overrides::HeadingFamily::Sans => theme.font_sans.clone(), crate::style_overrides::HeadingFamily::Mono => theme.font_mono.clone() }))
                .text_color(theme.text)
                .child(SharedString::from(title)),
        )
        .when_some(count, |el, count| {
            el.child(
                div()
                    .text_size(crate::typography::ui_rems(13.0))
                    .text_color(theme.text_muted.opacity(0.7))
                    .child(SharedString::from(format!("{count}"))),
            )
        })
}

pub(crate) fn settings_option_card(
    theme: &Theme,
    icon_path: &'static str,
    label: impl Into<SharedString>,
    selected: bool,
    preview: AnyElement,
) -> gpui::Div {
    let label: SharedString = label.into();
    let label = if super::is_industrial(theme) {
        SharedString::from(crate::popover::tracked_upper(label.as_ref()))
    } else {
        label
    };
    let selected_selector = format!("settings-option-selected-{label}");
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.0))
        .cursor_pointer()
        .child(
            div()
                .when(super::is_horizon(theme), |el| el.relative())
                .h(px(crate::settings::widgets::OPTION_CARD_HEIGHT))
                .w_full()
                .rounded(px(theme
                    .ui_style
                    .radius_roles()
                    .panel
                    .unwrap_or(crate::settings::widgets::OPTION_CARD_RADIUS)))
                .overflow_hidden()
                .border_1()
                .border_color(if selected { theme.accent } else { theme.border })
                .when(super::is_industrial(theme), |el| el.border_b_2())
                .child(preview)
                .when(selected && super::is_horizon(theme), |card| {
                    card.child(
                        div().absolute().inset_0().bg(theme.accent.opacity(0.10))
                            .child(
                                div().debug_selector(move || selected_selector.clone()).absolute().top(px(8.0)).right(px(8.0))
                                    .size(px(20.0)).rounded_full().bg(theme.accent)
                                    .flex().items_center().justify_center()
                                    .child(crate::icons::icon(crate::icons::CHECK).size(px(12.0)).text_color(theme.identity_text(theme.accent))),
                            ),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .when(super::is_horizon(theme), |el| {
                    el.rounded_full()
                        .px(px(10.0))
                        .py(px(4.0))
                        .bg(theme.accent.opacity(if selected { 0.16 } else { 0.06 }))
                })
                .text_size(crate::typography::ui_rems(13.0))
                .font_weight(if selected {
                    gpui::FontWeight::MEDIUM
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if selected {
                    theme.accent
                } else {
                    theme.text_muted
                })
                .child(crate::icons::icon(icon_path).size(px(16.0)).flex_none())
                .child(label),
        )
}

pub(crate) fn settings_row_tile(theme: &Theme, icon_path: &'static str) -> gpui::Div {
    settings_row_tile_tinted(theme, icon_path, None)
}

pub(crate) fn settings_row_tile_tinted(
    theme: &Theme,
    icon_path: &'static str,
    tint: Option<Hsla>,
) -> gpui::Div {
    div()
        .flex_none()
        .size(px(36.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(10.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(theme.border)
        .bg(if super::is_horizon(theme) {
            theme.accent.opacity(0.12)
        } else {
            ink(0.03)
        })
        .when(super::is_industrial(theme), |el| {
            el.bg(theme.surface_raised).border_b_2()
        })
        .flex()
        .items_center()
        .justify_center()
        .child(
            crate::icons::icon(icon_path)
                .size(px(16.0))
                .text_color(tint.unwrap_or(if super::is_horizon(theme) {
                    theme.accent
                } else {
                    theme.text_muted
                })),
        )
}

/// Selection control for a surface or background effect.
pub(crate) fn settings_choice(
    theme: &Theme,
    id: SharedString,
    label: &'static str,
    selected: bool,
    compact: bool,
) -> gpui::Stateful<gpui::Div> {
    let industrial = super::is_industrial(theme);
    let horizon = super::is_horizon(theme);
    let label = if industrial {
        crate::popover::tracked_upper(label)
    } else {
        label.to_owned()
    };
    div()
        .id(id)
        .h(px(if compact { 28.0 } else { 30.0 }))
        .px(px(if compact { 9.0 } else { 10.0 }))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(7.0)))
        .when(horizon, |el| el.rounded_full())
        .border_1()
        .when(industrial, |el| el.border_b_2())
        .border_color(if selected { theme.accent } else { theme.border })
        .bg(if selected {
            theme.accent_wash
        } else if horizon {
            theme.accent.opacity(0.08)
        } else {
            theme.surface_raised.opacity(0.28)
        })
        .text_size(crate::typography::ui_rems(if compact {
            11.0
        } else {
            11.5
        }))
        .font_weight(if selected {
            gpui::FontWeight::MEDIUM
        } else {
            gpui::FontWeight::NORMAL
        })
        .text_color(if selected {
            theme.accent
        } else {
            theme.text_muted
        })
        .flex()
        .items_center()
        .cursor_pointer()
        .when(!selected, |control| {
            control.hover(|style| style.bg(theme.surface_raised_hover))
        })
        .child(SharedString::from(label))
}

/// Shortcut recording key; the page owns its keyboard and pointer actions.
pub(crate) fn settings_shortcut_key(
    theme: &Theme,
    index: usize,
    recording: bool,
) -> gpui::Stateful<gpui::Div> {
    let horizon = super::is_horizon(theme);
    let industrial = super::is_industrial(theme);
    let key = div()
        .id(("shortcut-combo", index))
        .min_w(px(96.0))
        .px(px(12.0))
        .py(px(6.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(8.0)))
        .when(horizon, |el| el.rounded_full())
        .border_1()
        .when(industrial, |el| el.border_b_2())
        .flex()
        .justify_center()
        .font_family(theme.font_mono.clone())
        .text_size(crate::typography::ui_rems(12.0))
        .cursor_pointer();
    if recording {
        key.border_color(theme.text.opacity(0.3))
            .bg(theme.text)
            .text_color(theme.on_solid)
    } else {
        key.border_color(theme.border)
            .bg(if horizon {
                theme.accent.opacity(0.1)
            } else {
                theme.bg
            })
            .text_color(theme.text)
            .hover(|s| {
                s.border_color(theme.text.opacity(0.2))
                    .bg(crate::theme::ink(0.03))
            })
    }
}

/// A destination option in a settings row.
pub(crate) fn settings_destination(
    theme: &Theme,
    index: usize,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    let horizon = super::is_horizon(theme);
    div()
        .id(("appshot-destination", index))
        .px(px(10.0))
        .py(px(7.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(7.0)))
        .when(horizon, |el| el.rounded_full())
        .border_1()
        .when(super::is_industrial(theme), |el| el.border_b_2())
        .border_color(if selected {
            theme.text.opacity(0.24)
        } else {
            theme.border
        })
        .bg(if horizon {
            theme.accent.opacity(if selected { 0.2 } else { 0.08 })
        } else if selected {
            ink(0.09)
        } else {
            gpui::transparent_black()
        })
        .text_size(px(11.0))
        .text_color(if selected {
            theme.text
        } else {
            theme.text_muted
        })
        .cursor_pointer()
}

/// Picker chrome, leaving focus and menu behavior to its owner.
pub(crate) fn settings_picker(
    theme: &Theme,
    id: SharedString,
    width: f32,
    open: bool,
    palette: bool,
) -> gpui::Stateful<gpui::Div> {
    let horizon = super::is_horizon(theme);
    let industrial = super::is_industrial(theme);
    let control = div()
        .id(id)
        .relative()
        .when(palette, |el| el.flex_none())
        .w(px(width))
        .h(px(if palette { 34.0 } else { 36.0 }))
        .px(px(if palette { 10.0 } else { 11.0 }))
        .rounded(px(theme
            .ui_style
            .radius_roles()
            .control
            .unwrap_or(if palette { 8.0 } else { 9.0 })))
        .when(horizon, |el| el.rounded_full())
        .border_1()
        .when(industrial, |el| el.border_b_2())
        .border_color(if open {
            theme.border_strong
        } else {
            theme.border
        })
        .bg(if horizon {
            theme.accent.opacity(if open { 0.16 } else { 0.08 })
        } else if palette {
            theme.surface_raised.opacity(if open { 0.75 } else { 0.42 })
        } else {
            ink(0.025)
        })
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .cursor_pointer();
    if palette && !open {
        control.hover(|style| style.bg(theme.surface_raised_hover))
    } else {
        control
    }
}

fn bar(fraction: f32, tone: Hsla) -> gpui::Div {
    div()
        .h(px(5.0))
        .w(gpui::relative(fraction))
        .rounded(px(3.0))
        .bg(tone)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Corners {
    All,
    Left,
    Right,
}

fn miniature(theme: &Theme, corners: Corners, radius: f32) -> AnyElement {
    let line = theme.text.opacity(0.22);
    let strong = theme.text.opacity(0.34);
    let r = px(radius);
    let root = div().size_full().flex().flex_row().bg(theme.surface);
    let root = match corners {
        Corners::All => root.rounded(r),
        Corners::Left => root.rounded_tl(r).rounded_bl(r),
        Corners::Right => root.rounded_tr(r).rounded_br(r),
    };
    root.child(
        div()
            .w(px(44.0))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap(px(7.0))
            .px(px(8.0))
            .pt(px(14.0))
            .child(bar(0.70, strong))
            .child(bar(1.0, line))
            .child(bar(0.85, line))
            .child(bar(1.0, line)),
    )
    .child(
        div()
            .flex_1()
            .min_w_0()
            .my(px(8.0))
            .mr(px(8.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg)
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap(px(7.0))
            .p(px(10.0))
            .child(bar(0.62, strong))
            .child(bar(0.88, line))
            .child(bar(0.76, line))
            .child(bar(0.52, line)),
    )
    .into_any_element()
}

fn miniature_split(
    themes: &ThemeSelection,
    accent: AccentSelection,
    surface: SurfacePreference,
    radius: f32,
) -> AnyElement {
    let light = Theme::for_selection(Appearance::Light, &themes.light, accent, surface);
    let dark = Theme::for_selection(Appearance::Dark, &themes.dark, accent, surface);
    div()
        .size_full()
        .flex()
        .flex_row()
        .child(div().w_1_2().h_full().overflow_hidden().child(miniature(
            &light,
            Corners::Left,
            radius,
        )))
        .child(div().w_1_2().h_full().overflow_hidden().child(miniature(
            &dark,
            Corners::Right,
            radius,
        )))
        .into_any_element()
}

pub(crate) fn settings_appearance_preview(
    active_theme: &Theme,
    mode: AppearanceMode,
    themes: &ThemeSelection,
    accent: AccentSelection,
    surface: SurfacePreference,
) -> AnyElement {
    let radius = active_theme
        .ui_style
        .radius_roles()
        .panel
        .unwrap_or(crate::settings::widgets::OPTION_CARD_RADIUS);
    match mode {
        AppearanceMode::System => miniature_split(themes, accent, surface, radius),
        AppearanceMode::Light => miniature(
            &Theme::for_selection(Appearance::Light, &themes.light, accent, surface),
            Corners::All,
            radius,
        ),
        AppearanceMode::Dark => miniature(
            &Theme::for_selection(Appearance::Dark, &themes.dark, accent, surface),
            Corners::All,
            radius,
        ),
    }
}

/// Illustration of a style choice inside the appearance picker.
pub(crate) fn settings_style_preview(theme: &Theme, id: crate::ui_style::UiStyleId) -> AnyElement {
    let variants = id.recommended_themes();
    let variant = if theme.appearance.is_dark() { variants.1 } else { variants.0 };
    let mut preview = Theme::for_selection(theme.appearance, variant, Default::default(), theme.surface_preference);
    preview.ui_style = crate::ui_style::UiStyle::resolve(id, crate::ui_style::UiLayout::Minimal);
    let theme = &preview;
    div()
        .size_full()
        .rounded(px(theme
            .ui_style
            .radius_roles()
            .panel
            .unwrap_or(crate::settings::widgets::OPTION_CARD_RADIUS)))
        .bg(theme.surface)
        .p(px(16.0))
        .flex()
        .flex_col()
        .gap(px(id.preview_gap()))
        .children((0..4).map(|index| {
            div()
                .h(px(12.0))
                .w_full()
                .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(3.0)))
                .bg(if index == 0 { theme.accent } else { theme.surface_raised })
        }))
        .child(
            div()
                .mt(px(8.0))
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted)
                .child(id.description()),
        )
        .into_any_element()
}

/// Theme import sample with its original preview frame.
pub(crate) fn settings_theme_miniature(theme: &Theme) -> AnyElement {
    miniature(
        theme,
        Corners::All,
        crate::settings::widgets::OPTION_CARD_RADIUS,
    )
}

pub(crate) fn settings_accent_swatch(
    page_theme: &Theme,
    selection: AccentSelection,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    let swatch_theme = Theme::for_selection(
        page_theme.appearance,
        page_theme.variant_id.as_ref(),
        selection,
        page_theme.surface_preference,
    );
    let sample = match selection {
        AccentSelection::ThemeDefault => div()
            .size_full()
            .rounded(px(page_theme
                .ui_style
                .radius_roles()
                .control
                .unwrap_or(6.0)))
            .when(super::is_horizon(page_theme), |el| el.rounded_full())
            .bg(swatch_theme.accent_wash)
            .flex()
            .items_center()
            .justify_center()
            .gap(px(2.0))
            .child(
                div()
                    .w(px(4.0))
                    .h(px(13.0))
                    .rounded(px(2.0))
                    .bg(swatch_theme.glyph.light),
            )
            .child(
                div()
                    .w(px(4.0))
                    .h(px(16.0))
                    .rounded(px(2.0))
                    .bg(swatch_theme.glyph.mid),
            )
            .child(
                div()
                    .w(px(4.0))
                    .h(px(11.0))
                    .rounded(px(2.0))
                    .bg(swatch_theme.glyph.deep),
            ),
        AccentSelection::Preset(_) => div()
            .size_full()
            .rounded(px(page_theme
                .ui_style
                .radius_roles()
                .control
                .unwrap_or(6.0)))
            .when(super::is_horizon(page_theme), |el| el.rounded_full())
            .bg(swatch_theme.accent),
    };
    div()
        .id(SharedString::from(format!("accent-{}", selection.label())))
        .flex_none()
        .w(px(30.0))
        .h(px(34.0))
        .pb(px(4.0))
        .border_b_2()
        .border_color(if selected {
            swatch_theme.accent
        } else {
            gpui::transparent_black()
        })
        .cursor_pointer()
        .child(
            div()
                .size(px(30.0))
                .p(px(2.0))
                .rounded(px(page_theme
                    .ui_style
                    .radius_roles()
                    .control
                    .unwrap_or(8.0)))
                .when(super::is_horizon(page_theme), |el| el.rounded_full())
                .border_1()
                .when(super::is_industrial(page_theme), |el| el.border_b_2())
                .border_color(if selected {
                    page_theme.border_strong
                } else {
                    page_theme.border
                })
                .bg(page_theme.surface_raised.opacity(0.42))
                .child(sample),
        )
}

pub(crate) fn settings_compact_action(
    theme: &Theme,
    label: &str,
    id: impl Into<SharedString>,
) -> gpui::Stateful<gpui::Div> {
    let id = id.into();
    crate::popover::btn_ghost(theme, label, id.clone())
        .id(id)
        .h(px(28.0))
        .px(px(9.0))
        .py(px(0.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(7.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .when(super::is_industrial(theme), |el| el.border_b_2())
        .border_color(theme.border)
        .bg(if super::is_horizon(theme) {
            theme.accent.opacity(0.12)
        } else {
            theme.surface_raised.opacity(0.34)
        })
        .flex()
        .items_center()
        .text_size(crate::typography::ui_rems(11.5))
}

/// Import mode card with an accessible selection indicator.
pub(crate) fn settings_import_mode(
    theme: &Theme,
    id: SharedString,
    label: &'static str,
    description: &'static str,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    let title = if super::is_industrial(theme) {
        crate::popover::tracked_upper(label)
    } else {
        label.to_owned()
    };
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .p(px(10.0))
        .rounded(px(theme.ui_style.radius_roles().panel.unwrap_or(9.0)))
        .border_1()
        .when(super::is_industrial(theme), |el| el.border_b_2())
        .border_color(if active { theme.accent } else { theme.border })
        .bg(if active {
            theme.accent_wash
        } else if super::is_horizon(theme) {
            theme.accent.opacity(0.08)
        } else {
            theme.surface_raised.opacity(0.28)
        })
        .cursor_pointer()
        .when(!active, |control| {
            control.hover(|style| style.bg(theme.surface_raised_hover))
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.0))
                .child(
                    div()
                        .size(px(16.0))
                        .rounded_full()
                        .border_1()
                        .border_color(if active {
                            theme.accent
                        } else {
                            theme.border_strong
                        })
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(active, |dot| {
                            dot.child(div().size(px(8.0)).rounded_full().bg(theme.accent))
                        }),
                )
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(if active { theme.text } else { theme.text_muted })
                        .child(SharedString::from(title)),
                ),
        )
        .child(
            div()
                .mt(px(4.0))
                .ml(px(23.0))
                .text_size(crate::typography::ui_rems(10.5))
                .text_color(theme.text_muted)
                .child(description),
        )
}

/// Device selector shared by account and harness settings.
pub(crate) fn settings_device_switcher(
    theme: &Theme,
    id: &'static str,
    open: bool,
) -> gpui::Stateful<gpui::Div> {
    let horizon = super::is_horizon(theme);
    div()
        .id(id)
        .flex_none()
        .h(px(28.0))
        .px(px(8.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(6.0)))
        .when(horizon, |el| el.rounded_full())
        .when(super::is_industrial(theme), |el| {
            el.border_1()
                .border_b_2()
                .border_color(theme.border)
                .bg(theme.surface_raised)
        })
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .cursor_pointer()
        .bg(if horizon {
            theme.accent.opacity(if open { 0.18 } else { 0.08 })
        } else if open {
            ink(0.06)
        } else if super::is_industrial(theme) {
            theme.surface_raised
        } else {
            gpui::transparent_black()
        })
        .when(!open, |el| el.hover(|s| s.bg(ink(0.04))))
}

/// Autosave delay choice.
pub(crate) fn settings_autosave_choice(
    theme: &Theme,
    id: SharedString,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    let horizon = super::is_horizon(theme);
    div()
        .id(id)
        .h(px(28.0))
        .px(px(10.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(7.0)))
        .when(horizon, |el| el.rounded_full())
        .border_1()
        .when(super::is_industrial(theme), |el| el.border_b_2())
        .border_color(if active {
            theme.accent.opacity(0.7)
        } else {
            theme.border
        })
        .bg(if active {
            theme.accent.opacity(0.11)
        } else if horizon {
            theme.accent.opacity(0.06)
        } else {
            crate::theme::wash(0.025)
        })
        .text_size(px(11.5))
        .text_color(if active { theme.text } else { theme.text_muted })
        .flex()
        .items_center()
        .cursor_pointer()
        .hover(|style| style.bg(crate::theme::wash(0.08)))
}

/// Container and options for the send-key setting.
pub(crate) fn settings_send_behavior_group(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id("composer-send-behavior")
        .flex()
        .flex_row()
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(9.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .p(px(2.0))
        .bg(if super::is_horizon(theme) {
            theme.accent.opacity(0.08)
        } else {
            ink(0.04)
        })
}
pub(crate) fn settings_send_behavior_option(
    theme: &Theme,
    index: usize,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("composer-send-option", index))
        .min_w(px(72.0))
        .px(px(12.0))
        .py(px(6.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(7.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .flex()
        .items_center()
        .justify_center()
        .font_family(theme.font_mono.clone())
        .text_size(px(12.0))
        .text_color(if selected {
            theme.text
        } else {
            theme.text_muted
        })
        .when(selected, |el| {
            el.bg(if super::is_horizon(theme) {
                theme.accent.opacity(0.18)
            } else {
                theme.bg
            })
            .border_1()
            .border_color(theme.border.opacity(0.8))
            .when(super::is_industrial(theme), |el| el.border_b_2())
        })
}

/// Archived session row, glyph, and restore action.
pub(crate) fn settings_archived_row(theme: &Theme, index: usize) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("archived-row", index))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .rounded(px(theme.ui_style.radius_roles().panel.unwrap_or(8.0)))
        .px(px(12.0))
        .py(px(8.0))
        .hover(|s| s.bg(ink(0.03)))
}
pub(crate) fn settings_archive_tile(theme: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .size(px(32.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(6.0)))
        .when(super::is_horizon(theme), |el| {
            el.rounded_full().bg(theme.accent.opacity(0.1))
        })
        .border_1()
        .when(super::is_industrial(theme), |el| {
            el.bg(theme.surface_raised).border_b_2()
        })
        .border_color(theme.border)
        .flex()
        .items_center()
        .justify_center()
        .child(
            crate::icons::icon(crate::icons::ARCHIVE_MINIMALISTIC)
                .size(px(16.0))
                .text_color(theme.text_muted.opacity(0.6)),
        )
}
pub(crate) fn settings_unarchive_action(
    theme: &Theme,
    index: usize,
    hovered: bool,
    busy: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("unarchive", index))
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(10.0))
        .py(px(4.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(6.0)))
        .when(super::is_horizon(theme), |el| {
            el.rounded_full().bg(theme.accent.opacity(0.1))
        })
        .border_1()
        .when(super::is_industrial(theme), |el| {
            el.bg(theme.surface_raised).border_b_2()
        })
        .border_color(theme.border)
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted)
        .opacity(if hovered || busy { 1.0 } else { 0.0 })
        .when(busy, |el| el.opacity(0.4))
        .cursor_pointer()
        .hover(|s| s.bg(theme.surface_raised).text_color(theme.text))
}

/// Account actions with the page's compact geometry.
pub(crate) fn settings_account_forget(
    theme: &Theme,
    index: usize,
    busy: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("account-forget", index))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(6.0)))
        .when(super::is_horizon(theme), |el| {
            el.rounded_full().bg(theme.accent.opacity(0.1))
        })
        .when(super::is_industrial(theme), |el| {
            el.bg(theme.surface_raised)
                .border_1()
                .border_b_2()
                .border_color(theme.border)
        })
        .px(px(6.0))
        .py(px(4.0))
        .text_color(theme.text_muted)
        .cursor_pointer()
        .when(busy, |el| el.opacity(0.5))
        .hover(|s| s.bg(ink(0.06)).text_color(theme.text))
}
pub(crate) fn settings_account_switch(
    theme: &Theme,
    index: usize,
    busy: bool,
) -> gpui::Stateful<gpui::Div> {
    crate::popover::btn_primary(theme, if busy { "Switching…" } else { "Switch" })
        .id(("account-switch", index))
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(6.0)))
        .when(super::is_horizon(theme), |el| {
            el.rounded_full()
                .bg(theme.accent_strong)
                .text_color(theme.on_accent)
        })
        .when(super::is_industrial(theme), |el| el.border_b_2())
        .text_size(crate::typography::ui_rems(11.5))
        .when(busy, |el| el.opacity(0.5))
}

/// Team tab visibility choice.
pub(crate) fn settings_team_visibility(
    theme: &Theme,
    label: &'static str,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    let text = if super::is_industrial(theme) {
        crate::popover::tracked_upper(label)
    } else {
        label.to_owned()
    };
    div()
        .id(SharedString::from(format!("team-visibility-{label}")))
        .px(px(10.0))
        .py(px(5.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(6.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .when(super::is_industrial(theme), |el| {
            el.border_1().border_b_2().border_color(theme.border)
        })
        .bg(if active {
            theme.accent.opacity(0.15)
        } else if super::is_horizon(theme) {
            theme.accent.opacity(0.06)
        } else {
            crate::theme::wash(0.025)
        })
        .cursor_pointer()
        .child(SharedString::from(text))
}
/// A selectable imported theme variant.
pub(crate) fn settings_theme_import_row(
    theme: &Theme,
    id: SharedString,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .mt(px(8.0))
        .p(px(11.0))
        .rounded(px(theme.ui_style.radius_roles().panel.unwrap_or(10.0)))
        .border_1()
        .when(super::is_industrial(theme), |el| el.border_b_2())
        .border_color(if selected {
            theme.accent.opacity(0.7)
        } else {
            theme.border
        })
        .bg(if selected {
            theme.accent_wash.opacity(0.42)
        } else if super::is_horizon(theme) {
            theme.accent.opacity(0.08)
        } else {
            theme.surface_raised.opacity(0.22)
        })
        .flex()
        .flex_col()
}

pub(crate) fn settings_error_strip(theme: &Theme, message: impl Into<SharedString>) -> gpui::Div {
    let red = theme.danger; // red-400
    let red_text = theme.danger_muted; // red-300
    div()
        .mt(px(16.0))
        .px(px(16.0))
        .py(px(12.0))
        .rounded(px(theme.ui_style.radius_roles().panel.unwrap_or(12.0)))
        .border_1()
        .border_color(red.opacity(0.2))
        .bg(red.opacity(0.06))
        .text_size(crate::typography::ui_rems(12.5))
        .text_color(red_text.opacity(0.9))
        .flex()
        .flex_row()
        .items_start()
        .gap(px(8.0))
        .child(
            div().flex_none().mt(px(2.0)).child(
                crate::icons::icon(crate::icons::DANGER_TRIANGLE)
                    .size(px(16.0))
                    .text_color(red_text.opacity(0.9)),
            ),
        )
        .child(div().min_w_0().child(message.into()))
}

pub(crate) fn settings_warning_strip(theme: &Theme, message: impl Into<SharedString>) -> gpui::Div {
    let amber = theme.warning; // amber-400
    let amber_text = theme.warning_muted; // amber-200
    div()
        .mt(px(8.0))
        .px(px(16.0))
        .py(px(10.0))
        .rounded(px(theme.ui_style.radius_roles().panel.unwrap_or(12.0)))
        .border_1()
        .border_color(amber.opacity(0.2))
        .bg(amber.opacity(0.06))
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(amber_text.opacity(0.9))
        .flex()
        .flex_row()
        .items_start()
        .gap(px(8.0))
        .child(
            div().flex_none().mt(px(2.0)).child(
                crate::icons::icon(crate::icons::DANGER_TRIANGLE)
                    .size(px(14.0))
                    .text_color(amber_text.opacity(0.9)),
            ),
        )
        .child(div().min_w_0().child(message.into()))
}

/// Theme import selection checkbox.
pub(crate) fn settings_theme_import_checkbox(
    theme: &Theme,
    id: SharedString,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(18.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(5.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .when(super::is_industrial(theme), |el| el.border_b_2())
        .border_color(if selected {
            theme.accent
        } else {
            theme.border_strong
        })
        .bg(if selected { theme.accent } else { theme.bg })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(selected, |item| {
            item.child(
                crate::icons::icon(crate::icons::CHECK)
                    .size(px(12.0))
                    .text_color(theme.on_accent),
            )
        })
}
/// Thumbnail of the selected composer background.
pub(crate) fn settings_background_thumbnail(theme: &Theme, path: std::path::PathBuf) -> gpui::Div {
    let radius = theme.ui_style.radius_roles().control;
    div()
        .flex_none()
        .size(px(36.0))
        .rounded(px(radius.unwrap_or(10.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .overflow_hidden()
        .border_1()
        .border_color(crate::theme::hairline(0.10))
        .child(
            gpui::img(path)
                .size(px(34.0))
                .rounded(px(radius.unwrap_or(9.0)))
                .when(super::is_horizon(theme), |el| el.rounded_full())
                .object_fit(gpui::ObjectFit::Cover),
        )
}

pub(crate) fn settings_report_panel(
    theme: &Theme,
    report: &mast_theme::vscode::ImportReport,
) -> gpui::Stateful<gpui::Div> {
    let summary = format!(
        "{} mapped · {} adjusted · {} inferred/fallback · {} unsupported · {} warnings · {} validation",
        report.mappings.len(),
        report.adjustments.len(),
        report.fallbacks.len(),
        report.dropped.len(),
        report.warnings.len(),
        report.validation.len(),
    );
    div()
        .id(SharedString::from(format!(
            "theme-report-{}",
            report.source_hash
        )))
        .mt(px(8.0))
        .w_full()
        .max_h(px(168.0))
        .overflow_y_scroll()
        .rounded(px(theme.ui_style.radius_roles().panel.unwrap_or(8.0)))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_raised.opacity(0.35))
        .p(px(10.0))
        .text_size(crate::typography::ui_rems(11.0))
        .line_height(px(16.0))
        .text_color(theme.text_muted)
        .child(div().text_color(theme.text).child(summary))
        .children(report.adjustments.iter().map(|adjustment| {
            div().mt(px(4.0)).child(SharedString::from(format!(
                "Adjusted · {} {} → {} · {}",
                adjustment.mast_role, adjustment.original, adjustment.resolved, adjustment.reason
            )))
        }))
        .children(report.fallbacks.iter().map(|message| {
            div()
                .mt(px(4.0))
                .child(SharedString::from(format!("Fallback · {message}")))
        }))
        .children(report.warnings.iter().map(|message| {
            div()
                .mt(px(4.0))
                .child(SharedString::from(format!("Warning · {message}")))
        }))
        .children(report.validation.iter().map(|issue| {
            div().mt(px(4.0)).child(SharedString::from(format!(
                "Validation {:?} {:?} · {}",
                issue.category, issue.severity, issue.message
            )))
        }))
        .children(report.dropped.iter().map(|message| {
            div()
                .mt(px(4.0))
                .child(SharedString::from(format!("Unsupported · {message}")))
        }))
        .children(report.mappings.iter().map(|mapping| {
            div().mt(px(4.0)).child(SharedString::from(format!(
                "{} ← {}",
                mapping.mast_role, mapping.vscode_key
            )))
        }))
}

/// Dismiss action on the theme import dialog.
pub(crate) fn settings_import_close(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id("theme-import-close")
        .size(px(28.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(7.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .when(super::is_industrial(theme), |el| el.border_b_2())
        .border_color(theme.border)
        .bg(if super::is_horizon(theme) {
            theme.accent.opacity(0.1)
        } else {
            theme.surface_raised.opacity(0.28)
        })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|style| style.bg(theme.surface_raised_hover))
}

pub(crate) fn settings_row_title(theme: &Theme, title: impl Into<SharedString>) -> gpui::Div {
    div()
        .min_w_0()
        .truncate()
        .text_size(crate::typography::ui_rems(
            theme
                .ui_style
                .type_roles()
                .body
                .unwrap_or(crate::settings::widgets::ROW_TITLE_SIZE)
                + super::layout_type_adjustment(theme),
        ))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(title.into())
}

pub(crate) fn settings_meta_line(theme: &Theme, fragments: Vec<AnyElement>) -> gpui::Div {
    let mut line = div()
        .mt(px(Theme::TEXT_STACK_GAP))
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_x(px(8.0))
        .gap_y(px(2.0))
        .text_size(crate::typography::ui_rems(
            theme
                .ui_style
                .type_roles()
                .secondary
                .unwrap_or(crate::settings::widgets::ROW_DESCRIPTION_SIZE)
                + super::layout_type_adjustment(theme),
        ))
        .text_color(theme.text_muted.opacity(0.65));
    let mut first = true;
    for fragment in fragments {
        if !first {
            line = line.child(
                div()
                    .text_color(theme.text_muted.opacity(0.3))
                    .child(SharedString::from("·")),
            );
        }
        line = line.child(fragment);
        first = false;
    }
    line
}

/// Reset the send-key setting.
pub(crate) fn settings_send_behavior_reset(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id("composer-send-reset")
        .size(px(26.0))
        .rounded(px(theme.ui_style.radius_roles().control.unwrap_or(7.0)))
        .when(super::is_horizon(theme), |el| {
            el.rounded_full().bg(theme.accent.opacity(0.08))
        })
        .when(super::is_industrial(theme), |el| {
            el.bg(theme.surface_raised)
                .border_1()
                .border_b_2()
                .border_color(theme.border)
        })
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(|s| s.bg(ink(0.04)).text_color(theme.text))
}

/// Theme import warning and error surfaces.
pub(crate) fn settings_import_warning(theme: &Theme) -> gpui::Div {
    div()
        .mt(px(8.0))
        .p(px(10.0))
        .rounded(px(theme.ui_style.radius_roles().panel.unwrap_or(8.0)))
        .bg(theme.warning.opacity(0.08))
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.warning)
}
pub(crate) fn settings_import_error(theme: &Theme) -> gpui::Div {
    div()
        .mt(px(12.0))
        .p(px(10.0))
        .rounded(px(theme.ui_style.radius_roles().panel.unwrap_or(8.0)))
        .bg(theme.danger.opacity(0.08))
        .flex()
        .items_start()
        .gap(px(7.0))
        .text_size(crate::typography::ui_rems(11.0))
        .line_height(px(16.0))
        .text_color(theme.danger)
        .child(
            crate::icons::icon(crate::icons::DANGER_TRIANGLE)
                .size(px(13.0))
                .flex_none()
                .mt(px(1.0)),
        )
}

pub(crate) fn settings_page_subtitle(theme: &Theme, copy: impl Into<SharedString>) -> gpui::Div {
    div()
        .mt(px(4.0))
        .text_size(crate::typography::ui_rems(
            theme.ui_style.type_roles().body.unwrap_or(13.0) + super::layout_type_adjustment(theme),
        ))
        .text_color(theme.text_muted)
        .child(copy.into())
}
