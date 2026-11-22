use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use eframe::egui;
use serde_json::json;

use super::LANGUAGES;
use super::install::{Installer, Progress, Sources};
use crate::testing::TempDir;

fn language(name: &str) -> &'static super::Language {
    LANGUAGES
        .iter()
        .find(|language| language.name == name)
        .unwrap()
}

fn url(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn sha(path: &Path) -> String {
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn script(dir: &TempDir, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = dir.write(name, format!("#!/bin/sh\n{body}\n"));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn installer(dir: &TempDir) -> Installer {
    fs::create_dir_all(dir.path().join("bin")).unwrap();
    let mut installer = Installer::new(
        dir.path().join("servers"),
        format!("{}:/usr/bin:/bin", dir.path().join("bin").display()),
        true,
    );
    installer.sources = Sources {
        github: url(&dir.path().join("github")),
        jetbrains: url(&dir.path().join("jetbrains")),
        node: url(&dir.path().join("node")),
    };
    installer
}

fn progress() -> Progress {
    Progress::new(egui::Context::default())
}

fn output(command: &str) -> String {
    let output = Command::new(command).output().unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn pack(dir: &TempDir, staged: &str, archive: &Path, zip: bool) {
    let staging = dir.path().join("staging");
    let mut command = if zip {
        let mut command = Command::new("zip");
        command.arg("-qr").arg(archive).arg(staged);
        command
    } else {
        let mut command = Command::new("tar");
        command.arg("-czf").arg(archive).arg(staged);
        command
    };
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    assert!(command.current_dir(staging).status().unwrap().success());
}

fn publish_rust_analyzer(dir: &TempDir, tag: &str, says: &str) -> PathBuf {
    let binary = script(dir, &format!("downloads/{tag}"), &format!("echo {says}"));
    let gz = dir.path().join(format!("downloads/{tag}.gz"));
    let out = fs::File::create(&gz).unwrap();
    assert!(
        Command::new("gzip")
            .arg("-c")
            .arg(&binary)
            .stdout(out)
            .status()
            .unwrap()
            .success()
    );
    let target = if cfg!(target_os = "macos") {
        "apple-darwin"
    } else {
        "unknown-linux-gnu"
    };
    let release = json!({"tag_name": tag, "assets": [{
        "name": format!("rust-analyzer-{}-{target}.gz", std::env::consts::ARCH),
        "browser_download_url": url(&gz),
        "digest": format!("sha256:{}", sha(&gz)),
    }]});
    dir.write(
        "github/repos/rust-lang/rust-analyzer/releases/latest",
        release.to_string(),
    );
    gz
}

#[test]
fn downloads_a_release_and_uses_it_from_then_on() {
    let dir = TempDir::new("install-release");
    let installer = installer(&dir);
    let gz = publish_rust_analyzer(&dir, "2026-11-17", "analyzer");
    let found = installer.find(language("Rust"), &progress()).unwrap();
    let binary = dir.path().join("servers/rust/2026-11-17/rust-analyzer");
    assert_eq!(found, [binary.to_string_lossy()]);
    assert_eq!(output(&found[0]), "analyzer");
    let left: Vec<_> = fs::read_dir(dir.path().join("servers/rust"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left.len(), 2);

    fs::remove_file(gz).unwrap();
    assert_eq!(
        installer.find(language("Rust"), &progress()).unwrap(),
        found
    );
}

#[test]
fn refuses_a_download_that_does_not_match_its_checksum() {
    let dir = TempDir::new("install-checksum");
    let installer = installer(&dir);
    let gz = publish_rust_analyzer(&dir, "2026-11-17", "analyzer");
    fs::write(&gz, "tampered").unwrap();
    let error = installer.find(language("Rust"), &progress()).unwrap_err();
    assert_eq!(
        error,
        "Cannot install the Rust language server: the download does not match its checksum"
    );
    assert!(!dir.path().join("servers/rust/installed.json").exists());
}

#[test]
fn updates_a_day_later_and_keeps_the_old_one_when_offline() {
    let dir = TempDir::new("install-update");
    let installer = installer(&dir);
    publish_rust_analyzer(&dir, "2026-11-10", "old");
    let old = installer.find(language("Rust"), &progress()).unwrap();
    let installed = dir.path().join("servers/rust/installed.json");
    let age = |dir: &TempDir| {
        let text = fs::read_to_string(dir.path().join("servers/rust/installed.json")).unwrap();
        let mut held: serde_json::Value = serde_json::from_str(&text).unwrap();
        held["checked"] = json!(0);
        fs::write(
            dir.path().join("servers/rust/installed.json"),
            held.to_string(),
        )
        .unwrap();
    };

    publish_rust_analyzer(&dir, "2026-11-17", "new");
    assert_eq!(installer.find(language("Rust"), &progress()).unwrap(), old);
    age(&dir);
    let new = installer.find(language("Rust"), &progress()).unwrap();
    assert_eq!(output(&new[0]), "new");
    assert!(!dir.path().join("servers/rust/2026-11-10").exists());

    age(&dir);
    fs::remove_dir_all(dir.path().join("github")).unwrap();
    assert_eq!(installer.find(language("Rust"), &progress()).unwrap(), new);
    assert!(installed.exists());
}

#[test]
fn prefers_a_server_on_the_path_that_works() {
    let dir = TempDir::new("install-path");
    let mut installer = installer(&dir);
    installer.download = false;
    let broken = script(
        &dir,
        "bin/rust-analyzer",
        "echo 'Unknown binary' >&2; exit 1",
    );
    assert_eq!(
        installer.find(language("Rust"), &progress()).unwrap_err(),
        "No Rust language server found: rust-analyzer is not on the PATH and downloading \
         language servers is turned off"
    );
    script(&dir, "bin/rust-analyzer", "exit 0");
    assert_eq!(
        installer.find(language("Rust"), &progress()).unwrap(),
        [broken.to_string_lossy()]
    );
    script(&dir, "bin/kotlin-lsp", "exit 1");
    assert_eq!(
        installer.find(language("Kotlin"), &progress()).unwrap(),
        [
            dir.path()
                .join("bin/kotlin-lsp")
                .to_string_lossy()
                .into_owned(),
            "--stdio".to_string()
        ]
    );
}

#[test]
fn downloads_the_kotlin_server_of_jetbrains_with_its_own_java() {
    let dir = TempDir::new("install-kotlin");
    let installer = installer(&dir);
    script(
        &dir,
        "staging/kotlin-server-1.2/bin/intellij-server",
        "echo kotlin",
    );
    let zip = cfg!(target_os = "macos");
    let arch = if std::env::consts::ARCH == "aarch64" {
        "-aarch64"
    } else {
        ""
    };
    let name = format!(
        "kotlin-server-1.2{arch}.{}",
        if zip { "sit" } else { "tar.gz" }
    );
    let archive = dir.path().join(format!(
        "jetbrains/language-server/kotlin-server/1.2/{name}"
    ));
    pack(&dir, "kotlin-server-1.2", &archive, zip);
    dir.write(
        &format!("jetbrains/language-server/kotlin-server/1.2/{name}.sha256"),
        format!("{} *{name}\n", sha(&archive)),
    );
    dir.write(
        "github/repos/Kotlin/kotlin-lsp/releases/latest",
        json!({"tag_name": "kotlin-lsp/v1.2"}).to_string(),
    );
    let found = installer.find(language("Kotlin"), &progress()).unwrap();
    let binary = dir
        .path()
        .join("servers/kotlin/1.2/kotlin-server-1.2/bin/intellij-server");
    assert_eq!(found, [binary.to_string_lossy(), "--stdio".into()]);
    let installed = fs::read_to_string(dir.path().join("servers/kotlin/installed.json")).unwrap();
    assert!(!installed.contains("--stdio"), "{installed}");
    assert_eq!(
        installer.find(language("Kotlin"), &progress()).unwrap(),
        found
    );
    let ran = Command::new(&found[0]).arg("--stdio").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), "kotlin");
    assert!(!dir.path().join("servers/kotlin/download.part").exists());
}

const FAKE_NPM: &str = r#"case "$1" in
view) echo 1.5.0 ;;
install)
    prefix=$3
    mkdir -p "$prefix/node_modules/pyright"
    touch "$prefix/node_modules/pyright/langserver.index.js"
    echo "$@" > "$prefix/npm-args" ;;
esac"#;

#[test]
fn installs_node_servers_with_npm() {
    let dir = TempDir::new("install-npm");
    let installer = installer(&dir);
    let node = script(&dir, "bin/node", "echo v22.1.0");
    script(&dir, "bin/npm", FAKE_NPM);
    let found = installer.find(language("Python"), &progress()).unwrap();
    let servers = dir.path().join("servers/python");
    assert_eq!(
        found,
        [
            node.to_string_lossy().into_owned(),
            servers
                .join("node_modules/pyright/langserver.index.js")
                .to_string_lossy()
                .into_owned(),
            "--stdio".to_string(),
        ]
    );
    let args = fs::read_to_string(servers.join("npm-args")).unwrap();
    assert!(args.contains("pyright@latest"), "{args}");
}

#[test]
fn installs_typescript_servers_with_a_typescript_they_can_use() {
    let dir = TempDir::new("install-npm-typescript");
    let installer = installer(&dir);
    script(&dir, "bin/node", "echo v22.1.0");
    script(&dir, "bin/npm", FAKE_NPM);
    let servers = dir.path().join("servers");
    for (name, server) in [
        ("TypeScript", "typescript-language-server"),
        ("Svelte", "svelte-language-server"),
    ] {
        let installed = servers.join(name.to_lowercase());
        fs::create_dir_all(&installed).unwrap();
        fs::write(
            installed.join("installed.json"),
            json!({"version": "1.5.0", "command": [dir.path().join("bin/node")], "checked": 0})
                .to_string(),
        )
        .unwrap();
        installer.find(language(name), &progress()).unwrap();
        let args = fs::read_to_string(installed.join("npm-args")).unwrap();
        assert!(
            args.ends_with(&format!(" {server}@latest typescript@6\n")),
            "{args}"
        );
        let installed = fs::read_to_string(installed.join("installed.json")).unwrap();
        assert!(installed.contains("\"1.5.0 typescript@6\""), "{installed}");
    }
}

#[test]
fn downloads_node_when_there_is_none_new_enough() {
    let dir = TempDir::new("install-node");
    let installer = installer(&dir);
    script(&dir, "bin/node", "echo v16.0.0");
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
    let name = format!("node-v24.1.0-{os}-{arch}");
    script(&dir, &format!("staging/{name}/bin/node"), "echo v24.1.0");
    script(&dir, &format!("staging/{name}/bin/npm"), FAKE_NPM);
    let archive = dir.path().join(format!("node/v24.1.0/{name}.tar.gz"));
    pack(&dir, &name, &archive, false);
    dir.write(
        "node/v24.1.0/SHASUMS256.txt",
        format!("abc  other.tar.gz\n{}  {name}.tar.gz\n", sha(&archive)),
    );
    dir.write(
        "node/index.json",
        json!([
            {"version": "v25.0.0", "lts": false},
            {"version": "v24.1.0", "lts": "Krypton"},
        ])
        .to_string(),
    );
    let found = installer.find(language("Python"), &progress()).unwrap();
    let node = dir.path().join(format!("servers/node/{name}/bin/node"));
    assert_eq!(found[0], node.to_string_lossy());
}

#[test]
fn installs_gopls_with_go() {
    let dir = TempDir::new("install-go");
    let installer = installer(&dir);
    assert_eq!(
        installer.find(language("Go"), &progress()).unwrap_err(),
        "Cannot install the Go language server: Go is needed to install it, and go is not on \
         the PATH"
    );
    script(
        &dir,
        "bin/go",
        r#"mkdir -p "$GOBIN"
printf '#!/bin/sh\n' > "$GOBIN/gopls"
chmod +x "$GOBIN/gopls"
echo "$@" > "$GOBIN/../go-args""#,
    );
    let found = installer.find(language("Go"), &progress()).unwrap();
    let servers = dir.path().join("servers/go");
    assert_eq!(found, [servers.join("bin/gopls").to_string_lossy()]);
    assert_eq!(
        fs::read_to_string(servers.join("go-args")).unwrap().trim(),
        "install golang.org/x/tools/gopls@latest"
    );
}

#[test]
fn asks_for_servers_it_cannot_install() {
    let dir = TempDir::new("install-manual");
    let installer = installer(&dir);
    assert_eq!(
        installer.find(language("Java"), &progress()).unwrap_err(),
        "No Java language server found: install jdtls on the PATH"
    );
}
