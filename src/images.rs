use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use crate::agents::Agent;

pub const OLD: Duration = Duration::from_secs(24 * 60 * 60);

const EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"];

pub type Picture = arboard::ImageData<'static>;

#[derive(Clone, Copy)]
pub struct Clipboard {
    pub text: fn() -> Option<String>,
    pub image: fn() -> Option<Picture>,
}

impl Clipboard {
    pub fn read_text(&self) -> Option<String> {
        (self.text)()
    }

    pub fn read_image(&self) -> Option<Picture> {
        (self.image)()
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self {
            text: read_text,
            image: read_image,
        }
    }
}

fn read_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

fn read_image() -> Option<Picture> {
    arboard::Clipboard::new().ok()?.get_image().ok()
}

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

pub fn save(dir: &Path, picture: Picture) -> Result<PathBuf, String> {
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
        picture.width as u32,
        picture.height as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("Cannot write the image: {error}"))?;
    writer
        .write_image_data(&picture.bytes)
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
    use std::borrow::Cow;

    use super::*;
    use crate::testing::TempDir;

    fn picture() -> Picture {
        Picture {
            width: 2,
            height: 1,
            bytes: Cow::Owned(vec![255, 0, 0, 255, 0, 255, 0, 255]),
        }
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
        let path = save(dir.path(), picture()).unwrap();
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
        assert_ne!(save(dir.path(), picture()).unwrap(), path);
    }

    #[test]
    fn removes_the_pictures_it_was_given() {
        let dir = TempDir::new("images-remove");
        let paths = [
            save(dir.path(), picture()).unwrap(),
            save(dir.path(), picture()).unwrap(),
        ];
        remove(&paths);
        assert!(paths.iter().all(|path| !path.exists()));
    }

    #[test]
    fn removes_the_pictures_that_are_old() {
        let dir = TempDir::new("images-old");
        let path = save(dir.path(), picture()).unwrap();
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
