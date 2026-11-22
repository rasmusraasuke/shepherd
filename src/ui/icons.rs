use eframe::egui::{self, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    Files,
    Review,
    History,
    Keys,
    More,
    Folder { open: bool },
    File(FileType),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FileType {
    Kotlin,
    Java,
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Go,
    Json,
    Yaml,
    Toml,
    Markdown,
    Html,
    Xml,
    Css,
    Shell,
    Docker,
    Sql,
    Git,
    Svg,
    Svelte,
    Pdf,
    Text,
}

impl FileType {
    pub fn extension(path: &str) -> String {
        let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
        if name.starts_with("dockerfile") || name.starts_with("containerfile") {
            return "dockerfile".to_string();
        }
        name.rsplit_once('.')
            .map(|(_, extension)| extension.to_string())
            .unwrap_or_default()
    }

    pub fn of(path: &str) -> Self {
        match Self::extension(path).as_str() {
            "kt" | "kts" => Self::Kotlin,
            "java" => Self::Java,
            "rs" => Self::Rust,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "ts" | "mts" | "cts" | "tsx" => Self::TypeScript,
            "py" => Self::Python,
            "go" => Self::Go,
            "json" | "jsonc" => Self::Json,
            "yml" | "yaml" => Self::Yaml,
            "toml" => Self::Toml,
            "md" | "markdown" => Self::Markdown,
            "html" | "htm" => Self::Html,
            "xml" => Self::Xml,
            "css" | "scss" | "sass" | "less" => Self::Css,
            "sh" | "bash" | "zsh" | "fish" => Self::Shell,
            "dockerfile" | "dockerignore" => Self::Docker,
            "sql" => Self::Sql,
            "gitignore" | "gitkeep" | "gitattributes" | "gitmodules" => Self::Git,
            "svg" => Self::Svg,
            "svelte" => Self::Svelte,
            "pdf" => Self::Pdf,
            _ => Self::Text,
        }
    }

    pub fn opens_outside(self) -> bool {
        self == Self::Pdf
    }

    fn color(self) -> Option<Color32> {
        let rgb = Color32::from_rgb;
        Some(match self {
            Self::Kotlin => rgb(127, 82, 255),
            Self::Java => rgb(224, 118, 38),
            Self::Rust => rgb(206, 120, 80),
            Self::JavaScript => rgb(240, 219, 79),
            Self::TypeScript => rgb(49, 120, 198),
            Self::Python => rgb(55, 118, 171),
            Self::Go => rgb(0, 173, 216),
            Self::Json => rgb(203, 171, 58),
            Self::Yaml => rgb(203, 72, 72),
            Self::Toml => rgb(156, 110, 80),
            Self::Markdown => rgb(81, 154, 186),
            Self::Html => rgb(227, 76, 38),
            Self::Xml => rgb(230, 140, 40),
            Self::Css => rgb(86, 156, 214),
            Self::Shell => rgb(100, 180, 90),
            Self::Docker => rgb(29, 99, 237),
            Self::Sql => rgb(0, 150, 170),
            Self::Git => rgb(240, 80, 50),
            Self::Svg => rgb(84, 138, 247),
            Self::Svelte => rgb(255, 62, 0),
            Self::Pdf => rgb(220, 50, 50),
            Self::Text => return None,
        })
    }
}

pub const BAR_SIZE: f32 = 18.0;

pub fn paint(painter: &Painter, center: Pos2, size: f32, icon: Icon, color: Color32) {
    let color = match icon {
        Icon::File(file_type) => file_type.color().unwrap_or(color),
        _ => color,
    };
    let scale = size / BAR_SIZE;
    let stroke = Stroke::new((1.4 * scale).max(1.1), color);
    let rect = Rect::from_center_size(center, Vec2::splat(size));
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * scale;
    let line = |points: &[(f32, f32)]| {
        painter.line(points.iter().map(|&(x, y)| at(x, y)).collect(), stroke);
    };
    let frame = |min: (f32, f32), max: (f32, f32)| {
        painter.rect_stroke(
            Rect::from_min_max(at(min.0, min.1), at(max.0, max.1)),
            2.0 * scale,
            stroke,
            egui::StrokeKind::Middle,
        );
    };
    let block = |min: (f32, f32), max: (f32, f32), color: Color32| {
        painter.rect_filled(
            Rect::from_min_max(at(min.0, min.1), at(max.0, max.1)),
            2.0 * scale,
            color,
        );
    };
    let text = |pos: (f32, f32), align: egui::Align2, text: &str, font: FontId, color| {
        painter.text(at(pos.0, pos.1), align, text, font, color);
    };
    let page = || {
        line(&[
            (3.5, 1.5),
            (11.0, 1.5),
            (15.0, 5.5),
            (15.0, 16.5),
            (3.5, 16.5),
            (3.5, 1.5),
        ]);
        line(&[(11.0, 1.5), (11.0, 5.5), (15.0, 5.5)]);
    };
    let badge = |label: &str, foreground: Color32| {
        block((2.0, 2.0), (16.0, 16.0), color);
        let font = FontId::proportional((15.0 / label.len() as f32).min(7.5) * scale);
        text(
            (15.0, 16.2),
            egui::Align2::RIGHT_BOTTOM,
            label,
            font,
            foreground,
        );
    };
    match icon {
        Icon::Files => page(),
        Icon::Review => {
            line(&[(6.5, 1.5), (2.5, 1.5), (2.5, 16.5), (6.5, 16.5)]);
            line(&[(11.5, 1.5), (15.5, 1.5), (15.5, 16.5), (11.5, 16.5)]);
            for step in 0..7 {
                painter.circle_filled(at(9.0, 1.5 + step as f32 * 2.5), 0.75 * scale, color);
            }
            for y in [6.0, 9.0] {
                line(&[(4.5, y), (7.2, y)]);
            }
            for y in [6.0, 9.0, 12.0] {
                line(&[(10.8, y), (13.5, y)]);
            }
        }
        Icon::History => {
            for (x, y) in [(5.0, 3.5), (5.0, 14.5), (13.0, 4.5)] {
                painter.circle_stroke(at(x, y), 1.9 * scale, stroke);
            }
            line(&[(5.0, 5.4), (5.0, 12.6)]);
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                [at(13.0, 6.4), at(13.0, 10.5), at(5.0, 9.0), at(5.0, 12.6)],
                false,
                Color32::TRANSPARENT,
                stroke,
            ));
        }
        Icon::Keys => {
            frame((1.5, 4.0), (16.5, 14.0));
            for x in [4.5, 7.5, 10.5, 13.5] {
                painter.circle_filled(at(x, 7.3), 0.8 * scale, color);
            }
            line(&[(5.5, 11.0), (12.5, 11.0)]);
        }
        Icon::More => {
            for x in [4.0, 9.0, 14.0] {
                painter.circle_filled(at(x, 9.0), 1.4 * scale, color);
            }
        }
        Icon::Folder { open: false } => line(&[
            (1.5, 4.0),
            (6.5, 4.0),
            (8.5, 6.0),
            (16.5, 6.0),
            (16.5, 15.0),
            (1.5, 15.0),
            (1.5, 4.0),
        ]),
        Icon::Folder { open: true } => {
            line(&[
                (1.5, 15.0),
                (1.5, 4.0),
                (6.5, 4.0),
                (8.5, 6.0),
                (14.5, 6.0),
                (14.5, 8.5),
            ]);
            line(&[
                (1.5, 15.0),
                (4.0, 8.5),
                (17.0, 8.5),
                (14.5, 15.0),
                (1.5, 15.0),
            ]);
        }
        Icon::File(file_type) => match file_type {
            FileType::Kotlin => {
                let points = [
                    (3.0, 3.0),
                    (15.0, 3.0),
                    (9.0, 9.0),
                    (15.0, 15.0),
                    (3.0, 15.0),
                ];
                painter.add(Shape::convex_polygon(
                    points[..3]
                        .iter()
                        .chain(&points[4..])
                        .map(|&(x, y)| at(x, y))
                        .collect(),
                    color,
                    Stroke::NONE,
                ));
                painter.add(Shape::convex_polygon(
                    points[2..].iter().map(|&(x, y)| at(x, y)).collect(),
                    color,
                    Stroke::NONE,
                ));
            }
            FileType::Java => {
                line(&[
                    (3.5, 7.5),
                    (3.5, 12.5),
                    (5.5, 15.0),
                    (10.5, 15.0),
                    (12.5, 12.5),
                    (12.5, 7.5),
                    (3.5, 7.5),
                ]);
                line(&[(12.5, 9.0), (15.0, 9.0), (15.0, 11.5), (12.5, 12.5)]);
                line(&[(6.5, 2.0), (7.3, 3.5), (6.5, 5.0)]);
                line(&[(9.5, 2.0), (10.3, 3.5), (9.5, 5.0)]);
            }
            FileType::Rust => {
                painter.circle_stroke(at(9.0, 9.0), 4.8 * scale, stroke);
                painter.circle_filled(at(9.0, 9.0), 1.6 * scale, color);
                for tooth in 0..8 {
                    let direction = Vec2::angled(tooth as f32 * std::f32::consts::FRAC_PI_4);
                    painter.line_segment(
                        [
                            at(9.0, 9.0) + direction * 4.8 * scale,
                            at(9.0, 9.0) + direction * 7.0 * scale,
                        ],
                        stroke,
                    );
                }
            }
            FileType::JavaScript => badge("JS", Color32::from_rgb(30, 30, 30)),
            FileType::TypeScript => badge("TS", Color32::WHITE),
            FileType::Python => {
                block((2.0, 2.0), (11.0, 11.0), color);
                block((7.0, 7.0), (16.0, 16.0), Color32::from_rgb(240, 200, 60));
                painter.circle_filled(at(4.8, 4.8), 0.9 * scale, Color32::WHITE);
                painter.circle_filled(at(13.2, 13.2), 0.9 * scale, Color32::WHITE);
            }
            FileType::Go => {
                let font = FontId::proportional(9.0 * scale);
                text((9.0, 9.0), egui::Align2::CENTER_CENTER, "GO", font, color);
            }
            FileType::Json => {
                let font = FontId::monospace(12.0 * scale);
                text((9.0, 9.0), egui::Align2::CENTER_CENTER, "{}", font, color);
            }
            FileType::Yaml | FileType::Toml => {
                line(&[(2.5, 4.5), (10.0, 4.5)]);
                line(&[(6.0, 9.0), (15.5, 9.0)]);
                line(&[(6.0, 13.5), (13.0, 13.5)]);
            }
            FileType::Markdown => {
                frame((1.5, 4.0), (16.5, 14.0));
                line(&[
                    (4.0, 11.5),
                    (4.0, 6.5),
                    (6.25, 9.0),
                    (8.5, 6.5),
                    (8.5, 11.5),
                ]);
                line(&[(12.5, 6.5), (12.5, 11.5)]);
                line(&[(10.5, 9.5), (12.5, 11.5), (14.5, 9.5)]);
            }
            FileType::Html | FileType::Xml => {
                line(&[(6.0, 4.5), (2.5, 9.0), (6.0, 13.5)]);
                line(&[(12.0, 4.5), (15.5, 9.0), (12.0, 13.5)]);
                if file_type == FileType::Xml {
                    line(&[(10.0, 4.0), (8.0, 14.0)]);
                }
            }
            FileType::Css => {
                line(&[(6.5, 3.0), (5.5, 15.0)]);
                line(&[(12.5, 3.0), (11.5, 15.0)]);
                line(&[(3.0, 7.0), (15.5, 7.0)]);
                line(&[(2.5, 11.0), (15.0, 11.0)]);
            }
            FileType::Shell => {
                frame((1.5, 3.0), (16.5, 15.0));
                line(&[(4.5, 7.0), (7.0, 9.0), (4.5, 11.0)]);
                line(&[(8.5, 11.5), (12.5, 11.5)]);
            }
            FileType::Docker => {
                for (x, y) in [(3.0, 8.0), (6.5, 8.0), (10.0, 8.0), (6.5, 4.5), (10.0, 4.5)] {
                    block((x, y), (x + 2.8, y + 2.8), color);
                }
                line(&[
                    (1.5, 12.0),
                    (16.5, 12.0),
                    (14.5, 15.5),
                    (4.0, 15.5),
                    (1.5, 12.0),
                ]);
            }
            FileType::Sql => {
                let ellipse = |y: f32, from: f32, to: f32| {
                    let points: Vec<(f32, f32)> = (0..=16)
                        .map(|step| {
                            let angle = from + (to - from) * step as f32 / 16.0;
                            (9.0 + 6.5 * angle.cos(), y + 2.5 * angle.sin())
                        })
                        .collect();
                    line(&points);
                };
                let pi = std::f32::consts::PI;
                ellipse(4.0, 0.0, 2.0 * pi);
                ellipse(9.0, 0.0, pi);
                ellipse(14.0, 0.0, pi);
                line(&[(2.5, 4.0), (2.5, 14.0)]);
                line(&[(15.5, 4.0), (15.5, 14.0)]);
            }
            FileType::Git => {
                line(&[(9.0, 1.5), (16.5, 9.0), (9.0, 16.5), (1.5, 9.0), (9.0, 1.5)]);
                for (x, y) in [(7.0, 5.5), (7.0, 12.5), (11.5, 9.5)] {
                    painter.circle_filled(at(x, y), 1.3 * scale, color);
                }
                line(&[(7.0, 5.5), (7.0, 12.5)]);
                line(&[(7.0, 5.5), (11.5, 9.5)]);
            }
            FileType::Svg => {
                painter.rect_stroke(
                    Rect::from_min_max(at(2.0, 2.0), at(16.0, 16.0)),
                    3.0 * scale,
                    stroke,
                    egui::StrokeKind::Middle,
                );
                painter.circle_stroke(at(6.8, 6.8), 1.8 * scale, stroke);
                line(&[(2.0, 14.0), (9.5, 8.5), (16.0, 13.5)]);
            }
            FileType::Svelte => {
                let font = FontId::proportional(15.0 * scale);
                text((9.0, 9.0), egui::Align2::CENTER_CENTER, "S", font, color);
            }
            FileType::Pdf => badge("PDF", Color32::WHITE),
            FileType::Text => {
                page();
                for y in [8.0, 10.75, 13.5] {
                    line(&[(6.0, y), (12.5, y)]);
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_the_type_of_a_file_by_its_extension() {
        assert_eq!(FileType::of("src/main/kotlin/Shop.kt"), FileType::Kotlin);
        assert_eq!(FileType::of("build.gradle.kts"), FileType::Kotlin);
        assert_eq!(FileType::of("README.MD"), FileType::Markdown);
        assert_eq!(FileType::of("config/app.yml"), FileType::Yaml);
        assert_eq!(FileType::of("package.json"), FileType::Json);
        assert_eq!(FileType::of("notes.txt"), FileType::Text);
        assert_eq!(FileType::of("LICENSE"), FileType::Text);
        assert_eq!(FileType::of(".gitignore"), FileType::Git);
        assert_eq!(FileType::of("logs/.gitkeep"), FileType::Git);
        assert_eq!(FileType::of("Dockerfile"), FileType::Docker);
        assert_eq!(FileType::of("docker/Dockerfile.dev"), FileType::Docker);
        assert_eq!(FileType::of("app.dockerfile"), FileType::Docker);
        assert_eq!(FileType::of("Containerfile"), FileType::Docker);
        assert_eq!(FileType::of(".dockerignore"), FileType::Docker);
        assert_eq!(FileType::of("db/V1__init.sql"), FileType::Sql);
        assert_eq!(FileType::of("logo.svg"), FileType::Svg);
        assert_eq!(FileType::of("src/App.svelte"), FileType::Svelte);
        assert_eq!(FileType::of("docs/Manual.PDF"), FileType::Pdf);
        assert!(FileType::Pdf.opens_outside());
        assert!(!FileType::Svg.opens_outside());
        assert_eq!(FileType::of("archive.tar.gz"), FileType::Text);
    }
}
