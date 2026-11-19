use gpui::{
    Anchored, ClipboardItem, Context, ElementId, EventEmitter, KeyDownEvent, Pixels, Point, Window,
    anchored, div, prelude::*, px,
};

use super::EditorInput;
use crate::{
    editor_format::FormatMode,
    theme::{color, rgb, space, text},
};

#[derive(Clone, Copy)]
pub(crate) enum EditorToolRequest {
    Search,
    Format(FormatMode),
}
impl EventEmitter<EditorToolRequest> for EditorInput {}

#[derive(Clone, Copy)]
enum Action {
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    SelectAll,
    Format(FormatMode),
    Search,
}

const ITEMS: [(Action, &str, &str); 10] = [
    (Action::Undo, "撤销", "Ctrl+Z"),
    (Action::Redo, "重做", "Ctrl+Y"),
    (Action::Copy, "复制", "Ctrl+C"),
    (Action::Cut, "剪切", "Ctrl+X"),
    (Action::Paste, "粘贴", "Ctrl+V"),
    (Action::SelectAll, "全选", "Ctrl+A"),
    (
        Action::Format(FormatMode::Auto),
        "格式化文件",
        "Shift+Alt+F",
    ),
    (Action::Format(FormatMode::Nginx), "按 Nginx 配置格式化", ""),
    (Action::Format(FormatMode::Json), "按 JSON 格式化", ""),
    (Action::Search, "查找 / 替换", "Ctrl+F"),
];

pub(super) struct EditorMenu {
    position: Point<Pixels>,
    selected: Option<usize>,
}

impl EditorInput {
    pub(super) fn select_all(&mut self) {
        self.highlights_selection.clear();
        self.current_match = None;
        self.anchor = 0;
        self.caret = self.text.len();
    }

    pub(super) fn copy_selection(&mut self, cut: bool, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            if cut && !self.read_only {
                self.replace("", cx);
            }
        }
    }

    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        if !self.read_only
            && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
        {
            self.replace(&text, cx);
        }
    }

    pub(super) fn undo_edit(&mut self, redo: bool, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let edit = if redo {
            self.redo.pop()
        } else {
            self.undo.pop_back()
        };
        if let Some(edit) = edit {
            self.apply_edit(&edit, !redo);
            if redo {
                self.undo.push_back(edit);
            } else {
                self.redo.push(edit);
            }
            self.document_changed(cx);
        }
    }

    fn action_enabled(&self, action: Action) -> bool {
        let selected = !self.selection().is_empty() || !self.highlights_selection.is_empty();
        match action {
            Action::Undo => !self.read_only && !self.undo.is_empty(),
            Action::Redo => !self.read_only && !self.redo.is_empty(),
            Action::Copy => selected,
            Action::Cut => !self.read_only && selected,
            Action::Paste | Action::Format(_) => !self.read_only,
            Action::SelectAll => true,
            Action::Search => !self.read_only,
        }
    }

    pub(super) fn open_menu(
        &mut self,
        position: Point<Pixels>,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus(window);
        self.selection_drag = false;
        let selected = if keyboard {
            ITEMS
                .iter()
                .position(|(action, _, _)| self.action_enabled(*action))
        } else {
            None
        };
        self.menu = Some(EditorMenu { position, selected });
        cx.notify();
    }

    pub(super) fn handle_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.name.is_empty() {
            return false;
        }
        let key = event.keystroke.key.as_str();
        if key == "menu" || key == "f10" && event.keystroke.modifiers.shift {
            let position = self
                .caret_bounds(self.caret)
                .map(|bounds| bounds.bottom_left())
                .unwrap_or_else(|| window.mouse_position());
            self.open_menu(position, true, window, cx);
            return true;
        }
        let Some(menu) = self.menu.as_ref() else {
            return false;
        };
        match key {
            "escape" => self.menu = None,
            "up" | "down" => {
                let backwards = key == "up";
                let length = ITEMS.len();
                let start = menu
                    .selected
                    .unwrap_or(if backwards { 0 } else { length - 1 });
                let selected = (1..=length)
                    .map(|step| (start + if backwards { length - step } else { step }) % length)
                    .find(|index| self.action_enabled(ITEMS[*index].0));
                if let Some(menu) = self.menu.as_mut() {
                    menu.selected = selected;
                }
            }
            "enter" => {
                if let Some(index) = menu.selected {
                    self.perform_menu_action(ITEMS[index].0, cx);
                }
            }
            _ => {
                self.menu = None;
                cx.notify();
                return false;
            }
        }
        cx.notify();
        true
    }

    pub(super) fn render_menu(&self, menu: &EditorMenu, cx: &Context<Self>) -> Anchored {
        anchored()
            .position(menu.position)
            .snap_to_window_with_margin(space::SM)
            .child(
                div()
                    .id("remote-editor-context-menu")
                    .w(px(230.))
                    .p(space::XS)
                    .bg(rgb(color::SURFACE))
                    .border_1()
                    .border_color(rgb(color::LINE))
                    .rounded(space::CONTROL_RADIUS)
                    .occlude()
                    .shadow_md()
                    .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                        if view.menu.take().is_some() {
                            cx.notify();
                        }
                    }))
                    .children(ITEMS.into_iter().enumerate().map(
                        |(index, (action, label, shortcut))| {
                            let enabled = self.action_enabled(action);
                            div()
                                .when(matches!(index, 2 | 6 | 9), |group| {
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
                                        .id(ElementId::named_usize(
                                            "remote-editor-context-action",
                                            index,
                                        ))
                                        .h(space::CONTROL_HEIGHT)
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .px(space::SM)
                                        .text_size(text::DETAIL)
                                        .rounded(space::CONTROL_RADIUS)
                                        .text_color(rgb(if enabled {
                                            color::TEXT
                                        } else {
                                            color::MUTED
                                        }))
                                        .when(menu.selected == Some(index), |row| {
                                            row.bg(rgb(color::HOVER))
                                        })
                                        .child(label)
                                        .child(div().text_color(rgb(color::MUTED)).child(shortcut))
                                        .when(enabled, |row| {
                                            row.cursor_pointer()
                                                .hover(|style| style.bg(rgb(color::HOVER)))
                                                .on_click(cx.listener(move |view, _, _, cx| {
                                                    view.perform_menu_action(action, cx)
                                                }))
                                        }),
                                )
                        },
                    )),
            )
    }

    fn perform_menu_action(&mut self, action: Action, cx: &mut Context<Self>) {
        self.menu = None;
        if self.action_enabled(action) {
            match action {
                Action::Undo => self.undo_edit(false, cx),
                Action::Redo => self.undo_edit(true, cx),
                Action::Copy | Action::Cut => {
                    self.copy_selection(matches!(action, Action::Cut), cx)
                }
                Action::Paste => self.paste(cx),
                Action::SelectAll => self.select_all(),
                Action::Format(mode) => cx.emit(EditorToolRequest::Format(mode)),
                Action::Search => cx.emit(EditorToolRequest::Search),
            }
        }
        cx.notify();
    }
}
