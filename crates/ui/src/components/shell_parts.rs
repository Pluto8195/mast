//! Components used by shell chrome.

use crate::{icons::icon, motion, theme::Theme};
use gpui::{
    AnyElement, App, Div, ElementId, IntoElement, MouseButton, SharedString, Window,
    WindowControlArea, div, prelude::*, px,
};

pub(crate) fn sidebar_separator(theme: &Theme) -> gpui::Div {
    div()
        .h(px(if theme.ui_style.design_roles.sidebar_border.unwrap_or_else(|| !super::is_horizon(theme)) {
            theme.ui_style.tokens.rule_width.unwrap_or(1.0)
        } else { 0.0 }))
        .bg(if theme.ui_style.tokens.rule_width.is_some() {
            theme.border
        } else {
            theme.border.opacity(0.6)
        })
}

pub(crate) fn settings_sidebar_heading(theme: &Theme) -> gpui::Div {
    div()
        .px(px(Theme::SPACE_SM))
        .pt(px(12.0))
        .pb(px(4.0))
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text_muted.opacity(0.6))
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child("Settings")
}

pub(crate) fn settings_sidebar_row(
    theme: &Theme,
    id: SharedString,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    let row = div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .rounded(px(8.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .px(px(Theme::SPACE_SM))
        .py(px(6.0))
        .text_size(crate::typography::ui_rems(13.0))
        .when(selected, |el| {
            el.bg(crate::theme::glass_selected_bg())
                .font_weight(gpui::FontWeight::MEDIUM)
        })
        .text_color(if selected {
            theme.text
        } else {
            theme.text_muted
        })
        .cursor_pointer()
        .hover(|el| el.bg(theme.glass_hover()).text_color(theme.text));
    super::segmented_key(row, theme, selected)
}

pub(crate) fn settings_sidebar_back(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let row = div()
        .id("settings-back")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .rounded(px(8.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .px(px(Theme::SPACE_SM))
        .py(px(6.0))
        .text_size(crate::typography::ui_rems(13.0))
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(|el| el.bg(theme.glass_hover()).text_color(theme.text));
    super::segmented_key(row, theme, false)
}

pub(crate) fn update_strip(
    theme: &Theme,
    label: SharedString,
    failed: bool,
    clickable: bool,
) -> gpui::Stateful<gpui::Div> {
    let tone = if failed { theme.danger } else { theme.accent };
    let (background, hover) = if failed {
        (theme.danger.opacity(0.14), theme.danger.opacity(0.22))
    } else {
        (theme.accent_wash, theme.accent.opacity(0.16))
    };
    let strip = div()
        .id("update-strip")
        .mx(px(Theme::SPACE_SM))
        .px(px(Theme::SPACE_SM))
        .py(px(6.0))
        .rounded(px(Theme::CONTROL_RADIUS))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .bg(background)
        .flex()
        .flex_row()
        .items_center()
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(tone)
        .child(div().flex_1().min_w_0().child(label))
        .when(clickable, |el| {
            el.cursor_pointer().hover(move |el| el.bg(hover))
        });
    super::segmented_key(strip, theme, false)
}

pub(crate) fn offline_indicator(theme: &Theme) -> AnyElement {
    div()
        .size(px(5.0))
        .rounded_full()
        .bg(theme.warning)
        .into_any_element()
}

pub(crate) fn connection_line(theme: &Theme, label: SharedString, glyph: AnyElement) -> AnyElement {
    crate::motion::fade_in(
        "connection-pill",
        div()
            .id("connection-pill")
            .mx(px(Theme::SPACE_SM + 4.0))
            .mb(px(Theme::SPACE_SM))
            .flex()
            .items_center()
            .gap(px(6.0))
            .when(super::is_horizon(theme), |el| {
                el.rounded_full().bg(theme.identity_color("").opacity(0.08))
            })
            .when(super::is_industrial(theme), |el| {
                el.font_family(theme.font_mono.clone())
                    .border_1()
                    .border_color(theme.industrial_colors().rule)
            })
            .child(glyph)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_faint)
                    .child(label),
            ),
    )
    .into_any_element()
}

pub(crate) fn sidebar_section_label(theme: &Theme, label: SharedString) -> gpui::Div {
    let emphasized = super::is_industrial(theme) || super::is_horizon(theme);
    let label = if theme.ui_style.design_roles.tracked_caps.unwrap_or(emphasized) {
        SharedString::from(crate::popover::tracked_upper(&label))
    } else {
        label
    };
    div()
        .text_size(crate::typography::ui_rems(
            theme.ui_style.tokens.label_size.unwrap_or(12.0),
        ))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(if emphasized {
            theme.text_muted
        } else {
            theme.text_muted.opacity(0.5)
        })
        .child(label)
}

pub(crate) fn chat_header_chip(
    id: SharedString,
    selector: String,
    theme: &Theme,
    border: gpui::Hsla,
    text: gpui::Hsla,
    model: bool,
) -> gpui::Stateful<gpui::Div> {
    let chip = div()
        .id(id)
        .debug_selector(move || selector.clone())
        .max_w(px(230.0))
        .h(px(20.0))
        .when(super::is_horizon(theme), |el| {
            el.h(px(theme.ui_style.tokens.control_size))
        })
        .px(px(7.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(border)
        .bg(theme.element_hover.opacity(0.55))
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque,
            |el| el.bg(theme.surface_raised),
        )
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(text)
        .when(super::is_horizon(theme), |el| {
            el.bg(theme.bg)
                .border_color(theme.bg)
                .text_color(theme.text_muted)
        })
        .when(model && super::is_industrial(theme), |chip| {
            let dark = theme.appearance.is_dark();
            chip.rounded(px(0.0))
                .bg(if dark { theme.bg } else { theme.solid })
                .border_color(theme.industrial_colors().rule)
                .font_family(theme.font_mono.clone())
                .text_color(if dark { theme.text } else { theme.on_solid })
        })
        .when_some(theme.ui_style.design_roles.chip_radius, |el, radius| el.rounded(px(radius)))
        .when_some(theme.ui_style.design_roles.secondary_colour, |el, role| { let colour = role.colour(theme); el.border_color(colour).text_color(colour) });
    super::design_surface(chip, theme, crate::style_overrides::DesignSurface::HeaderChip)
}

pub(crate) fn chat_header_chip_icon_color(
    theme: &Theme,
    text: gpui::Hsla,
    model: bool,
    link: bool,
) -> gpui::Hsla {
    if super::is_horizon(theme) {
        theme.text_muted
    } else if model && super::is_industrial(theme) {
        if theme.appearance.is_dark() {
            theme.text
        } else {
            theme.on_solid
        }
    } else if link {
        theme.accent
    } else {
        text
    }
}

pub(crate) fn right_surface_tab(
    index: usize,
    group: SharedString,
    width: f32,
    accessible_label: String,
    active: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("right-surface-tab", index))
        .debug_selector(move || format!("right-surface-tab-{index}"))
        .group(group)
        .h(px(24.0))
        .when(super::is_horizon(theme), |el| {
            el.h(px(theme.ui_style.tokens.control_size))
        })
        .w(px(width))
        .flex_none()
        .pl(px(4.0))
        .pr(px(8.0))
        .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(6.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(3.0))
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label(accessible_label)
        .when(active, |el| el.bg(crate::theme::wash(0.10)))
        .when(!active, |el| el.hover(|s| s.bg(crate::theme::wash(0.06))))
        .when(true, |el| super::segmented_key(el, theme, active))
        .when(super::is_horizon(theme), |el| el.bg(theme.bg))
        .children(crate::surface_chrome::active_key_dot(theme, active))
}

pub(crate) fn right_panel_frame(theme: &Theme, expanded: bool) -> gpui::Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .when(!expanded && !super::is_horizon(theme), |el| {
            el.border_l_1().border_color(theme.border)
        })
        .bg(if super::is_horizon(theme) {
            theme.bg
        } else {
            theme.panel_bg()
        })
}

pub(crate) fn surface_close_slot(theme: &Theme, index: usize) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("right-surface-close", index))
        .debug_selector(move || format!("right-surface-close-{index}"))
        .flex_none()
        .size(px(18.0))
        .rounded(px(4.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .relative()
        .hover(|el| el.bg(crate::theme::wash(0.12)))
}

pub(crate) fn surface_tab_title(theme: &Theme, title: SharedString, active: bool) -> gpui::Div {
    div()
        .min_w_0()
        .truncate()
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(if active { theme.text } else { theme.text_muted })
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child(title)
}

pub(crate) fn surface_dirty_indicator(theme: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .size(px(6.0))
        .rounded_full()
        .bg(theme.text_muted)
}

pub(crate) fn surface_add_button(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let fade = "right-surface-add-fade";
    let button = div()
        .id("right-surface-add")
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .cursor_pointer()
        .bg(motion::hover_blend(
            fade,
            crate::theme::wash(0.0),
            crate::theme::wash(0.11),
        ))
        .on_hover(motion::hover_listener(fade))
        .block_mouse_except_scroll();
    super::segmented_key(button, theme, false)
}

pub(crate) fn activity_indicator(
    status: crate::status_style::Status,
    key: String,
    theme: &Theme,
    view: gpui::EntityId,
    cx: &mut App,
) -> AnyElement {
    if theme.ui_style.status_indicator != crate::ui_style::StatusIndicator::Dot
        && status == crate::status_style::Status::Working
    {
        let color = crate::status_style::status_style(status, theme.appearance).color;
        crate::loaders::mini_mono_spinner(key, 1.5, color, view, cx).into_any_element()
    } else {
        crate::status_style::indicator_for_theme(status, theme).into_any_element()
    }
}

pub(crate) fn chat_header_frame(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id("full-chat-header-metadata-wrap")
        .debug_selector(|| "chat-header".into())
        .relative()
        .flex_none()
        .mt(px(Theme::TITLEBAR_HEIGHT))
        .px(px(12.0))
        .py(px(4.0))
        .border_b_1()
        .border_color(theme.border.opacity(0.7))
        .bg(theme.surface.opacity(0.94))
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque,
            |el| el.bg(theme.surface).border_color(theme.border).px(px(16.0)),
        )
        .when(super::is_horizon(theme), |el| {
            el.bg(theme.bg).border_color(theme.bg)
        })
}

