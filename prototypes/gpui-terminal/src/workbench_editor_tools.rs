use gpui::{
    Context, Div, ElementId, Entity, FontWeight, Window, deferred, div, prelude::*, px,
};

use super::{EditorTab, Workbench, control, icon_control};
use crate::{
    editor_format::FormatMode,
    editor_input::SearchAction,
    editor_search::SearchQuery,
    icons::Icon,
    text_field::TextField,
    theme::{color, rgb, space, text},
};

pub(super) struct EditorSearch {
    find: Entity<TextField>,
    replace: Entity<TextField>,
    case_sensitive: bool,
    regex: bool,
    whole_word: bool,
    open: bool,
}

impl EditorSearch {
    pub(super) fn visible(&self) -> bool {
        self.open
    }

    fn query(&self, cx: &Context<Workbench>) -> Option<SearchQuery> {
        self.open.then(|| SearchQuery {
            text: self.find.read(cx).value().to_owned(),
            case_sensitive: self.case_sensitive,
            regex: self.regex,
            whole_word: self.whole_word,
        })
    }
}

#[derive(Clone, Copy)]
enum OptionKind {
    Case,
    Regex,
    Word,
}

impl Workbench {
    fn active_editor_mut(&mut self) -> Option<&mut EditorTab> {
        let id = self.editors.active.as_deref()?;
        self.editors.tabs.iter_mut().find(|tab| tab.id == id)
    }

