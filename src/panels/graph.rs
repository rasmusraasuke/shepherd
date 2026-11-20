use eframe::egui::{self, Color32, Painter, Pos2, Rect, Stroke, epaint::CubicBezierShape};

use crate::git::Commit;

pub const LANE: f32 = 12.0;

const COLORS: [Color32; 8] = [
    Color32::from_rgb(98, 151, 232),
    Color32::from_rgb(214, 132, 76),
    Color32::from_rgb(106, 176, 112),
    Color32::from_rgb(196, 112, 196),
    Color32::from_rgb(214, 186, 72),
    Color32::from_rgb(82, 178, 188),
    Color32::from_rgb(222, 108, 120),
    Color32::from_rgb(150, 140, 220),
];

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Row {
    pub lane: usize,
    pub upper: Vec<(usize, usize)>,
    pub lower: Vec<(usize, usize)>,
    pub merge: bool,
}

impl Row {
    pub fn width(&self) -> f32 {
        let lanes = self
            .upper
            .iter()
            .chain(&self.lower)
            .flat_map(|&(from, to)| [from, to])
            .chain([self.lane])
            .max()
            .unwrap_or(0);
        (lanes + 1) as f32 * LANE
    }

    pub fn paint(&self, painter: &Painter, rect: Rect, background: Color32) {
        let x = |lane: usize| rect.left() + (lane as f32 + 0.5) * LANE;
        let middle = rect.center().y;
        let stroke = |lane: usize| Stroke::new(1.6, COLORS[lane % COLORS.len()]);
        for &(from, to) in &self.upper {
            curve(
                painter,
                Pos2::new(x(from), rect.top()),
                Pos2::new(x(to), middle),
                stroke(from),
            );
        }
        for &(from, to) in &self.lower {
            curve(
                painter,
                Pos2::new(x(from), middle),
                Pos2::new(x(to), rect.bottom()),
                stroke(to),
            );
        }
        let center = Pos2::new(x(self.lane), middle);
        let color = COLORS[self.lane % COLORS.len()];
        if self.merge {
            painter.circle_filled(center, 4.0, background);
            painter.circle_stroke(center, 3.2, Stroke::new(1.6, color));
        } else {
            painter.circle_filled(center, 4.0, color);
        }
    }
}

fn curve(painter: &Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    if from.x == to.x {
        painter.line_segment([from, to], stroke);
        return;
    }
    let bend = (to.y - from.y) * 0.6;
    painter.add(CubicBezierShape::from_points_stroke(
        [
            from,
            Pos2::new(from.x, from.y + bend),
            Pos2::new(to.x, to.y - bend),
            to,
        ],
        false,
        Color32::TRANSPARENT,
        stroke,
    ));
}

pub fn layout(commits: &[Commit]) -> Vec<Row> {
    let mut lanes: Vec<Option<&str>> = Vec::new();
    commits
        .iter()
        .map(|commit| {
            let lane = lanes
                .iter()
                .position(|hash| *hash == Some(commit.hash.as_str()))
                .unwrap_or_else(|| free(&mut lanes));
            let upper: Vec<(usize, usize)> = lanes
                .iter()
                .enumerate()
                .filter_map(|(index, hash)| {
                    hash.map(|hash| (index, if hash == commit.hash { lane } else { index }))
                })
                .collect();
            for hash in lanes.iter_mut() {
                if *hash == Some(commit.hash.as_str()) {
                    *hash = None;
                }
            }
            lanes[lane] = commit.parents.first().map(String::as_str);
            for parent in commit.parents.iter().skip(1) {
                if !lanes.contains(&Some(parent.as_str())) {
                    let index = free(&mut lanes);
                    lanes[index] = Some(parent.as_str());
                }
            }
            let mut lower = Vec::new();
            for (index, hash) in lanes.iter().enumerate() {
                let Some(hash) = hash else { continue };
                if index == lane {
                    lower.push((lane, lane));
                    continue;
                }
                if commit.parents.iter().skip(1).any(|parent| parent == hash)
                    && commit.parents[0] != *hash
                {
                    lower.push((lane, index));
                }
                if upper.contains(&(index, index)) {
                    lower.push((index, index));
                }
            }
            while lanes.last() == Some(&None) {
                lanes.pop();
            }
            Row {
                lane,
                upper,
                lower,
                merge: commit.parents.len() > 1,
            }
        })
        .collect()
}

fn free<'a>(lanes: &mut Vec<Option<&'a str>>) -> usize {
    lanes.iter().position(Option::is_none).unwrap_or_else(|| {
        lanes.push(None);
        lanes.len() - 1
    })
}

pub fn paint_refs(
    ui: &egui::Ui,
    painter: &Painter,
    mut left: f32,
    top: f32,
    refs: &[String],
) -> f32 {
    let font = egui::TextStyle::Small.resolve(ui.style());
    for name in refs {
        let galley =
            painter.layout_no_wrap(name.clone(), font.clone(), ui.visuals().strong_text_color());
        let rect = Rect::from_min_size(Pos2::new(left, top), galley.size() + egui::vec2(8.0, 2.0));
        painter.rect_filled(rect, 3.0, ui.visuals().widgets.inactive.weak_bg_fill);
        painter.galley(
            rect.min + egui::vec2(4.0, 1.0),
            galley,
            Color32::PLACEHOLDER,
        );
        left = rect.right() + 4.0;
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.to_string(),
            parents: parents.iter().map(|parent| parent.to_string()).collect(),
            ..Commit::default()
        }
    }

    #[test]
    fn keeps_a_straight_history_in_one_lane() {
        let rows = layout(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);
        assert!(rows.iter().all(|row| row.lane == 0 && !row.merge));
        assert_eq!(rows[0].upper, []);
        assert_eq!(rows[1].upper, [(0, 0)]);
        assert_eq!(rows[1].lower, [(0, 0)]);
        assert_eq!(rows[2].lower, []);
    }

    #[test]
    fn branches_off_and_merges_back() {
        let rows = layout(&[
            commit("m", &["b", "f"]),
            commit("f", &["a"]),
            commit("b", &["a"]),
            commit("a", &[]),
        ]);
        assert!(rows[0].merge);
        assert_eq!(rows[0].lower, [(0, 0), (0, 1)]);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[1].upper, [(0, 0), (1, 1)]);
        assert_eq!(rows[1].lower, [(0, 0), (1, 1)]);
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[2].lower, [(0, 0), (1, 1)]);
        assert_eq!(rows[3].lane, 0);
        assert_eq!(rows[3].upper, [(0, 0), (1, 0)]);
        assert_eq!(rows[3].width(), 2.0 * LANE);
    }

    #[test]
    fn gives_a_branch_tip_a_lane_of_its_own() {
        let rows = layout(&[commit("x", &["a"]), commit("y", &["a"]), commit("a", &[])]);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[1].upper, [(0, 0)]);
        assert_eq!(rows[1].lower, [(0, 0), (1, 1)]);
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[2].upper, [(0, 0), (1, 0)]);
    }
}