pub(crate) fn chat_header_title(theme: &Theme, title: String) -> gpui::Div {
    div()
        .debug_selector(|| "chat-header-title".into())
        .min_w_0()
        .truncate()
        .text_size(crate::typography::ui_rems(
            theme.ui_style.tokens.title_size.unwrap_or(15.0),
        ))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.text)
        .child(SharedString::from(title))
}

pub(crate) fn breadcrumb_frame(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id("chat-header-breadcrumb")
        .debug_selector(|| "chat-header-breadcrumb".into())
        .flex_none()
        .max_w(px(280.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted)
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
}

pub(crate) fn breadcrumb_segment(
    theme: &Theme,
    index: usize,
    title: String,
    last: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("chat-header-crumb", index))
        .debug_selector(move || format!("chat-header-crumb-{index}"))
        .min_w_0()
        .truncate()
        .rounded(px(3.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .px(px(2.0))
        .cursor_pointer()
        .when(last, |el| {
            el.font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text)
        })
        .hover(|el| el.bg(theme.element_hover).text_color(theme.text))
        .child(SharedString::from(title))
}

pub(crate) fn header_attention_badge(
    theme: &Theme,
    id: &'static str,
    glyph: &'static str,
    count: usize,
    color: gpui::Hsla,
) -> gpui::Stateful<gpui::Div> {
    let badge = div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_none()
        .h(px(18.0))
        .when(super::is_horizon(theme), |el| {
            el.h(px(theme.ui_style.tokens.control_size))
        })
        .px(px(5.0))
        .flex()
        .items_center()
        .gap(px(3.0))
        .rounded(px(theme.ui_style.tokens.chip_radius.unwrap_or(5.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(color)
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(color)
        .when(super::is_horizon(theme), |el| {
            el.border_color(theme.bg).text_color(theme.text_muted)
        })
        .hover(|el| el.bg(theme.element_hover))
        .child(glyph)
        .when(count > 0, |el| {
            el.child(SharedString::from(count.to_string()))
        });
    super::segmented_key(badge, theme, false)
}

pub(crate) fn admin_badge(
    theme: &Theme,
    selector: String,
    text: String,
    border: gpui::Hsla,
    color: gpui::Hsla,
) -> gpui::Stateful<gpui::Div> {
    let badge = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex_none()
        .max_w_full()
        .h(px(20.0))
        .pl(px(6.0))
        .pr(px(2.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded(px(5.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(border)
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(color)
        .child(icon(crate::icons::SHIELD).size(px(12.0)).text_color(color))
        .child(div().min_w_0().truncate().child(SharedString::from(text)));
    super::segmented_key(badge, theme, false)
}

pub(crate) fn admin_revoke_button(
    theme: &Theme,
    selector: String,
    label: String,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex_none()
        .px(px(5.0))
        .rounded(px(3.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .cursor_pointer()
        .hover(|el| el.bg(theme.element_hover))
        .role(gpui::Role::Button)
        .aria_label(SharedString::from(format!("{label}: revoke")))
        .child(crate::admin::REVOKE_LABEL)
}

pub(crate) fn chat_header_menu(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let menu = div()
        .id("chat-header-menu")
        .debug_selector(|| "chat-header-menu".into())
        .cursor_pointer()
        .when(super::is_horizon(theme), |el| {
            el.size(px(theme.ui_style.tokens.control_size))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
        })
        .child(
            icon(crate::icons::MORE_HORIZONTAL)
                .size(px(16.0))
                .text_color(theme.text_muted),
        );
    super::segmented_key(menu, theme, false)
}

pub(crate) fn review_chip(theme: &Theme, selector: String) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .h(px(20.0))
        .px(px(7.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded(px(theme.ui_style.tokens.chip_radius.unwrap_or(5.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(theme.border)
        .bg(theme.element_hover.opacity(0.55))
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque,
            |el| el.bg(theme.surface_raised),
        )
        .cursor_pointer()
        .hover(|el| el.bg(theme.element_hover))
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.accent)
}

pub(crate) fn sidebar_frame(
    theme: &Theme,
    width: f32,
    inner: AnyElement,
    on_new_conversation: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .h_full()
        .flex_none()
        .overflow_hidden()
        .w(px(width))
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque,
            |el| {
                el.bg(if super::is_horizon(theme) {
                    theme.bg
                } else {
                    theme.surface
                })
            },
        )
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque
                && !super::is_horizon(theme),
            |el| el.border_r_1().border_color(theme.border),
        )
        .child(if super::is_horizon(theme) {
            div()
                .h_full()
                .pt(px(Theme::TITLEBAR_HEIGHT))
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(px(16.0))
                        .pb(px(12.0))
                        .font_family("EB Garamond")
                        .text_size(crate::typography::ui_rems(26.0))
                        .text_color(theme.text_muted)
                        .child("Mast"),
                )
                .child(
                    div()
                        .id("horizon-new-session")
                        .mx(px(12.0))
                        .mb(px(12.0))
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded_full()
                        .border_1()
                        .border_color(theme.border_strong)
                        .text_color(theme.text_muted)
                        .cursor_pointer()
                        .child("+  New conversation")
                        .on_click(on_new_conversation),
                )
                .child(div().flex_1().min_h_0().overflow_hidden().child(inner))
        } else {
            div().h_full().pt(px(Theme::TITLEBAR_HEIGHT)).child(inner)
        })
        .into_any_element()
}

pub(crate) fn item_hover_card(theme: &Theme, width: f32) -> gpui::Stateful<gpui::Div> {
    div()
        .id("chat-item-card")
        .debug_selector(|| "chat-item-card".into())
        .absolute()
        .top_full()
        .left(px(12.0))
        .w(px(width))
        .max_w_full()
        .mt(px(4.0))
        .p(px(10.0))
        .rounded(px(theme.ui_style.tokens.panel_radius.unwrap_or(8.0)))
        .border_1()
        .border_color(theme.border_strong)
        .bg(theme.surface_raised)
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::ThemeDefault,
            |el| el.shadow_md(),
        )
        .occlude()
        .flex()
        .flex_col()
        .gap(px(8.0))
}

pub(crate) fn item_card_footer(theme: &Theme, pinned: bool) -> gpui::Div {
    div()
        .debug_selector(|| "chat-item-card-footer".into())
        .text_size(crate::typography::ui_rems(10.0))
        .text_color(theme.text_faint)
        .child(if pinned {
            "⌥⌘I unpins · pinned open"
        } else {
            "⌥⌘I pins this card open"
        })
}

pub(crate) fn item_needs_card(theme: &Theme, count: usize) -> gpui::Div {
    div()
        .debug_selector(|| "chat-item-needs-you".into())
        .flex()
        .flex_col()
        .gap(px(4.0))
        .p(px(8.0))
        .rounded(px(6.0))
        .when(super::is_horizon(theme), |el| el.rounded(px(12.0)))
        .border_1()
        .border_color(if count > 0 {
            theme.warning.opacity(0.6)
        } else {
            theme.border
        })
        .when(super::is_horizon(theme), |el| el.border_color(theme.bg))
        .when(super::is_industrial(theme), |el| {
            el.rounded(px(0.0)).bg(theme.industrial_colors().panel)
        })
}

pub(crate) fn item_needs_count(theme: &Theme, count: usize) -> gpui::Div {
    div()
        .debug_selector(|| "chat-item-needs-you-count".into())
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if super::is_horizon(theme) {
            theme.text_muted
        } else {
            theme.warning
        })
        .child(SharedString::from(count.to_string()))
}

pub(crate) fn item_account_badge(theme: &Theme, account: String) -> gpui::Div {
    let badge = div()
        .debug_selector(|| "chat-item-account-badge".into())
        .flex_none()
        .h(px(20.0))
        .px(px(7.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded(px(5.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(theme.border_strong)
        .bg(theme.element_hover.opacity(0.55))
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text)
        .child(
            icon(crate::icons::KEY_MINIMALISTIC)
                .size(px(11.0))
                .text_color(theme.text),
        )
        .child(SharedString::from(account));
    super::segmented_key(badge, theme, false)
}

pub(crate) fn instruction_entry(theme: &Theme, entry: &str) -> gpui::Div {
    div()
        .line_clamp(2)
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text)
        .child(SharedString::from(format!("• {entry}")))
}

pub(crate) fn main_panel(theme: &Theme, main: AnyElement) -> AnyElement {
    let main = if super::is_industrial(theme) {
        div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .border_1()
            .border_color(theme.industrial_colors().rule)
            .bg(theme.bg)
            .child(main)
            .into_any_element()
    } else {
        main
    };
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_row()
        .overflow_hidden()
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque,
            |el| el.bg(theme.bg),
        )
        .when(super::is_industrial(theme), |el| {
            el.p(px(8.0)).rounded(px(12.0)).bg(theme.surface)
        })
        .child(main)
        .into_any_element()
}

pub(crate) fn sidebar_tone(theme: &Theme, width: f32, corner: f32) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .w(px(width))
        .when(corner > 0.0, |el| {
            if width >= 2.0 * corner {
                el.rounded_tl(px(corner)).rounded_bl(px(corner))
            } else {
                el.top(px(corner)).bottom(px(corner))
            }
        })
        .bg(crate::theme::wash(0.05))
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque,
            |el| el.bg(theme.surface),
        )
        .border_r_1()
        .border_color(if super::is_horizon(theme) {
            theme.bg
        } else {
            theme.border
        })
        .when(super::is_horizon(theme), |el| el.bg(theme.bg))
}

pub(crate) fn titlebar_overlay(theme: &Theme, title_bar: AnyElement) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque,
            |el| {
                el.bg(if super::is_horizon(theme) {
                    theme.bg
                } else {
                    theme.surface
                })
            },
        )
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque
                && !super::is_horizon(theme),
            |el| el.border_b_1().border_color(theme.border),
        )
        .child(title_bar)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn horizon_sidebar_chat_row(
    theme: &Theme,
    row_id: String,
    chat_id: &str,
    identity: gpui::Hsla,
    title: SharedString,
    status: crate::status_style::Status,
    status_label: &'static str,
    selected: bool,
    searching: bool,
    account: Option<String>,
    change_request: Option<mast_proto::ChangeRequestSummary>,
    jump_label: Option<SharedString>,
    preview: bool,
    hovered: bool,
    archived: bool,
    on_archive: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> Option<gpui::Stateful<gpui::Div>> {
    if !super::is_horizon(theme) || searching {
        return None;
    }
    let foreground = if theme.ui_style.design_roles.surface(crate::style_overrides::DesignSurface::SidebarRow).page_colour.is_some() {
        theme.text
    } else if selected {
        theme.identity_text(theme.ui_style.design_roles.primary_colour.map(|role| role.colour(theme)).unwrap_or(identity))
    } else {
        theme.text
    };
    let selector = format!("chat-{chat_id}");
    let account_selector = format!("chat-account-{chat_id}");
    let status_selector = format!("chat-status-{chat_id}");
    let badge = change_request.map(|summary| {
        let badge_id = SharedString::from(format!("{row_id}-compact-pr"));
        if preview {
            crate::change_requests::pull_request_badge_preview(
                badge_id,
                summary,
                crate::change_requests::ChangeRequestBadgeSurface::Sidebar,
                theme,
            )
        } else {
            crate::change_requests::pull_request_badge(
                badge_id,
                summary,
                crate::change_requests::ChangeRequestBadgeSurface::Sidebar,
                theme,
            )
        }
    });
    let corner = if hovered && !preview {
        div()
            .id(SharedString::from(format!("{row_id}-corner")))
            .debug_selector({
                let selector = format!("{row_id}-corner");
                move || selector.clone()
            })
            .aria_label(if archived { "Unarchive" } else { "Archive" })
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |event, window, cx| {
                cx.stop_propagation();
                on_archive(event, window, cx);
            })
            .child(
                icon(if archived {
                    crate::icons::ARCHIVE_UP_MINIMALISTIC
                } else {
                    crate::icons::ARCHIVE_MINIMALISTIC
                })
                .size(px(14.0))
                .text_color(foreground),
            )
            .into_any_element()
    } else {
        div()
            .debug_selector(move || status_selector.clone())
            .flex()
            .items_center()
            .gap(px(4.0))
            .text_color(if selected {
                foreground
            } else {
                theme.text_muted
            })
            .when(theme.ui_style.behavior.status_words, |el| {
                el.child(
                    div()
                        .text_size(crate::typography::ui_rems(10.0))
                        .child(status_label),
                )
            })
            .into_any_element()
    };
    Some(super::design_surface(
        div()
            .id(SharedString::from(row_id))
            .aria_label(SharedString::from(format!("{title} · {status_label}")))
            .debug_selector(move || selector.clone())
            .h(px(theme.ui_style.tokens.sidebar_row_compact_h))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded_full()
            .text_color(foreground)
            .when(archived && !selected && !hovered, |el| el.opacity(0.4))
            .when(selected, |el| el.bg(identity))
            .when_some(theme.ui_style.design_roles.control_radius, |el, radius| el.rounded(px(radius)))
            .when_some(theme.ui_style.design_roles.primary_colour.filter(|_| selected), |el, role| { let colour = role.colour(theme); el.bg(colour).text_color(theme.identity_text(colour)) })
            .when(!selected, |el| {
                el.hover(|style| style.bg(theme.element_hover))
            })
            .child(
                div()
                    .flex_none()
                    .size(px(8.0))
                    .rounded_full()
                    .bg(crate::status_style::dot_color(status, theme)),
            )
            .child(div().flex_1().min_w_0().truncate().child(title))
            .children(account.map(|label| {
                crate::managed_accounts::account_pill(theme, label)
                    .debug_selector(move || account_selector.clone())
            }))
            .children(badge)
            .children(jump_label.map(|label| super::badge(theme, label)))
            .child(corner),
        theme, crate::style_overrides::DesignSurface::SidebarRow,
    ))
}

pub(crate) fn awaiting_input_label(theme: &Theme) -> &'static str {
    if theme.ui_style.layout == crate::ui_style::UiLayout::Guided {
        "Needs you"
    } else {
        "Input"
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn sidebar_status_glyph(
    theme: &Theme,
    status: crate::status_style::Status,
    working: bool,
    completed: bool,
    key: String,
    color: gpui::Hsla,
    view: gpui::EntityId,
    cx: &mut App,
) -> AnyElement {
    if theme.ui_style.status_indicator == crate::ui_style::StatusIndicator::Dot {
        crate::status_style::indicator_for_theme(
            status,
            theme,
        )
        .into_any_element()
    } else if working {
        crate::loaders::mini_mono_spinner(key, 2.0, color, view, cx).into_any_element()
    } else if completed {
        icon(crate::icons::CHECK)
            .size(px(11.0))
            .text_color(color)
            .into_any_element()
    } else {
        crate::status_style::indicator_for_theme(status, theme).into_any_element()
    }
}

pub(crate) fn compact_sidebar_status(
    theme: &Theme,
    id: String,
    selector: String,
    label: &'static str,
    color: gpui::Hsla,
    glyph: AnyElement,
) -> AnyElement {
    let guided = theme.ui_style.layout == crate::ui_style::UiLayout::Guided;
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .size(px(13.0))
        .when(guided, |el| el.w_auto().gap(px(4.0)))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .aria_label(label)
        .child(glyph)
        .when(guided, |el| {
            el.child(
                div()
                    .text_size(crate::typography::ui_rems(10.0))
                    .text_color(color)
                    .child(label),
            )
        })
        .into_any_element()
}

pub(crate) fn sidebar_idle_time(theme: &Theme, time: SharedString) -> AnyElement {
    div()
        .text_size(crate::typography::ui_rems(10.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .when(
            theme.ui_style.status_indicator == crate::ui_style::StatusIndicator::Dot,
            |el| {
                el.flex().items_center().gap(px(8.0)).child(
                    crate::status_style::indicator_for_theme(
                        crate::status_style::Status::Idle,
                        theme,
                    ),
                )
            },
        )
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child(time)
        .into_any_element()
}

pub(crate) fn sidebar_jump_badge(theme: &Theme, label: SharedString) -> AnyElement {
    let tone = theme.text_muted;
    div()
        .h(px(16.0))
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .px(px(4.0))
        .rounded(px(4.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .bg(tone.opacity(0.08))
        .text_size(crate::typography::ui_rems(10.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(tone.opacity(0.85))
        .font_family(theme.font_mono.clone())
        .child(label)
        .into_any_element()
}

pub(crate) fn sidebar_archive_glyph(
    theme: &Theme,
    compact: bool,
    archived: bool,
    compact_icon_size: f32,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .h(px(18.0))
        .when(!compact, |el| {
            el.px(px(4.0))
                .mr(px(-4.0))
                .rounded(px(5.0))
                .bg(crate::theme::wash(0.10))
                .hover(|el| el.bg(crate::theme::wash(0.18)))
        })
        .child(
            icon(if archived {
                crate::icons::ARCHIVE_UP_MINIMALISTIC
            } else {
                crate::icons::ARCHIVE_MINIMALISTIC
            })
            .size(px(if compact { compact_icon_size } else { 11.0 }))
            .flex_none()
            .text_color(theme.text_muted),
        )
        .when(!compact, |el| {
            el.child(
                div()
                    .text_size(crate::typography::ui_rems(10.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(if archived {
                        "Unarchive"
                    } else {
                        "Archive"
                    })),
            )
        })
        .into_any_element()
}

pub(crate) fn sidebar_status_word(
    label: &'static str,
    color: gpui::Hsla,
    glyph: AnyElement,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .child(glyph)
        .child(
            div()
                .text_size(crate::typography::ui_rems(10.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(color)
                .child(SharedString::from(label)),
        )
        .into_any_element()
}

pub(crate) fn sidebar_compact_time(
    theme: &Theme,
    id: String,
    color: gpui::Hsla,
    time: SharedString,
) -> gpui::Div {
    div()
        .debug_selector(move || format!("chat-time-{id}"))
        .w(px(30.0))
        .flex_none()
        .text_right()
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(color)
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child(time)
}

pub(crate) fn sidebar_row_colors(
    theme: &Theme,
    identity: gpui::Hsla,
    selected: bool,
) -> (gpui::Hsla, gpui::Hsla, Option<gpui::Div>) {
    let horizon = super::is_horizon(theme);
    let text = if theme.ui_style.design_roles.surface(crate::style_overrides::DesignSurface::SidebarRow).page_colour.is_some() {
        theme.text
    } else if horizon && selected {
        theme.identity_text(theme.ui_style.design_roles.primary_colour.map(|role| role.colour(theme)).unwrap_or(identity))
    } else {
        theme.text
    };
    let selected_wash = if selected && theme.ui_style.design_roles.primary_colour.is_some() {
        theme.ui_style.design_roles.primary_colour.unwrap().colour(theme)
    } else if horizon {
        identity
    } else if theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque {
        theme.element_active
    } else {
        crate::theme::glass_selected_bg()
    };
    let dot = horizon.then(|| {
        div()
            .flex_none()
            .size(px(8.0))
            .rounded_full()
            .bg(if selected {
                theme.identity_text(theme.ui_style.design_roles.primary_colour.map(|role| role.colour(theme)).unwrap_or(identity))
            } else {
                identity
            })
    });
    (text, selected_wash, dot)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn sidebar_row_frame(
    theme: &Theme,
    row_id: String,
    height: f32,
    searching: bool,
    fade_key: &str,
    rest_text: gpui::Hsla,
    hover_text: gpui::Hsla,
    rest_bg: gpui::Hsla,
    hover_bg: gpui::Hsla,
) -> gpui::Stateful<gpui::Div> {
    let row = div()
        .id(SharedString::from(row_id.clone()))
        .group("sidebar-session-row")
        .debug_selector(move || row_id.clone())
        .h(px(height))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(
            if searching {
                crate::popover::PALETTE_ITEM_RADIUS
            } else {
                8.0
            },
        )))
        .px(px(Theme::SPACE_SM))
        .py(px(6.0))
        .text_color(motion::hover_blend(fade_key, rest_text, hover_text))
        .bg(motion::hover_blend(fade_key, rest_bg, hover_bg));
    super::design_surface(row, theme, crate::style_overrides::DesignSurface::SidebarRow)
}

pub(crate) fn sidebar_context_text(
    theme: &Theme,
    color: gpui::Hsla,
    content: AnyElement,
) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(
            theme.ui_style.tokens.secondary_size.unwrap_or(11.0),
        ))
        .line_height(px(14.0))
        .text_color(color)
        .child(content)
}

pub(crate) fn sidebar_title_text(theme: &Theme, content: AnyElement) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(
            theme.ui_style.tokens.body_size.unwrap_or(13.0),
        ))
        .line_height(px(17.0))
        .child(content)
}

pub(crate) fn sidebar_grille(theme: &Theme) -> Option<gpui::Div> {
    super::is_industrial(theme).then(|| {
        div()
            .px(px(12.0))
            .pb(px(8.0))
            .child(crate::surface_chrome::grille(theme))
    })
}

pub(crate) fn main_grille(theme: &Theme) -> Option<gpui::Div> {
    super::is_industrial(theme).then(|| {
        crate::surface_chrome::grille(theme)
            .absolute()
            .right(px(32.0))
            .top(px(110.0))
    })
}

pub(crate) fn sidebar_action_frame(
    id: &'static str,
    theme: &Theme,
    width: f32,
) -> gpui::Stateful<gpui::Div> {
    let guided = theme.ui_style.behavior.labels == crate::ui_style::LabelMode::All;
    div()
        .id(id)
        .size(px(29.0))
        .when(guided, |el| el.size(px(32.0)))
        .when(guided && width >= 400.0, |el| {
            el.w_auto().px(px(6.0)).gap(px(4.0))
        })
}

pub(crate) fn sidebar_action_label(
    theme: &Theme,
    width: f32,
    label: &'static str,
    color: Option<gpui::Hsla>,
) -> Option<gpui::Div> {
    (theme.ui_style.behavior.labels == crate::ui_style::LabelMode::All && width >= 400.0).then(
        || {
            div()
                .text_size(crate::typography::ui_rems(11.0))
                .when_some(color, |el, color| el.text_color(color))
                .child(label)
        },
    )
}

pub(crate) fn shortcuts_footer(theme: &Theme, keymap: &crate::settings::KeymapConfig) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text_faint)
        .child(crate::work_outline::hints::shortcuts_footer(keymap))
        .when(
            theme.ui_style.behavior.hints == crate::ui_style::HintMode::Inline,
            |el| {
                el.children(
                    crate::work_outline::hints::shortcut_combo(
                        keymap,
                        crate::settings::ShortcutId::ShowOutlineShortcuts,
                    )
                    .map(|combo| crate::work_outline::hints::kbd_chip(theme, &combo)),
                )
            },
        )
}

/// A size-6 icon button for the titlebar strip (zeron window-controls.tsx:
/// `grid size-6 place-items-center rounded-md text-muted-foreground`).
pub(crate) fn window_control_button(
    id: &'static str,
    icon_path: &'static str,
    theme: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let muted = theme.text_muted;
    let fade_key = format!("window-control-{id}");
    div()
        .id(id)
        .size(px(24.0))
        .when(super::is_horizon(theme), |el| {
            el.size(px(theme.ui_style.tokens.control_size))
        })
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(6.0)))
        .cursor_pointer()
        // zeron window-controls.tsx: `transition-colors` — the wash fades.
        .bg(motion::hover_blend(
            &fade_key,
            theme.glass_hover().opacity(0.0),
            theme.glass_hover(),
        ))
        .when(super::is_horizon(theme), |el| {
            el.rounded_full().bg(gpui::transparent_black())
        })
        .when(true, |el| super::segmented_key(el, theme, false))
        .when(true, |el| super::toolbar_control(el, theme, false))
        .on_hover(motion::hover_listener(fade_key))
        // Buttons in/over a titlebar drag strip must be EXCLUDED from the
        // strip's event surface entirely. `.occlude()` (gpui
        // `HitboxBehavior::BlockMouse`) makes the window hit-test STOP at the
        // button, so every `is_hovered`-guarded strip listener — the
        // mouse-down that arms the drag, the mouse-move that hands AppKit a
        // native drag session (`performWindowDragWithEvent:`, whose second
        // quick click zooms NATIVELY on macOS), and the `click_count == 2`
        // zoom handler — never fires with the pointer over a button. It also
        // removes the button's rect from the native Drag control-area
        // hit-test on Windows/Linux. The click-level stop_propagation is
        // zed's ButtonLike belt on top. Double-click on EMPTY strip space
        // still zooms — nothing occludes it there.
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            on_click(event, window, cx)
        })
        .child(icon(icon_path).size(px(16.0)).text_color(muted))
}

/// A titlebar history button (zeron window-controls.tsx): enabled it is a
/// normal window-control button; disabled it dims to 35% opacity and ignores
/// the pointer (`disabled:pointer-events-none disabled:opacity-35`).
pub(crate) fn nav_history_button(
    id: &'static str,
    icon_path: &'static str,
    enabled: bool,
    theme: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    if !enabled {
        return div()
            .size(px(24.0))
            .when(super::is_horizon(theme), |el| {
                el.size(px(theme.ui_style.tokens.control_size))
            })
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .when(true, |el| super::toolbar_control(el, theme, false))
            // Even disabled it reads as a control — occlude so double-clicks
            // on it don't fall through to the titlebar strip's zoom handler.
            .occlude()
            .child(
                icon(icon_path)
                    .size(px(16.0))
                    .text_color(theme.text_muted.opacity(0.35)),
            )
            .into_any_element();
    }
    window_control_button(id, icon_path, theme, on_click).into_any_element()
}

/// A size-7 icon button for the main-panel header (zeron __root.tsx:
/// `grid size-7 place-items-center rounded-md text-muted-foreground`).
pub(crate) fn header_icon_button(
    id: &'static str,
    icon_path: &'static str,
    label: &'static str,
    labels_fit: bool,
    theme: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let muted = theme.text_muted;
    let show_label =
        labels_fit && theme.ui_style.behavior.labels != crate::ui_style::LabelMode::IconOnly;
    let fade_key = format!("header-icon-{id}");
    let rest = crate::theme::wash(0.0);
    div()
        .id(id)
        .size(px(theme.ui_style.tokens.control_size))
        .when(show_label, |el| el.w_auto().px(px(6.0)).gap(px(4.0)))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(6.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .when(true, |el| super::segmented_key(el, theme, false))
        .when(true, |el| super::toolbar_control(el, theme, false))
        .cursor_pointer()
        // zeron __root.tsx header buttons: `transition-colors`.
        .when(
            !super::is_industrial(theme)
                && theme.ui_style.behavior.motion == crate::ui_style::Motion::Soft,
            |el| {
                el.bg(motion::hover_blend(
                    &fade_key,
                    rest,
                    crate::theme::wash(0.11),
                ))
                .on_hover(motion::hover_listener(fade_key))
            },
        )
        .when(
            !super::is_industrial(theme)
                && theme.ui_style.behavior.motion == crate::ui_style::Motion::Instant,
            |el| el.hover(|el| el.bg(crate::theme::wash(0.11))),
        )
        .when(super::is_industrial(theme), |el| {
            el.hover(|style| style.bg(theme.industrial_colors().pressed))
        })
        // Same occlusion + click-swallowing as [`window_control_button`]: this
        // button sits inside the chat header's titlebar drag region, so its
        // rect must be carved out of the strip's drag/double-click surface.
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            on_click(event, window, cx)
        })
        .child(
            icon(icon_path)
                .size(px(theme.ui_style.tokens.control_icon_size))
                .text_color(muted),
        )
        .when(show_label, |el| {
            el.child(
                div()
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(muted)
                    .child(label),
            )
        })
}

pub(crate) fn rail_tick(
    theme: &Theme,
    index: usize,
    slot: f32,
    active: bool,
    hovered: bool,
) -> gpui::Stateful<gpui::Div> {
    let color = if active || hovered {
        theme.text.opacity(0.8)
    } else {
        crate::theme::ink(0.16)
    };
    let width = if hovered { 20.0 } else { 12.0 };
    div()
        .id(("rail-tick", index))
        .relative()
        .h(px(slot))
        .w_full()
        .flex()
        .items_center()
        .cursor_pointer()
        .child(
            div()
                .h(px(2.0))
                .w(px(width))
                .rounded(px(1.0))
                .bg(color)
                .when(super::is_horizon(theme), |el| {
                    el.h(px(4.0)).rounded_full().bg(if active {
                        theme.identity_color("")
                    } else {
                        color
                    })
                })
                .when(super::is_industrial(theme), |el| {
                    el.rounded(px(0.0)).bg(if active {
                        theme.industrial_colors().signal
                    } else {
                        theme.industrial_colors().rule
                    })
                }),
        )
}

pub(crate) fn rail_preview(
    theme: &Theme,
    prompt: String,
    reply: Option<String>,
    bucket_len: usize,
) -> AnyElement {
    let theme = theme.for_popup();
    let card = crate::popover::popover_card(&theme)
        .w(px(280.0))
        .p(px(Theme::SPACE_SM))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text)
                .child(SharedString::from(prompt)),
        )
        .when_some(reply, |el, reply| {
            el.child(
                div()
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(reply)),
            )
        })
        .when(bucket_len > 1, |el| {
            el.child(
                div()
                    .text_size(crate::typography::ui_rems(10.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!("{bucket_len} prompts"))),
            )
        });
    crate::frost::frosted(12.0, crate::frost::MENU_BLUR, card).into_any_element()
}

