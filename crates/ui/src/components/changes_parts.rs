//! Presentation primitives for changes and review walkthrough surfaces.

use crate::theme::Theme;
use gpui::{div, prelude::*, px};

pub(crate) fn review_action_label(label: &'static str, theme: &Theme) -> gpui::SharedString {
    if super::is_industrial(theme) {
        crate::popover::tracked_upper(label).into()
    } else {
        label.into()
    }
}

pub(crate) fn file_toolbar_selection<T: Styled + FluentBuilder>(
    element: T,
    theme: &Theme,
    selected: bool,
) -> T {
    element
        .when(selected, |element| element.bg(crate::theme::wash(0.1)))
        .when(selected && super::is_industrial(theme), |element| {
            element.bg(theme.industrial_colors().pressed)
        })
        .when(selected && super::is_horizon(theme), |element| {
            element.bg(theme.accent.opacity(0.18))
        })
}

pub(crate) fn review_text_action<T: Styled + ParentElement + FluentBuilder>(
    element: T,
    theme: &Theme,
) -> T {
    let element = element.when(super::is_industrial(theme), |element| {
        element.px(px(8.0)).py(px(4.0))
    });
    let element = super::segmented_key(element, theme, false);
    element.when(super::is_horizon(theme), |element| {
        element
            .px(px(8.0))
            .py(px(4.0))
            .rounded_full()
            .bg(theme.accent.opacity(0.10))
    })
}

pub(crate) fn changes_revision_label<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .rounded(px(theme
            .ui_style
            .tokens
            .chip_radius
            .unwrap_or(crate::surface_chrome::CONTROL_RADIUS)))
        .bg(crate::theme::ink(0.05))
        .when(super::is_industrial(theme), |element| {
            element
                .bg(theme.surface_raised)
                .border_1()
                .border_color(theme.border)
        })
        .when(super::is_horizon(theme), |element| {
            element.rounded_full().bg(theme.accent.opacity(0.10))
        })
}

pub(crate) fn changes_tooltip<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(6.0)))
        .when(super::is_industrial(theme), |element| {
            element.bg(theme.surface_raised).border_color(theme.border)
        })
        .when(super::is_horizon(theme), |element| {
            element.rounded_full().bg(crate::theme::flatten(
                theme.accent.opacity(0.10),
                theme.surface_raised,
            ))
        })
}

pub(crate) fn file_drag_ghost<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(6.0)))
        .when(super::is_horizon(theme), |element| {
            element.rounded_full().bg(crate::theme::flatten(
                theme.accent.opacity(0.10),
                theme.surface_raised,
            ))
        })
}

pub(crate) fn file_editor_tooltip(label: gpui::SharedString, theme: &Theme) -> impl IntoElement {
    let radius = theme.ui_style.tokens.control_radius.unwrap_or(6.0);
    let card = div()
        .max_w(px(360.0))
        .px(px(9.0))
        .py(px(6.0))
        .rounded(px(radius))
        .border_1()
        .border_color(theme.border)
        .bg(crate::popover::surface_bg(theme))
        .font_family(theme.font_sans.clone())
        .text_size(px(10.5))
        .text_color(theme.text_muted)
        .child(label)
        .when(super::is_horizon(theme), |element| {
            element.bg(crate::theme::flatten(
                theme.accent.opacity(0.10),
                theme.surface_raised,
            ))
        });
    crate::frost::frosted(radius, crate::frost::MENU_BLUR, card)
}

pub(crate) fn walkthrough_heading(label: &str, theme: &Theme) -> gpui::SharedString {
    if super::is_industrial(theme) {
        crate::popover::tracked_upper(label).into()
    } else {
        label.to_owned().into()
    }
}

pub(crate) fn walkthrough_section_label(label: &'static str, theme: &Theme) -> gpui::SharedString {
    if super::is_horizon(theme) {
        match label {
            "HIGHLIGHTS" => "Highlights",
            "QUESTIONS" => "Questions",
            _ => label,
        }
        .into()
    } else if super::is_industrial(theme) {
        crate::popover::tracked_upper(label).into()
    } else {
        label.into()
    }
}

