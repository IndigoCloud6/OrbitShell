use gpui::{
    Anchored, ClipboardItem, Context, ElementId, KeyDownEvent, Pixels, Point, Window, anchored,
    div, point, prelude::*, px,
};

use super::{FindSelection, SaveSelection, TerminalView};
use crate::theme::{color, rgb, space, text};

#[derive(Clone, Copy)]
enum Action {
    Copy,
    Paste,
    SelectAll,
    Find,
    Save,
    Clear,
}

const ITEMS: [(Action, &str, &str); 6] = [
    (Action::Copy, "复制选中文本", "Ctrl+C"),
    (Action::Paste, "粘贴到终端", "Ctrl+V"),
    (Action::SelectAll, "全选终端内容", ""),
    (Action::Find, "查找选中文本", ""),
    (Action::Save, "保存为快捷指令…", ""),
    (Action::Clear, "清空终端显示", ""),
];

pub(super) struct TerminalMenu {
    position: Point<Pixels>,
    selection: Option<String>,
    cursor: Option<usize>,
}

impl TerminalMenu {
    fn enables(&self, action: Action, can_paste: bool) -> bool {
        match action {
            Action::Copy | Action::Find | Action::Save => self.selection.is_some(),
            Action::Paste => can_paste,
            Action::SelectAll | Action::Clear => true,
        }
    }
}

impl TerminalView {
    pub(super) fn open_terminal_menu(
        &mut self,
        position: Point<Pixels>,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window);
        self.selecting = false;
        let mut menu = TerminalMenu {
            position,
            selection: self.terminal.selected_text(),
            cursor: None,
        };
        if keyboard {
            menu.cursor = ITEMS
                .iter()
                .position(|(action, _, _)| menu.enables(*action, self.input_enabled()));
        }
        self.menu = Some(menu);
        cx.notify();
    }

    pub(super) fn render_terminal_menu(&self, menu: &TerminalMenu, cx: &Context<Self>) -> Anchored {
        anchored()
            .position(menu.position)
            .snap_to_window_with_margin(space::SM)
            .child(
                div()
                    .id("terminal-context-menu")
                    .w(px(216.))
                    .p(space::XS)
                    .bg(rgb(color::SURFACE))
                    .border_1()
                    .border_color(rgb(color::LINE))
                    .rounded(space::CONTROL_RADIUS)
                    .shadow_md()
                    .occlude()
                    .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                        if view.menu.take().is_some() {
                            cx.notify();
                        }
                    }))
                    .children(ITEMS.into_iter().enumerate().map(
                        |(index, (action, label, shortcut))| {
                            let enabled = menu.enables(action, self.input_enabled());
                            div()
                                .when(matches!(action, Action::Find | Action::Clear), |group| {
                                    group.child(
                                        div()
                                            .h(px(1.))
                                            .mx(space::XS)
                                            .my(space::XS)
                                            .bg(rgb(color::LINE)),
                                    )
                                })
                                .child(
                                    div()
                                        .id(ElementId::named_usize("terminal-action", index))
                                        .h(space::CONTROL_HEIGHT)
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap(space::SM)
                                        .px(space::SM)
                                        .rounded(space::CONTROL_RADIUS)
                                        .text_size(text::DETAIL)
                                        .text_color(rgb(if enabled {
                                            color::TEXT
                                        } else {
                                            color::MUTED
                                        }))
                                        .when(menu.cursor == Some(index), |row| {
                                            row.bg(rgb(color::HOVER))
                                        })
                                        .child(label)
                                        .child(div().text_color(rgb(color::MUTED)).child(shortcut))
                                        .when(enabled, |row| {
                                            row.cursor_pointer()
                                                .hover(|style| style.bg(rgb(color::HOVER)))
                                                .on_click(cx.listener(move |view, _, _, cx| {
                                                    view.perform_terminal_action(action, cx)
                                                }))
                                        }),
                                )
                        },
                    )),
            )
    }

    pub(super) fn handle_terminal_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        if key == "menu" || key == "f10" && event.keystroke.modifiers.shift {
            let pos = self
                .surface_bounds
                .map(|bounds| point(bounds.origin.x, bounds.origin.y + space::ROW_HEIGHT))
                .unwrap_or_else(|| window.mouse_position());
            self.open_terminal_menu(pos, true, window, cx);
            return true;
        }
        let can_paste = self.input_enabled();
        let Some(menu) = self.menu.as_mut() else {
            return false;
        };
        match key {
            "down" | "up" => {
                let len = ITEMS.len();
                let back = key == "up";
                let start = menu.cursor.unwrap_or(if back { 0 } else { len - 1 });
                menu.cursor = (1..=len)
                    .map(|offset| (start + if back { len - offset } else { offset }) % len)
                    .find(|index| menu.enables(ITEMS[*index].0, can_paste));
            }
            "enter" => {
                if let Some(index) = menu.cursor {
                    self.perform_terminal_action(ITEMS[index].0, cx);
                }
            }
            "escape" => self.menu = None,
            _ => return false,
        }
        cx.notify();
        true
    }

    fn perform_terminal_action(&mut self, action: Action, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.take() else { return };
        cx.notify();
        if !menu.enables(action, self.input_enabled()) {
            return;
        }
        match action {
            Action::Copy => {
                if let Some(text) = menu.selection {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            Action::Paste => self.paste(cx),
            Action::SelectAll => self.terminal.select_all(),
            Action::Find => {
                if let Some(text) = menu.selection {
                    cx.emit(FindSelection(text));
                }
            }
            Action::Save => {
                if let Some(text) = menu.selection {
                    cx.emit(SaveSelection(text));
                }
            }
            Action::Clear => self.clear_display(cx),
        }
    }
}