const WINDOWS_CAPTION_BUTTON_WIDTH: f32 = 36.0;

pub(crate) fn panel_tab(
    theme: &Theme,
    id: &'static str,
    accessible: &'static str,
    label: String,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .h(px(24.0))
        .when(super::is_horizon(theme), |el| {
            el.h(px(theme.ui_style.tokens.control_size))
        })
        .flex_none()
        .px(px(8.0))
        .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(6.0)))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .flex()
        .items_center()
        .gap(px(5.0))
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label(accessible)
        .block_mouse_except_scroll()
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .when(active, |el| el.bg(crate::theme::wash(0.10)))
        .when(!active, |el| el.hover(|s| s.bg(crate::theme::wash(0.06))))
        .when(true, |el| super::segmented_key(el, theme, active))
        .when(super::is_horizon(theme), |el| el.bg(theme.bg))
        .children(crate::surface_chrome::active_key_dot(theme, active))
        .child(
            div()
                .text_size(crate::typography::ui_rems(11.5))
                .text_color(if active && !super::is_horizon(theme) {
                    theme.text
                } else {
                    theme.text_muted
                })
                .child(SharedString::from(label)),
        )
}

pub(crate) fn panel_root(theme: &Theme, id: &'static str) -> gpui::Stateful<gpui::Div> {
    let padding = match theme.ui_style.layout {
        crate::ui_style::UiLayout::Minimal => 14.0,
        crate::ui_style::UiLayout::Compact => 10.0,
        crate::ui_style::UiLayout::Guided => 16.0,
    };
    div()
        .id(id)
        .debug_selector(move || id.into())
        .size_full()
        .overflow_y_scroll()
        .p(px(padding))
        .when(super::is_industrial(theme), |el| {
            el.bg(theme.industrial_colors().panel)
        })
}