pub(crate) fn file_save_status<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .rounded(px(theme
            .ui_style
            .tokens
            .control_radius
            .unwrap_or(crate::surface_chrome::CONTROL_RADIUS)))
        .when(super::is_industrial(theme), |element| {
            element
                .bg(theme.surface_raised)
                .border_1()
                .border_color(theme.border)
        })
        .when(super::is_horizon(theme), |element| {
            element.rounded_full().bg(theme.accent.opacity(0.08))
        })
}

pub(crate) enum ChangesWarning {
    Dirty,
    Partial,
}

pub(crate) fn changes_warning_badge<T: Styled + FluentBuilder>(
    element: T,
    theme: &Theme,
    warning: ChangesWarning,
) -> T {
    let (radius, opacity) = match warning {
        ChangesWarning::Dirty => (3.0, 0.85),
        ChangesWarning::Partial => (4.0, 0.75),
    };
    element
        .rounded(px(theme.ui_style.tokens.chip_radius.unwrap_or(radius)))
        .bg(theme.warning.opacity(0.08))
        .text_color(theme.warning.opacity(opacity))
        .when(super::is_industrial(theme), |element| {
            element.border_1().border_color(theme.border)
        })
        .when(super::is_horizon(theme), |element| element.rounded_full())
}

pub(crate) fn changes_suggested_badge<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .rounded(px(theme.ui_style.tokens.chip_radius.unwrap_or(4.0)))
        .bg(theme.accent.opacity(0.14))
        .text_color(theme.accent)
        .when(super::is_industrial(theme), |element| {
            element.border_1().border_color(theme.border)
        })
        .when(super::is_horizon(theme), |element| element.rounded_full())
}

pub(crate) enum FileCommentAction {
    Add,
    Remove,
}

pub(crate) fn file_comment_action<T: Styled + ParentElement + FluentBuilder>(
    element: T,
    theme: &Theme,
    action: FileCommentAction,
) -> T {
    let element = element.rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(4.0)));
    match action {
        FileCommentAction::Add => element
            .bg(theme.solid)
            .when(super::is_horizon(theme), |element| {
                element.rounded_full().bg(theme.accent)
            }),
        FileCommentAction::Remove => {
            let element = super::segmented_key(element, theme, false);
            element.when(super::is_horizon(theme), |element| {
                element.rounded_full().bg(theme.accent.opacity(0.10))
            })
        }
    }
}

pub(crate) fn file_comment_input<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(7.0)))
        .border_1()
        .border_color(theme.border)
        .bg(theme.input_glass_bg())
        .when(super::is_industrial(theme), |element| {
            element.bg(theme.input_bg)
        })
}

pub(crate) enum FileAction {
    Retry,
    Refresh,
}

pub(crate) fn file_action<T: Styled + ParentElement + FluentBuilder>(
    element: T,
    theme: &Theme,
    action: FileAction,
) -> T {
    let radius = match action {
        FileAction::Retry => 7.0,
        FileAction::Refresh => 5.0,
    };
    let element = element.rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(radius)));
    let element = super::segmented_key(element, theme, false);
    element.when(super::is_horizon(theme), |element| {
        element.rounded_full().bg(theme.accent.opacity(0.10))
    })
}

pub(crate) fn changes_repository_chip<T: Styled + ParentElement + FluentBuilder>(
    element: T,
    theme: &Theme,
    active: bool,
) -> T {
    let element = element
        .rounded(px(theme.ui_style.tokens.chip_radius.unwrap_or(4.0)))
        .border_1()
        .border_color(if active { theme.accent } else { theme.border })
        .when(active, |element| element.bg(theme.accent.opacity(0.08)));
    let element = super::segmented_key(element, theme, active);
    element.when(super::is_horizon(theme), |element| {
        element
            .rounded_full()
            .bg(theme.accent.opacity(if active { 0.18 } else { 0.06 }))
    })
}

