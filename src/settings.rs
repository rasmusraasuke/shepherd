use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dirs;

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub system_fonts: bool,
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        dirs::config().map(|dir| dir.join("settings.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|path| Self::load_from(&path))
            .unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).unwrap_or_default();
        fs::write(path, json + "\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn remembers_which_fonts_the_editor_should_use() {
        let dir = TempDir::new("settings");
        let path = dir.path().join("settings.json");
        assert!(Settings::load_from(&path).is_none());

        Settings { system_fonts: true }.save_to(&path).unwrap();

        assert!(Settings::load_from(&path).unwrap().system_fonts);
        assert_eq!(
            Settings::load_from(&path).unwrap(),
            Settings { system_fonts: true }
        );
    }

    #[test]
    fn ignores_a_settings_file_it_cannot_read() {
        let dir = TempDir::new("settings-broken");
        let path = dir.write("settings.json", "{not json");
        assert!(Settings::load_from(&path).is_none());
    }
}
