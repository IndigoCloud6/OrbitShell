use std::fmt::Write as _;

use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Boundary, Column, Direction, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{
        Config, MIN_COLUMNS, MIN_SCREEN_LINES, RenderableContent, TermMode,
        cell::Flags,
        search::{Match, RegexSearch},
    },
    vte::ansi::Processor,
};
use tokio::sync::mpsc;
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};
use zeroize::Zeroizing;

pub const BURST_LINES: usize = 40_000;
const LINES_PER_BATCH: usize = 160;
const HISTORY_LINES: usize = 5_000;
const INPUT_LIMIT: usize = 4_096;
const PASTE_LIMIT: usize = 32 * 1024;
const PROMPT: &str = "\x1b[36m本地\x1b[0m > ";
const SAMPLE: [&str; 8] = [
    "# OrbitShell 终端示例",
    "",
    "[terminal]",
    "font = \"Consolas\"",
    "title = \"中文 · café · 日本語\"",
    "wide = \"A中B界C\"",
    "combining = \"e\u{301}\"",
    "scrollback = 5000",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub columns: usize,
    pub rows: usize,
}

impl GridSize {
    pub fn new(columns: usize, rows: usize) -> Self {
        Self {
            columns: columns.max(MIN_COLUMNS),
            rows: rows.max(MIN_SCREEN_LINES),
        }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

struct TerminalEvents(Option<mpsc::Sender<Zeroizing<Vec<u8>>>>);

impl EventListener for TerminalEvents {
    fn send_event(&self, event: Event) {
        if let (Some(input), Event::PtyWrite(reply)) = (&self.0, event) {
            let _ = input.try_send(Zeroizing::new(reply.into_bytes()));
        }
    }
}

pub struct Terminal {
    grid: Term<TerminalEvents>,
    parser: Processor,
    pub size: GridSize,
    selection_row: Option<usize>,
    input: String,
    search_hit: Option<(String, Match)>,
    pub burst_lines: usize,
}

impl Terminal {
    pub fn new(address: &str) -> Self {
        let mut terminal = Self::empty(GridSize::new(80, 24), None);
        terminal.feed(format!("\x1b[1;36m{address}\x1b[0m\r\n\r\n").as_bytes());
        terminal.feed("本地回显。输入文字后按 Enter 换行。\r\n\r\n".as_bytes());
        terminal.feed(PROMPT.as_bytes());
        terminal
    }

    pub fn empty(size: GridSize, input: Option<mpsc::Sender<Zeroizing<Vec<u8>>>>) -> Self {
        let config = Config {
            scrolling_history: HISTORY_LINES,
            ..Config::default()
        };
        Self {
            grid: Term::new(config, &size, TerminalEvents(input)),
            parser: Processor::new(),
            size,
            selection_row: None,
            input: String::new(),
            search_hit: None,
            burst_lines: 0,
        }
    }

    pub fn receive(&mut self, bytes: &[u8]) {
        self.feed(bytes);
    }

    pub fn mode(&self) -> TermMode {
        *self.grid.mode()
    }

    pub fn clear_remote_display(&mut self) {
        self.scroll_to_bottom();
        let mut clear: Processor = Processor::new();
        clear.advance(&mut self.grid, b"\x1b[2J\x1b[3J\x1b[H");
        self.search_hit = None;
    }

    pub fn content(&self) -> RenderableContent<'_> {
        self.grid.renderable_content()
    }

    pub fn start_selection(&mut self, point: Point, side: Side, clicks: usize, extend: bool) {
        if extend && let Some(selection) = self.grid.selection.as_mut() {
            selection.update(point, side);
            return;
        }
        let kind = match clicks {
            0 | 1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            _ => SelectionType::Lines,
        };
        self.grid.selection = Some(Selection::new(kind, point, side));
    }

    pub fn extend_selection(&mut self, point: Point, side: Side) {
        if let Some(selection) = self.grid.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn select_all(&mut self) {
        let first = Point::new(self.grid.grid().topmost_line(), Column(0));
        let last = Point::new(Line(self.size.rows as i32 - 1), self.size.last_column());
        let mut selection = Selection::new(SelectionType::Simple, first, Side::Left);
        selection.update(last, Side::Right);
        self.grid.selection = Some(selection);
    }

    pub fn selected_text(&self) -> Option<String> {
        self.grid
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    pub fn clear_selection(&mut self) {
        self.grid.selection = None;
    }

    pub fn at_shell_prompt(&self) -> bool {
        if self.mode().contains(TermMode::ALT_SCREEN) {
            return false;
        }
        let grid = self.grid.grid();
        let cursor = grid.cursor.point;
        if cursor.column.0 == 0 {
            return false;
        }
        let mut first = cursor.line;
        while first > grid.topmost_line()
            && grid[first - 1usize][self.size.last_column()]
                .flags
                .contains(Flags::WRAPLINE)
        {
            if cursor.line.0 - first.0 >= 16 {
                return false;
            }
            first -= 1;
        }
        let last = if grid.cursor.input_needs_wrap {
            cursor.column
        } else {
            cursor.column - 1usize
        };
        let prompt = self
            .grid
            .bounds_to_string(Point::new(first, Column(0)), Point::new(cursor.line, last));
        if prompt.starts_with("PS ") || prompt.len() > 4096 || prompt.chars().any(char::is_control)
        {
            return false;
        }
        let mut chars = prompt.trim_end().chars();
        let Some(marker) = chars.next_back() else {
            return false;
        };
        let prefix = chars.as_str();
        if prefix.is_empty() {
            return matches!(marker, '$' | '#' | '%' | '❯' | '➜');
        }
        let prefix = prefix
            .strip_prefix('(')
            .and_then(|text| text.split_once(')'))
            .map_or(prefix, |(_, rest)| rest.trim_start());
        // A marker followed by a separator earlier on the same line can be an
        // echoed command, even if that command also happens to end in a marker.
        if prefix.char_indices().any(|(index, c)| {
            matches!(c, '$' | '#') && prefix[index + 1..].starts_with(char::is_whitespace)
        }) {
            return false;
        }
        matches!(marker, '$' | '#')
            && (prefix.starts_with('[') && prefix.ends_with(']')
                || prefix.split_once('@').is_some_and(|(user, host)| {
                    !user.is_empty() && !user.chars().any(char::is_whitespace) && !host.is_empty()
                })
                || prefix.split_once(':').is_some_and(|(host, path)| {
                    !host.is_empty() && !host.chars().any(char::is_whitespace) && !path.is_empty()
                })
                || marker == '#'
                    && prefix
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
    }

    pub fn search_contains(&self, point: Point) -> bool {
        self.search_hit
            .as_ref()
            .is_some_and(|(_, found)| found.contains(&point))
    }

    pub fn find(&mut self, query: &str, backwards: bool) -> bool {
        if query.is_empty() {
            self.search_hit = None;
            return false;
        }
        let mut pattern = String::new();
        for character in query.chars() {
            if r"\.^$|?*+()[]{}".contains(character) {
                pattern.push('\\');
            }
            pattern.push(character);
        }
        let Ok(mut search) = RegexSearch::new(&pattern) else {
            self.search_hit = None;
            return false;
        };
        let previous = self
            .search_hit
            .as_ref()
            .filter(|(text, _)| text == query)
            .map(|(_, hit)| hit);
        let offset = self.grid.grid().display_offset() as i32;
        let origin = if backwards {
            previous.map_or(
                Point::new(
                    Line(self.size.rows as i32 - 1 - offset),
                    self.size.last_column(),
                ),
                |hit| hit.end().sub(&self.grid, Boundary::None, 1),
            )
        } else {
            previous.map_or(Point::new(Line(-offset), Column(0)), |hit| {
                hit.start().add(&self.grid, Boundary::None, 1)
            })
        };
        let (direction, side) = if backwards {
            (Direction::Left, Side::Right)
        } else {
            (Direction::Right, Side::Left)
        };
        self.search_hit = self
            .grid
            .search_next(&mut search, origin, direction, side, None)
            .map(|hit| {
                self.grid.scroll_to_point(*hit.start());
                (query.to_owned(), hit)
            });
        self.search_hit.is_some()
    }

    pub fn visible_text(&self) -> String {
        let offset = self.grid.grid().display_offset() as i32;
        let start = Point::new(Line(-offset), Column(0));
        let end = Point::new(
            Line(self.size.rows as i32 - 1 - offset),
            self.size.last_column(),
        );
        self.grid.bounds_to_string(start, end).trim_end().to_owned()
    }

    pub fn clear_display(&mut self) {
        self.scroll_to_bottom();
        self.feed(b"\x1b[2J\x1b[3J\x1b[H");
        self.feed(PROMPT.as_bytes());
        let input = self.input.clone();
        self.feed(input.as_bytes());
    }

    pub fn has_input(&self) -> bool {
        !self.input.is_empty()
    }

    pub fn resize(&mut self, size: GridSize) {
        if self.size == size {
            return;
        }
        self.grid.resize(size);
        self.size = size;
        self.search_hit = None;
        self.draw_alternate_screen();
    }

    pub fn scroll(&mut self, lines: i32) {
        self.grid.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.grid.scroll_display(Scroll::Bottom);
    }

    pub fn in_alternate_screen(&self) -> bool {
        self.selection_row.is_some()
    }

    pub fn toggle_alternate_screen(&mut self) {
        if self.selection_row.take().is_some() {
            self.feed(b"\x1b[?1049l\x1b[?25h");
        } else {
            self.selection_row = Some(0);
            self.feed(b"\x1b[?1049h\x1b[?25l");
            self.draw_alternate_screen();
        }
    }

    pub fn move_selection(&mut self, offset: isize) {
        if let Some(row) = self.selection_row.as_mut() {
            *row = row.saturating_add_signed(offset).min(SAMPLE.len() - 1);
            self.draw_alternate_screen();
        }
    }

    pub fn insert_text(&mut self, text: &str) {
        if self.in_alternate_screen() {
            match text {
                "j" => self.move_selection(1),
                "k" => self.move_selection(-1),
                "q" => self.toggle_alternate_screen(),
                _ => {}
            }
            return;
        }

        self.scroll_to_bottom();
        let normalized = text[..bounded_end(text, PASTE_LIMIT)]
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        for (index, line) in normalized.split('\n').enumerate() {
            if index > 0 {
                self.submit_line();
            }
            let available = INPUT_LIMIT.saturating_sub(self.input.len());
            let printable: String = line
                .chars()
                .filter(|character| !character.is_control())
                .collect();
            let printable = &printable[..bounded_end(&printable, available)];
            self.input.push_str(printable);
            self.feed(printable.as_bytes());
        }
    }

    pub fn submit_line(&mut self) -> String {
        let input = std::mem::take(&mut self.input);
        self.scroll_to_bottom();
        self.feed(b"\r\n");
        self.feed(PROMPT.as_bytes());
        input
    }

    pub fn backspace(&mut self) {
        let Some((start, grapheme)) = self.input.grapheme_indices(true).next_back() else {
            return;
        };
        let width: usize = grapheme
            .chars()
            .filter_map(|character| character.width())
            .sum();
        self.input.truncate(start);
        self.scroll_to_bottom();

        let cursor = &self.grid.grid().cursor;
        let mut row = cursor.point.line.0 as usize;
        let mut column = cursor.point.column.0 + usize::from(cursor.input_needs_wrap);
        for _ in 0..width {
            if column == 0 {
                if row == 0 {
                    break;
                }
                row -= 1;
                column = self.size.columns;
            }
            column -= 1;
            self.feed(format!("\x1b[{};{}H\x1b[X", row + 1, column + 1).as_bytes());
        }
        // A wide glyph starting on the next row left a spacer at the previous margin.
        if row > 0 && column == 0 {
            let previous = Line(row as i32 - 1);
            let last_column = self.size.last_column();
            if self.grid.grid()[previous][last_column]
                .flags
                .contains(Flags::LEADING_WIDE_CHAR_SPACER)
            {
                self.feed(format!("\x1b[{row};{}H\x1b[X", self.size.columns).as_bytes());
            }
        }
    }

    pub fn begin_burst(&mut self) {
        self.burst_lines = 0;
        self.scroll_to_bottom();
        self.feed("\r\n\x1b[?25l\x1b[36m开始生成本地输出\x1b[0m\r\n".as_bytes());
    }

    pub fn burst_batch(&mut self) -> bool {
        let end = (self.burst_lines + LINES_PER_BATCH).min(BURST_LINES);
        let mut output = String::with_capacity(16 * 1024);
        for line in self.burst_lines + 1..=end {
            let _ = write!(
                output,
                "\x1b[90m{line:05}\x1b[0m  \x1b[32mINFO\x1b[0m  本地模拟输出  A中B界C  café\r\n"
            );
        }
        // Exercise partial UTF-8 and escape sequences across byte chunk boundaries.
        for chunk in output.as_bytes().chunks(1024) {
            self.feed(chunk);
        }
        self.burst_lines = end;
        end < BURST_LINES
    }

    pub fn end_burst(&mut self) {
        self.feed(b"\x1b[?25h");
        self.feed(PROMPT.as_bytes());
        let input = self.input.clone();
        self.feed(input.as_bytes());
    }

    fn draw_alternate_screen(&mut self) {
        let Some(selected) = self.selection_row else {
            return;
        };
        let GridSize { columns, rows } = self.size;
        let mut output = String::from("\x1b[2J\x1b[H\x1b[1;36m");
        output.push_str(fit("orbit-demo.toml", columns));
        output.push_str("\x1b[0m");

        let body_rows = rows.saturating_sub(2);
        let first = (selected + 1).saturating_sub(body_rows);
        for (index, text) in SAMPLE.iter().enumerate().skip(first).take(body_rows) {
            let row = index - first + 2;
            let text = fit(text, columns);
            let _ = write!(output, "\x1b[{row};1H");
            if index == selected {
                output.push_str("\x1b[48;2;54;83;124;38;2;247;250;255m");
                output.push_str(text);
                output.extend(std::iter::repeat_n(
                    ' ',
                    columns.saturating_sub(text.width()),
                ));
                output.push_str("\x1b[0m");
            } else {
                output.push_str(text);
            }
        }
        if rows >= 2 {
            let status = format!(
                " {} / {}  ·  {} 列 × {} 行  ·  ↑ ↓ 移动  ·  Esc 返回 ",
                selected + 1,
                SAMPLE.len(),
                columns,
                rows
            );
            let _ = write!(
                output,
                "\x1b[{rows};1H\x1b[7m{}\x1b[0m",
                fit(&status, columns)
            );
        }
        self.feed(output.as_bytes());
    }

    fn feed(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.search_hit = None;
        }
        self.parser.advance(&mut self.grid, bytes);
    }
}

fn fit(text: &str, columns: usize) -> &str {
    let mut width = 0;
    for (index, grapheme) in text.grapheme_indices(true) {
        width += grapheme.width();
        if width > columns {
            return &text[..index];
        }
    }
    text
}

fn bounded_end(text: &str, limit: usize) -> usize {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}
