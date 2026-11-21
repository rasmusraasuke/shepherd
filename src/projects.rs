use std::fs;
use std::path::{Path, PathBuf};

use crate::dirs;

pub fn path() -> Option<PathBuf> {
    dirs::config().map(|dir| dir.join("projects"))
}

pub fn load(store: &Path) -> Vec<PathBuf> {
    fs::read_to_string(store)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn remember(store: &Path, root: &Path) {
    let mut projects = load(store);
    projects.retain(|project| project != root);
    projects.insert(0, root.to_path_buf());
    save(store, &projects);
}

pub fn forget(store: &Path, root: &Path) {
    let mut projects = load(store);
    projects.retain(|project| project != root);
    save(store, &projects);
}

fn save(store: &Path, projects: &[PathBuf]) {
    if let Some(dir) = store.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let text: String = projects
        .iter()
        .map(|project| format!("{}\n", project.display()))
        .collect();
    let _ = fs::write(store, text);
}

pub fn name(root: &Path) -> String {
    root.file_name().map_or_else(
        || root.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

pub fn shown_path(root: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home
        .as_deref()
        .and_then(|home| root.strip_prefix(home).ok())
    {
        Some(inside) if inside.as_os_str().is_empty() => "~".to_string(),
        Some(inside) => format!("~/{}", inside.display()),
        None => root.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn remembers_every_project_most_recent_first() {
        let dir = TempDir::new("projects");
        let store = dir.path().join("config/projects");
        assert!(load(&store).is_empty());

        remember(&store, Path::new("/work/shop"));
        remember(&store, Path::new("/work/blog"));
        remember(&store, Path::new("/work/shop"));
        assert_eq!(
            load(&store),
            [PathBuf::from("/work/shop"), PathBuf::from("/work/blog")]
        );

        forget(&store, Path::new("/work/shop"));
        assert_eq!(load(&store), [PathBuf::from("/work/blog")]);
    }

    #[test]
    fn shows_a_path_from_the_home_directory() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(shown_path(&home.join("Projects/shop")), "~/Projects/shop");
        assert_eq!(shown_path(&home), "~");
        assert_eq!(shown_path(Path::new("/opt/shop")), "/opt/shop");
        assert_eq!(name(Path::new("/work/shop")), "shop");
    }
}