pub(crate) fn changes_diff_header<T: Styled + StatefulInteractiveElement + FluentBuilder>(
    element: T,
    theme: &Theme,
    default_hover: gpui::Hsla,
) -> T {
    let horizon_background = crate::theme::flatten(theme.accent.opacity(0.08), theme.surface);
    let horizon_hover = crate::theme::flatten(theme.accent.opacity(0.14), theme.surface);
    let industrial_background = theme.surface_raised;
    let hover = if super::is_industrial(theme) {
        theme.element_hover
    } else if super::is_horizon(theme) {
        horizon_hover
    } else {
        default_hover
    };
    element
        .when(super::is_industrial(theme), |element| {
            element
                .bg(industrial_background)
                .border_b_1()
                .border_color(theme.border)
        })
        .when(super::is_horizon(theme), |element| element.bg(horizon_background))
        .hover(move |style| style.bg(hover))
}

pub(crate) enum ChangesSelector {
    Scope,
    Reference,
    Target,
}

pub(crate) fn changes_selector<T: Styled + ParentElement + FluentBuilder>(
    element: T,
    theme: &Theme,
    kind: ChangesSelector,
) -> T {
    let fallback = match kind {
        ChangesSelector::Scope | ChangesSelector::Reference => 6.0,
        ChangesSelector::Target => 5.0,
    };
    let element = element.rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(fallback)));
    let element = super::segmented_key(element, theme, false);
    element.when(super::is_horizon(theme), |element| {
        element.rounded_full().bg(theme.accent.opacity(0.10))
    })
}

pub(crate) fn file_search_row<T: Styled + StatefulInteractiveElement + FluentBuilder>(
    element: T,
    theme: &Theme,
    selected: bool,
) -> T {
    let element = element
        .when(selected, |element| element.bg(crate::theme::wash(0.1)))
        .when(!selected, |element| {
            element.hover(|style| style.bg(crate::theme::wash(0.055)))
        });
    element
        .when(super::is_industrial(theme) && selected, |element| {
            element
                .bg(theme.surface_raised)
                .border_l_2()
                .border_color(theme.accent)
        })
        .when(super::is_horizon(theme), |element| {
            element
                .rounded_full()
                .when(selected, |element| element.bg(theme.accent.opacity(0.16)))
        })
}

pub(crate) fn changes_file_row<T: Styled + StatefulInteractiveElement + FluentBuilder>(
    element: T,
    theme: &Theme,
) -> T {
    let hover = theme.element_hover;
    element
        .hover(move |style| style.bg(hover))
        .when(super::is_industrial(theme), |element| {
            element.w_full().border_b_1().border_color(theme.border)
        })
        .when(super::is_horizon(theme), |element| element.rounded_full())
}

pub(crate) fn changes_file_list<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .when(super::is_industrial(theme), |element| {
            element.bg(theme.surface)
        })
        .when(super::is_horizon(theme), |element| {
            element.bg(theme.accent.opacity(0.06))
        })
}

pub(crate) fn file_reveal<T: Styled + InteractiveElement>(
    element: T,
    theme: &Theme,
    group: impl Into<gpui::SharedString>,
) -> T {
    crate::ui_style::reveal_on_hover(element, group, theme.ui_style, false)
}

pub(crate) fn file_tree_row<T: Styled + StatefulInteractiveElement + FluentBuilder>(
    element: T,
    theme: &Theme,
    selected: bool,
    focused: bool,
) -> T {
    let element = element
        .when(selected, |element| {
            element.bg(crate::theme::wash(if focused { 0.12 } else { 0.08 }))
        })
        .when(!selected, |element| {
            element.hover(|style| style.bg(crate::theme::wash(0.055)))
        });
    element
        .when(super::is_industrial(theme) && selected, |element| {
            element
                .bg(theme.surface_raised)
                .border_l_2()
                .border_color(theme.accent)
        })
        .when(super::is_horizon(theme), |element| {
            element
                .rounded_full()
                .when(selected, |element| element.bg(theme.accent.opacity(0.16)))
        })
}