    pub(super) fn open_editor_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_editor_mut() else {
            return;
        };
        tab.format_menu = None;
        let seed = tab
            .input
            .read(cx)
            .selected_text()
            .filter(|value| value.len() <= 100 && !value.contains('\n'));
        let id = tab.id.clone();
        let search = tab.search.get_or_insert_with(|| {
            let find = cx.new(|cx| {
                TextField::new(cx, "查找")
                    .with_limit(4096)
                    .with_tab_index(0)
            });
            let target = id.clone();
            cx.observe(&find, move |view, _, cx| {
                view.update_editor_query(&target, cx)
            })
            .detach();
            EditorSearch {
                find,
                replace: cx.new(|cx| {
                    TextField::new(cx, "替换")
                        .with_limit(4096)
                        .with_tab_index(1)
                }),
                case_sensitive: false,
                regex: false,
                whole_word: false,
                open: false,
            }
        });
        let was_open = search.open;
        search.open = true;
        if !was_open && let Some(seed) = seed {
            let seed = if search.regex {
                regex::escape(&seed)
            } else {
                seed.replace('\\', "\\\\").replace('\t', "\\t")
            };
            search
                .find
                .update(cx, |field, cx| field.set_value(&seed, cx));
        }
        search.find.update(cx, |field, cx| {
            field.focus(window);
            field.select_all();
            cx.notify();
        });
        self.update_editor_query(&id, cx);
    }

    fn update_editor_query(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.editors.tabs.iter().find(|tab| tab.id == id) else {
            return;
        };
        let query = tab.search.as_ref().and_then(|search| search.query(cx));
        tab.input
            .update(cx, |input, cx| input.set_search(query, cx));
        cx.notify();
    }

    fn close_editor_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.active_editor_mut() else {
            return false;
        };
        let Some(search) = tab.search.as_mut().filter(|search| search.open) else {
            return false;
        };
        search.open = false;
        tab.input.update(cx, |input, cx| input.set_search(None, cx));
        tab.focus(window, cx);
        cx.notify();
        true
    }

    fn toggle_editor_search_option(&mut self, option: OptionKind, cx: &mut Context<Self>) {
        let Some(tab) = self.active_editor_mut() else {
            return;
        };
        let Some(search) = tab.search.as_mut() else {
            return;
        };
        let selected = match option {
            OptionKind::Case => &mut search.case_sensitive,
            OptionKind::Regex => &mut search.regex,
            OptionKind::Word => &mut search.whole_word,
        };
        *selected = !*selected;
        let id = tab.id.clone();
        self.update_editor_query(&id, cx);
    }

    fn perform_editor_search(
        &mut self,
        action: SearchAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.editors.active() else {
            return;
        };
        if tab.busy && matches!(action, SearchAction::Replace | SearchAction::ReplaceAll) {
            return;
        }
        let Some(search) = tab.search.as_ref().filter(|search| search.open) else {
            return;
        };
        let replacement = search.replace.read(cx).value().to_owned();
        tab.input
            .update(cx, |view, cx| view.search(action, &replacement, cx));
        if matches!(action, SearchAction::SelectAll) {
            tab.focus(window, cx);
        }
    }

    pub(super) fn format_active_editor(
        &mut self,
        mode: FormatMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_editor_mut().filter(|tab| !tab.busy) else {
            return;
        };
        tab.format_menu = None;
        tab.input
            .update(cx, |input, cx| input.format_file(mode, cx));
        tab.focus(window, cx);
        cx.notify();
    }

    pub(in crate::workbench) fn handle_editor_key(
        &mut self,
        key: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.editors.active() else {
            return false;
        };
        let modifiers = key.modifiers;
        let control = modifiers.control && !modifiers.alt;
        let backwards = if modifiers.shift {
            SearchAction::Previous
        } else {
            SearchAction::Next
        };
        if let Some(index) = tab.format_menu {
            let next = match key.key.as_str() {
                "escape" => None,
                "up" => Some((index + FormatMode::MENU.len() - 1) % FormatMode::MENU.len()),
                "down" => Some((index + 1) % FormatMode::MENU.len()),
                "enter" => {
                    self.format_active_editor(FormatMode::MENU[index].0, window, cx);
                    return true;
                }
                _ => Some(index),
            };
            if matches!(key.key.as_str(), "escape" | "up" | "down") {
                if let Some(tab) = self.active_editor_mut() {
                    tab.format_menu = next;
                    if next.is_none() {
                        tab.focus(window, cx);
                    }
                }
                cx.notify();
                return true;
            }
        }
        match key.key.as_str() {
            "escape" => return self.close_editor_search(window, cx),
            "f" if modifiers.alt && modifiers.shift && !modifiers.control => {
                self.format_active_editor(FormatMode::Auto, window, cx);
            }
            "f" if control => self.open_editor_search(window, cx),
            "g" | "f3" if key.key == "f3" || control => {
                if !tab.search.as_ref().is_some_and(EditorSearch::visible) {
                    self.open_editor_search(window, cx);
                }
                self.perform_editor_search(backwards, window, cx);
            }
            "tab" if !modifiers.control && !modifiers.alt => {
                let Some(search) = tab.search.as_ref().filter(|search| search.open) else {
                    return false;
                };
                if search.find.read(cx).is_focused(window) {
                    search.replace.read(cx).focus(window);
                } else if search.replace.read(cx).is_focused(window) {
                    search.find.read(cx).focus(window);
                } else {
                    return false;
                }
            }
            "enter" => {
                let Some(search) = tab.search.as_ref().filter(|search| search.open) else {
                    return false;
                };
                let action = if search.find.read(cx).is_focused(window)
                    && !search.find.read(cx).is_composing()
                {
                    backwards
                } else if search.replace.read(cx).is_focused(window)
                    && !search.replace.read(cx).is_composing()
                {
                    SearchAction::Replace
                } else {
                    return false;
                };
                self.perform_editor_search(action, window, cx);
            }
            _ => return false,
        }
        true
    }

    pub(super) fn editor_tool_controls(&self, tab: &EditorTab, cx: &Context<Self>) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(space::XS)
            .child(
                icon_control(
                    "editor-find",
                    Icon::Search,
                    "查找 / 替换 · Ctrl+F",
                    tab.search.as_ref().is_some_and(EditorSearch::visible),
                )
                .size(space::CONTROL_HEIGHT)
                .on_click(cx.listener(|view, _, window, cx| view.open_editor_search(window, cx))),
            )
            .when(!tab.busy, |row| {
                row.child(
                    div()
                        .relative()
                        .track_focus(&tab.format_focus)
                        .child(
                            control("editor-format", "格式化 ▾", tab.format_menu.is_some())
                                .on_click(cx.listener(|view, _, window, cx| {
                                    if let Some(tab) = view.active_editor_mut() {
                                        tab.format_menu = if tab.format_menu.is_some() {
                                            None
                                        } else {
                                            Some(0)
                                        };
                                        if tab.format_menu.is_some() {
                                            tab.format_focus.focus(window);
                                        } else {
                                            tab.focus(window, cx);
                                        }
                                        cx.notify();
                                    }
                                })),
                        )
                        .when_some(tab.format_menu, |button, active| {
                            button.child(deferred(
                                div()
                                    .id("editor-format-options")
                                    .absolute()
                                    .top(space::CONTROL_HEIGHT + space::XS)
                                    .right(px(0.))
                                    .w(px(204.))
                                    .flex()
                                    .flex_col()
                                    .p(space::XS)
                                    .occlude()
                                    .shadow_md()
                                    .bg(rgb(color::SURFACE))
                                    .border_1()
                                    .border_color(rgb(color::LINE))
                                    .rounded(space::CONTROL_RADIUS)
                                    .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                                        if let Some(tab) = view.active_editor_mut() {
                                            tab.format_menu = None;
                                            cx.notify();
                                        }
                                    }))
                                    .children(FormatMode::MENU.into_iter().enumerate().map(
                                        |(index, (mode, label))| {
                                            control(
                                                ElementId::named_usize("format-mode", index),
                                                label,
                                                index == active,
                                            )
                                            .justify_start()
                                            .on_click(
                                                cx.listener(move |view, _, window, cx| {
                                                    view.format_active_editor(mode, window, cx)
                                                }),
                                            )
                                        },
                                    )),
                            ))
                        }),
                )
            })
    }

    pub(super) fn editor_search_panel(&self, tab: &EditorTab, cx: &Context<Self>) -> Div {
        let Some(search) = &tab.search else {
            return div();
        };
        let editor = tab.input.read(cx);
        let info = if let Some(notice) = editor.search_notice() {
            notice.to_owned()
        } else {
            match editor.match_count() {
                Some((_, 0)) => "无匹配".into(),
                Some((Some(index), count)) => format!("{index} / {count}"),
                Some((None, count)) => format!("{count} 处匹配"),
                None if !search.find.read(cx).value().is_empty() => "正在查找…".into(),
                _ => String::new(),
            }
        };
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(space::XS)
            .px(space::MD)
            .py(space::SM)
            .bg(rgb(color::TOOLBAR))
            .border_b_1()
            .border_color(rgb(color::LINE))
            .text_size(text::DETAIL)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::XS)
                    .child(div().flex_1().min_w(px(56.)).child(search.find.clone()))
                    .child(editor_search_button(
                        "editor-search-previous",
                        "上一个",
                        SearchAction::Previous,
                        cx,
                    ))
                    .child(editor_search_button(
                        "editor-search-next",
                        "下一个",
                        SearchAction::Next,
                        cx,
                    ))
                    .child(editor_search_button(
                        "editor-search-all",
                        "全部",
                        SearchAction::SelectAll,
                        cx,
                    ))
                    .child(
                        icon_control("editor-search-close", Icon::Close, "关闭查找 / 替换", false)
                            .size(space::CONTROL_HEIGHT)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.close_editor_search(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::XS)
                    .children(
                        [
                            (OptionKind::Case, "区分大小写", search.case_sensitive),
                            (OptionKind::Regex, "正则表达式", search.regex),
                            (OptionKind::Word, "全词匹配", search.whole_word),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(index, (option, label, checked))| {
                            control(
                                ElementId::named_usize("editor-search-option", index),
                                label,
                                checked,
                            )
                            .when(checked, |button| button.font_weight(FontWeight::SEMIBOLD))
                            .on_click(cx.listener(
                                move |view, _, _, cx| view.toggle_editor_search_option(option, cx),
                            ))
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_color(rgb(color::MUTED))
                            .child(info),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::XS)
                    .child(div().flex_1().min_w(px(56.)).child(search.replace.clone()))
                    .child(editor_search_button(
                        "editor-replace-next",
                        "替换",
                        SearchAction::Replace,
                        cx,
                    ))
                    .child(editor_search_button(
                        "editor-replace-all",
                        "全部替换",
                        SearchAction::ReplaceAll,
                        cx,
                    )),
            )
    }
}

fn editor_search_button(
    id: &'static str,
    text: &'static str,
    action: SearchAction,
    cx: &Context<Workbench>,
) -> gpui::Stateful<Div> {
    control(id, text, false).on_click(
        cx.listener(move |view, _, window, cx| view.perform_editor_search(action, window, cx)),
    )
}
