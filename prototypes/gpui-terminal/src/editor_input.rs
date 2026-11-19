use std::{collections::VecDeque, ops::Range, sync::Arc};

use gpui::{
    Bounds, Context, ElementInputHandler, EntityInputHandler, FocusHandle, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollWheelEvent, TextRun,
    UTF16Selection, Window, canvas, div, fill, font, point, prelude::*, px, size,
};
use orbitshell_ssh::remote_editor::MAX_BYTES;
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use crate::{
    editor_search::{SearchQuery, SearchResults},
    editor_syntax::{Language, Span, Token},
    text_field::{utf8_index, utf16_index},
    theme::{ThemeColor, color, rgb, space, text},
};

#[path = "editor_input_menu.rs"]
mod menu;
#[path = "editor_input_tools.rs"]
mod tools;
use menu::EditorMenu;
pub(crate) use menu::EditorToolRequest;
use tools::EditorTask;
pub(crate) use tools::SearchAction;

const LINE_HEIGHT: Pixels = px(21.);
const GUTTER: Pixels = px(52.);

struct Edit {
    start: usize,
    removed: String,
    inserted: String,
    before: [usize; 2],
    after: [usize; 2],
}

pub struct EditorInput {
    text: String,
    line_starts: Vec<usize>,
    anchor: usize,
    caret: usize,
    first_line: usize,
    first_column: usize,
    composition: String,
    undo: VecDeque<Edit>,
    redo: Vec<Edit>,
    selection_drag: bool,
    focus: FocusHandle,
    bounds: Option<Bounds<Pixels>>,
    cell_width: Pixels,
    read_only: bool,
    limit_reached: bool,
    name: String,
    language: Language,
    version: u64,
    highlights: Vec<Span>,
    highlight_task: Option<EditorTask>,
    query: Option<SearchQuery>,
    search_task: Option<EditorTask>,
    results: Option<Arc<SearchResults>>,
    current_match: Option<Range<usize>>,
    search_notice: Option<String>,
    pending_search_action: Option<(SearchAction, String)>,
    highlights_selection: Vec<Range<usize>>,
    format_task: Option<EditorTask>,
    format_notice: Option<(String, bool)>,
    menu: Option<EditorMenu>,
}

impl EditorInput {
    pub fn new(text: String, read_only: bool, cx: &Context<Self>) -> Self {
        let line_starts = line_offsets(&text);
        Self {
            text,
            line_starts,
            anchor: 0,
            caret: 0,
            first_line: 0,
            first_column: 0,
            composition: String::new(),
            undo: VecDeque::new(),
            redo: Vec::new(),
            selection_drag: false,
            focus: cx.focus_handle(),
            bounds: None,
            cell_width: px(8.),
            read_only,
            limit_reached: false,
            name: String::new(),
            language: Language::Plain,
            version: 0,
            highlights: Vec::new(),
            highlight_task: None,
            query: None,
            search_task: None,
            results: None,
            current_match: None,
            search_notice: None,
            pending_search_action: None,
            highlights_selection: Vec::new(),
            format_task: None,
            format_notice: None,
            menu: None,
        }
    }

