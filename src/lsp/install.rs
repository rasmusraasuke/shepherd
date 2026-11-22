use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Language;

const RECHECK: u64 = 24 * 60 * 60;

const NODE_MAJOR: u64 = 20;

pub enum Method {
    Release {
        repo: &'static str,
        asset: &'static str,
    },
    JetBrains,
    Npm {
        packages: &'static [&'static str],
        entry: &'static str,
    },
    Go {
        module: &'static str,
    },
    Manual,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Installed {
    pub version: String,
    pub command: Vec<String>,
    pub checked: u64,
}

#[derive(Clone)]
pub struct Sources {
    pub github: String,
    pub jetbrains: String,
    pub node: String,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            github: "https://api.github.com".to_string(),
            jetbrains: "https://download.jetbrains.com".to_string(),
            node: "https://nodejs.org/dist".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct Progress {
    text: Arc<Mutex<Option<String>>>,
    ctx: egui::Context,
}

impl Progress {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            text: Arc::default(),
            ctx,
        }
    }

    fn say(&self, text: String) {
        if let Ok(mut held) = self.text.lock() {
            *held = Some(text);
        }
        self.ctx.request_repaint();
    }

    pub fn text(&self) -> Option<String> {
        self.text.lock().ok()?.clone()
    }
}

#[derive(Clone)]
pub struct Installer {
    pub dir: PathBuf,
    pub path_env: String,
    pub download: bool,
    pub sources: Sources,
}

type Found = Option<(String, Vec<String>)>;

impl Installer {
    pub fn new(dir: PathBuf, path_env: String, download: bool) -> Self {
        Self {
            dir,
            path_env,
            download,
            sources: Sources::default(),
        }
    }

    pub fn find(&self, language: &Language, progress: &Progress) -> Result<Vec<String>, String> {
        let mut command = self.program_of(language, progress)?;
        command.extend(language.args.iter().map(|arg| arg.to_string()));
        Ok(command)
    }

    fn program_of(&self, language: &Language, progress: &Progress) -> Result<Vec<String>, String> {
        if let Some(command) = self.on_path(language) {
            return Ok(command);
        }
        let dir = self.dir.join(language.name.to_lowercase());
        let installed = read_installed(&dir);
        if let Some(installed) = &installed
            && now().saturating_sub(installed.checked) < RECHECK
        {
            return Ok(installed.command.clone());
        }
        if !self.download {
            return installed.map(|installed| installed.command).ok_or_else(|| {
                format!(
                    "No {} language server found: {} is not on the PATH and downloading \
                     language servers is turned off",
                    language.name, language.program
                )
            });
        }
        match self.install(language, &dir, installed.as_ref(), progress) {
            Ok(installed) => Ok(installed.command),
            Err(error) => installed.map(|installed| installed.command).ok_or(error),
        }
    }

    fn on_path(&self, language: &Language) -> Option<Vec<String>> {
        let program = self.program(language.program)?;
        let works = language.check.is_empty()
            || Command::new(&program)
                .args(language.check)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
        works.then(|| command(&program))
    }

    fn program(&self, name: &str) -> Option<PathBuf> {
        std::env::split_paths(&self.path_env)
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
    }

    fn install(
        &self,
        language: &Language,
        dir: &Path,
        installed: Option<&Installed>,
        progress: &Progress,
    ) -> Result<Installed, String> {
        let current = installed.map(|installed| installed.version.as_str());
        let fail = |error: String| {
            format!(
                "Cannot install the {} language server: {error}",
                language.name
            )
        };
        fs::create_dir_all(dir).map_err(|e| fail(e.to_string()))?;
        progress.say(format!(
            "Looking for the newest {} language server…",
            language.name
        ));
        let found = match &language.method {
            Method::Release { repo, asset } => {
                self.release(language, dir, repo, asset, current, progress)
            }
            Method::JetBrains => self.jetbrains(language, dir, current, progress),
            Method::Npm { packages, entry } => {
                self.npm(language, dir, packages, entry, current, progress)
            }
            Method::Go { module } => self.go(language, dir, module, progress),
            Method::Manual => {
                return Err(format!(
                    "No {} language server found: install {} on the PATH",
                    language.name, language.program
                ));
            }
        }
        .map_err(fail)?;
        let installed = match (found, installed) {
            (Some((version, command)), _) => Installed {
                version,
                command,
                checked: now(),
            },
            (None, Some(installed)) => Installed {
                checked: now(),
                ..installed.clone()
            },
            (None, None) => return Err(fail("nothing was installed".to_string())),
        };
        let json = serde_json::to_string_pretty(&installed).unwrap_or_default();
        fs::write(dir.join("installed.json"), json).map_err(|e| fail(e.to_string()))?;
        Ok(installed)
    }

