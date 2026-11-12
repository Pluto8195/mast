//! Accent-independent presentation of chat and worker states.

use crate::theme::Appearance;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Div, EntityId, IntoElement, ParentElement, SharedString, Styled, div, px,
};
use mast_proto::ChatIndicator;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Working,
    Waiting,
    Errored,
    Ready,
    Done,
    Idle,
}

impl From<ChatIndicator> for Status {
    fn from(value: ChatIndicator) -> Self {
        match value {
            ChatIndicator::Working => Self::Working,
            ChatIndicator::AwaitingInput => Self::Waiting,
            ChatIndicator::Errored => Self::Errored,
            ChatIndicator::Completed => Self::Done,
            ChatIndicator::Idle => Self::Idle,
        }
    }
}

impl From<crate::team::WorkerState> for Status {
    fn from(value: crate::team::WorkerState) -> Self {
        use crate::team::WorkerState;
        match value {
            WorkerState::Working => Self::Working,
            WorkerState::Waiting => Self::Waiting,
            WorkerState::Errored => Self::Errored,
            WorkerState::Done => Self::Done,
            WorkerState::Idle => Self::Idle,
        }
    }
}

impl From<crate::running_chats::RunState> for Status {
    fn from(value: crate::running_chats::RunState) -> Self {
        use crate::running_chats::RunState;
        match value {
            RunState::Working => Self::Working,
            RunState::Awaiting => Self::Waiting,
            RunState::Errored | RunState::ToolFailed => Self::Errored,
            RunState::IdleBackground => Self::Idle,
        }
    }
}

pub(crate) struct StatusStyle {
    pub color: gpui::Hsla,
    pub rgb: [u8; 3],
    pub glyph: &'static str,
}

pub(crate) fn status_style(status: Status, appearance: Appearance) -> StatusStyle {
    let dark = appearance.is_dark();
    let color = match (status, dark) {
        (Status::Working, true) => 0x38d059,
        (Status::Working, false) => 0x245f31,
        (Status::Waiting, true) => 0xdfbe1b,
        (Status::Waiting, false) => 0xa68a00,
        (Status::Errored, true) => 0xfe4b25,
        (Status::Errored, false) => 0xd93a15,
        (Status::Ready, _) => 0x3f69cc,
        (Status::Done, _) => 0x619c82,
        (Status::Idle, true) => 0xb5b5b5,
        (Status::Idle, false) => 0x183028,
    };
    StatusStyle {
        color: gpui::rgb(color).into(),
        rgb: [(color >> 16) as u8, (color >> 8) as u8, color as u8],
        glyph: glyph(status),
    }
}

pub(crate) fn glyph(status: Status) -> &'static str {
    match status {
        Status::Working => "◌",
        Status::Waiting => "!",
        Status::Errored => "✕",
        Status::Ready => "→",
        Status::Done => "✓",
        Status::Idle => "○",
    }
}

/// A fixed-size status marker for rows that do not host a live spinner.
pub(crate) fn indicator(status: Status, appearance: Appearance) -> Div {
    let style = status_style(status, appearance);
    div()
        .flex_none()
        .size(px(10.0))
        .flex()
        .items_center()
        .justify_center()
        .text_size(crate::typography::ui_rems(10.0))
        .text_color(style.color)
        .when(status == Status::Waiting, |element| {
            element
                .rounded_full()
                .bg(style.color)
                .text_color(gpui::rgb(0x183028))
        })
        .child(style.glyph)
}

pub(crate) fn indicator_for_theme(status: Status, theme: &crate::theme::Theme) -> Div {
    if theme.ui_style.status_indicator == crate::ui_style::StatusIndicator::Dot {
        div().flex_none().size(px(8.0)).rounded_full().bg(dot_color(status, theme))
    } else {
        indicator(status, theme.appearance)
    }
}

pub(crate) fn dot_color(status: Status, theme: &crate::theme::Theme) -> gpui::Hsla {
    if crate::components::is_horizon(theme) {
        match status {
            Status::Working => theme.accent,
            Status::Waiting => theme.warning,
            Status::Errored => theme.danger,
            Status::Done => theme.success,
            Status::Ready => theme.accent,
            Status::Idle => theme.text_muted,
        }
    } else if theme.ui_style.status_indicator == crate::ui_style::StatusIndicator::Dot {
        match status { Status::Working => theme.accent, Status::Done => theme.success, _ => theme.text_muted }
    } else {
        status_style(status, theme.appearance).color
    }
}

