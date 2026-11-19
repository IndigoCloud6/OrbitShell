use std::cell::Cell;

use gpui::{Div, ElementId, Rgba, SharedString, Stateful, div, prelude::*};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UiTheme {
    #[default]
    Light,
    Dark,
}

thread_local! {
    // Workbench elements and their deferred overlays are rendered on the GPUI thread.
    static UI_THEME: Cell<UiTheme> = const { Cell::new(UiTheme::Light) };
}

pub fn ui_theme() -> UiTheme {
    UI_THEME.with(Cell::get)
}

pub fn set_ui_theme(theme: UiTheme) {
    UI_THEME.with(|current| current.set(theme));
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ThemeColor(u32, u32);

impl ThemeColor {
    pub const fn light_rgb(self) -> u32 {
        self.0
    }
}

pub fn rgb(color: ThemeColor) -> Rgba {
    gpui::rgb(match ui_theme() {
        UiTheme::Light => color.0,
        UiTheme::Dark => color.1,
    })
}

pub fn modal_scrim() -> Rgba {
    gpui::rgba(match ui_theme() {
        UiTheme::Light => 0x202a3855,
        UiTheme::Dark => 0x080c14a0,
    })
}

pub mod color {
    use super::ThemeColor;

    pub const FRAME: ThemeColor = ThemeColor(0xeaf0f6, 0x141922);
    pub const SIDEBAR: ThemeColor = ThemeColor(0xf4f6fa, 0x1a212c);
    pub const SURFACE: ThemeColor = ThemeColor(0xffffff, 0x1f2733);
    pub const TOOLBAR: ThemeColor = ThemeColor(0xf8f9fc, 0x232d3a);
    pub const LINE: ThemeColor = ThemeColor(0xdbe2eb, 0x354354);
    pub const TEXT: ThemeColor = ThemeColor(0x202a38, 0xedf2f8);
    pub const MUTED: ThemeColor = ThemeColor(0x556474, 0xa9b8ca);
    pub const ACCENT: ThemeColor = ThemeColor(0x316aa8, 0x79b6ff);
    pub const ACCENT_TEXT: ThemeColor = ThemeColor(0x244e80, 0xa9ceff);
    pub const SELECTED: ThemeColor = ThemeColor(0xe3edf9, 0x29415f);
    pub const SELECTED_HOVER: ThemeColor = ThemeColor(0xd8e7f8, 0x325074);
    pub const HOVER: ThemeColor = ThemeColor(0xecf1f7, 0x293749);
    pub const SESSION_IDLE: ThemeColor = ThemeColor(0x91a0b2, 0x8192a8);
    pub const WINDOW_CLOSE_HOVER: ThemeColor = ThemeColor(0xf7dce0, 0x542d39);
    pub const DANGER: ThemeColor = ThemeColor(0xa93444, 0xff9aa9);
    pub const EDITOR_COMMENT: ThemeColor = ThemeColor(0x657385, 0x9eb1c7);
    pub const EDITOR_KEYWORD: ThemeColor = ThemeColor(0x713fa3, 0xd8baff);
    pub const EDITOR_STRING: ThemeColor = ThemeColor(0x2c714b, 0x91d6ae);
    pub const EDITOR_NUMBER: ThemeColor = ThemeColor(0x935022, 0xf3b995);
    pub const EDITOR_PROPERTY: ThemeColor = ThemeColor(0x215e99, 0x91c8ff);
    pub const EDITOR_MATCH: ThemeColor = ThemeColor(0xf7e5aa, 0x524324);
    pub const EDITOR_CURRENT_MATCH: ThemeColor = ThemeColor(0xf4cb71, 0x705329);
}

pub mod space {
    use gpui::{Pixels, px};

    pub const XS: Pixels = px(4.);
    pub const SM: Pixels = px(8.);
    pub const MD: Pixels = px(12.);
    pub const LG: Pixels = px(16.);
    pub const XL: Pixels = px(24.);
    pub const SPLITTER_WIDTH: Pixels = px(6.);
    pub const WORKSPACE_RADIUS: Pixels = px(12.);
    pub const CONTROL_RADIUS: Pixels = px(6.);
    pub const ROW_HEIGHT: Pixels = px(34.);
    pub const HEADER_HEIGHT: Pixels = px(50.);
    pub const TOOLBAR_HEIGHT: Pixels = px(39.);
    pub const TITLEBAR_HEIGHT: Pixels = px(38.);
    pub const FOOTER_HEIGHT: Pixels = px(30.);
    pub const CONTROL_HEIGHT: Pixels = px(27.);
    pub const ICON_SIZE: Pixels = px(20.);
    pub const COMPACT_ICON_SIZE: Pixels = px(15.);
}

pub mod text {
    use gpui::{Pixels, px};

    pub const DETAIL: Pixels = px(12.);
    pub const BODY: Pixels = px(14.);
    pub const HEADING: Pixels = px(15.);
    pub const BRAND: Pixels = px(14.);
    pub const UI_FONT: &str = "Segoe UI";
    pub const MONO_FONT: &str = "Consolas";
}

pub fn control(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
) -> Stateful<Div> {
    button(id, label)
        .text_color(rgb(if selected {
            color::ACCENT_TEXT
        } else {
            color::TEXT
        }))
        .when(selected, |button| button.bg(rgb(color::SELECTED)))
        .hover(move |style| {
            style.bg(rgb(if selected {
                color::SELECTED_HOVER
            } else {
                color::HOVER
            }))
        })
}

pub fn primary_control(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    button(id, label)
        .bg(rgb(color::ACCENT))
        .text_color(rgb(color::SURFACE))
        .hover(|style| style.bg(rgb(color::ACCENT_TEXT)))
}

pub fn danger_control(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    button(id, label)
        .text_color(rgb(color::DANGER))
        .hover(|style| style.bg(rgb(color::WINDOW_CLOSE_HOVER)))
}

fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .h(space::CONTROL_HEIGHT)
        .px(space::SM)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(space::CONTROL_RADIUS)
        .cursor_pointer()
        .child(label.into())
}