    pub(crate) fn for_file(
        text: String,
        read_only: bool,
        name: &str,
        path: &str,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut input = Self::new(text, read_only, cx);
        input.name = name.to_owned();
        input.language = Language::for_file(name, path);
        input.schedule_highlight(cx);
        input
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn limit_reached(&self) -> bool {
        self.limit_reached
    }
    pub fn is_composing(&self) -> bool {
        !self.composition.is_empty()
    }

    pub fn replace_all(&mut self, text: &str, cx: &mut Context<Self>) {
        self.apply_change(0..self.text.len(), text.to_owned(), None, cx);
    }
    pub fn focus(&self, window: &mut Window) {
        self.focus.focus(window);
    }

    pub fn reset(&mut self, text: String, cx: &mut Context<Self>) {
        self.text = text;
        self.line_starts = line_offsets(&self.text);
        self.anchor = 0;
        self.caret = 0;
        self.first_line = 0;
        self.first_column = 0;
        self.undo.clear();
        self.redo.clear();
        self.composition.clear();
        self.limit_reached = false;
        self.highlights.clear();
        self.highlights_selection.clear();
        self.current_match = None;
        self.document_changed(cx);
        cx.notify();
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    fn line_of(&self, index: usize) -> usize {
        self.line_starts.partition_point(|&start| start <= index) - 1
    }

    fn line_range(&self, line: usize) -> Range<usize> {
        self.line_starts[line]
            ..self
                .line_starts
                .get(line + 1)
                .map_or(self.text.len(), |next| next - 1)
    }

    fn tab_width(&self) -> usize {
        if self.name.is_empty() {
            4
        } else {
            self.language.tab_width()
        }
    }

    fn column_of(&self, line: usize, byte: usize) -> usize {
        let text = &self.text[self.line_starts[line]..byte];
        text.graphemes(true).fold(0, |column, grapheme| {
            column + cell_count(grapheme, column, self.tab_width())
        })
    }

    fn byte_at(&self, line: usize, target: usize) -> usize {
        let range = self.line_range(line);
        let mut column = 0;
        for (offset, grapheme) in self.text[range.clone()].grapheme_indices(true) {
            let advance = cell_count(grapheme, column, self.tab_width());
            if target < column + advance.div_ceil(2) {
                return range.start + offset;
            }
            column += advance;
        }
        range.end
    }

    fn visible_size(&self, bounds: Bounds<Pixels>) -> (usize, usize) {
        (
            ((bounds.size.height / LINE_HEIGHT).floor() as usize).max(1),
            (((bounds.size.width - GUTTER - space::SM) / self.cell_width).floor() as usize).max(1),
        )
    }

    fn reveal(&mut self) {
        let Some(bounds) = self.bounds else { return };
        let (rows, columns) = self.visible_size(bounds);
        let line = self.line_of(self.caret);
        self.first_line = self
            .first_line
            .min(line)
            .max((line + 1).saturating_sub(rows));
        let column = self.column_of(line, self.caret);
        self.first_column = self
            .first_column
            .min(column)
            .max((column + 1).saturating_sub(columns));
    }

    fn move_caret(&mut self, next: usize, extend: bool) {
        self.highlights_selection.clear();
        self.current_match = None;
        self.caret = next;
        if !extend {
            self.anchor = next;
        }
        self.composition.clear();
        self.reveal();
    }

    fn replace(&mut self, input: &str, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let inserted: String = input
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .chars()
            .filter(|c| !c.is_control() || matches!(*c, '\t' | '\n'))
            .collect();
        self.composition.clear();
        if !self.highlights_selection.is_empty() {
            self.replace_selected_matches(&inserted, cx);
            return;
        }
        self.apply_change(self.selection(), inserted, None, cx);
    }

    fn apply_change(
        &mut self,
        range: Range<usize>,
        inserted: String,
        selection: Option<[usize; 2]>,
        cx: &mut Context<Self>,
    ) {
        self.limit_reached = self.text.len() - range.len() + inserted.len() > MAX_BYTES;
        if self.limit_reached || &self.text[range.clone()] == inserted.as_str() {
            cx.notify();
            return;
        }
        let end = range.start + inserted.len();
        let edit = Edit {
            start: range.start,
            removed: self.text[range].to_owned(),
            inserted,
            before: [self.anchor, self.caret],
            after: selection.unwrap_or([end, end]),
        };
        self.apply_edit(&edit, false);
        self.redo.clear();
        self.undo.push_back(edit);
        while self.undo.len() > 64
            || self
                .undo
                .iter()
                .map(|edit| edit.inserted.len() + edit.removed.len())
                .sum::<usize>()
                > MAX_BYTES * 3
        {
            self.undo.pop_front();
        }
        self.document_changed(cx);
        cx.notify();
    }

    fn apply_edit(&mut self, edit: &Edit, undo: bool) {
        let (before, after) = if undo {
            (&edit.inserted, &edit.removed)
        } else {
            (&edit.removed, &edit.inserted)
        };
        let end = edit.start + before.len();
        self.highlights.retain_mut(|span| {
            if span.range.end <= edit.start {
                return true;
            }
            if span.range.start < end {
                return false;
            }
            span.range = (span.range.start - before.len() + after.len())
                ..(span.range.end - before.len() + after.len());
            true
        });
        self.text
            .replace_range(edit.start..edit.start + before.len(), after);
        self.line_starts = line_offsets(&self.text);
        self.limit_reached = false;
        let [anchor, caret] = if undo { edit.before } else { edit.after };
        self.move_caret(caret, false);
        self.anchor = anchor;
    }

    fn previous(&self) -> usize {
        self.text[..self.caret]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(offset, _)| offset)
    }
    fn next(&self) -> usize {
        self.caret
            + self.text[self.caret..]
                .graphemes(true)
                .next()
                .map_or(0, str::len)
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.composition.is_empty() {
            return;
        }
        if self.handle_menu_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        let modifiers = event.keystroke.modifiers;
        let control = modifiers.control && !modifiers.alt;
        let shift = modifiers.shift;
        let key = event.keystroke.key.as_str();
        let selection = self.selection();
        let line = self.line_of(self.caret);
        match key {
            "a" if control => self.select_all(),
            "c" | "x" if control => self.copy_selection(key == "x", cx),
            "v" if control => self.paste(cx),
            "z" if control && !shift => self.undo_edit(false, cx),
            "y" | "z" if control => self.undo_edit(true, cx),
            "left" if !control => self.move_caret(
                if !shift && !selection.is_empty() {
                    selection.start
                } else {
                    self.previous()
                },
                shift,
            ),
            "right" if !control => self.move_caret(
                if !shift && !selection.is_empty() {
                    selection.end
                } else {
                    self.next()
                },
                shift,
            ),
            "home" => self.move_caret(if control { 0 } else { self.line_starts[line] }, shift),
            "end" => self.move_caret(
                if control {
                    self.text.len()
                } else {
                    self.line_range(line).end
                },
                shift,
            ),
            "up" | "down" | "pageup" | "pagedown" => {
                let rows = self.bounds.map_or(1, |bounds| self.visible_size(bounds).0);
                let target = match key {
                    "up" => line.saturating_sub(1),
                    "down" => line + 1,
                    "pageup" => line.saturating_sub(rows),
                    _ => line + rows,
                }
                .min(self.line_starts.len() - 1);
                self.move_caret(
                    self.byte_at(target, self.column_of(line, self.caret)),
                    shift,
                );
            }
            "enter" if !control => self.replace("\n", cx),
            "tab" if !control && !shift && !self.read_only => {
                if self.name.is_empty() {
                    self.replace("\t", cx);
                } else {
                    self.replace(&" ".repeat(self.language.tab_width()), cx);
                }
            }
            "backspace" | "delete" if !control && !self.read_only => {
                if selection.is_empty() && self.highlights_selection.is_empty() {
                    self.anchor = if key == "backspace" {
                        self.previous()
                    } else {
                        self.next()
                    };
                }
                self.replace("", cx);
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn point_to_index(&self, position: Point<Pixels>) -> Option<usize> {
        let bounds = self.bounds?;
        let line = (self.first_line
            + ((position.y - bounds.origin.y) / LINE_HEIGHT).floor() as usize)
            .min(self.line_starts.len() - 1);
        let column = self.first_column
            + ((position.x - bounds.origin.x - GUTTER) / self.cell_width).round() as usize;
        Some(self.byte_at(line, column))
    }

    fn begin_selection(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus(window);
        self.menu = None;
        if let Some(index) = self.point_to_index(event.position) {
            self.move_caret(index, event.modifiers.shift);
            if event.click_count >= 3 {
                let range = self.line_range(self.line_of(index));
                self.anchor = range.start;
                self.caret = range.end;
            } else if event.click_count == 2 {
                let line = self.line_range(self.line_of(index));
                if let Some((offset, word)) = self.text[line.clone()]
                    .split_word_bound_indices()
                    .find(|(offset, word)| {
                        (line.start + offset..line.start + offset + word.len()).contains(&index)
                    })
                {
                    self.anchor = line.start + offset;
                    self.caret = self.anchor + word.len();
                }
            }
        }
        self.selection_drag = true;
        cx.notify();
    }

    fn drag_selection(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection_drag
            && event.dragging()
            && let Some(index) = self.point_to_index(event.position)
        {
            self.move_caret(index, true);
            cx.notify();
        }
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(LINE_HEIGHT);
        let lines = (delta.y / LINE_HEIGHT).round() as isize;
        let columns = (delta.x / self.cell_width).round() as isize;
        if event.modifiers.shift {
            self.first_column = self.first_column.saturating_add_signed(-lines);
        } else {
            self.first_line = self
                .first_line
                .saturating_add_signed(-lines)
                .min(self.line_starts.len() - 1);
        }
        self.first_column = self
            .first_column
            .saturating_add_signed(-columns)
            .min(MAX_BYTES);
        cx.notify();
    }

    fn caret_bounds(&self, byte: usize) -> Option<Bounds<Pixels>> {
        let bounds = self.bounds?;
        let line = self.line_of(byte);
        let column = self.column_of(line, byte);
        Some(Bounds::new(
            point(
                bounds.origin.x
                    + GUTTER
                    + self.cell_width * (column as f32 - self.first_column as f32),
                bounds.origin.y + LINE_HEIGHT * (line as f32 - self.first_line as f32),
            ),
            size(px(1.), LINE_HEIGHT),
        ))
    }

    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let (rows, columns) = self.visible_size(bounds);
        let focused = self.focus.is_focused(window) && window.is_window_active();
        let selection = self.selection();
        window.paint_quad(fill(
            Bounds::new(bounds.origin, size(GUTTER - space::SM, bounds.size.height)),
            rgb(color::TOOLBAR),
        ));
        for line in self.first_line..(self.first_line + rows + 1).min(self.line_starts.len()) {
            let y = bounds.origin.y + LINE_HEIGHT * (line - self.first_line) as f32;
            let range = self.line_range(line);
            let right = self.first_column + columns + 1;
            let mut column = 0;
            let mut first = None;
            let mut glyphs = String::new();
            let mut runs: Vec<TextRun> = Vec::new();
            let mut cells = Vec::new();
            let mut last_byte = range.start;
            let mut run_color = color::TEXT;
            let mut tokens = self.highlights[self
                .highlights
                .partition_point(|span| span.range.end <= range.start)..]
                .iter()
                .peekable();
            for (offset, grapheme) in self.text[range.clone()].grapheme_indices(true) {
                if column >= right {
                    break;
                }
                let byte = range.start + offset;
                let width = cell_count(grapheme, column, self.language.tab_width());
                if column + width > self.first_column {
                    first.get_or_insert(column);
                    cells.push((byte, column));
                    while tokens.peek().is_some_and(|span| span.range.end <= byte) {
                        tokens.next();
                    }
                    let tint = tokens
                        .peek()
                        .filter(|span| span.range.contains(&byte))
                        .map_or(color::TEXT, |span| token_color(span.token));
                    let previous_len = glyphs.len();
                    if grapheme == "\t" {
                        glyphs.extend(std::iter::repeat_n(' ', width));
                    } else {
                        glyphs.push_str(grapheme);
                    }
                    let len = glyphs.len() - previous_len;
                    if let Some(last) = runs.last_mut().filter(|_| tint == run_color) {
                        last.len += len;
                    } else {
                        runs.push(text_run(len, tint));
                        run_color = tint;
                    }
                }
                column += width;
                last_byte = byte + grapheme.len();
            }
            if column >= self.first_column {
                cells.push((last_byte, column));
                let visible = cells[0].0..last_byte;
                let first_column = self.first_column;
                let draw = |window: &mut Window, hit: &Range<usize>, tint| {
                    let from = cells
                        .partition_point(|&(byte, _)| byte <= hit.start)
                        .saturating_sub(1);
                    let to = cells
                        .partition_point(|&(byte, _)| byte < hit.end)
                        .min(cells.len() - 1);
                    let left = cells[from].1.max(first_column);
                    let end = (cells[to].1 + usize::from(hit.is_empty() || hit.end > range.end))
                        .min(right);
                    if end > left {
                        let origin = point(
                            bounds.origin.x
                                + GUTTER
                                + self.cell_width * (left - first_column) as f32,
                            y,
                        );
                        window.paint_quad(fill(
                            Bounds::new(
                                origin,
                                size(self.cell_width * (end - left) as f32, LINE_HEIGHT),
                            ),
                            rgb(tint),
                        ));
                    }
                };
                if let Some(search) = &self.results {
                    for hit in line_ranges(&search.ranges, &visible) {
                        draw(
                            window,
                            hit,
                            if self.current_match.as_ref() == Some(hit) {
                                color::EDITOR_CURRENT_MATCH
                            } else {
                                color::EDITOR_MATCH
                            },
                        );
                    }
                }
                for hit in line_ranges(&self.highlights_selection, &visible) {
                    draw(window, hit, color::SELECTED);
                }
                if self.current_match.as_ref() != Some(&selection)
                    && !selection.is_empty()
                    && selection.start <= visible.end
                    && selection.end > visible.start
                {
                    draw(window, &selection, color::SELECTED);
                }
            }
            if let Some(first) = first {
                let x = bounds.origin.x
                    + GUTTER
                    + self.cell_width * (first as f32 - self.first_column as f32);
                window.with_content_mask(
                    Some(gpui::ContentMask {
                        bounds: Bounds::new(
                            point(bounds.origin.x + GUTTER, bounds.origin.y),
                            size((bounds.size.width - GUTTER).max(px(0.)), bounds.size.height),
                        ),
                    }),
                    |window| {
                        let _ = window
                            .text_system()
                            .shape_line(glyphs.into(), text::BODY, &runs, None)
                            .paint(point(x, y), LINE_HEIGHT, window, cx);
                    },
                );
            }
            let number = shape(window, (line + 1).to_string(), color::MUTED);
            let _ = number.paint(
                point(bounds.origin.x + GUTTER - space::LG - number.width, y),
                LINE_HEIGHT,
                window,
                cx,
            );
        }
        if focused
            && let Some(caret) = self.caret_bounds(self.caret)
            && caret.origin.x >= bounds.origin.x + GUTTER
            && bounds.contains(&caret.origin)
        {
            if self.composition.is_empty() {
                window.paint_quad(fill(caret, rgb(color::ACCENT)));
            } else {
                let line = shape(window, self.composition.clone(), color::TEXT);
                window.paint_quad(fill(
                    Bounds::new(caret.origin, size(line.width, LINE_HEIGHT)),
                    rgb(color::SELECTED),
                ));
                let _ = line.paint(caret.origin, LINE_HEIGHT, window, cx);
            }
        }
    }
}

impl Render for EditorInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .id("remote-editor-text")
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .overflow_hidden()
            .bg(rgb(color::SURFACE))
            .cursor(gpui::CursorStyle::IBeam)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll_wheel(cx.listener(Self::scroll))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_selection))
            .when(!self.name.is_empty(), |root| {
                root.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|view, event: &MouseDownEvent, window, cx| {
                        view.open_menu(event.position, false, window, cx);
                        cx.stop_propagation();
                    }),
                )
            })
            .on_mouse_move(cx.listener(Self::drag_selection))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, _| view.selection_drag = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, _| view.selection_drag = false),
            )
            .when_some(self.menu.as_ref(), |root, menu| {
                root.child(gpui::deferred(self.render_menu(menu, cx)))
            })
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, cx| {
                        entity.update(cx, |view, cx| {
                            view.bounds = Some(bounds);
                            view.cell_width =
                                shape(window, "M".into(), color::TEXT).width.max(px(1.));
                            if view.focus.is_focused(window) && !view.read_only {
                                window.handle_input(
                                    &view.focus,
                                    ElementInputHandler::new(bounds, cx.entity()),
                                    cx,
                                );
                            }
                            view.paint(bounds, window, cx);
                        });
                    },
                )
                .size_full(),
            )
    }
}

