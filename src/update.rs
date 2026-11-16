use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use eframe::egui;
use serde::Deserialize;

use crate::dirs;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const REPOSITORY: &str = "rasmusraasuke/shepherd";

const API: &str = "https://api.github.com/repos";

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    version: String,
    url: String,
}

impl Release {
    pub fn version(&self) -> &str {
        &self.version
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Downloading(Release),
    Ready(Release),
    Failed(String),
}

pub struct Updates {
    phase: Arc<Mutex<Phase>>,
    ctx: egui::Context,
}

impl Updates {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            phase: Arc::default(),
            ctx,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase.lock().unwrap().clone()
    }

    pub fn start(&self) {
        {
            let mut phase = self.phase.lock().unwrap();
            if matches!(
                *phase,
                Phase::Checking | Phase::Downloading(_) | Phase::Ready(_)
            ) {
                return;
            }
            *phase = Phase::Checking;
        }
        let phase = Arc::clone(&self.phase);
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let found = latest();
            let release = match found {
                Ok(Some(release)) => release,
                Ok(None) => {
                    *phase.lock().unwrap() = Phase::Idle;
                    ctx.request_repaint();
                    return;
                }
                Err(e) => {
                    eprintln!("cannot check for updates: {e}");
                    *phase.lock().unwrap() = Phase::Idle;
                    ctx.request_repaint();
                    return;
                }
            };
            *phase.lock().unwrap() = Phase::Downloading(release.clone());
            ctx.request_repaint();
            let result = download(&release);
            let mut state = phase.lock().unwrap();
            if matches!(*state, Phase::Downloading(_)) {
                *state = match result {
                    Ok(()) => Phase::Ready(release),
                    Err(e) => Phase::Failed(e),
                };
            }
            drop(state);
            ctx.request_repaint();
        });
    }

    pub fn dismiss(&self) {
        *self.phase.lock().unwrap() = Phase::Idle;
    }

    pub fn fail(&self, error: String) {
        *self.phase.lock().unwrap() = Phase::Failed(error);
    }

    pub fn install(&self, root: &Path) -> Result<(), String> {
        let Phase::Ready(_) = self.phase() else {
            return Err("No update is ready to install".to_string());
        };
        let bundle = bundled().ok_or("Shepherd is not running from an application bundle")?;
        let unpacked = updates_dir()?;
        swap(&bundle, &unpacked.join("Shepherd.app"))?;
        Command::new("open")
            .arg("-n")
            .arg("-a")
            .arg(&bundle)
            .arg("--args")
            .arg(root)
            .status()
            .map_err(|e| format!("Cannot start the new Shepherd: {e}"))?;
        Ok(())
    }

    pub fn reveal(&self) -> Result<(), String> {
        let Phase::Ready(release) = self.phase() else {
            return Err("No update has been downloaded".to_string());
        };
        let archive = updates_dir()?.join(archive_name(&release.version));
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = Command::new("open");
            command.arg("-R").arg(&archive);
            command
        };
        #[cfg(not(target_os = "macos"))]
        let mut command = {
            let mut command = Command::new("xdg-open");
            command.arg(archive.parent().unwrap_or(Path::new(".")));
            command
        };
        command
            .status()
            .map_err(|e| format!("Cannot show {}: {e}", archive.display()))?;
        Ok(())
    }
}

pub fn bundled() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?;
    bundle
        .extension()
        .is_some_and(|extension| extension == "app")
        .then(|| bundle.to_path_buf())
}