pub(crate) fn titlebar_title(theme: &Theme, title: SharedString, on_canvas: bool) -> gpui::Div {
    div()
        .debug_selector(|| "titlebar-title".into())
        .min_w_0()
        .truncate()
        .text_size(crate::typography::ui_rems(12.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(if on_canvas {
            theme.text_muted.opacity(0.7)
        } else {
            theme.text.opacity(0.85)
        })
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .when(super::is_horizon(theme), |el| {
            el.font_weight(gpui::FontWeight::SEMIBOLD)
        })
        .child(title)
}

pub(crate) fn titlebar_route_label(theme: &Theme, label: &'static str) -> gpui::Div {
    div()
        .debug_selector(|| "titlebar-title".into())
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted)
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child(SharedString::from(label))
}

pub(crate) fn titlebar_target(theme: &Theme, target: SharedString) -> gpui::Div {
    div()
        .min_w_0()
        .flex_shrink(1.0)
        .truncate()
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted.opacity(0.5))
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child(target)
}

pub(crate) fn panel_chip(
    theme: &Theme,
    id: String,
    label: String,
    border: gpui::Hsla,
    text: gpui::Hsla,
) -> gpui::Stateful<gpui::Div> {
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .flex_none()
        .h(px(18.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .rounded(px(5.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(border)
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(text)
        .child(SharedString::from(label))
}

pub(crate) fn worker_row(theme: &Theme, selector: String) -> gpui::Stateful<gpui::Div> {
    let vertical_padding = match theme.ui_style.layout {
        crate::ui_style::UiLayout::Minimal => 5.0,
        crate::ui_style::UiLayout::Compact => 3.0,
        crate::ui_style::UiLayout::Guided => 7.0,
    };
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex()
        .flex_col()
        .gap(px(2.0))
        .px(px(6.0))
        .py(px(vertical_padding))
        .rounded(px(5.0))
        .border_1()
        .border_color(theme.border)
        .cursor_pointer()
        .hover(|el| el.bg(theme.element_hover.opacity(0.5)))
        .when(super::is_horizon(theme), |el| {
            el.rounded(px(12.0)).border_color(theme.bg)
        })
        .when(super::is_industrial(theme), |el| {
            el.rounded(px(0.0))
                .border_color(theme.industrial_colors().rule)
                .bg(theme.industrial_colors().panel)
        })
}

pub(crate) fn worker_title(theme: &Theme, title: String) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(crate::typography::ui_rems(12.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.text)
        .child(SharedString::from(title))
}

pub(crate) fn team_notice(theme: &Theme, selector: &'static str, text: &'static str) -> gpui::Div {
    div()
        .debug_selector(move || selector.into())
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text_faint)
        .child(text)
}

pub(crate) fn team_rollup(theme: &Theme) -> gpui::Div {
    div()
        .debug_selector(|| "team-rollup".into())
        .text_size(crate::typography::ui_rems(12.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.text)
}

pub(crate) fn item_team_rollup(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let row = div()
        .id("chat-item-team-rollup")
        .debug_selector(|| "chat-item-team-rollup".into())
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(12.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.text)
        .hover(|el| el.text_color(theme.accent))
        .flex()
        .items_center()
        .gap(px(4.0))
        .child("Team ·")
        .when(super::is_horizon(theme), |el| el.rounded_full());
    super::segmented_key(row, theme, false)
}

pub(crate) fn item_team_all(theme: &Theme, count: usize) -> gpui::Stateful<gpui::Div> {
    div()
        .id("chat-item-team-all")
        .debug_selector(|| "chat-item-team-all".into())
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.accent)
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .child(SharedString::from(format!("All {count} workers → Team")))
}

pub(crate) fn worker_metadata(theme: &Theme, label: String, color: gpui::Hsla) -> gpui::Div {
    div()
        .flex_none()
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(color)
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child(SharedString::from(label))
}

/// A small icon + label action at the worker row's metadata size (e.g. "Changes").
pub(crate) fn worker_action_link(theme: &Theme, icon: &'static str, label: &'static str) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(3.0))
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(theme.text_muted)
        .child(crate::icons::icon(icon).size(px(12.0)))
        .child(label)
}

pub(crate) fn worker_activity(
    theme: &Theme,
    selector: String,
    text: String,
    expanded: bool,
) -> gpui::Div {
    div()
        .debug_selector(move || selector.clone())
        .pl(px(13.0))
        .when(!expanded, |el| el.truncate())
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(if expanded {
            theme.text
        } else {
            theme.text_muted
        })
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child(SharedString::from(text))
}

pub(crate) fn worker_diff(theme: &Theme, selector: String, added: u32, removed: u32) -> gpui::Div {
    div()
        .debug_selector(move || selector.clone())
        .flex_none()
        .flex()
        .gap(px(4.0))
        .text_size(crate::typography::ui_rems(10.5))
        .child(
            div()
                .text_color(theme.success)
                .child(SharedString::from(format!("+{added}"))),
        )
        .child(
            div()
                .text_color(theme.danger)
                .child(SharedString::from(format!("−{removed}"))),
        )
}

pub(crate) fn worker_archive_hint(theme: &Theme, selector: String, reason: &str) -> gpui::Div {
    div()
        .debug_selector(move || selector.clone())
        .pl(px(13.0))
        .truncate()
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(theme.text_faint)
        .child(SharedString::from(format!("can archive · {reason}")))
}

pub(crate) fn worker_ready_label(text: String, color: gpui::Hsla) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(color)
        .child(SharedString::from(format!("→ {text}")))
}

