use eframe::egui::{self, Color32, Painter, Pos2, Rect, Stroke, epaint::CubicBezierShape};

use crate::git::Commit;

pub const LANE: f32 = 12.0;

const COLORS: [Color32; 6] = [
    Color32::from_rgb(98, 151, 232),
    Color32::from_rgb(106, 176, 112),
    Color32::from_rgb(196, 112, 196),
    Color32::from_rgb(82, 178, 188),
    Color32::from_rgb(222, 108, 120),
    Color32::from_rgb(150, 140, 220),
];

const LOCAL: Color32 = Color32::from_rgb(226, 156, 52);

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub color: usize,
    pub local: bool,
}

impl Edge {
    fn stroke(&self) -> Stroke {
        Stroke::new(1.6, color(self.color, self.local))
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Row {
    pub lane: usize,
    pub color: usize,
    pub local: bool,
    pub upper: Vec<Edge>,
    pub lower: Vec<Edge>,
    pub merge: bool,
}

fn color(index: usize, local: bool) -> Color32 {
    if local {
        LOCAL
    } else {
        COLORS[index % COLORS.len()]
    }
}

impl Row {
    pub fn width(&self) -> f32 {
        let lanes = self
            .upper
            .iter()
            .chain(&self.lower)
            .flat_map(|edge| [edge.from, edge.to])
            .chain([self.lane])
            .max()
            .unwrap_or(0);
        (lanes + 1) as f32 * LANE
    }

    pub fn color(&self) -> Color32 {
        color(self.color, self.local)
    }

    pub fn paint(&self, painter: &Painter, rect: Rect, background: Color32) {
        let x = |lane: usize| rect.left() + (lane as f32 + 0.5) * LANE;
        let middle = rect.center().y;
        for edge in &self.upper {
            curve(
                painter,
                Pos2::new(x(edge.from), rect.top()),
                Pos2::new(x(edge.to), middle),
                edge.stroke(),
            );
        }
        for edge in &self.lower {
            curve(
                painter,
                Pos2::new(x(edge.from), middle),
                Pos2::new(x(edge.to), rect.bottom()),
                edge.stroke(),
            );
        }
        let center = Pos2::new(x(self.lane), middle);
        let color = self.color();
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

#[derive(Clone, Copy)]
struct Lane<'a> {
    hash: &'a str,
    color: usize,
    local: bool,
}

pub fn layout(commits: &[Commit]) -> Vec<Row> {
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut colors = 0;
    let mut next_color = || {
        colors += 1;
        colors - 1
    };
    commits
        .iter()
        .map(|commit| {
            let found = lanes
                .iter()
                .position(|lane| lane.is_some_and(|lane| lane.hash == commit.hash));
            let (lane, color) = match found {
                Some(index) => (index, lanes[index].map_or(0, |lane| lane.color)),
                None => (free(&mut lanes), next_color()),
            };
            let upper: Vec<Edge> = lanes
                .iter()
                .enumerate()
                .filter_map(|(index, state)| {
                    state.map(|state| Edge {
                        from: index,
                        to: if state.hash == commit.hash {
                            lane
                        } else {
                            index
                        },
                        color: state.color,
                        local: state.local,
                    })
                })
                .collect();
            for state in lanes.iter_mut() {
                if state.is_some_and(|state| state.hash == commit.hash) {
                    *state = None;
                }
            }
            lanes[lane] = commit.parents.first().map(|parent| Lane {
                hash: parent,
                color,
                local: commit.local,
            });
            for parent in commit.parents.iter().skip(1) {
                if !lanes.iter().flatten().any(|state| state.hash == parent) {
                    let index = free(&mut lanes);
                    lanes[index] = Some(Lane {
                        hash: parent,
                        color: next_color(),
                        local: commit.local,
                    });
                }
            }
            let mut lower = Vec::new();
            for (index, state) in lanes.iter().enumerate() {
                let Some(state) = state else { continue };
                if index == lane {
                    lower.push(Edge {
                        from: lane,
                        to: lane,
                        color,
                        local: commit.local,
                    });
                    continue;
                }
                if commit
                    .parents
                    .iter()
                    .skip(1)
                    .any(|parent| parent == state.hash)
                    && commit.parents[0] != state.hash
                {
                    lower.push(Edge {
                        from: lane,
                        to: index,
                        color: state.color,
                        local: commit.local,
                    });
                }
                if upper
                    .iter()
                    .any(|edge| edge.from == index && edge.to == index)
                {
                    lower.push(Edge {
                        from: index,
                        to: index,
                        color: state.color,
                        local: state.local,
                    });
                }
            }
            while lanes.last().is_some_and(Option::is_none) {
                lanes.pop();
            }
            Row {
                lane,
                color,
                local: commit.local,
                upper,
                lower,
                merge: commit.parents.len() > 1,
            }
        })
        .collect()
}

fn free<T>(lanes: &mut Vec<Option<T>>) -> usize {
    lanes.iter().position(Option::is_none).unwrap_or_else(|| {
        lanes.push(None);
        lanes.len() - 1
    })
}

pub fn labels(refs: &[String]) -> Vec<&str> {
    refs.iter()
        .map(String::as_str)
        .filter(|name| !name.ends_with("/HEAD"))
        .filter(|name| {
            !name
                .split_once('/')
                .is_some_and(|(_, branch)| refs.iter().any(|other| other == branch))
        })
        .collect()
}

pub fn paint_refs(
    ui: &egui::Ui,
    painter: &Painter,
    mut left: f32,
    middle: f32,
    color: Color32,
    refs: &[String],
) -> f32 {
    let font = egui::TextStyle::Small.resolve(ui.style());
    for name in labels(refs) {
        let galley = painter.layout_no_wrap(name.to_string(), font.clone(), color);
        let rect = Rect::from_min_size(
            Pos2::new(left, middle - galley.size().y / 2.0 - 1.0),
            galley.size() + egui::vec2(8.0, 2.0),
        );
        painter.rect_filled(rect, 3.0, color.gamma_multiply(0.16));
        painter.galley(rect.min + egui::vec2(4.0, 1.0), galley, color);
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

    fn pairs(edges: &[Edge]) -> Vec<(usize, usize)> {
        edges.iter().map(|edge| (edge.from, edge.to)).collect()
    }

    #[test]
    fn keeps_a_straight_history_in_one_lane() {
        let rows = layout(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);
        assert!(
            rows.iter()
                .all(|row| row.lane == 0 && row.color == 0 && !row.merge)
        );
        assert_eq!(pairs(&rows[0].upper), []);
        assert_eq!(pairs(&rows[1].upper), [(0, 0)]);
        assert_eq!(pairs(&rows[1].lower), [(0, 0)]);
        assert_eq!(pairs(&rows[2].lower), []);
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
        assert_eq!(pairs(&rows[0].lower), [(0, 0), (0, 1)]);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(pairs(&rows[1].upper), [(0, 0), (1, 1)]);
        assert_eq!(pairs(&rows[1].lower), [(0, 0), (1, 1)]);
        assert_eq!(rows[2].lane, 0);
        assert_eq!(pairs(&rows[2].lower), [(0, 0), (1, 1)]);
        assert_eq!(rows[3].lane, 0);
        assert_eq!(pairs(&rows[3].upper), [(0, 0), (1, 0)]);
        assert_eq!(rows[3].width(), 2.0 * LANE);
    }

    #[test]
    fn gives_every_branch_a_color_of_its_own() {
        let rows = layout(&[
            commit("m", &["b", "f"]),
            commit("x", &["b"]),
            commit("f", &["a"]),
            commit("b", &["a"]),
            commit("a", &[]),
        ]);
        let colors: Vec<usize> = rows.iter().map(|row| row.color).collect();
        assert_eq!(colors, [0, 2, 1, 0, 0]);
        assert_eq!(rows[0].lower[1].color, 1);
        assert_eq!(
            rows[4]
                .upper
                .iter()
                .map(|edge| edge.color)
                .collect::<Vec<_>>(),
            [0, 1]
        );
    }

    #[test]
    fn colors_the_commits_that_are_not_pushed() {
        let mut commits = [commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])];
        commits[0].local = true;
        let rows = layout(&commits);
        assert!(rows[0].local);
        assert!(rows[0].lower[0].local);
        assert!(rows[1].upper[0].local);
        assert!(!rows[1].local);
        assert!(!rows[1].lower[0].local);
        assert_eq!(rows[0].color(), LOCAL);
        assert_eq!(rows[1].color(), COLORS[0]);
    }

    #[test]
    fn leaves_out_remote_names_a_local_branch_already_shows() {
        let refs = [
            "main",
            "origin/main",
            "origin/HEAD",
            "origin/feature",
            "v1.0",
        ]
        .map(String::from);
        assert_eq!(labels(&refs), ["main", "origin/feature", "v1.0"]);
    }
}
