use std::ops::Range;

use gpui::{
    Bounds, Context, ElementInputHandler, EntityInputHandler, FocusHandle, KeyDownEvent,
    MouseButton, Pixels, Point, ShapedLine, TextRun, UTF16Selection, Window, canvas, div, fill,
    font, point, prelude::*, px, size,
};
use unicode_segmentation::UnicodeSegmentation as _;
use zeroize::{Zeroize, Zeroizing};

use crate::theme::{ThemeColor, color, rgb, space, text};

pub struct TextField {
    value: Zeroizing<String>,
    selection: Range<usize>,
    composition: Zeroizing<String>,
    placeholder: &'static str,
    focus: FocusHandle,
    secret: bool,
    limit: usize,
}

impl TextField {
    pub fn new(cx: &mut Context<Self>, placeholder: &'static str) -> Self {
        Self {
            value: Zeroizing::default(),
            selection: 0..0,
            composition: Zeroizing::default(),
            placeholder,
            focus: cx.focus_handle(),
            secret: false,
            limit: 256,
        }
    }

    pub fn value(&self) -> &str {
        if self.secret { "" } else { &self.value }
    }

    pub fn password(cx: &mut Context<Self>) -> Self {
        Self::secret(cx, "输入密码或私钥口令", 16 * 1024)
    }

    pub fn secret(cx: &mut Context<Self>, placeholder: &'static str, limit: usize) -> Self {
        Self {
            secret: true,
            limit,
            ..Self::new(cx, placeholder)
        }
    }

    pub fn has_value(&self) -> bool {
        !self.value.is_empty()
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    pub fn with_tab_index(mut self, index: isize) -> Self {
        self.focus = self.focus.tab_index(index).tab_stop(true);
        self
    }

    pub fn take_secret(&mut self, cx: &mut Context<Self>) -> Zeroizing<String> {
        self.selection = 0..0;
        self.composition.zeroize();
        cx.notify();
        std::mem::take(&mut self.value)
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus.focus(window);
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    pub fn is_composing(&self) -> bool {
        !self.composition.is_empty()
    }

    pub fn select_all(&mut self) {
        self.selection = 0..self.value.len();
    }

    pub fn set_value(&mut self, value: &str, cx: &mut Context<Self>) {
        self.value.zeroize();
        self.selection = 0..0;
        self.replace(value, cx);
    }

    fn replace(&mut self, value: &str, cx: &mut Context<Self>) {
        let remaining = self.limit.saturating_sub(
            self.value.chars().count() - self.value[self.selection.clone()].chars().count(),
        );
        let value = Zeroizing::new(
            value
                .chars()
                .filter(|character| !character.is_control())
                .take(remaining)
                .collect::<String>(),
        );
        let cursor = self.selection.start + value.len();
        if self.secret {
            let mut next = Zeroizing::new(String::with_capacity(
                self.value.len() - self.selection.len() + value.len(),
            ));
            next.push_str(&self.value[..self.selection.start]);
            next.push_str(&value);
            next.push_str(&self.value[self.selection.end..]);
            self.value = next;
        } else {
            self.value.replace_range(self.selection.clone(), &value);
        }
        self.selection = cursor..cursor;
        self.composition.zeroize();
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let control = event.keystroke.modifiers.control && !event.keystroke.modifiers.alt;
        let previous = || {
            self.value[..self.selection.start]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(index, _)| index)
        };
        let next = || {
            self.selection.end
                + self.value[self.selection.end..]
                    .graphemes(true)
                    .next()
                    .map_or(0, str::len)
        };
        match event.keystroke.key.as_str() {
            "a" if control => self.select_all(),
            "v" if control => {
                if let Some(input) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace(&Zeroizing::new(input), cx);
                }
            }
            "c" | "x" if control => {
                if !self.secret && !self.selection.is_empty() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        self.value[self.selection.clone()].to_owned(),
                    ));
                    if event.keystroke.key == "x" {
                        self.replace("", cx);
                    }
                }
            }
            "backspace" => {
                if self.selection.is_empty() {
                    self.selection.start = previous();
                }
                self.replace("", cx);
            }
            "delete" => {
                if self.selection.is_empty() {
                    self.selection.end = next();
                }
                self.replace("", cx);
            }
            "left" => {
                let cursor = if self.selection.is_empty() {
                    previous()
                } else {
                    self.selection.start
                };
                self.selection = cursor..cursor;
            }
            "right" => {
                let cursor = if self.selection.is_empty() {
                    next()
                } else {
                    self.selection.end
                };
                self.selection = cursor..cursor;
            }
            "home" => self.selection = 0..0,
            "end" => {
                let end = self.value.len();
                self.selection = end..end;
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let empty = self.value.is_empty() && self.composition.is_empty();
        let display = if empty {
            self.placeholder.to_owned()
        } else if self.secret {
            let count = self.value.chars().count() + self.composition.chars().count();
            let replaced = if self.composition.is_empty() {
                0
            } else {
                self.value[self.selection.clone()].chars().count()
            };
            "•".repeat(count - replaced)
        } else {
            format!(
                "{}{}{}",
                &self.value[..self.selection.start],
                self.composition.as_str(),
                &self.value[if self.composition.is_empty() {
                    self.selection.start
                } else {
                    self.selection.end
                }..]
            )
        };
        let offset = |index| {
            if self.secret {
                self.value[..index].chars().count() * '•'.len_utf8()
            } else {
                index
            }
        };
        let selected = if empty || !self.composition.is_empty() {
            0..0
        } else {
            offset(self.selection.start)..offset(self.selection.end)
        };
        let cursor = if !self.composition.is_empty() {
            offset(self.selection.start)
                + if self.secret {
                    self.composition.chars().count() * '•'.len_utf8()
                } else {
                    self.composition.len()
                }
        } else if empty {
            0
        } else {
            offset(self.selection.end)
        };
        let entity = cx.entity();
        let focus = self.focus.clone();

        div()
            .w_full()
            .h(space::ROW_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .px(space::SM)
            .rounded(space::CONTROL_RADIUS)
            .border_1()
            .border_color(rgb(if focused { color::ACCENT } else { color::LINE }))
            .bg(rgb(color::SURFACE))
            .overflow_hidden()
            .cursor(gpui::CursorStyle::IBeam)
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, _| view.focus(window)),
            )
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, cx| {
                        let line = shape(
                            window,
                            display,
                            if empty { color::MUTED } else { color::TEXT },
                        );
                        let caret = line.x_for_index(cursor);
                        let scroll = (caret - bounds.size.width + px(2.)).max(px(0.));
                        let origin = point(bounds.origin.x - scroll, bounds.origin.y);
                        if focused {
                            window.handle_input(
                                &focus,
                                ElementInputHandler::new(bounds, entity),
                                cx,
                            );
                            if !selected.is_empty() {
                                let left = line.x_for_index(selected.start);
                                let right = line.x_for_index(selected.end);
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(origin.x + left, origin.y + px(5.)),
                                        size(right - left, px(21.)),
                                    ),
                                    rgb(color::SELECTED),
                                ));
                            } else {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(origin.x + caret, origin.y + px(7.)),
                                        size(px(1.), px(18.)),
                                    ),
                                    rgb(color::ACCENT),
                                ));
                            }
                        }
                        let _ = line.paint(origin, bounds.size.height, window, cx);
                    },
                )
                .size_full(),
            )
    }
}