pub(crate) fn worker_jump_button(
    theme: &Theme,
    selector: String,
    color: gpui::Hsla,
) -> gpui::Stateful<gpui::Div> {
    let button = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex_none()
        .px(px(6.0))
        .rounded(px(4.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(color.opacity(0.5))
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(color)
        .cursor_pointer()
        .hover(|el| el.bg(color.opacity(0.12)))
        .child("Jump");
    super::segmented_key(button, theme, false)
}

pub(crate) fn archive_confirmation(theme: &Theme, count: usize) -> gpui::Div {
    div()
        .debug_selector(|| "team-archive-confirm".into())
        .flex()
        .flex_col()
        .gap(px(3.0))
        .p(px(6.0))
        .rounded(px(5.0))
        .when(super::is_horizon(theme), |el| el.rounded(px(12.0)))
        .border_1()
        .border_color(theme.border)
        .when(super::is_industrial(theme), |el| {
            el.rounded(px(0.0)).bg(theme.industrial_colors().panel)
        })
        .child(
            div()
                .text_size(crate::typography::ui_rems(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(SharedString::from(format!(
                    "Archive {count} finished {}?",
                    if count == 1 { "worker" } else { "workers" }
                ))),
        )
}

pub(crate) fn archive_confirmation_worker(
    theme: &Theme,
    selector: String,
    title: &str,
    reason: &str,
) -> gpui::Div {
    div()
        .debug_selector(move || selector.clone())
        .truncate()
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text_muted)
        .child(SharedString::from(format!("{title} · {reason}")))
}

pub(crate) fn archive_confirmation_note(theme: &Theme) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(theme.text_faint)
        .child("They move to Show archived; Unarchive brings one back.")
}

pub(crate) fn archive_suggestion(theme: &Theme, count: usize) -> gpui::Div {
    div()
        .debug_selector(|| "team-archive-suggest".into())
        .flex_1()
        .min_w_0()
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text_muted)
        .child(SharedString::from(format!(
            "{count} finished workers can be archived"
        )))
}

pub(crate) fn archived_worker_title(theme: &Theme, title: String) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text_faint)
        .child(SharedString::from(title))
}