pub(crate) fn file_toolbar_button(
    id: &'static str,
    label: &'static str,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let guided = theme.ui_style.behavior.labels == crate::ui_style::LabelMode::All;
    gpui::div()
        .id(id)
        .size(px(crate::surface_chrome::CONTROL_SIZE))
        .when(super::is_industrial(theme), |element| {
            element.size(px(theme.ui_style.tokens.control_size))
        })
        .flex_none()
        .when(guided, |element| {
            element
                .h(px(theme.ui_style.tokens.control_size))
                .w_auto()
                .min_w(px(theme.ui_style.tokens.control_size))
                .flex_shrink_1()
                .px(px(6.0))
                .gap(px(4.0))
                .flex_row_reverse()
                .child(
                    gpui::div()
                        .min_w_0()
                        .truncate()
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_muted)
                        .child(label),
                )
        })
        .rounded(px(theme
            .ui_style
            .tokens
            .control_radius
            .unwrap_or(crate::surface_chrome::CONTROL_RADIUS)))
        .when(true, |element| {
            crate::surface_chrome::icon_key(element, theme, false)
        })
        .when(super::is_horizon(theme), |element| {
            element.rounded_full().bg(theme.accent.opacity(0.08))
        })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label(label)
        .occlude()
        .on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
            window.prevent_default()
        })
        .hover(|style| style.bg(crate::theme::wash(0.14)))
}

pub(crate) fn changes_loading(
    theme: &Theme,
    id: &'static str,
    owner: gpui::EntityId,
    cx: &mut gpui::App,
) -> gpui::AnyElement {
    if theme.ui_style.status_indicator == crate::ui_style::StatusIndicator::Dot {
        crate::status_style::indicator_for_theme(crate::status_style::Status::Working, theme)
            .into_any_element()
    } else {
        crate::loaders::gradient_spinner(id, theme, 2.0, owner, cx).into_any_element()
    }
}

pub(crate) fn explanation_trigger<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(6.0)))
        .bg(theme.surface_raised)
        .when(super::is_industrial(theme), |element| {
            element.border_1().border_b_2().border_color(theme.border)
        })
}

pub(crate) fn walkthrough_title<T: Styled>(element: T, theme: &Theme) -> T {
    element.text_size(crate::typography::ui_rems(
        theme.ui_style.tokens.title_size.unwrap_or(13.0),
    ))
}

pub(crate) fn walkthrough_chip<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .rounded(px(theme.ui_style.tokens.chip_radius.unwrap_or(6.0)))
        .bg(theme.surface_raised)
        .when(super::is_horizon(theme), |element| {
            element.rounded_full().bg(theme.accent.opacity(0.12))
        })
}

pub(crate) fn walkthrough_guidance<T: FluentBuilder>(
    element: T,
    theme: &Theme,
    build: impl FnOnce(T) -> T,
) -> T {
    element.when(theme.ui_style.behavior.help_cards, build)
}

pub(crate) fn walkthrough_hints<T: FluentBuilder>(
    element: T,
    theme: &Theme,
    build: impl FnOnce(T) -> T,
) -> T {
    element.when(
        theme.ui_style.behavior.hints == crate::ui_style::HintMode::Inline,
        build,
    )
}

pub(crate) fn walkthrough_note_more<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .when(super::is_industrial(theme), |element| {
            element
                .px(px(8.0))
                .py(px(4.0))
                .bg(theme.surface_raised)
                .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(4.0)))
                .border_1()
                .border_color(theme.border)
        })
        .when(super::is_horizon(theme), |element| {
            element
                .self_start()
                .px(px(8.0))
                .py(px(4.0))
                .rounded_full()
                .bg(theme.accent.opacity(0.10))
        })
}

