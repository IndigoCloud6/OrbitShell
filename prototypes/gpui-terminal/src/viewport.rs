use alacritty_terminal::{
    grid::Indexed,
    index::{Column, Line, Point as CellPoint, Side},
    term::{
        cell::{Cell, Flags},
        color::Colors,
    },
    vte::ansi::{Color, CursorShape, NamedColor},
};
use gpui::{
    App, BorderStyle, Bounds, Font, FontStyle, FontWeight, Pixels, Point, StrikethroughStyle,
    TextRun, UnderlineStyle, Window, fill, font, outline, point, px, rgb, size,
};

use crate::{
    terminal::{GridSize, Terminal},
    theme,
};

struct Palette {
    background: u32,
    foreground: u32,
    cursor: u32,
    composition: u32,
    search: u32,
    selection: u32,
    ansi: [u32; 16],
}

const LIGHT: Palette = Palette {
    background: theme::color::SURFACE.light_rgb(),
    foreground: theme::color::TEXT.light_rgb(),
    cursor: theme::color::ACCENT.light_rgb(),
    composition: theme::color::SELECTED.light_rgb(),
    search: 0xffe6a3,
    selection: 0xbcd7f5,
    ansi: [
        0x202a38, 0xa73548, 0x277044, 0x805000, 0x2a58a1, 0x823e91, 0x176879, 0xe8edf2, 0x667588,
        0xa02e4c, 0x226942, 0x785000, 0x2c5398, 0x793d85, 0x146473, 0xffffff,
    ],
};

const DARK: Palette = Palette {
    background: 0x14171c,
    foreground: 0xe0e5ed,
    cursor: 0xc0dcfb,
    composition: 0x253850,
    search: 0x59441c,
    selection: 0x365884,
    ansi: [
        0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xd7dce5, 0x798293,
        0xff8590, 0xb2df99, 0xffda92, 0x89c9ff, 0xe299f5, 0x80d5e2, 0xf4f6fa,
    ],
};

pub struct Viewport {
    font: Font,
    font_size: u8,
    cell_width: Pixels,
    pub line_height: Pixels,
    pub dark: bool,
}

impl Viewport {
    pub fn new(window: &Window, font_size: u8) -> Self {
        let font = font(theme::text::MONO_FONT);
        let (cell_width, line_height) = cell_metrics(window, &font, font_size);
        Self {
            font,
            font_size,
            cell_width,
            line_height,
            dark: false,
        }
    }

    pub fn set_font_size(&mut self, window: &Window, font_size: u8) {
        if font_size != self.font_size {
            (self.cell_width, self.line_height) = cell_metrics(window, &self.font, font_size);
            self.font_size = font_size;
        }
    }

