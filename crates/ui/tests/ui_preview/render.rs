use gpui::{AnyView, App, Entity, Render, Window, div, prelude::*, px, size};
use mast_ui::{
    theme::{Appearance, Theme},
    ui_style::{UiLayout, UiStyleId},
};
use std::path::PathBuf;

struct Frame {
    screen: String,
    label: String,
    view: AnyView,
}
impl Render for Frame {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
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
                    .child(format!("{}   |   {}", self.screen, self.label)),
            )
            .child(div().flex_1().min_h_0().child(self.view.clone()))
    }
}

struct Controls {
    input: Entity<mast_ui::composer::ComposerInput>,
}
impl Render for Controls {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx);
        div()
            .size_full()
            .p(px(32.))
            .flex()
            .flex_col()
            .gap(px(18.))
            .bg(t.surface)
            .text_color(t.text)
            .child("Buttons and toggles")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(mast_ui::components::button(
                        "preview-primary",
                        "Primary",
                        mast_ui::components::ButtonVariant::Primary,
                        mast_ui::components::ButtonSize::Md,
                        t,
                    ))
                    .child(mast_ui::components::button(
                        "preview-secondary",
                        "Secondary",
                        mast_ui::components::ButtonVariant::Ghost,
                        mast_ui::components::ButtonSize::Md,
                        t,
                    ))
                    .child(mast_ui::components::button(
                        "preview-danger",
                        "Delete",
                        mast_ui::components::ButtonVariant::Danger,
                        mast_ui::components::ButtonSize::Md,
                        t,
                    ))
                    .child(mast_ui::components::toggle_switch(t, true))
                    .child(mast_ui::components::toggle_switch(t, false))
                    .child(mast_ui::components::badge(t, "Draft"))
                    .child(mast_ui::components::badge_active(t, "Active")),
            )
            .child("Tabs and status")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(mast_ui::components::tab(t, "Overview", true))
                    .child(mast_ui::components::tab(t, "Activity", false))
                    .children(
                        [
                            (mast_ui::components::Status::Success, "Ready"),
                            (mast_ui::components::Status::Warning, "Waiting"),
                            (mast_ui::components::Status::Danger, "Error"),
                        ]
                        .into_iter()
                        .map(|(status, label)| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(mast_ui::components::status_dot(t, status))
                                .child(label)
                        }),
                    ),
            )
            .child("Input")
            .child(
                mast_ui::components::text_input(t, self.input.clone().into_any_element())
                    .w(px(360.)),
            )
    }
}

fn initialize(
    data_dir: &std::path::Path,
    style: UiStyleId,
    layout: UiLayout,
    appearance: Appearance,
    theme_id: &str,
    cx: &mut App,
) {
    let mut settings = mast_ui::settings::UiSettings::default();
    settings.ui_style = style;
    settings.ui_layout = layout;
    match appearance {
        Appearance::Light => {
            settings.appearance = mast_ui::appearance::AppearanceMode::Light;
            settings.theme_selection.light = theme_id.to_owned();
        }
        Appearance::Dark => {
            settings.appearance = mast_ui::appearance::AppearanceMode::Dark;
            settings.theme_selection.dark = theme_id.to_owned();
        }
    }
    mast_ui::settings::init(settings.clone(), data_dir, cx);
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
    mast_ui::history::init(
        settings.git_history_columns,
        settings.git_history_column_widths,
        settings.git_history_column_order,
        settings.git_history_author_display,
        cx,
    );
    mast_ui::composer::init(cx, settings.composer_send_behavior);
    mast_ui::terminal::panel::init(cx);
    Theme::install_selection(
        appearance,
        theme_id,
        mast_theme::AccentSelection::ThemeDefault,
        mast_theme::SurfacePreference::default(),
        cx,
    );
}

pub fn render(
    screen: &str,
    style: UiStyleId,
    layout: UiLayout,
    theme_id: &str,
    appearance: Appearance,
    label: String,
    data_dir: PathBuf,
    name: &str,
) -> anyhow::Result<PathBuf> {
    let screen = screen.to_owned();
    let theme_id = theme_id.to_owned();
    super::snapshot(
        name,
        size(px(super::preview_width() as f32), px(900.)),
        style,
        layout,
        appearance,
        move |_, cx| {
            initialize(&data_dir, style, layout, appearance, &theme_id, cx);
            let view: AnyView = match screen.as_str() {
                "chat" => mast_ui::snapshot::chat_fixture(data_dir, cx).into(),
                "handoff" => mast_ui::snapshot::handoff_fixture(data_dir, cx).into(),
                "overview" => mast_ui::snapshot::overview_fixture(cx).into(),
                "linear" => mast_ui::snapshot::linear_overview_fixture(cx).into(),
                "linear-settings" => mast_ui::snapshot::linear_settings_fixture(cx).into(),
                "team" => mast_ui::snapshot::team_fixture(data_dir, cx).into(),
                "details" => mast_ui::snapshot::outline_fixture(data_dir, cx).into(),
                "walkthrough" => mast_ui::snapshot::walkthrough_fixture(cx).into(),
                "appearance" => cx
                    .new(mast_ui::settings::appearance::AppearancePage::new)
                    .into(),
                "controls" => cx
                    .new(|cx| Controls {
                        input: cx
                            .new(|cx| mast_ui::composer::ComposerInput::new("Type a message…", cx)),
                    })
                    .into(),
                _ => unreachable!(),
            };
            cx.new(|_| Frame {
                screen,
                label,
                view,
            })
        },
    )
}

