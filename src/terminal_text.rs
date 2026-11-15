use std::fs;
use std::sync::Arc;

use eframe::egui::epaint::QuadraticBezierShape;
use eframe::egui::{self, Color32, FontData, FontFamily, Painter, Pos2, Rect, Stroke, Vec2};

const MENLO: &str = "/System/Library/Fonts/Menlo.ttc";

const FALLBACKS: [(&str, f32); 2] = [
    ("/System/Library/Fonts/Supplemental/STIXTwoMath.otf", 0.25),
    ("/System/Library/Fonts/Apple Symbols.ttf", 0.15),
];

pub fn family(fonts_ready: bool, bold: bool, italic: bool) -> FontFamily {
    if !fonts_ready {
        return FontFamily::Monospace;
    }
    let name = match (bold, italic) {
        (false, false) => "terminal",
        (true, false) => "terminal-bold",
        (false, true) => "terminal-italic",
        (true, true) => "terminal-bold-italic",
    };
    FontFamily::Name(name.into())
}

pub fn fonts_ready(ctx: &egui::Context) -> bool {
    if ctx.fonts(|fonts| {
        fonts
            .definitions()
            .families
            .contains_key(&family(true, false, false))
    }) {
        return true;
    }
    let id = egui::Id::unique("terminal fonts added");
    if !ctx.data_mut(|data| std::mem::replace(data.get_temp_mut_or_default::<bool>(id), true)) {
        install_fonts(ctx);
    }
    false
}

fn install_fonts(ctx: &egui::Context) {
    let mut definitions = ctx.fonts(|fonts| fonts.definitions().clone());
    let mut fallbacks: Vec<String> = FALLBACKS
        .iter()
        .filter_map(|(path, y_offset_factor)| {
            let font = fs::read(path).ok()?;
            let tweak = egui::FontTweak {
                y_offset_factor: *y_offset_factor,
                ..Default::default()
            };
            definitions.font_data.insert(
                path.to_string(),
                Arc::new(FontData::from_owned(font).tweak(tweak)),
            );
            Some(path.to_string())
        })
        .collect();
    fallbacks.extend(definitions.families[&FontFamily::Monospace].clone());
    let menlo: Option<egui::epaint::text::Blob> =
        fs::read(MENLO).ok().map(|font| Arc::new(font) as _);
    for (index, (bold, italic)) in [(false, false), (true, false), (false, true), (true, true)]
        .into_iter()
        .enumerate()
    {
        let mut fonts = fallbacks.clone();
        if let Some(menlo) = &menlo {
            let name = format!("{MENLO}#{index}");
            definitions.font_data.insert(
                name.clone(),
                Arc::new(FontData::from_blob(menlo.clone(), index as u32)),
            );
            fonts.insert(0, name);
        }
        definitions
            .families
            .insert(family(true, bold, italic), fonts);
    }
    ctx.set_fonts(definitions);
}

pub fn draw(painter: &Painter, cell: Rect, c: char, color: Color32, pixel: f32) -> bool {
    let cell = Rect::from_min_max(snap(cell.min, pixel), snap(cell.max, pixel));
    match c {
        '\u{2571}'..='\u{2573}' => {
            let stroke = Stroke::new(line_width(cell, pixel), color);
            if c != '\u{2572}' {
                painter.line_segment([cell.left_bottom(), cell.right_top()], stroke);
            }
            if c != '\u{2571}' {
                painter.line_segment([cell.left_top(), cell.right_bottom()], stroke);
            }
        }
        '\u{256D}'..='\u{2570}' => draw_arc(painter, cell, c, color, pixel),
        '\u{2500}'..='\u{257F}' => draw_lines(painter, cell, box_arms(c), color, pixel),
        '\u{2800}'..='\u{28FF}' => draw_braille(painter, cell, c, color),
        '\u{2580}'..='\u{259F}' => {
            for (part, alpha) in block_parts(c) {
                let rect = Rect::from_min_max(
                    cell.min + part.min.to_vec2() * cell.size(),
                    cell.min + part.max.to_vec2() * cell.size(),
                );
                painter.rect_filled(
                    Rect::from_min_max(snap(rect.min, pixel), snap(rect.max, pixel)),
                    0.0,
                    color.gamma_multiply(alpha),
                );
            }
        }
        _ => return false,
    }
    true
}

fn snap(pos: Pos2, pixel: f32) -> Pos2 {
    Pos2::new(
        (pos.x / pixel).round() * pixel,
        (pos.y / pixel).round() * pixel,
    )
}