    fn release(
        &self,
        language: &Language,
        dir: &Path,
        repo: &str,
        asset: &str,
        current: Option<&str>,
        progress: &Progress,
    ) -> Result<Found, String> {
        let release = self.latest_release(repo)?;
        let version = release["tag_name"].as_str().ok_or("no release")?;
        if current == Some(version) {
            return Ok(None);
        }
        let name = format!("{asset}-{}-{}.gz", std::env::consts::ARCH, target());
        let found = release["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|held| held["name"] == name)
            .ok_or_else(|| format!("no {name} in release {version}"))?;
        let url = found["browser_download_url"]
            .as_str()
            .ok_or("no download link")?;
        let sha = found["digest"]
            .as_str()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .ok_or("no checksum")?;
        let archive = dir.join("download.part");
        let label = format!("{} language server {version}", language.name);
        self.download(url, &archive, &label, progress)?;
        verify(&archive, sha)?;
        let target = dir.join(version);
        fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        let binary = target.join(language.program);
        let out = fs::File::create(&binary).map_err(|e| e.to_string())?;
        run(Command::new("gunzip").arg("-c").arg(&archive).stdout(out))?;
        make_executable(&binary)?;
        prune(dir, version);
        Ok(Some((version.to_string(), command(&binary))))
    }

    fn jetbrains(
        &self,
        language: &Language,
        dir: &Path,
        current: Option<&str>,
        progress: &Progress,
    ) -> Result<Found, String> {
        let release = self.latest_release("Kotlin/kotlin-lsp")?;
        let tag = release["tag_name"].as_str().ok_or("no release")?;
        let version = tag.trim_start_matches("kotlin-lsp/v");
        if current == Some(version) {
            return Ok(None);
        }
        let arch = if std::env::consts::ARCH == "aarch64" {
            "-aarch64"
        } else {
            ""
        };
        let extension = if cfg!(target_os = "macos") {
            "sit"
        } else {
            "tar.gz"
        };
        let url = format!(
            "{}/language-server/kotlin-server/{version}/kotlin-server-{version}{arch}.{extension}",
            self.sources.jetbrains
        );
        let sums = fetch(&format!("{url}.sha256"))?;
        let sha = sums.split_whitespace().next().ok_or("no checksum")?;
        let archive = dir.join("download.part");
        let label = format!("{} language server {version}", language.name);
        self.download(&url, &archive, &label, progress)?;
        verify(&archive, sha)?;
        progress.say(format!("Unpacking the {label}…"));
        let target = dir.join(version);
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        unpack(&archive, &target, extension == "sit")?;
        let _ = fs::remove_file(&archive);
        let binary = target.join(format!("kotlin-server-{version}/bin/intellij-server"));
        if !binary.is_file() {
            return Err(format!("no {} in the download", binary.display()));
        }
        prune(dir, version);
        Ok(Some((version.to_string(), command(&binary))))
    }

    fn npm(
        &self,
        language: &Language,
        dir: &Path,
        packages: &[&str],
        entry: &str,
        current: Option<&str>,
        progress: &Progress,
    ) -> Result<Found, String> {
        let node = self.node(progress)?;
        let bin = node.parent().unwrap_or(Path::new("")).to_path_buf();
        let path_env = std::env::join_paths(
            std::iter::once(bin.clone()).chain(std::env::split_paths(&self.path_env)),
        )
        .map_err(|e| e.to_string())?;
        let npm = |args: &[&str]| {
            run(Command::new(bin.join("npm"))
                .args(args)
                .env("PATH", &path_env)
                .current_dir(dir))
        };
        let version = npm(&["view", packages[0], "version"])?.trim().to_string();
        if current == Some(version.as_str()) {
            return Ok(None);
        }
        progress.say(format!(
            "Installing the {} language server {version} with npm…",
            language.name
        ));
        let prefix = dir.to_string_lossy().into_owned();
        let latest: Vec<String> = packages
            .iter()
            .map(|name| format!("{name}@latest"))
            .collect();
        let mut args = vec![
            "install",
            "--prefix",
            &prefix,
            "--no-audit",
            "--no-fund",
            "--no-package-lock",
            "--save-exact",
        ];
        args.extend(latest.iter().map(String::as_str));
        npm(&args)?;
        let command = vec![
            node.to_string_lossy().into_owned(),
            dir.join(entry).to_string_lossy().into_owned(),
        ];
        Ok(Some((version, command)))
    }

    fn node(&self, progress: &Progress) -> Result<PathBuf, String> {
        if let Some(node) = self.program("node")
            && run(Command::new(&node).arg("--version"))
                .ok()
                .and_then(|version| node_major(&version))
                .is_some_and(|major| major >= NODE_MAJOR)
        {
            return Ok(node);
        }
        let dir = self.dir.join("node");
        if let Some(installed) = read_installed(&dir) {
            return Ok(PathBuf::from(&installed.command[0]));
        }
        progress.say("Looking for the newest Node.js…".to_string());
        let index: Value =
            serde_json::from_str(&fetch(&format!("{}/index.json", self.sources.node))?)
                .map_err(|e| e.to_string())?;
        let version = index
            .as_array()
            .into_iter()
            .flatten()
            .find(|release| release["lts"].is_string())
            .and_then(|release| release["version"].as_str())
            .ok_or("no Node.js release")?;
        let os = if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        };
        let arch = if std::env::consts::ARCH == "aarch64" {
            "arm64"
        } else {
            "x64"
        };
        let name = format!("node-{version}-{os}-{arch}");
        let sums = fetch(&format!("{}/{version}/SHASUMS256.txt", self.sources.node))?;
        let sha = sums
            .lines()
            .find(|line| line.ends_with(&format!("{name}.tar.gz")))
            .and_then(|line| line.split_whitespace().next())
            .ok_or("no checksum for Node.js")?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let archive = dir.join("download.part");
        let url = format!("{}/{version}/{name}.tar.gz", self.sources.node);
        self.download(&url, &archive, &format!("Node.js {version}"), progress)?;
        verify(&archive, sha)?;
        unpack(&archive, &dir, false)?;
        let _ = fs::remove_file(&archive);
        let node = dir.join(format!("{name}/bin/node"));
        let installed = Installed {
            version: version.to_string(),
            command: vec![node.to_string_lossy().into_owned()],
            checked: now(),
        };
        let json = serde_json::to_string_pretty(&installed).unwrap_or_default();
        fs::write(dir.join("installed.json"), json).map_err(|e| e.to_string())?;
        Ok(node)
    }

