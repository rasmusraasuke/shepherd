mod checks;
mod login;
mod view;

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub use checks::Checks;

use crate::dirs;

const API: &str = "https://api.github.com";

const CLIENT_ID: &str = "Ov23lincIii5y2zb8gYx";

pub fn repository(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let rest = ["https://", "http://", "ssh://", "git://"]
        .iter()
        .find_map(|scheme| remote.strip_prefix(scheme))
        .unwrap_or(remote);
    let rest = rest.split_once('@').map_or(rest, |(_, host)| host);
    let path = rest.strip_prefix("github.com")?.strip_prefix([':', '/'])?;
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    (!owner.is_empty() && !name.is_empty() && !name.contains('/'))
        .then(|| format!("{owner}/{name}"))
}

fn token_path() -> Option<PathBuf> {
    dirs::config().map(|dir| dir.join("github-token"))
}

fn token() -> Option<String> {
    read_token(&token_path()?)
}

fn read_token(path: &Path) -> Option<String> {
    let token = fs::read_to_string(path).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn save_token(token: &str) -> Result<(), String> {
    let path = token_path().ok_or("No folder to keep the GitHub login in")?;
    write_token(&path, token)
}

fn write_token(path: &Path, token: &str) -> Result<(), String> {
    let cannot = |e: std::io::Error| format!("Cannot keep the GitHub login: {e}");
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(cannot)?;
    }
    let _ = fs::remove_file(path);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(path)
        .and_then(|mut file| file.write_all(token.as_bytes()))
        .map_err(cannot)
}

fn forget_token() {
    if let Some(path) = token_path() {
        let _ = fs::remove_file(path);
    }
}

struct Response {
    status: u16,
    body: String,
}

fn get(path: &str, query: &[(&str, &str)], token: Option<&str>) -> Result<Response, String> {
    let mut headers = "Accept: application/vnd.github+json\n".to_string();
    if let Some(token) = token {
        headers.push_str(&format!("Authorization: Bearer {token}\n"));
    }
    let mut args = vec!["--get".to_string(), "-H".to_string(), "@-".to_string()];
    for (name, value) in query {
        args.push("--data-urlencode".to_string());
        args.push(format!("{name}={value}"));
    }
    args.push(format!("{API}{path}"));
    curl(&args, &headers)
}

fn post(
    url: &str,
    form: &[(&str, &str)],
    secret: Option<(&str, &str)>,
) -> Result<Response, String> {
    let mut args = vec!["-H".to_string(), "Accept: application/json".to_string()];
    for (name, value) in form {
        args.push("--data-urlencode".to_string());
        args.push(format!("{name}={value}"));
    }
    if let Some((name, _)) = secret {
        args.push("--data-urlencode".to_string());
        args.push(format!("{name}@-"));
    }
    args.push(url.to_string());
    curl(&args, secret.map_or("", |(_, value)| value))
}

fn curl(args: &[String], input: &str) -> Result<Response, String> {
    let mut child = Command::new("curl")
        .args(["-sS", "-A", "Shepherd", "--connect-timeout", "10"])
        .args(["--max-time", "30", "-w", "\n%{http_code}"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Cannot run curl: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("Cannot run curl: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Cannot run curl: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Cannot reach GitHub: {}", stderr.trim()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (body, status) = stdout.rsplit_once('\n').ok_or("GitHub gave no answer")?;
    let status = status.trim().parse().map_err(|_| "GitHub gave no answer")?;
    Ok(Response {
        status,
        body: body.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn finds_the_repository_of_a_github_remote() {
        for remote in [
            "git@github.com:rasmus/shepherd.git",
            "https://github.com/rasmus/shepherd.git",
            "https://github.com/rasmus/shepherd",
            "https://rasmus@github.com/rasmus/shepherd/",
            "ssh://git@github.com/rasmus/shepherd.git",
            "git://github.com/rasmus/shepherd.git\n",
        ] {
            assert_eq!(
                repository(remote).as_deref(),
                Some("rasmus/shepherd"),
                "{remote}"
            );
        }
    }

    #[test]
    fn ignores_remotes_that_are_not_on_github() {
        for remote in [
            "git@gitlab.com:rasmus/shepherd.git",
            "https://github.company.com/rasmus/shepherd",
            "/Users/rasmus/remote.git",
            "https://github.com/rasmus",
            "https://github.com/rasmus/shepherd/tree/main",
        ] {
            assert_eq!(repository(remote), None, "{remote}");
        }
    }

    #[test]
    fn keeps_the_token_readable_only_by_the_user() {
        let dir = TempDir::new("github-token");
        let path = dir.path().join("login/github-token");
        assert_eq!(read_token(&path), None);
        write_token(&path, "first").unwrap();
        write_token(&path, "gho_secret").unwrap();
        assert_eq!(read_token(&path).as_deref(), Some("gho_secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
