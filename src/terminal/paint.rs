use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use eframe::egui::{self, Color32, Vec2};

use super::{Listener, text};
use crate::editor::code::CodeStyle;

#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    ui: &egui::Ui,
    term: &Term<Listener>,
    rect: egui::Rect,
    origin: egui::Pos2,
    style: &CodeStyle,
    palette: &Palette,
    focused: bool,
    fonts_ready: bool,
) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let background = palette.color(
        term,
        Color::Named(NamedColor::Background),
        NamedColor::Background as usize,
    );
    painter.rect_filled(rect, 0.0, background);

    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let cell_rect = |point: Point, width: usize| {
        let top_left = origin
            + Vec2::new(
                point.column.0 as f32 * style.digit_width,
                (point.line.0 + offset) as f32 * style.row_height,
            );
        egui::Rect::from_min_size(
            top_left,
            Vec2::new(style.digit_width * width as f32, style.row_height),
        )
    };
    let cursor = content.cursor;
    let show_cursor =
        cursor.shape != CursorShape::Hidden && term.mode().contains(TermMode::SHOW_CURSOR);

    for cell in content.display_iter {
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER)
            || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let width = if cell.flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        };
        let rect = cell_rect(cell.point, width);
        let mut fg = palette.cell_color(term, cell.fg, cell.flags, true);
        let mut bg = palette.cell_color(term, cell.bg, cell.flags, false);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if content
            .selection
            .is_some_and(|selection| selection.contains_cell(&cell, cursor.point, cursor.shape))
        {
            bg = ui.visuals().selection.bg_fill;
        }
        let under_cursor = show_cursor
            && focused
            && cell.point == cursor.point
            && cursor.shape == CursorShape::Block;
        if under_cursor {
            (fg, bg) = (
                background,
                palette.color(
                    term,
                    Color::Named(NamedColor::Cursor),
                    NamedColor::Cursor as usize,
                ),
            );
        }
        if bg != background {
            painter.rect_filled(rect, 0.0, bg);
        }
        let pixel = 1.0 / ui.ctx().pixels_per_point();
        if cell.c != ' '
            && !cell.flags.contains(Flags::HIDDEN)
            && !text::draw(&painter, rect, cell.c, fg, pixel)
        {
            let family = text::family(
                fonts_ready,
                cell.flags.contains(Flags::BOLD),
                cell.flags.contains(Flags::ITALIC),
            );
            let format = egui::TextFormat {
                font_id: egui::FontId::new(style.font.size, family),
                color: fg,
                underline: if cell.flags.intersects(Flags::ALL_UNDERLINES) {
                    egui::Stroke::new(1.0, fg)
                } else {
                    egui::Stroke::NONE
                },
                strikethrough: if cell.flags.contains(Flags::STRIKEOUT) {
                    egui::Stroke::new(1.0, fg)
                } else {
                    egui::Stroke::NONE
                },
                ..Default::default()
            };
            let mut text = cell.c.to_string();
            text.extend(cell.zerowidth().into_iter().flatten());
            let galley = ui.fonts_mut(|fonts| {
                fonts.layout_job(egui::text::LayoutJob::single_section(text, format))
            });
            painter.galley(rect.min + Vec2::new(0.0, 1.0), galley, fg);
        }
    }

    if show_cursor && (!focused || cursor.shape != CursorShape::Block) {
        let rect = cell_rect(cursor.point, 1);
        let color = palette.color(
            term,
            Color::Named(NamedColor::Cursor),
            NamedColor::Cursor as usize,
        );
        match cursor.shape {
            CursorShape::Beam => painter.rect_filled(
                egui::Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())),
                0.0,
                color,
            ),
            CursorShape::Underline => painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 2.0), rect.max),
                0.0,
                color,
            ),
            _ => painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, color),
                egui::StrokeKind::Inside,
            ),
        };
    }
}

pub(super) fn screen_text(term: &Term<Listener>) -> String {
    let grid = term.grid();
    let mut lines = Vec::new();
    for line in 0..grid.screen_lines() {
        let row = &grid[Line(line as i32 - grid.display_offset() as i32)];
        let text: String = (0..grid.columns())
            .map(|column| row[Column(column)].c)
            .collect();
        lines.push(text.trim_end().to_string());
    }
    lines.join("\n").trim_end().to_string()
}

pub(super) struct Palette {
    pub(super) dark: bool,
    pub(super) foreground: Color32,
    pub(super) background: Color32,
}

pub(super) const DARK_COLORS: [u32; 16] = [
    0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, 0x666666,
    0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
];
const LIGHT_COLORS: [u32; 16] = [
    0x000000, 0xc91b00, 0x00a600, 0xa08000, 0x0225c7, 0xc930c7, 0x00a5b2, 0xbfbfbf, 0x686868,
    0xff6e67, 0x2fb500, 0xb59c00, 0x6871ff, 0xff77ff, 0x00b8c2, 0xffffff,
];

pub(super) fn rgb(color: u32) -> Color32 {
    Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8)
}

impl Palette {
    pub(super) fn new(visuals: &egui::Visuals) -> Self {
        Self {
            dark: visuals.dark_mode,
            foreground: visuals.text_color(),
            background: visuals.panel_fill,
        }
    }

    pub(super) fn standard(&self, index: usize) -> Color32 {
        let named = if self.dark { DARK_COLORS } else { LIGHT_COLORS };
        match index {
            0..16 => rgb(named[index]),
            16..232 => {
                let level = |value: usize| {
                    if value == 0 {
                        0
                    } else {
                        (55 + value * 40) as u8
                    }
                };
                let index = index - 16;
                Color32::from_rgb(level(index / 36), level(index / 6 % 6), level(index % 6))
            }
            232..256 => Color32::from_gray((8 + (index - 232) * 10) as u8),
            _ if index == NamedColor::Background as usize => self.background,
            _ if index == NamedColor::Cursor as usize => self.foreground,
            _ if index == NamedColor::BrightForeground as usize => self.foreground,
            _ if index == NamedColor::DimForeground as usize => {
                self.foreground.gamma_multiply(0.66)
            }
            _ if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize)
                .contains(&index) =>
            {
                let base = self.standard(index - NamedColor::DimBlack as usize);
                Color32::from_rgb(
                    (base.r() as f32 * 0.66) as u8,
                    (base.g() as f32 * 0.66) as u8,
                    (base.b() as f32 * 0.66) as u8,
                )
            }
            _ => self.foreground,
        }
    }

    pub(super) fn color(&self, term: &Term<Listener>, color: Color, index: usize) -> Color32 {
        match color {
            Color::Spec(Rgb { r, g, b }) => Color32::from_rgb(r, g, b),
            _ => term.colors()[index].map_or_else(
                || self.standard(index),
                |Rgb { r, g, b }| Color32::from_rgb(r, g, b),
            ),
        }
    }

    fn cell_color(
        &self,
        term: &Term<Listener>,
        color: Color,
        flags: Flags,
        foreground: bool,
    ) -> Color32 {
        let index = match color {
            Color::Named(named) => named as usize,
            Color::Indexed(index) => index as usize,
            Color::Spec(_) => 0,
        };
        let index = if foreground
            && flags.contains(Flags::BOLD)
            && index < 8
            && !matches!(color, Color::Spec(_))
        {
            index + 8
        } else {
            index
        };
        let color = self.color(term, color, index);
        if foreground && flags.contains(Flags::DIM) {
            color.gamma_multiply(0.66)
        } else {
            color
        }
    }
}