    fn palette(&self) -> &'static Palette {
        if self.dark { &DARK } else { &LIGHT }
    }

    pub fn background(&self) -> u32 {
        self.palette().background
    }

    pub fn grid_size(&self, bounds: Bounds<Pixels>) -> GridSize {
        GridSize::new(
            (bounds.size.width / self.cell_width) as usize,
            (bounds.size.height / self.line_height) as usize,
        )
    }

    pub fn selection_position(
        &self,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        terminal: &Terminal,
    ) -> (CellPoint, Side) {
        let x = (position.x - bounds.origin.x) / self.cell_width;
        let y = (position.y - bounds.origin.y) / self.line_height;
        let column = (x.floor() as usize).min(terminal.size.columns - 1);
        let row = (y.floor() as usize).min(terminal.size.rows - 1);
        let side = if x - (column as f32) < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        let line = Line(row as i32 - terminal.content().display_offset as i32);
        (CellPoint::new(line, Column(column)), side)
    }

    pub fn cursor_bounds(
        &self,
        bounds: Bounds<Pixels>,
        terminal: &Terminal,
    ) -> Option<Bounds<Pixels>> {
        let content = terminal.content();
        let row = content.cursor.point.line.0 + content.display_offset as i32;
        (content.cursor.shape != CursorShape::Hidden
            && (0..terminal.size.rows as i32).contains(&row))
        .then(|| self.cell(bounds.origin, content.cursor.point, content.display_offset))
    }

    pub fn range_at_cursor(
        &self,
        cursor: Bounds<Pixels>,
        start: usize,
        width: usize,
    ) -> Bounds<Pixels> {
        Bounds::new(
            point(
                cursor.origin.x + self.cell_width * start as f32,
                cursor.origin.y,
            ),
            size(self.cell_width * width.max(1) as f32, self.line_height),
        )
    }

    pub fn paint(
        &self,
        bounds: Bounds<Pixels>,
        terminal: &Terminal,
        focused: bool,
        composition: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        let content = terminal.content();
        let colors = content.colors;
        let palette = self.palette();
        let background = resolve_color(Color::Named(NamedColor::Background), colors, palette);
        window.paint_quad(fill(bounds, rgb(background)));
        let selected = |cell: &Indexed<&Cell>| {
            content.selection.is_some_and(|selection| {
                selection.contains_cell(cell, content.cursor.point, content.cursor.shape)
            })
        };

        for cell in content.display_iter {
            let color = if selected(&cell) {
                palette.selection
            } else if terminal.search_contains(cell.point) {
                palette.search
            } else {
                let color = if cell.flags.contains(Flags::INVERSE) {
                    cell.fg
                } else {
                    cell.bg
                };
                resolve_color(color, colors, palette)
            };
            if color != background {
                window.paint_quad(fill(
                    self.cell(bounds.origin, cell.point, content.display_offset),
                    rgb(color),
                ));
            }
        }

        let cursor = self.cursor_bounds(bounds, terminal);
        let cursor_color = resolve_color(Color::Named(NamedColor::Cursor), colors, palette);
        let block_cursor =
            focused && content.cursor.shape == CursorShape::Block && composition.is_empty();
        if let Some(cursor) = cursor.filter(|_| block_cursor) {
            window.paint_quad(fill(cursor, rgb(cursor_color)));
        }

        let mut cells = terminal.content().display_iter.peekable();
        while let Some(cell) = cells.next() {
            let mut style = GlyphStyle::of_cell(
                &cell,
                colors,
                palette,
                block_cursor && cell.point == content.cursor.point,
            );
            if selected(&cell) && !(block_cursor && cell.point == content.cursor.point) {
                style.foreground = palette.foreground;
            }
            if !style.visible(cell.c) {
                continue;
            }
            let origin = self
                .cell(bounds.origin, cell.point, content.display_offset)
                .origin;
            let mut text = String::from(cell.c);
            let monospace = cell.c.is_ascii() && cell.zerowidth().is_none();
            if monospace {
                // Shape adjacent ASCII cells together. Wide and combining glyphs keep their grid positions.
                while let Some(next) = cells.peek() {
                    let mut next_style = GlyphStyle::of_cell(
                        next,
                        colors,
                        palette,
                        block_cursor && next.point == content.cursor.point,
                    );
                    if selected(next) && !(block_cursor && next.point == content.cursor.point) {
                        next_style.foreground = palette.foreground;
                    }
                    if next.point.line != cell.point.line
                        || !next.c.is_ascii()
                        || next.zerowidth().is_some()
                        || next_style != style
                    {
                        break;
                    }
                    text.push(next.c);
                    cells.next();
                }
            } else if let Some(combining) = cell.zerowidth() {
                text.extend(combining);
            }
            self.paint_text(text, style, origin, monospace, window, cx);
        }

        if let Some(cursor) = cursor {
            if focused && !composition.is_empty() {
                let line = self.shape_text(
                    composition.into(),
                    GlyphStyle {
                        foreground: palette.foreground,
                        flags: Flags::UNDERLINE,
                    },
                    false,
                    window,
                );
                window.paint_quad(fill(
                    Bounds::new(cursor.origin, size(line.width, self.line_height)),
                    rgb(palette.composition),
                ));
                let _ = line.paint(cursor.origin, self.line_height, window, cx);
            } else if !block_cursor {
                let quad = match (focused, content.cursor.shape) {
                    (true, CursorShape::Beam) => fill(
                        Bounds::new(cursor.origin, size(px(2.), self.line_height)),
                        rgb(cursor_color),
                    ),
                    (true, CursorShape::Underline) => fill(
                        Bounds::new(
                            point(cursor.origin.x, cursor.origin.y + self.line_height - px(2.)),
                            size(self.cell_width, px(2.)),
                        ),
                        rgb(cursor_color),
                    ),
                    _ => outline(cursor, rgb(cursor_color), BorderStyle::Solid),
                };
                window.paint_quad(quad);
            }
        }
    }

    fn cell(&self, origin: Point<Pixels>, cell: CellPoint, offset: usize) -> Bounds<Pixels> {
        Bounds::new(
            point(
                origin.x + self.cell_width * cell.column.0 as f32,
                origin.y + self.line_height * (cell.line.0 + offset as i32) as f32,
            ),
            size(self.cell_width, self.line_height),
        )
    }

    fn paint_text(
        &self,
        text: String,
        style: GlyphStyle,
        origin: Point<Pixels>,
        monospace: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line = self.shape_text(text, style, monospace, window);
        let _ = line.paint(origin, self.line_height, window, cx);
    }

    fn shape_text(
        &self,
        text: String,
        style: GlyphStyle,
        monospace: bool,
        window: &Window,
    ) -> gpui::ShapedLine {
        let mut font = self.font.clone();
        if style.flags.contains(Flags::BOLD) {
            font.weight = FontWeight::BOLD;
        }
        if style.flags.contains(Flags::ITALIC) {
            font.style = FontStyle::Italic;
        }
        let run =
            TextRun {
                len: text.len(),
                font,
                color: rgb(style.foreground).into(),
                background_color: None,
                underline: style.flags.intersects(Flags::ALL_UNDERLINES).then_some(
                    UnderlineStyle {
                        thickness: px(1.),
                        color: None,
                        wavy: style.flags.contains(Flags::UNDERCURL),
                    },
                ),
                strikethrough: style.flags.contains(Flags::STRIKEOUT).then_some(
                    StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    },
                ),
            };
        window.text_system().shape_line(
            text.into(),
            px(f32::from(self.font_size)),
            &[run],
            monospace.then_some(self.cell_width),
        )
    }
}

