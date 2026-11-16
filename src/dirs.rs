use std::path::PathBuf;

pub fn config() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Shepherd")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("shepherd")
    })
}

pub fn cache() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Caches/Shepherd")
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".cache"))
            .join("shepherd")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_its_files_under_the_home_directory() {
        let config = config().unwrap();
        let cache = cache().unwrap();
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert!(config.starts_with(&home));
        assert!(cache.starts_with(&home));
        assert_ne!(config, cache);
    }
}