impl EntityInputHandler for EditorInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let start = utf8_index(&self.text, range.start);
        let end = utf8_index(&self.text, range.end).max(start);
        *adjusted = Some(utf16_index(&self.text, start)..utf16_index(&self.text, end));
        Some(self.text[start..end].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = self.selection();
        Some(UTF16Selection {
            range: utf16_index(&self.text, range.start)..utf16_index(&self.text, range.end),
            reversed: self.caret < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let start = utf16_index(&self.text, self.selection().start);
        (!self.composition.is_empty())
            .then(|| start..start + self.composition.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.composition.clear();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        input: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composition.is_empty()
            && let Some(range) = range
        {
            self.anchor = utf8_index(&self.text, range.start);
            self.caret = utf8_index(&self.text, range.end);
        }
        self.replace(input, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        input: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composition.is_empty()
            && let Some(range) = range
        {
            self.anchor = utf8_index(&self.text, range.start);
            self.caret = utf8_index(&self.text, range.end);
        }
        self.composition = input.chars().filter(|c| !c.is_control()).collect();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.caret_bounds(utf8_index(&self.text, range.start))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.point_to_index(position)
            .map(|index| utf16_index(&self.text, index))
    }
}

fn line_offsets(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(offset, _)| offset + 1))
        .collect()
}

fn cell_count(grapheme: &str, column: usize, tab_width: usize) -> usize {
    if grapheme == "\t" {
        tab_width - column % tab_width
    } else {
        grapheme.width().max(1)
    }
}

fn shape(window: &Window, text: String, color: ThemeColor) -> gpui::ShapedLine {
    let run = text_run(text.len(), color);
    window
        .text_system()
        .shape_line(text.into(), text::BODY, &[run], None)
}

fn text_run(len: usize, color: ThemeColor) -> TextRun {
    TextRun {
        len,
        font: font(crate::theme::text::MONO_FONT),
        color: rgb(color).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn token_color(token: Token) -> ThemeColor {
    match token {
        Token::Comment => color::EDITOR_COMMENT,
        Token::Keyword => color::EDITOR_KEYWORD,
        Token::String => color::EDITOR_STRING,
        Token::Number => color::EDITOR_NUMBER,
        Token::Property => color::EDITOR_PROPERTY,
    }
}

fn line_ranges<'a>(
    ranges: &'a [Range<usize>],
    line: &'a Range<usize>,
) -> impl Iterator<Item = &'a Range<usize>> {
    ranges[ranges.partition_point(|hit| hit.end < line.start)..]
        .iter()
        .take_while(|hit| hit.start <= line.end)
        .filter(|hit| hit.end > line.start || hit.is_empty())
}
