use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        static CREATED: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("editor-test-{}-{}-{name}", std::process::id(), CREATED.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn write(&self, name: &str, content: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