fn cell_metrics(window: &Window, font: &Font, font_size: u8) -> (Pixels, Pixels) {
    let font_size = f32::from(font_size);
    let text_system = window.text_system();
    let width = text_system
        .advance(text_system.resolve_font(font), px(font_size), 'M')
        .map_or(px(font_size * 0.6), |advance| advance.width.max(px(1.)));
    (width, px((font_size * 22. / 15.).round()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct GlyphStyle {
    foreground: u32,
    flags: Flags,
}

impl GlyphStyle {
    fn of_cell(
        cell: &alacritty_terminal::term::cell::Cell,
        colors: &Colors,
        palette: &Palette,
        cursor: bool,
    ) -> Self {
        let color = if cursor {
            Color::Named(NamedColor::Background)
        } else if cell.flags.contains(Flags::INVERSE) {
            cell.bg
        } else {
            cell.fg
        };
        Self {
            foreground: resolve_color(color, colors, palette),
            flags: cell.flags
                & (Flags::BOLD_ITALIC
                    | Flags::ALL_UNDERLINES
                    | Flags::STRIKEOUT
                    | Flags::HIDDEN
                    | Flags::WIDE_CHAR_SPACER
                    | Flags::LEADING_WIDE_CHAR_SPACER),
        }
    }

    fn visible(self, character: char) -> bool {
        !self
            .flags
            .intersects(Flags::HIDDEN | Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            && (character != ' '
                || self
                    .flags
                    .intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT))
    }
}

fn resolve_color(color: Color, colors: &Colors, palette: &Palette) -> u32 {
    let index = match color {
        Color::Named(color) => color as usize,
        Color::Indexed(index) => usize::from(index),
        Color::Spec(color) => return u32::from_be_bytes([0, color.r, color.g, color.b]),
    };
    if let Some(color) = colors[index] {
        return u32::from_be_bytes([0, color.r, color.g, color.b]);
    }
    match index {
        0..16 => palette.ansi[index],
        16..232 => {
            let index = index - 16;
            let levels = [0, 95, 135, 175, 215, 255];
            u32::from_be_bytes([
                0,
                levels[index / 36],
                levels[index / 6 % 6],
                levels[index % 6],
            ])
        }
        232..256 => (8 + (index as u32 - 232) * 10) * 0x010101,
        index if index == NamedColor::Background as usize => palette.background,
        index if index == NamedColor::Cursor as usize => palette.cursor,
        _ => palette.foreground,
    }
}