/// Animate working markers with the shared, throttled activity loader.
pub(crate) fn animated_indicator(
    status: Status,
    appearance: Appearance,
    key: impl Into<SharedString>,
    view: EntityId,
    cx: &mut App,
) -> AnyElement {
    if let Some(theme) = cx.try_global::<crate::theme::Theme>()
        && theme.ui_style.status_indicator == crate::ui_style::StatusIndicator::Dot
    {
        return indicator_for_theme(status, theme).into_any_element();
    }
    if status == Status::Working {
        crate::loaders::mini_mono_spinner(
            key,
            1.5,
            status_style(status, appearance).color,
            view,
            cx,
        )
        .into_any_element()
    } else {
        indicator(status, appearance).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{AccentColor, Theme};

    #[test]
    fn industrial_dot_uses_signal_for_working_and_neutral_for_idle() {
        let theme = Theme::for_selection(Appearance::Light, "industrial-light", Default::default(), mast_theme::SurfacePreference::Opaque)
            .with_ui_style(crate::ui_style::UiStyle::INDUSTRIAL);
        assert_eq!(dot_color(Status::Working, &theme), theme.accent);
        assert_eq!(dot_color(Status::Idle, &theme), theme.text_muted);
        assert_eq!(dot_color(Status::Done, &theme), theme.success);
    }

    #[test]
    fn view_models_share_status_mapping() {
        use crate::running_chats::RunState;
        use crate::team::WorkerState;
        let rows = [
            (
                ChatIndicator::Working,
                WorkerState::Working,
                RunState::Working,
                Status::Working,
            ),
            (
                ChatIndicator::AwaitingInput,
                WorkerState::Waiting,
                RunState::Awaiting,
                Status::Waiting,
            ),
            (
                ChatIndicator::Errored,
                WorkerState::Errored,
                RunState::Errored,
                Status::Errored,
            ),
            (
                ChatIndicator::Idle,
                WorkerState::Idle,
                RunState::IdleBackground,
                Status::Idle,
            ),
        ];
        for (chat, worker, running, expected) in rows {
            assert_eq!(Status::from(chat), expected);
            assert_eq!(Status::from(worker), expected);
            assert_eq!(Status::from(running), expected);
        }
        assert_eq!(Status::from(ChatIndicator::Completed), Status::Done);
        assert_eq!(Status::from(WorkerState::Done), Status::Done);
        assert_eq!(Status::from(RunState::ToolFailed), Status::Errored);
    }

    fn luminance(rgb: [u8; 3]) -> f64 {
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
    }

    #[test]
    fn status_pairs_are_distinct_for_every_accent_and_appearance() {
        let states = [
            Status::Working,
            Status::Waiting,
            Status::Errored,
            Status::Ready,
            Status::Done,
            Status::Idle,
        ];
        for appearance in [Appearance::Light, Appearance::Dark] {
            for accent in AccentColor::ALL {
                let theme = Theme::for_preferences(appearance, accent);
                for (index, state) in states.iter().enumerate() {
                    let style = status_style(*state, theme.appearance);
                    for other in &states[index + 1..] {
                        let other = status_style(*other, theme.appearance);
                        assert_ne!((style.rgb, style.glyph), (other.rgb, other.glyph));
                        assert_ne!(style.glyph, other.glyph);
                    }
                    for background in if appearance.is_dark() {
                        [[6; 3], [13; 3]]
                    } else {
                        [[255; 3], [246; 3]]
                    } {
                        let a = luminance(style.rgb);
                        let b = luminance(background);
                        let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                        assert!(contrast >= 2.9, "{state:?} on {background:?}: {contrast}");
                    }
                }
                let waiting = status_style(Status::Waiting, theme.appearance).rgb;
                let working = status_style(Status::Working, theme.appearance).rgb;
                let distance = waiting
                    .into_iter()
                    .zip(working)
                    .map(|(a, b)| (f64::from(a) - f64::from(b)).powi(2))
                    .sum::<f64>()
                    .sqrt();
                assert!(distance > 100.0, "{appearance:?} {accent:?}: {distance}");
            }
        }
    }
}
