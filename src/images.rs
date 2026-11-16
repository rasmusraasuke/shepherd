use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use eframe::egui::{self, ColorImage};

use crate::agents::Agent;

pub const OLD: Duration = Duration::from_secs(24 * 60 * 60);

const EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"];

pub fn dir() -> PathBuf {
    std::env::temp_dir().join("shepherd-images")
}

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            EXTENSIONS
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

pub fn take_pasted(ctx: &egui::Context) -> Vec<Arc<ColorImage>> {
    ctx.input_mut(|input| {
        let mut pictures = Vec::new();
        input.events.retain(|event| match event {
            egui::Event::PasteImage(picture) => {
                pictures.push(picture.clone());
                false
            }
            _ => true,
        });
        pictures
    })
}

pub fn save(dir: &Path, picture: &ColorImage) -> Result<PathBuf, String> {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    fs::create_dir_all(dir).map_err(|error| format!("Cannot write the image: {error}"))?;
    let name = format!(
        "image-{}-{}.png",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::SeqCst)
    );
    let path = dir.join(name);
    let file =
        fs::File::create(&path).map_err(|error| format!("Cannot write the image: {error}"))?;
    let mut encoder = png::Encoder::new(
        BufWriter::new(file),
        picture.size[0] as u32,
        picture.size[1] as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("Cannot write the image: {error}"))?;
    let pixels: Vec<u8> = picture
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_array())
        .collect();
    writer
        .write_image_data(&pixels)
        .map_err(|error| format!("Cannot write the image: {error}"))?;
    Ok(path)
}

pub fn hand(agent: Option<Agent>, paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| agent.map_or_else(|| path.display().to_string(), |agent| agent.image(path)))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn remove(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

pub fn remove_old(dir: &Path, older_than: Duration) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| {
                now.duration_since(modified)
                    .is_ok_and(|age| age > older_than)
            });
        if old && is_image(&entry.path()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::Color32;

    use super::*;
    use crate::testing::TempDir;

    fn picture() -> ColorImage {
        ColorImage::new(
            [2, 1],
            vec![Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 255, 0)],
        )
    }

    #[test]
    fn knows_the_files_that_hold_pictures() {
        for name in [
            "shot.png",
            "photo.JPG",
            "a.jpeg",
            "b.gif",
            "c.webp",
            "d.tiff",
        ] {
            assert!(is_image(Path::new(name)), "{name}");
        }
        for name in ["main.rs", "notes.txt", "png", "a.png.bak"] {
            assert!(!is_image(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn writes_the_picture_as_a_png_file() {
        let dir = TempDir::new("images-save");
        let path = save(dir.path(), &picture()).unwrap();
        assert_eq!(path.extension().unwrap(), "png");
        let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(
            &bytes[..info.buffer_size()],
            &[255, 0, 0, 255, 0, 255, 0, 255]
        );
        assert_ne!(save(dir.path(), &picture()).unwrap(), path);
    }

    #[test]
    fn removes_the_pictures_it_was_given() {
        let dir = TempDir::new("images-remove");
        let paths = [
            save(dir.path(), &picture()).unwrap(),
            save(dir.path(), &picture()).unwrap(),
        ];
        remove(&paths);
        assert!(paths.iter().all(|path| !path.exists()));
    }

    #[test]
    fn removes_the_pictures_that_are_old() {
        let dir = TempDir::new("images-old");
        let path = save(dir.path(), &picture()).unwrap();
        remove_old(dir.path(), Duration::from_secs(3600));
        assert!(path.exists());
        remove_old(dir.path(), Duration::ZERO);
        assert!(!path.exists());
    }

    #[test]
    fn hands_pictures_over_as_text() {
        let paths = [
            PathBuf::from("/tmp/shot.png"),
            PathBuf::from("/tmp/two.png"),
        ];
        assert_eq!(
            hand(Some(Agent::Claude), &paths),
            "/tmp/shot.png /tmp/two.png"
        );
        assert_eq!(hand(None, &paths), "/tmp/shot.png /tmp/two.png");
        assert_eq!(hand(Some(Agent::Opencode), &paths[..1]), "/tmp/shot.png");
    }
}