pub(crate) fn walkthrough_note_more_visible(
    theme: &Theme,
    expanded: bool,
    height: f32,
    limit: f32,
) -> bool {
    !super::is_industrial(theme) || expanded || height > limit + 0.5
}

pub(crate) fn walkthrough_note_content(
    what: String,
    why: String,
    risk: String,
    measured: std::rc::Rc<std::cell::Cell<f32>>,
    owner: gpui::EntityId,
    theme: &Theme,
) -> gpui::AnyElement {
    let why = format!("Why: {why}");
    if super::is_industrial(theme) {
        let what = gpui::StyledText::new(what);
        let why = gpui::StyledText::new(why);
        let risk = gpui::StyledText::new(risk);
        let layouts = [
            what.layout().clone(),
            why.layout().clone(),
            risk.layout().clone(),
        ];
        gpui::div()
            .relative()
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |_, _, _, cx| {
                        let height = layouts
                            .iter()
                            .map(|layout| {
                                let lines: usize = layout
                                    .line_layouts()
                                    .iter()
                                    .map(|line| line.wrap_boundaries.len() + 1)
                                    .sum();
                                lines.max(1) as f32 * f32::from(layout.line_height())
                            })
                            .sum::<f32>()
                            + 8.0;
                        if (measured.get() - height).abs() > 0.5 {
                            measured.set(height);
                            cx.notify(owner);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .flex()
            .flex_col()
            .gap_1()
            .child(what)
            .child(gpui::div().text_color(theme.text_muted).child(why))
            .child(gpui::div().text_color(theme.text_muted).child(risk))
            .into_any_element()
    } else {
        gpui::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(what)
            .child(gpui::div().text_color(theme.text_muted).child(why))
            .child(gpui::div().text_color(theme.text_muted).child(risk))
            .into_any_element()
    }
}

pub(crate) fn walkthrough_empty_decoration(theme: &Theme, empty: bool) -> Option<gpui::Div> {
    (empty && super::is_industrial(theme)).then(|| crate::surface_chrome::grille(theme))
}

pub(crate) fn walkthrough_review_action<T: Styled + ParentElement + FluentBuilder>(
    element: T,
    theme: &Theme,
    reviewed: bool,
) -> T {
    let industrial = super::is_industrial(theme);
    let element = element.when(industrial, |element| {
        element
            .self_start()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(8.0))
    });
    let element = crate::surface_chrome::segmented_key(element, theme, reviewed);
    element
        .when(!reviewed && industrial, |element| {
            let foreground = if crate::theme::contrast_ratio(theme.text, theme.accent)
                >= crate::theme::contrast_ratio(theme.bg, theme.accent)
            {
                theme.text
            } else {
                theme.bg
            };
            element.bg(theme.accent).text_color(foreground)
        })
        .when(super::is_horizon(theme), |element| {
            element
                .self_start()
                .px(px(12.0))
                .py(px(6.0))
                .rounded_full()
                .bg(theme.accent.opacity(0.16))
        })
        .children(crate::surface_chrome::active_key_dot(theme, reviewed))
}

pub(crate) fn walkthrough_progress<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .when(super::is_industrial(theme), |element| {
            element.font_family(theme.font_mono.clone())
        })
        .when(super::is_horizon(theme), |element| {
            element
                .px(px(8.0))
                .py(px(4.0))
                .rounded_full()
                .bg(theme.accent.opacity(0.12))
        })
}

pub(crate) fn walkthrough_progress_label(progress: String, theme: &Theme) -> gpui::AnyElement {
    if super::is_industrial(theme) {
        crate::surface_chrome::display(theme)
            .child(progress)
            .into_any_element()
    } else {
        gpui::div().child(progress).into_any_element()
    }
}

pub(crate) fn walkthrough_mode<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .when(super::is_industrial(theme), |element| {
            element
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(4.0)))
                .bg(theme.surface_raised)
                .border_1()
                .border_b_2()
                .border_color(theme.border)
        })
        .when(super::is_horizon(theme), |element| {
            element
                .px(px(8.0))
                .py(px(4.0))
                .rounded_full()
                .bg(theme.accent.opacity(0.12))
        })
}