fn swap(bundle: &Path, new: &Path) -> Result<(), String> {
    let staged = bundle.with_file_name(".Shepherd.app.new");
    let old = bundle.with_file_name(".Shepherd.app.old");
    let _ = fs::remove_dir_all(&staged);
    let _ = fs::remove_dir_all(&old);
    copy(new, &staged)?;
    fs::rename(bundle, &old).map_err(|e| format!("Cannot move {}: {e}", bundle.display()))?;
    if let Err(e) = fs::rename(&staged, bundle) {
        let _ = fs::rename(&old, bundle);
        return Err(format!("Cannot put {} in place: {e}", bundle.display()));
    }
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

fn updates_dir() -> Result<PathBuf, String> {
    dirs::cache()
        .map(|dir| dir.join("updates"))
        .ok_or_else(|| "No cache directory to keep the update in".to_string())
}

fn latest() -> Result<Option<Release>, String> {
    let url = std::env::var("SHEPHERD_UPDATE_API")
        .unwrap_or_else(|_| format!("{API}/{REPOSITORY}/releases/latest"));
    let mut args = vec![
        "-fsSL".to_string(),
        "--connect-timeout".to_string(),
        "10".to_string(),
        "--max-time".to_string(),
        "30".to_string(),
    ];
    if let Some(token) = token() {
        args.extend(["-H".to_string(), format!("Authorization: Bearer {token}")]);
    }
    args.push(url);
    let body = run("curl", &args)?;
    parse(&body, VERSION, std::env::consts::OS, std::env::consts::ARCH)
}

fn token() -> Option<String> {
    ["SHEPHERD_GITHUB_TOKEN", "GITHUB_TOKEN"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok())
        .filter(|token| !token.is_empty())
        .or_else(|| {
            let path = dirs::config()?.join("github-token");
            let token = fs::read_to_string(path).ok()?;
            let token = token.trim();
            (!token.is_empty()).then(|| token.to_string())
        })
}

fn archive_name(version: &str) -> String {
    let extension = if cfg!(target_os = "macos") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("Shepherd-{version}.{extension}")
}

fn download(release: &Release) -> Result<(), String> {
    let dir = updates_dir()?;
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot make {}: {e}", dir.display()))?;
    let archive = dir.join(archive_name(&release.version));
    let mut args = vec![
        "-fsSL".to_string(),
        "--connect-timeout".to_string(),
        "10".to_string(),
        "--max-time".to_string(),
        "600".to_string(),
        "-H".to_string(),
        "Accept: application/octet-stream".to_string(),
    ];
    if let Some(token) = token() {
        args.extend(["-H".to_string(), format!("Authorization: Bearer {token}")]);
    }
    args.extend([
        "-o".to_string(),
        archive.display().to_string(),
        release.url.clone(),
    ]);
    run("curl", &args)?;
    check_archive(&archive)?;
    unpack(&archive, &dir)
}

fn check_archive(path: &Path) -> Result<(), String> {
    use std::io::Read as _;
    let mut magic = [0; 4];
    let mut file =
        fs::File::open(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    file.read_exact(&mut magic)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let expected: &[u8] = if cfg!(target_os = "macos") {
        b"PK\x03\x04"
    } else {
        &[0x1f, 0x8b]
    };
    if magic.starts_with(expected) {
        Ok(())
    } else {
        Err("The update that was downloaded is not an application".to_string())
    }
}

#[cfg(target_os = "macos")]
fn unpack(archive: &Path, dir: &Path) -> Result<(), String> {
    run(
        "ditto",
        &[
            "-x".to_string(),
            "-k".to_string(),
            archive.display().to_string(),
            dir.display().to_string(),
        ],
    )?;
    exists(dir)
}

#[cfg(not(target_os = "macos"))]
fn unpack(archive: &Path, dir: &Path) -> Result<(), String> {
    run(
        "tar",
        &[
            "-xzf".to_string(),
            archive.display().to_string(),
            "-C".to_string(),
            dir.display().to_string(),
        ],
    )?;
    exists(dir)
}

#[cfg(target_os = "macos")]
fn copy(from: &Path, to: &Path) -> Result<(), String> {
    run(
        "ditto",
        &[from.display().to_string(), to.display().to_string()],
    )
    .map(|_| ())
}

#[cfg(not(target_os = "macos"))]
fn copy(from: &Path, to: &Path) -> Result<(), String> {
    run(
        "cp",
        &[
            "-R".to_string(),
            from.display().to_string(),
            to.display().to_string(),
        ],
    )
    .map(|_| ())
}

#[cfg(target_os = "macos")]
fn exists(dir: &Path) -> Result<(), String> {
    let app = dir.join("Shepherd.app");
    app.is_dir()
        .then_some(())
        .ok_or_else(|| "The update does not contain Shepherd.app".to_string())
}

#[cfg(not(target_os = "macos"))]
fn exists(dir: &Path) -> Result<(), String> {
    let found = dir.join("shepherd").is_file()
        || fs::read_dir(dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| entry.path().join("shepherd").is_file());
    found
        .then_some(())
        .ok_or_else(|| "The update does not contain the shepherd program".to_string())
}

fn run(program: &str, args: &[String]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("Cannot run {program}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            format!("{program} could not do that")
        } else {
            format!("{program}: {stderr}")
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    url: String,
}

impl Asset {
    fn fits(&self, platform: &str, arch: &str) -> bool {
        let extension = if platform == "macos" {
            ".zip"
        } else {
            ".tar.gz"
        };
        self.name.contains(platform)
            && self.name.ends_with(extension)
            && (self.name.contains("universal") || self.name.contains(arch))
    }
}

fn parse(json: &str, current: &str, platform: &str, arch: &str) -> Result<Option<Release>, String> {
    let release: ApiRelease =
        serde_json::from_str(json).map_err(|e| format!("Cannot read the update: {e}"))?;
    let version = release.tag_name.trim_start_matches('v').to_string();
    if !newer(&version, current) {
        return Ok(None);
    }
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.fits(platform, arch) && asset.name.contains("universal"))
        .or_else(|| {
            release
                .assets
                .iter()
                .find(|asset| asset.fits(platform, arch))
        })
        .ok_or_else(|| format!("Shepherd {version} has no build for this computer"))?;
    Ok(Some(Release {
        version,
        url: asset.url.clone(),
    }))
}

pub fn newer(latest: &str, current: &str) -> bool {
    parts(latest) > parts(current)
}

fn parts(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = r#"{
        "tag_name": "v0.2.0",
        "assets": [
            {"name": "Shepherd-0.2.0-macos-x86_64.zip", "url": "https://example.com/intel"},
            {"name": "Shepherd-0.2.0-macos-universal.zip", "url": "https://example.com/universal"},
            {"name": "latest.json", "url": "https://example.com/latest"}
        ]
    }"#;

    #[test]
    fn compares_versions_by_their_numbers() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(newer("0.10.0", "0.9.0"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn picks_the_universal_build_of_a_newer_release() {
        let release = parse(RELEASE, "0.1.0", "macos", "aarch64")
            .unwrap()
            .unwrap();
        assert_eq!(release.version(), "0.2.0");
        assert_eq!(release.url, "https://example.com/universal");
        let release = parse(RELEASE, "0.1.0", "macos", "x86_64").unwrap().unwrap();
        assert_eq!(release.url, "https://example.com/universal");
    }

    #[test]
    fn picks_the_build_for_linux() {
        let json = r#"{
            "tag_name": "v0.2.0",
            "assets": [
                {"name": "Shepherd-0.2.0-linux-x86_64.tar.gz", "url": "https://example.com/linux"},
                {"name": "Shepherd-0.2.0-macos-universal.zip", "url": "https://example.com/mac"}
            ]
        }"#;
        let release = parse(json, "0.1.0", "linux", "x86_64").unwrap().unwrap();
        assert_eq!(release.url, "https://example.com/linux");
        assert!(parse(json, "0.1.0", "linux", "aarch64").is_err());
        let release = parse(json, "0.1.0", "macos", "aarch64").unwrap().unwrap();
        assert_eq!(release.url, "https://example.com/mac");
    }

    #[test]
    fn names_the_archive_the_platform_downloads() {
        let name = archive_name("1.2.3");
        if cfg!(target_os = "macos") {
            assert_eq!(name, "Shepherd-1.2.3.zip");
        } else {
            assert_eq!(name, "Shepherd-1.2.3.tar.gz");
        }
    }

    #[test]
    fn ignores_the_release_that_is_running() {
        assert!(
            parse(RELEASE, "0.2.0", "macos", "aarch64")
                .unwrap()
                .is_none()
        );
        assert!(
            parse(RELEASE, "0.3.0", "macos", "aarch64")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn says_when_a_release_has_no_build_for_this_computer() {
        let json = r#"{"tag_name": "v0.2.0", "assets": [{"name": "other.zip", "url": "u"}]}"#;
        assert!(parse(json, "0.1.0", "macos", "aarch64").is_err());
        assert!(parse("not json", "0.1.0", "macos", "aarch64").is_err());
    }

    #[test]
    fn replaces_the_app_with_the_downloaded_one() {
        let dir = crate::testing::TempDir::new("update-swap");
        let bundle = dir.path().join("Shepherd.app");
        let new = dir.path().join("downloaded/Shepherd.app");
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::create_dir_all(new.join("Contents/MacOS")).unwrap();
        fs::write(bundle.join("Contents/MacOS/shepherd"), "old").unwrap();
        fs::write(new.join("Contents/MacOS/shepherd"), "new").unwrap();

        swap(&bundle, &new).unwrap();

        let marker = bundle.join("Contents/MacOS/shepherd");
        assert_eq!(fs::read_to_string(marker).unwrap(), "new");
        assert!(!dir.path().join(".Shepherd.app.old").exists());
        assert!(!dir.path().join(".Shepherd.app.new").exists());
        assert!(new.join("Contents/MacOS/shepherd").exists());
    }
}