pub(crate) fn worker_report_toggle(
    theme: &Theme,
    id: String,
    selector: String,
    text: String,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .pl(px(13.0))
        .truncate()
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text_muted)
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child(SharedString::from(text))
}

pub(crate) fn worker_unseen_badge(theme: &Theme, selector: String) -> gpui::Stateful<gpui::Div> {
    let badge = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex_none()
        .flex()
        .items_center()
        .gap(px(3.0))
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(theme.warning)
        .child(div().size(px(5.0)).rounded_full().bg(theme.warning))
        .child("unseen")
        .when(super::is_horizon(theme), |el| {
            el.rounded_full().bg(theme.warning.opacity(0.12))
        });
    super::segmented_key(badge, theme, false)
}

pub(crate) fn archived_worker_toggle(
    theme: &Theme,
    shown: bool,
    count: usize,
) -> gpui::Stateful<gpui::Div> {
    let button = div()
        .id("team-show-archived")
        .debug_selector(|| "team-show-archived".into())
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text_muted)
        .hover(|el| el.text_color(theme.text))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .child(SharedString::from(format!(
            "{} archived ({count})",
            if shown { "Hide" } else { "Show" }
        )));
    super::segmented_key(button, theme, shown)
}

pub(crate) fn archived_worker_row(theme: &Theme, selector: String) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(6.0))
        .py(px(3.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .when(super::is_industrial(theme), |el| {
            el.border_1().border_color(theme.industrial_colors().rule)
        })
}

pub(crate) fn item_accordion_header(
    theme: &Theme,
    name: &'static str,
    label: &'static str,
    summary: String,
    warn: bool,
    open: bool,
    icon_path: &'static str,
) -> gpui::Stateful<gpui::Div> {
    let toggle = format!("chat-item-{name}-toggle");
    let height = match theme.ui_style.layout {
        crate::ui_style::UiLayout::Minimal => 22.0,
        crate::ui_style::UiLayout::Compact => 20.0,
        crate::ui_style::UiLayout::Guided => 26.0,
    };
    div()
        .id(SharedString::from(toggle.clone()))
        .debug_selector(move || toggle.clone())
        .h(px(height))
        .flex()
        .items_center()
        .gap(px(5.0))
        .cursor_pointer()
        .rounded(px(4.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .when(super::is_industrial(theme), |el| {
            el.border_1().border_color(theme.industrial_colors().rule)
        })
        .hover(|el| el.bg(theme.element_hover.opacity(0.5)))
        .child(
            icon(if open {
                crate::icons::ALT_ARROW_DOWN
            } else {
                crate::icons::ALT_ARROW_RIGHT
            })
            .size(px(10.0))
            .text_color(theme.text_muted),
        )
        .child(section_label(
            theme,
            icon_path,
            label,
            format!("chat-item-{name}-icon"),
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(if warn {
                    theme.warning
                } else {
                    theme.text_muted
                })
                .child(SharedString::from(summary)),
        )
}

pub(crate) fn panel_text(theme: &Theme, value: String, muted: bool) -> gpui::Div {
    div()
        .min_w_0()
        .text_size(crate::typography::ui_rems(if muted { 11.0 } else { 12.0 }))
        .text_color(if muted { theme.text_muted } else { theme.text })
        .child(SharedString::from(value))
}

pub(crate) fn panel_link(
    theme: &Theme,
    id: String,
    color: gpui::Hsla,
    underline: bool,
) -> gpui::Stateful<gpui::Div> {
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .flex_none()
        .px(px(4.0))
        .rounded(px(3.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(if super::is_horizon(theme) {
            theme.text_muted
        } else {
            color
        })
        .when(underline, |el| el.underline())
        .hover(|el| el.bg(theme.element_hover))
}

pub(crate) fn identifier_copy_button(
    theme: &Theme,
    id: SharedString,
    copied: bool,
) -> gpui::Stateful<gpui::Div> {
    let selector = id.to_string();
    let button = div()
        .id(id)
        .debug_selector(move || selector.clone())
        .flex_none()
        .px(px(4.0))
        .rounded(px(3.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .cursor_pointer()
        .text_color(theme.accent)
        .hover(|el| el.bg(theme.element_hover))
        .child(if copied { "Copied" } else { "Copy" });
    super::segmented_key(button, theme, false)
}

pub(crate) fn identifier_row(theme: &Theme) -> gpui::Div {
    let size = match theme.ui_style.layout {
        crate::ui_style::UiLayout::Minimal => 11.0,
        crate::ui_style::UiLayout::Compact => 10.0,
        crate::ui_style::UiLayout::Guided => 12.0,
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(crate::typography::ui_rems(size))
}

pub(crate) fn item_path_segment(
    theme: &Theme,
    index: usize,
    title: String,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("chat-item-path-segment", index))
        .debug_selector(move || format!("chat-item-path-segment-{index}"))
        .rounded(px(3.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .px(px(2.0))
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text)
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .hover(|el| el.bg(theme.element_hover))
        .child(SharedString::from(title))
}

pub(crate) fn panel_reference_row(theme: &Theme, id: String) -> gpui::Stateful<gpui::Div> {
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .w_full()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(4.0))
        .rounded(px(3.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .when(super::is_industrial(theme), |el| {
            el.border_1().border_color(theme.industrial_colors().rule)
        })
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text)
        .hover(|el| el.bg(theme.element_hover))
}

pub(crate) fn reference_label(theme: &Theme, label: SharedString, color: gpui::Hsla) -> gpui::Div {
    div()
        .flex_none()
        .text_size(crate::typography::ui_rems(10.0))
        .text_color(color)
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child(label)
}

pub(crate) fn reference_status_dot(color: gpui::Hsla) -> gpui::Div {
    div().flex_none().size(px(6.0)).rounded_full().bg(color)
}

pub(crate) fn wake_toggle(
    theme: &Theme,
    selector: String,
    label: String,
    accessible: String,
    border: gpui::Hsla,
    text: gpui::Hsla,
    active: bool,
    dimmed: bool,
) -> gpui::Stateful<gpui::Div> {
    let button = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex_none()
        .h(px(20.0))
        .px(px(7.0))
        .flex()
        .items_center()
        .rounded(px(5.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(border)
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(text)
        .when(dimmed, |el| el.opacity(0.5))
        .cursor_pointer()
        .hover(|el| el.bg(theme.element_hover.opacity(0.5)))
        .role(gpui::Role::Button)
        .aria_label(SharedString::from(accessible))
        .child(SharedString::from(label));
    super::segmented_key(button, theme, active)
}

pub(crate) fn held_wake_badge(
    theme: &Theme,
    selector: String,
    label: String,
    border: gpui::Hsla,
    text: gpui::Hsla,
) -> gpui::Div {
    let badge = div()
        .debug_selector(move || selector.clone())
        .flex_none()
        .h(px(18.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .rounded(px(5.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(border)
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(text)
        .child(SharedString::from(label));
    super::segmented_key(badge, theme, false)
}

pub(crate) fn held_wake_reason(theme: &Theme, selector: String, reason: &str) -> gpui::Div {
    div()
        .debug_selector(move || selector.clone())
        .min_w_0()
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(theme.text_muted)
        .child(SharedString::from(reason.to_owned()))
}

pub(crate) fn deliver_wake_button(theme: &Theme, selector: String) -> gpui::Stateful<gpui::Div> {
    let button = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex_none()
        .px(px(6.0))
        .rounded(px(4.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(theme.accent.opacity(0.5))
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(theme.accent)
        .cursor_pointer()
        .hover(|el| el.bg(theme.accent.opacity(0.12)))
        .child("Deliver now");
    super::segmented_key(button, theme, false)
}

pub(crate) fn spaces_filter(theme: &Theme, open: bool) -> gpui::Stateful<gpui::Div> {
    let frame = div()
        .id("spaces-filter")
        .flex_1()
        .min_w_0()
        .h(px(29.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(Theme::SPACE_SM))
        .rounded(px(8.0))
        .px(px(Theme::SPACE_SM))
        .text_size(crate::typography::ui_rems(13.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(motion::hover_blend(
            "spaces-filter",
            theme.text.opacity(0.8),
            theme.text,
        ))
        .bg(if open {
            theme.glass_hover()
        } else {
            motion::hover_blend(
                "spaces-filter",
                theme.glass_hover().opacity(0.0),
                theme.glass_hover(),
            )
        })
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .on_hover(motion::hover_listener("spaces-filter"))
        .cursor_pointer();
    super::segmented_key(frame, theme, open)
}

pub(crate) fn account_menu_trigger(
    theme: &Theme,
    user_line: &str,
    initial: SharedString,
    open: bool,
    icon_size: f32,
) -> gpui::Stateful<gpui::Div> {
    let avatar = div()
        .size(px(icon_size))
        .flex_none()
        .rounded_full()
        .bg(theme.text)
        .when(super::is_horizon(theme), |el| {
            el.bg(theme.identity_color(user_line))
        })
        .flex()
        .items_center()
        .justify_center()
        .font_family(theme.font_mono.clone())
        .text_size(px(9.0))
        .line_height(px(icon_size))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.bg)
        .when(super::is_horizon(theme), |el| {
            el.text_color(theme.identity_text(theme.identity_color(user_line)))
        })
        .child(div().w_full().text_center().child(initial));
    let trigger = div()
        .id("user-menu")
        .debug_selector(|| "user-menu".into())
        .role(gpui::Role::Button)
        .aria_label(format!("Account menu: {user_line}"))
        .relative()
        .size(px(icon_size + 8.0))
        .flex_none()
        .rounded_full()
        .p(px(4.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .bg(if open {
            theme.glass_hover()
        } else {
            motion::hover_blend(
                "user-menu-trigger",
                theme.glass_hover().opacity(0.0),
                theme.glass_hover().opacity(0.8),
            )
        })
        .on_hover(motion::hover_listener("user-menu-trigger"))
        .child(avatar);
    super::segmented_key(trigger, theme, false)
}

/// A Windows-owned caption target using the same system glyphs and native
/// non-client hit-test areas as GPUI/Zed's platform titlebar.
pub(crate) fn windows_caption_button(
    id: &'static str,
    glyph: &'static str,
    area: WindowControlArea,
    theme: &Theme,
    close: bool,
) -> impl IntoElement {
    let (hover_bg, hover_fg, active_bg, active_fg) = if close {
        let red: gpui::Hsla = gpui::rgb(0xe81123).into();
        (
            red,
            gpui::white(),
            red.opacity(0.8),
            gpui::white().opacity(0.8),
        )
    } else {
        (
            theme.glass_hover(),
            theme.text,
            theme.glass_hover().opacity(0.7),
            theme.text,
        )
    };
    div()
        .id(id)
        .w(px(WINDOWS_CAPTION_BUTTON_WIDTH))
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_size(crate::typography::ui_rems(10.0))
        .text_color(theme.text)
        .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
        .active(move |style| style.bg(active_bg).text_color(active_fg))
        .occlude()
        .window_control_area(area)
        .child(glyph)
}

/// A Linux caption button in zeron's own cluster style (24px, rounded-6,
/// 16px linear icon). gpui's `WindowControlArea` hit-testing is inert on
/// Linux, so unlike the Windows cluster these carry explicit click handlers
/// (`minimize_window` / `zoom_window` / `remove_window`), the same calls
/// zed's Linux titlebar makes. `occlude` + `prevent_default` keep them out
/// of the drag strip's event surface (see [`window_control_button`]).
pub(crate) fn linux_caption_button(
    id: &'static str,
    icon_path: &'static str,
    close: bool,
    theme: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (muted, hover_bg, hover_fg) = if close {
        let red: gpui::Hsla = gpui::rgb(0xe81123).into();
        (theme.text_muted, red, gpui::white())
    } else {
        (theme.text_muted, theme.glass_hover(), theme.text)
    };
    div()
        .id(id)
        // gpui svgs don't inherit the div's text color — recolor the glyph
        // on hover through the group instead (zed's WindowControl idiom).
        .group("linux-caption-button")
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .when(super::is_horizon(theme), |el| {
            el.rounded_full().bg(theme.identity_color("").opacity(0.12))
        })
        .when(true, |el| super::segmented_key(el, theme, false))
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            on_click(event, window, cx)
        })
        .child(
            icon(icon_path)
                .size(px(16.0))
                .text_color(muted)
                .group_hover("linux-caption-button", move |style| {
                    style.text_color(hover_fg)
                }),
        )
}

/// A section label: icon, then the name in small muted semibold caps.
pub(crate) fn section_label(
    theme: &Theme,
    icon_path: &'static str,
    label: &str,
    icon_selector: String,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .child(
            div()
                .debug_selector(move || icon_selector.clone())
                .flex_none()
                .child(
                    crate::icons::icon(icon_path)
                        .size(px(11.0))
                        .text_color(theme.text_muted),
                ),
        )
        .child(
            div()
                .text_size(crate::typography::ui_rems(10.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text_muted)
                .when(super::is_industrial(theme), |el| {
                    el.font_family(theme.font_mono.clone())
                })
                .child(SharedString::from(if super::is_horizon(theme) {
                    label.to_owned()
                } else {
                    label.to_uppercase()
                })),
        )
}

pub(crate) fn start_chat_section_label(text: &'static str, theme: &Theme) -> gpui::Div {
    div()
        .mt(px(10.0))
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.text_muted)
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child(text)
}

pub(crate) fn start_chat_note(
    selector: &'static str,
    text: impl Into<SharedString>,
    color: gpui::Hsla,
) -> gpui::Div {
    div()
        .debug_selector(move || selector.into())
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(color)
        .child(text.into())
}

pub(crate) fn start_chat_validation_issue(
    theme: &Theme,
    index: usize,
    message: String,
    remediation: String,
) -> gpui::Div {
    div()
        .debug_selector(move || format!("start-chat-issue-{index}"))
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(crate::typography::ui_rems(11.5))
                .text_color(theme.text)
                .child(SharedString::from(message)),
        )
        .child(
            div()
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(remediation)),
        )
}

pub(crate) fn start_chat_worktree_input(theme: &Theme) -> gpui::Div {
    div()
        .ml(px(22.0))
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(6.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(theme.border_strong)
        .bg(theme.input_bg)
        .when(super::is_industrial(theme), |el| {
            el.rounded(px(0.0))
                .border_color(theme.industrial_colors().rule)
        })
}

pub(crate) fn start_chat_prompt(theme: &Theme) -> gpui::Div {
    div()
        .relative()
        .debug_selector(|| "start-chat-prompt".into())
        .flex_none()
        .mt(px(8.0))
        .w_full()
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .when(super::is_horizon(theme), |el| el.rounded(px(12.0)))
        .border_1()
        .border_color(theme.border_strong)
        .bg(theme.input_bg)
        .when(super::is_industrial(theme), |el| {
            el.rounded(px(0.0))
                .border_color(theme.industrial_colors().rule)
        })
}

pub(crate) fn start_chat_instructions_toggle(
    theme: &Theme,
    open: bool,
    count: usize,
) -> gpui::Stateful<gpui::Div> {
    let toggle = div()
        .id("start-chat-instructions-toggle")
        .debug_selector(|| "start-chat-instructions-toggle".into())
        .mt(px(10.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.text_muted)
        .hover(|el| el.text_color(theme.text))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .child(
            icon(if open {
                crate::icons::ALT_ARROW_DOWN
            } else {
                crate::icons::ALT_ARROW_RIGHT
            })
            .size(px(10.0))
            .text_color(theme.text_muted),
        )
        .child(SharedString::from(format!(
            "Effective instructions ({count})"
        )));
    super::segmented_key(toggle, theme, open)
}

pub(crate) fn start_chat_instruction(
    theme: &Theme,
    order: u32,
    scope: &str,
    inherited: bool,
    text: &str,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(crate::typography::ui_rems(10.5))
                .text_color(theme.text_faint)
                .child(SharedString::from(format!(
                    "{}. {}{}",
                    order + 1,
                    scope,
                    if inherited { " (inherited)" } else { "" }
                ))),
        )
        .child(
            div()
                .text_size(crate::typography::ui_rems(11.5))
                .text_color(theme.text)
                .child(SharedString::from(text.to_owned())),
        )
}

pub(crate) fn start_chat_readout(theme: &Theme, selector: &'static str, text: String) -> gpui::Div {
    div()
        .debug_selector(move || selector.into())
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(theme.border_strong)
        .bg(theme.input_bg)
        .when(super::is_industrial(theme), |el| {
            el.rounded(px(0.0))
                .border_color(theme.industrial_colors().rule)
                .font_family(theme.font_mono.clone())
        })
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text)
        .child(SharedString::from(text))
}

pub(crate) fn start_chat_account_toggle(
    theme: &Theme,
    open: bool,
    label: String,
) -> gpui::Stateful<gpui::Div> {
    let toggle = div()
        .id("start-chat-account-chip")
        .debug_selector(|| "start-chat-account-chip".into())
        .flex()
        .items_center()
        .gap(px(4.0))
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted)
        .hover(|el| el.text_color(theme.text))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .child(
            icon(if open {
                crate::icons::ALT_ARROW_DOWN
            } else {
                crate::icons::ALT_ARROW_RIGHT
            })
            .size(px(10.0))
            .text_color(theme.text_muted),
        )
        .child(SharedString::from(label));
    super::segmented_key(toggle, theme, open)
}

pub(crate) fn team_button(
    theme: &Theme,
    id: impl Into<String>,
    label: String,
    primary: bool,
) -> gpui::Stateful<gpui::Div> {
    let color = if primary {
        theme.accent
    } else {
        theme.text_muted
    };
    let id: String = id.into();
    let selector = id.clone();
    let button = div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .flex_none()
        .px(px(7.0))
        .py(px(2.0))
        .rounded(px(4.0))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .border_1()
        .border_color(color.opacity(0.5))
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(color)
        .cursor_pointer()
        .hover(|el| el.bg(color.opacity(0.12)))
        .child(SharedString::from(label));
    super::segmented_key(button, theme, primary)
}

pub(crate) fn interrupted_label(label: &'static str, count: usize, theme: &Theme) -> Div {
    div()
        .px(px(8.0))
        .pt(px(4.0))
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.text_faint)
        .child(SharedString::from(format!("{label} \u{b7} {count}")))
}

pub(crate) fn running_button(
    id: impl Into<ElementId>,
    label: &'static str,
    primary: bool,
    theme: &Theme,
) -> gpui::Stateful<Div> {
    let button = div()
        .id(id)
        .px(px(10.0))
        .py(px(4.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .border_1()
        .text_size(crate::typography::ui_rems(12.0));
    let button = if primary {
        button
            .border_color(theme.accent)
            .bg(theme.accent)
            .text_color(theme.on_accent)
    } else {
        button
            .border_color(theme.border_strong)
            .hover(|el| el.bg(theme.glass_hover()))
    }
    .when(super::is_horizon(theme), |el| el.rounded_full())
    .child(SharedString::from(label));
    super::segmented_key(button, theme, primary)
}

/// Persistent copy-identity button beside the titlebar's chat name. The text
/// label and quiet border make the action discoverable without hover; it is
/// kept outside the title's overflow clipping above. Like
/// [`header_icon_button`], it is occluded out of the drag strip and swallows
/// the click. The label flips to `Copied` with the icon while `copied`.
pub(crate) fn chat_identity_button(
    copied: bool,
    label: &'static str,
    theme: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let fade_key = "chat-identity-copy".to_string();
    div()
        .id("chat-identity-copy")
        .h(px(22.0))
        .when(super::is_horizon(theme), |el| {
            el.h(px(theme.ui_style.tokens.control_size))
        })
        .px(px(6.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(4.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(theme.border)
        .cursor_pointer()
        .bg(motion::hover_blend(
            &fade_key,
            crate::theme::wash(0.05),
            crate::theme::wash(0.13),
        ))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .when(true, |el| super::segmented_key(el, theme, false))
        .when(true, |el| super::toolbar_control(el, theme, false))
        .when(super::is_horizon(theme), |el| {
            el.bg(theme.bg).border_color(theme.bg)
        })
        .on_hover(motion::hover_listener(fade_key))
        .occlude()
        .role(gpui::Role::Button)
        .aria_label(if copied {
            "Chat name and IDs copied"
        } else {
            "Copy chat name and IDs"
        })
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            on_click(event, window, cx)
        })
        .child(
            icon(if copied {
                crate::icons::CHECK
            } else {
                crate::icons::COPY
            })
            .size(px(12.0))
            .text_color(theme.text_muted),
        )
        .child(
            div()
                .text_size(crate::typography::ui_rems(10.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(label)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, TestAppContext};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    struct HorizonRow {
        archived: Arc<AtomicBool>,
        hovered: bool,
    }

    impl Render for HorizonRow {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let mut theme = Theme::default();
            theme.ui_style = crate::ui_style::UiStyle::resolve(
                crate::ui_style::UiStyleId::Horizon,
                crate::ui_style::UiLayout::Guided,
            );
            let archived = self.archived.clone();
            horizon_sidebar_chat_row(
                &theme,
                "chat-test".into(),
                "test",
                theme.agent_identity_color(Some(mast_proto::HarnessId::ClaudeCode), Some("opus")),
                "Session".into(),
                crate::status_style::Status::Waiting,
                "Queued",
                true,
                false,
                Some("Work".into()),
                None,
                None,
                false,
                self.hovered,
                false,
                move |_, _, _| {
                    archived.store(true, Ordering::SeqCst);
                },
            )
            .unwrap()
        }
    }

    #[gpui::test]
    fn horizon_row_archive_action_is_clickable(cx: &mut TestAppContext) {
        let archived = Arc::new(AtomicBool::new(false));
        let recorded = archived.clone();
        let (_, cx) = cx.add_window_view(move |_, _| HorizonRow {
            archived: recorded,
            hovered: true,
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("chat-account-test").is_some());
        let corner = cx
            .debug_bounds("chat-test-corner")
            .expect("archive action")
            .center();
        cx.simulate_mouse_down(corner, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(corner, MouseButton::Left, gpui::Modifiers::default());
        assert!(archived.load(Ordering::SeqCst));
    }

    #[gpui::test]
    fn horizon_row_shows_status_and_account(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(move |_, _| HorizonRow {
            archived: Arc::new(AtomicBool::new(false)),
            hovered: false,
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("chat-account-test").is_some());
        assert!(cx.debug_bounds("chat-status-test").is_some());
    }
}

pub(crate) fn action_control(theme: &Theme) -> gpui::Div {
    div()
        .relative()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .h(px(24.0))
        .when(super::is_horizon(theme), |el| {
            el.h(px(theme.ui_style.tokens.control_size))
        })
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .when(super::is_horizon(theme), |el| el.border_color(theme.bg))
        .occlude()
}

pub(crate) fn action_segment(theme: &Theme, id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h_full()
        .px(px(7.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .text_size(px(11.5))
        .text_color(theme.text.opacity(0.9))
        .when(super::is_horizon(theme), |el| el.text_color(theme.text_muted))
        .when(super::is_horizon(theme), |el| el.rounded_full())
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .hover(|style| style.bg(crate::theme::ink(0.07)))
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
}

pub(crate) fn action_chevron(
    theme: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id("project-actions-chevron")
        .h_full()
        .w(px(23.0))
        .flex()
        .items_center()
        .justify_center()
        .border_l_1()
        .border_color(theme.border)
        .when(super::is_horizon(theme), |el| {
            el.rounded_full().border_color(theme.bg)
        })
        .cursor_pointer()
        .hover(|style| style.bg(crate::theme::ink(0.07)))
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            on_click(event, window, cx)
        })
        .child(
            icon(crate::icons::ALT_ARROW_DOWN)
                .size(px(11.0))
                .text_color(theme.text_muted),
        )
}

pub(crate) fn action_field_label(theme: &Theme, label: &str) -> gpui::Div {
    div()
        .mt(px(12.0))
        .mb(px(5.0))
        .text_size(px(12.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text_muted)
        .when(super::is_industrial(theme), |el| {
            el.font_family(theme.font_mono.clone())
        })
        .child(SharedString::from(label.to_string()))
}

/// Keep the fade short so only the last few glyphs recede. Tracking clipped
/// content lets the shared paint-time overflow gate leave fitting labels intact.
pub(crate) fn sidebar_faded_label(
    id: SharedString,
    fill: bool,
    label: impl IntoElement,
) -> impl IntoElement {
    let overflow = gpui::ScrollHandle::new();
    crate::edge_fade::edge_faded(
        20.0,
        false,
        false,
        div()
            .id(id.clone())
            .debug_selector(move || id.to_string())
            .when(fill, |el| el.flex_1())
            .min_w_0()
            .overflow_hidden()
            .track_scroll(&overflow)
            .flex()
            .child(div().flex_none().whitespace_nowrap().child(label)),
    )
    .fade_right(true)
    .fade_label_overflow(&overflow)
}

/// Exact active-session row height. Harness identity lives on the title line
/// and the Working glyph lives in the status corner, so neither adds a third
/// line. Compact rows omit the metadata line and its preceding gap entirely;
/// branch / pull-request rows add the exact height of their tallest child.
/// Keeping this calculation beside the renderer's metrics prevents disclosure
/// clips when view options alter the row structure.
pub(crate) fn chat_row_height(shows_branch: bool, shows_pull_request: bool) -> f32 {
    let mut metadata_height: f32 = 0.0;
    if shows_branch {
        metadata_height = metadata_height.max(14.0);
    }
    if shows_pull_request {
        metadata_height = metadata_height.max(16.0);
    }
    if metadata_height == 0.0 {
        45.0
    } else {
        47.0 + metadata_height
    }
}
pub(crate) fn sidebar_row_height(
    compact: bool,
    show_label: bool,
    branch: bool,
    pr: bool,
    tokens: crate::ui_style::StyleTokens,
) -> f32 {
    if compact {
        tokens.sidebar_row_compact_h
    } else {
        chat_row_height(branch, pr) - (if show_label { 0.0 } else { 16.0 })
            + tokens.sidebar_row_extra_h
    }
}