fn line_width(cell: Rect, pixel: f32) -> f32 {
    (cell.width() / 8.0 / pixel).round().max(1.0) * pixel
}

const BOX_ARMS: &[u8] = b"\
    0101 0202 1010 2020 0101 0202 1010 2020 0101 0202 1010 2020 0110 0210 0120 0220 \
    0011 0012 0021 0022 1100 1200 2100 2200 1001 1002 2001 2002 1110 1210 2110 1120 \
    2120 2210 1220 2220 1011 1012 2011 1021 2021 2012 1022 2022 0111 0112 0211 0212 \
    0121 0122 0221 0222 1101 1102 1201 1202 2101 2102 2201 2202 1111 1112 1211 1212 \
    2111 1121 2121 2112 2211 1122 1221 2212 1222 2122 2221 2222 0101 0202 1010 2020 \
    0303 3030 0310 0130 0330 0013 0031 0033 1300 3100 3300 1003 3001 3003 1310 3130 \
    3330 1013 3031 3033 0313 0131 0333 1303 3101 3303 1313 3131 3333 0110 0011 1001 \
    1100 0000 0000 0000 0001 1000 0100 0010 0002 2000 0200 0020 0201 1020 0102 2010";

fn box_arms(c: char) -> [u8; 4] {
    let start = (c as usize - 0x2500) * 5;
    let arms = &BOX_ARMS[start..start + 4];
    [
        arms[0] - b'0',
        arms[1] - b'0',
        arms[2] - b'0',
        arms[3] - b'0',
    ]
}

fn draw_lines(painter: &Painter, cell: Rect, arms: [u8; 4], color: Color32, pixel: f32) {
    let light = line_width(cell, pixel);
    let center = snap(cell.center(), pixel);
    let gap = light * 1.5;
    let reach = if arms.contains(&3) {
        gap + light / 2.0
    } else {
        light * if arms.contains(&2) { 1.0 } else { 0.5 }
    };
    let ends = [cell.top(), cell.right(), cell.bottom(), cell.left()];
    for (side, (weight, end)) in arms.into_iter().zip(ends).enumerate() {
        let (width, offsets): (f32, &[f32]) = match weight {
            0 => continue,
            1 => (light, &[0.0]),
            2 => (light * 2.0, &[0.0]),
            _ => (light, &[-gap, gap]),
        };
        for offset in offsets {
            let rect = if side % 2 == 0 {
                let x = center.x + offset;
                let (from, to) = if side == 0 {
                    (end, center.y + reach)
                } else {
                    (center.y - reach, end)
                };
                Rect::from_min_max(
                    Pos2::new(x - width / 2.0, from),
                    Pos2::new(x + width / 2.0, to),
                )
            } else {
                let y = center.y + offset;
                let (from, to) = if side == 3 {
                    (end, center.x + reach)
                } else {
                    (center.x - reach, end)
                };
                Rect::from_min_max(
                    Pos2::new(from, y - width / 2.0),
                    Pos2::new(to, y + width / 2.0),
                )
            };
            painter.rect_filled(rect, 0.0, color);
        }
    }
}

fn draw_arc(painter: &Painter, cell: Rect, c: char, color: Color32, pixel: f32) {
    let center = snap(cell.center(), pixel);
    let vertical = Pos2::new(
        center.x,
        if matches!(c, '╭' | '╮') {
            cell.bottom()
        } else {
            cell.top()
        },
    );
    let horizontal = Pos2::new(
        if matches!(c, '╭' | '╰') {
            cell.right()
        } else {
            cell.left()
        },
        center.y,
    );
    let stroke = Stroke::new(line_width(cell, pixel), color);
    painter.add(QuadraticBezierShape::from_points_stroke(
        [vertical, center, horizontal],
        false,
        Color32::TRANSPARENT,
        stroke,
    ));
}

fn draw_braille(painter: &Painter, cell: Rect, c: char, color: Color32) {
    let radius = (cell.width() / 4.0).min(cell.height() / 8.0) * 0.7;
    for (row, column) in braille_dots(c) {
        let center = cell.min
            + Vec2::new(
                cell.width() * (1.0 + 2.0 * column as f32) / 4.0,
                cell.height() * (1.0 + 2.0 * row as f32) / 8.0,
            );
        painter.circle_filled(center, radius, color);
    }
}

fn braille_dots(c: char) -> Vec<(usize, usize)> {
    const DOTS: [(usize, usize); 8] = [
        (0, 0),
        (1, 0),
        (2, 0),
        (0, 1),
        (1, 1),
        (2, 1),
        (3, 0),
        (3, 1),
    ];
    let bits = c as u32 - 0x2800;
    DOTS.into_iter()
        .enumerate()
        .filter(|(bit, _)| bits & (1 << bit) != 0)
        .map(|(_, dot)| dot)
        .collect()
}

