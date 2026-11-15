use eframe::egui::{self, Painter, Pos2, Rect, Stroke, Vec2};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    Files,
    Review,
    Keys,
}

pub const SIZE: f32 = 18.0;

pub fn paint(painter: &Painter, center: Pos2, icon: Icon, color: egui::Color32) {
    let stroke = Stroke::new(1.4, color);
    let rect = Rect::from_center_size(center, Vec2::splat(SIZE));
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y);
    match icon {
        Icon::Files => {
            let page = [
                at(3.5, 1.5),
                at(11.0, 1.5),
                at(15.0, 5.5),
                at(15.0, 16.5),
                at(3.5, 16.5),
                at(3.5, 1.5),
            ];
            painter.line(page.to_vec(), stroke);
            painter.line(vec![at(11.0, 1.5), at(11.0, 5.5), at(15.0, 5.5)], stroke);
        }
        Icon::Review => {
            painter.line(
                vec![at(6.5, 1.5), at(2.5, 1.5), at(2.5, 16.5), at(6.5, 16.5)],
                stroke,
            );
            painter.line(
                vec![at(11.5, 1.5), at(15.5, 1.5), at(15.5, 16.5), at(11.5, 16.5)],
                stroke,
            );
            for step in 0..7 {
                painter.circle_filled(at(9.0, 1.5 + step as f32 * 2.5), 0.75, color);
            }
            for y in [6.0, 9.0] {
                painter.line_segment([at(4.5, y), at(7.2, y)], stroke);
            }
            for y in [6.0, 9.0, 12.0] {
                painter.line_segment([at(10.8, y), at(13.5, y)], stroke);
            }
        }
        Icon::Keys => {
            painter.rect_stroke(
                Rect::from_min_max(at(1.5, 4.0), at(16.5, 14.0)),
                2.0,
                stroke,
                egui::StrokeKind::Middle,
            );
            for x in [4.5, 7.5, 10.5, 13.5] {
                painter.circle_filled(at(x, 7.3), 0.8, color);
            }
            painter.line_segment([at(5.5, 11.0), at(12.5, 11.0)], stroke);
        }
    }
}