/// A row-per-component comparison of all independent style/layout pairs.
struct ComponentMatrix {
    inputs: Vec<Entity<mast_ui::composer::ComposerInput>>,
}

impl Render for ComponentMatrix {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        use mast_ui::components as c;
        use mast_ui::ui_style::{UiLayout, UiStyle, UiStyleId};
        let base = Theme::of(cx).clone();
        let names = [
            "Primary",
            "Secondary",
            "Danger",
            "Ghost",
            "Icon",
            "Toggle on",
            "Toggle off",
            "Chip",
            "Tab",
            "Input",
            "Section header",
            "Card",
            "List row",
            "Selected row",
            "Menu item",
            "Status",
            "Readout",
            "User bubble",
            "Agent bubble",
        ];
        let mut table = div()
            .size_full()
            .bg(base.surface)
            .text_color(base.text)
            .flex()
            .flex_col();
        let mut header = div()
            .flex()
            .h(px(42.))
            .child(div().w(px(150.)).child("Component"));
        for &style in UiStyleId::ALL {
            for &layout in UiLayout::ALL {
                header = header.child(div().w(px(170.)).child(format!(
                    "{} / {}",
                    style.label(),
                    layout.label()
                )));
            }
        }
        table = table.child(header);
        for (row, name) in names.into_iter().enumerate() {
            let mut line = div()
                .flex()
                .min_h(px(55.))
                .items_center()
                .child(div().w(px(150.)).child(name));
            for (column, (&style, &layout)) in UiStyleId::ALL
                .iter()
                .flat_map(|style| UiLayout::ALL.iter().map(move |layout| (style, layout)))
                .enumerate()
            {
                let theme = base.clone().with_ui_style(UiStyle::resolve(style, layout));
                let id = gpui::SharedString::from(format!("matrix-{row}-{column}"));
                let element = match row {
                    0..=3 => c::button(
                        id,
                        name,
                        [
                            c::ButtonVariant::Primary,
                            c::ButtonVariant::Secondary,
                            c::ButtonVariant::Danger,
                            c::ButtonVariant::Ghost,
                        ][row],
                        c::ButtonSize::Md,
                        &theme,
                    )
                    .into_any_element(),
                    4 => c::icon_button(id, mast_ui::icons::PLUS, "Add", &theme).into_any_element(),
                    5 | 6 => c::toggle_switch(&theme, row == 5).into_any_element(),
                    7 => c::badge(&theme, "Draft").into_any_element(),
                    8 => c::tab(&theme, "Selected", true).into_any_element(),
                    9 => c::text_input(&theme, self.inputs[column].clone().into_any_element())
                        .into_any_element(),
                    10 => c::section_header(&theme, "Section", Some("Helpful guidance"))
                        .into_any_element(),
                    11 => c::section_card_with_label(&theme, "PANEL")
                        .mt(px(0.))
                        .child("Card")
                        .into_any_element(),
                    12 | 13 => c::list_row(&theme, row == 13)
                        .child("Row")
                        .into_any_element(),
                    14 => c::menu_row(&theme, true, format!("matrix-menu-{column}"))
                        .child("Menu")
                        .into_any_element(),
                    15 => {
                        c::status_indicator(&theme, c::Status::Success, "Ready").into_any_element()
                    }
                    16 => c::display(&theme).child("MODEL").into_any_element(),
                    17 => c::user_message_bubble(
                        &theme,
                        theme.identity_color(""),
                        160.,
                        22.,
                        false,
                        div().child("Hello").into_any_element(),
                    ),
                    _ => c::agent_message_bubble(
                        &theme,
                        theme.identity_color(""),
                        true,
                        true,
                        false,
                        false,
                        0.0,
                        Some("Model".into()),
                        div().child("Reply").into_any_element(),
                    ),
                };
                line = line.child(
                    div()
                        .w(px(170.))
                        .when(matches!(row, 0..=8 | 15..=16), |cell| {
                            cell.flex().items_start()
                        })
                        .child(element),
                );
            }
            table = table.child(line);
        }
        table
    }
}

pub fn render_component_matrix(
    theme_id: &str,
    appearance: Appearance,
    data_dir: PathBuf,
    name: &str,
) -> anyhow::Result<PathBuf> {
    let theme_id = theme_id.to_owned();
    super::snapshot(
        name,
        size(px(1680.), px(1200.)),
        UiStyleId::Default,
        UiLayout::Minimal,
        appearance,
        move |_, cx| {
            initialize(
                &data_dir,
                UiStyleId::Default,
                UiLayout::Minimal,
                appearance,
                &theme_id,
                cx,
            );
            cx.new(|cx| ComponentMatrix {
                inputs: (0..9)
                    .map(|_| cx.new(|cx| mast_ui::composer::ComposerInput::new("Type…", cx)))
                    .collect(),
            })
        },
    )
}