fn block_parts(c: char) -> Vec<(Rect, f32)> {
    let part = |left: f32, top: f32, right: f32, bottom: f32| {
        (
            Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom)),
            1.0,
        )
    };
    let eighth = |n: u32| n as f32 / 8.0;
    match c as u32 {
        0x2580 => vec![part(0.0, 0.0, 1.0, 0.5)],
        code @ 0x2581..=0x2588 => vec![part(0.0, 1.0 - eighth(code - 0x2580), 1.0, 1.0)],
        code @ 0x2589..=0x258F => vec![part(0.0, 0.0, eighth(0x2590 - code), 1.0)],
        0x2590 => vec![part(0.5, 0.0, 1.0, 1.0)],
        code @ 0x2591..=0x2593 => vec![(
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            (code - 0x2590) as f32 / 4.0,
        )],
        0x2594 => vec![part(0.0, 0.0, 1.0, eighth(1))],
        0x2595 => vec![part(eighth(7), 0.0, 1.0, 1.0)],
        code @ 0x2596..=0x259F => {
            const QUADRANTS: [u8; 10] = [
                0b0100, 0b1000, 0b0001, 0b1101, 0b1001, 0b0111, 0b1011, 0b0010, 0b0110, 0b1110,
            ];
            let quadrants = QUADRANTS[(code - 0x2596) as usize];
            [(0.0, 0.0), (0.5, 0.0), (0.0, 0.5), (0.5, 0.5)]
                .into_iter()
                .enumerate()
                .filter(|(index, _)| quadrants & (1 << index) != 0)
                .map(|(_, (left, top))| part(left, top, left + 0.5, top + 0.5))
                .collect()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_the_lines_of_box_drawing_characters() {
        assert_eq!(box_arms('─'), [0, 1, 0, 1]);
        assert_eq!(box_arms('┏'), [0, 2, 2, 0]);
        assert_eq!(box_arms('╃'), [2, 1, 1, 2]);
        assert_eq!(box_arms('╬'), [3, 3, 3, 3]);
        assert_eq!(box_arms('╒'), [0, 3, 1, 0]);
        assert_eq!(box_arms('╿'), [2, 0, 1, 0]);
    }

    #[test]
    fn knows_the_parts_of_block_characters() {
        let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        assert_eq!(block_parts('█'), [(full, 1.0)]);
        assert_eq!(
            block_parts('▁'),
            [(
                Rect::from_min_max(Pos2::new(0.0, 0.875), Pos2::new(1.0, 1.0)),
                1.0
            )]
        );
        assert_eq!(
            block_parts('▏'),
            [(Rect::from_min_max(Pos2::ZERO, Pos2::new(0.125, 1.0)), 1.0)]
        );
        assert_eq!(block_parts('▒'), [(full, 0.5)]);
        let quadrants = |c| {
            block_parts(c)
                .into_iter()
                .map(|(rect, _)| (rect.min.x, rect.min.y))
                .collect::<Vec<_>>()
        };
        assert_eq!(quadrants('▛'), [(0.0, 0.0), (0.5, 0.0), (0.0, 0.5)]);
        assert_eq!(quadrants('▟'), [(0.5, 0.0), (0.0, 0.5), (0.5, 0.5)]);
        assert_eq!(quadrants('▖'), [(0.0, 0.5)]);
    }

    #[test]
    fn knows_the_dots_of_braille_characters() {
        assert_eq!(braille_dots('⠀'), []);
        assert_eq!(braille_dots('⠁'), [(0, 0)]);
        assert_eq!(braille_dots('⠋'), [(0, 0), (1, 0), (0, 1)]);
        assert_eq!(braille_dots('⢀'), [(3, 1)]);
        assert_eq!(braille_dots('⣿').len(), 8);
    }

    #[test]
    fn draws_box_and_block_characters_and_leaves_the_rest_to_the_fonts() {
        let ctx = egui::Context::default();
        let painter = Painter::new(ctx, egui::LayerId::background(), Rect::EVERYTHING);
        let cell = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(7.0, 15.0));
        for c in ['─', '╭', '╳', '▐', '▓', '▟', '⠋'] {
            assert!(draw(&painter, cell, c, Color32::WHITE, 0.5), "{c}");
        }
        for c in ['a', '⏺', '✻'] {
            assert!(!draw(&painter, cell, c, Color32::WHITE, 0.5), "{c}");
        }
    }
}