pub(crate) fn walkthrough_step_row<T: Styled + FluentBuilder>(
    element: T,
    theme: &Theme,
    selected: bool,
) -> T {
    element
        .when(super::is_industrial(theme), |row| {
            row.px(px(8.0))
                .py(px(8.0))
                .border_b_1()
                .border_color(theme.border)
        })
        .when(selected, |row| row.bg(theme.element_active))
        .when(super::is_horizon(theme), |row| {
            row.rounded_full()
                .when(selected, |row| row.bg(theme.accent.opacity(0.16)))
        })
}

pub(crate) fn walkthrough_navigation<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque
                && theme.ui_style.id != crate::ui_style::UiStyleId::Horizon,
            |element| {
                element
                    .bg(theme.surface)
                    .p(px(8.0))
                    .border_r_1()
                    .border_color(theme.border)
            },
        )
        .when(super::is_horizon(theme), |element| {
            element.bg(theme.accent.opacity(0.06))
        })
}

pub(crate) fn walkthrough_notes<T: Styled + FluentBuilder>(element: T, theme: &Theme) -> T {
    element
        .when(
            theme.ui_style.surface == crate::ui_style::SurfaceModel::Opaque,
            |element| {
                element
                    .bg(theme.surface)
                    .p(px(8.0))
                    .border_l_1()
                    .border_color(theme.border)
            },
        )
        .when(super::is_horizon(theme), |element| {
            element.bg(theme.accent.opacity(0.06))
        })
}

/// Icon action in the changes pane header.
pub(crate) fn changes_header_toggle(
    id: &'static str,
    icon_path: &'static str,
    active: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(crate::surface_chrome::CONTROL_SIZE))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(crate::surface_chrome::CONTROL_RADIUS))
        .cursor_pointer()
        // Latched: the blend is neither read nor driven, and its listener
        // would dirty the whole window on every enter/leave for nothing.
        .map(|el| {
            if active {
                el.bg(crate::theme::wash(0.14))
            } else {
                el.bg(crate::motion::hover_blend(
                    id,
                    crate::theme::wash(0.0),
                    crate::theme::wash(0.14),
                ))
                .on_hover(crate::motion::hover_listener(id))
            }
        })
        .when(super::is_industrial(theme), |el| {
            super::toolbar_control(el, theme, active)
        })
        .when(super::is_horizon(theme), |el| {
            el.rounded_full()
                .when(active, |el| el.bg(theme.accent.opacity(0.18)))
        })
        .occlude()
        .on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
            window.prevent_default()
        })
        .child(
            crate::icons::icon(icon_path)
                .size(px(crate::surface_chrome::ICON_SIZE))
                .text_color(if active {
                    theme.text
                } else {
                    theme.text_muted.opacity(0.7)
                }),
        )
}

pub(crate) fn file_comment_button(
    id: &'static str,
    label: &'static str,
    primary: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(22.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(theme.ui_style.tokens.control_radius.unwrap_or(6.0)))
        .text_size(px(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .cursor_pointer()
        .when(primary, |element| {
            element.bg(theme.solid).text_color(theme.on_solid)
        })
        .when(!primary, |element| {
            element
                .text_color(crate::motion::hover_blend(id, theme.text_muted, theme.text))
                .bg(crate::motion::hover_blend(
                    id,
                    gpui::transparent_black(),
                    theme.element_hover,
                ))
                .on_hover(crate::motion::hover_listener(id))
        })
        .when(true, |element| {
            super::segmented_key(element, theme, primary)
        })
        .when(super::is_horizon(theme), |element| {
            element
                .rounded_full()
                .bg(theme.accent.opacity(if primary { 0.20 } else { 0.08 }))
                .text_color(theme.text)
        })
        .child(gpui::SharedString::from(label))
}
