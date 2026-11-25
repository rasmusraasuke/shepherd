use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::thread;

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use resvg::{tiny_skia, usvg};

const MAX_SIDE: u32 = 4096;
const SVG_SIDE: f32 = 2048.0;
const SVG_ZOOM: f32 = 8.0;
const CHECKER: f32 = 8.0;
const PADDING: f32 = 16.0;

pub enum Source {
    Svg { text: String, dir: Option<PathBuf> },
    File(PathBuf),
}

#[derive(Debug)]
pub struct Picture {
    pub image: ColorImage,
    pub width: u32,
    pub height: u32,
    pub vector: bool,
}

struct Shown {
    texture: TextureHandle,
    width: u32,
    height: u32,
    vector: bool,
}

#[derive(Default)]
pub struct ImageView {
    loading: Option<(u64, Receiver<Result<Picture, String>>)>,
    loaded: Option<u64>,
    shown: Option<Shown>,
    error: Option<String>,
    checker: Option<TextureHandle>,
}

impl ImageView {
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        key: u64,
        source: impl FnOnce() -> Source,
    ) -> Option<String> {
        self.receive(ui.ctx());
        if self.loading.is_none() && self.loaded != Some(key) {
            let (sender, receiver) = mpsc::channel();
            let source = source();
            let ctx = ui.ctx().clone();
            thread::spawn(move || {
                let _ = sender.send(decode(source));
                ctx.request_repaint();
            });
            self.loading = Some((key, receiver));
        }
        let rect = ui.max_rect();
        ui.allocate_rect(rect, egui::Sense::hover());
        let Some(shown) = &self.shown else {
            let message = self.error.as_deref().unwrap_or("Loading…");
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                message,
                egui::TextStyle::Body.resolve(ui.style()),
                ui.visuals().weak_text_color(),
            );
            return None;
        };
        let natural = egui::vec2(shown.width as f32, shown.height as f32).max(egui::vec2(1.0, 1.0));
        let room = (rect.size() - egui::vec2(PADDING, PADDING) * 2.0).max(egui::vec2(1.0, 1.0));
        let zoom = (room.x / natural.x)
            .min(room.y / natural.y)
            .min(if shown.vector { SVG_ZOOM } else { 1.0 });
        let place = egui::Rect::from_center_size(rect.center(), natural * zoom);
        let checker = self.checker.get_or_insert_with(|| checker(ui.ctx()));
        let painter = ui.painter().with_clip_rect(rect);
        painter.image(
            checker.id(),
            place,
            egui::Rect::from_min_size(egui::Pos2::ZERO, place.size() / (2.0 * CHECKER)),
            Color32::WHITE,
        );
        painter.image(
            shown.texture.id(),
            place,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        Some(
            self.error
                .clone()
                .unwrap_or_else(|| format!("{} × {} px", shown.width, shown.height)),
        )
    }

    fn receive(&mut self, ctx: &egui::Context) {
        let Some((key, receiver)) = &self.loading else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("Cannot show the image".to_string()),
        };
        self.loaded = Some(*key);
        self.loading = None;
        match result {
            Ok(picture) => {
                self.error = None;
                self.shown = Some(Shown {
                    texture: ctx.load_texture(
                        "image-view",
                        picture.image,
                        TextureOptions {
                            mipmap_mode: Some(egui::TextureFilter::Linear),
                            ..TextureOptions::LINEAR
                        },
                    ),
                    width: picture.width,
                    height: picture.height,
                    vector: picture.vector,
                });
            }
            Err(error) => self.error = Some(error),
        }
    }
}

fn checker(ctx: &egui::Context) -> TextureHandle {
    let (light, dark) = (Color32::from_gray(204), Color32::from_gray(153));
    ctx.load_texture(
        "image-checker",
        ColorImage::new([2, 2], vec![light, dark, dark, light]),
        TextureOptions {
            wrap_mode: egui::TextureWrapMode::Repeat,
            ..TextureOptions::NEAREST
        },
    )
}

pub fn decode(source: Source) -> Result<Picture, String> {
    match source {
        Source::Svg { text, dir } => svg(&text, dir),
        Source::File(path) => {
            let bytes = fs::read(&path).map_err(|e| format!("Cannot open the image: {e}"))?;
            raster(&bytes)
        }
    }
}

fn raster(bytes: &[u8]) -> Result<Picture, String> {
    let image =
        image::load_from_memory(bytes).map_err(|e| format!("Cannot show the image: {e}"))?;
    let (width, height) = (image.width(), image.height());
    let image = if width.max(height) > MAX_SIDE {
        image.thumbnail(MAX_SIDE, MAX_SIDE)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Ok(Picture {
        image: ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        ),
        width,
        height,
        vector: false,
    })
}

fn svg(text: &str, dir: Option<PathBuf>) -> Result<Picture, String> {
    let options = usvg::Options {
        resources_dir: dir,
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree =
        usvg::Tree::from_str(text, &options).map_err(|e| format!("Cannot show the SVG: {e}"))?;
    let size = tree.size();
    let scale = SVG_SIDE / size.width().max(size.height());
    let (width, height) = (
        (size.width() * scale).ceil().max(1.0) as u32,
        (size.height() * scale).ceil().max(1.0) as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("The SVG has no size")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(Picture {
        image: ColorImage::from_rgba_premultiplied(
            [width as usize, height as usize],
            pixmap.data(),
        ),
        width: size.width().round() as u32,
        height: size.height().round() as u32,
        vector: true,
    })
}

fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut fonts = usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        })
        .clone()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        let pixels: Vec<u8> = (0..width * height).flat_map(|_| [255, 0, 0, 255]).collect();
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
        bytes
    }

    #[test]
    fn decodes_pictures() {
        let picture = raster(&png(3, 2)).unwrap();
        assert_eq!((picture.width, picture.height), (3, 2));
        assert_eq!(picture.image.size, [3, 2]);
        assert_eq!(picture.image.pixels[0], Color32::RED);
        assert!(!picture.vector);
        assert!(
            raster(b"not an image")
                .unwrap_err()
                .starts_with("Cannot show the image")
        );
    }

    #[test]
    fn makes_big_pictures_small_enough_for_the_screen() {
        let picture = raster(&png(MAX_SIDE * 2, 4)).unwrap();
        assert_eq!((picture.width, picture.height), (MAX_SIDE * 2, 4));
        assert_eq!(picture.image.size[0], MAX_SIDE as usize);
    }

    #[test]
    fn draws_svgs_sharp_at_any_size() {
        let picture = svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12"><rect width="24" height="12" fill="blue"/></svg>"#,
            None,
        )
        .unwrap();
        assert_eq!((picture.width, picture.height), (24, 12));
        assert_eq!(picture.image.size, [2048, 1024]);
        assert_eq!(picture.image.pixels[0], Color32::BLUE);
        assert!(picture.vector);
        assert!(
            svg("<svg", None)
                .unwrap_err()
                .starts_with("Cannot show the SVG")
        );
    }
}