    fn go(
        &self,
        language: &Language,
        dir: &Path,
        module: &str,
        progress: &Progress,
    ) -> Result<Found, String> {
        let go = self
            .program("go")
            .ok_or("Go is needed to install it, and go is not on the PATH")?;
        progress.say(format!(
            "Installing the {} language server with go install…",
            language.name
        ));
        let bin = dir.join("bin");
        run(Command::new(go)
            .args(["install", &format!("{module}@latest")])
            .env("GOBIN", &bin)
            .env("GO111MODULE", "on")
            .env("PATH", &self.path_env)
            .current_dir(dir))?;
        let binary = bin.join(language.program);
        Ok(Some((now().to_string(), command(&binary))))
    }

    fn latest_release(&self, repo: &str) -> Result<Value, String> {
        let text = fetch(&format!(
            "{}/repos/{repo}/releases/latest",
            self.sources.github
        ))?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    fn download(
        &self,
        url: &str,
        to: &Path,
        label: &str,
        progress: &Progress,
    ) -> Result<(), String> {
        let total = run(Command::new("curl").args(["-fsSLI", url]))
            .ok()
            .and_then(|headers| content_length(&headers));
        let _ = fs::remove_file(to);
        let mut child = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(to)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run curl: {e}"))?;
        loop {
            let done = fs::metadata(to).map_or(0, |meta| meta.len());
            progress.say(match total {
                Some(total) => format!(
                    "Downloading the {label}: {} of {} MB",
                    done / 1_000_000,
                    total / 1_000_000
                ),
                None => format!("Downloading the {label}: {} MB", done / 1_000_000),
            });
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                if status.success() {
                    return Ok(());
                }
                let mut error = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut stderr, &mut error);
                }
                let _ = fs::remove_file(to);
                return Err(format!("cannot download {url}: {}", error.trim()));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

fn command(program: &Path) -> Vec<String> {
    vec![program.to_string_lossy().into_owned()]
}

fn target() -> &'static str {
    if cfg!(target_os = "macos") {
        "apple-darwin"
    } else {
        "unknown-linux-gnu"
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn read_installed(dir: &Path) -> Option<Installed> {
    let installed: Installed =
        serde_json::from_str(&fs::read_to_string(dir.join("installed.json")).ok()?).ok()?;
    Path::new(installed.command.first()?)
        .is_file()
        .then_some(installed)
}

fn prune(dir: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name != keep && name != "installed.json" {
            let path = entry.path();
            let _ = fs::remove_dir_all(&path).or_else(|_| fs::remove_file(&path));
        }
    }
}

fn run(command: &mut Command) -> Result<String, String> {
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let error = String::from_utf8_lossy(&output.stderr);
    Err(error
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("failed")
        .trim()
        .to_string())
}

fn fetch(url: &str) -> Result<String, String> {
    run(Command::new("curl").args(["-fsSL", url]))
        .map_err(|error| format!("cannot fetch {url}: {error}"))
}

fn content_length(headers: &str) -> Option<u64> {
    headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .filter_map(|(_, value)| value.trim().parse().ok())
        .next_back()
}

fn node_major(version: &str) -> Option<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

fn verify(path: &Path, sha: &str) -> Result<(), String> {
    let output = run(Command::new("shasum").args(["-a", "256"]).arg(path))
        .or_else(|_| run(Command::new("sha256sum").arg(path)))?;
    let actual = output.split_whitespace().next().unwrap_or_default();
    if actual.eq_ignore_ascii_case(sha) {
        Ok(())
    } else {
        let _ = fs::remove_file(path);
        Err("the download does not match its checksum".to_string())
    }
}

fn unpack(archive: &Path, into: &Path, zip: bool) -> Result<(), String> {
    if zip {
        run(Command::new("unzip")
            .args(["-q", "-o"])
            .arg(archive)
            .arg("-d")
            .arg(into))?;
    } else {
        run(Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(into))?;
    }
    Ok(())
}

fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())
}