fn shape(window: &Window, value: String, tint: ThemeColor) -> ShapedLine {
    let run = TextRun {
        len: value.len(),
        font: font(text::UI_FONT),
        color: rgb(tint).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(value.into(), text::BODY, &[run], None)
}

pub(crate) fn utf16_index(text: &str, byte: usize) -> usize {
    text[..byte].encode_utf16().count()
}

pub(crate) fn utf8_index(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        units += character.len_utf16();
        if units > utf16 {
            return byte;
        }
    }
    text.len()
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        if self.secret {
            return None;
        }
        let start = utf8_index(&self.value, range.start);
        let end = utf8_index(&self.value, range.end).max(start);
        *adjusted = Some(utf16_index(&self.value, start)..utf16_index(&self.value, end));
        Some(self.value[start..end].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: utf16_index(&self.value, self.selection.start)
                ..utf16_index(&self.value, self.selection.end),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let start = utf16_index(&self.value, self.selection.start);
        (!self.composition.is_empty())
            .then(|| start..start + self.composition.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.composition.zeroize();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        value: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composition = Zeroizing::new(
            value
                .chars()
                .filter(|character| !character.is_control())
                .take(self.limit)
                .collect(),
        );
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let index = utf8_index(&self.value, range.start);
        let prefix = if self.secret {
            "•".repeat(self.value[..index].chars().count())
        } else {
            self.value[..index].to_owned()
        };
        let width = shape(window, prefix, color::TEXT)
            .width
            .min(bounds.size.width);
        Some(Bounds::new(
            point(bounds.origin.x + width, bounds.origin.y),
            size(px(1.), bounds.size.height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
