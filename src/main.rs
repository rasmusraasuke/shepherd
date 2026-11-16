mod agents;
mod app;
mod buffer;
mod code;
mod comments;
mod control;
mod dirs;
mod editor;
mod file_tree;
mod file_view;
mod flow;
mod git;
mod highlight;
mod icons;
mod keys;
mod line_diff;
mod quick_chat;
mod quick_open;
mod review;
mod search;
mod terminal;
mod terminal_text;
#[cfg(test)]
mod testing;
mod theme;
mod update;
mod watcher;
mod word_diff;
mod workspace;

use std::path::{Path, PathBuf};

use workspace::Workspace;

const HELP: &str = "\
shepherd [<path>]              Open the repository at the path, the current directory or the last one
shepherd ctl <command>         Ask the editor that is running what the user sees
shepherd --version             Show the version
shepherd --help                Show this";

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ctl") => std::process::exit(control::run_command(&args[1..])),
        Some("--version" | "-V") => {
            println!("shepherd {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--help" | "-h") => {
            println!("{HELP}");
            return Ok(());
        }
        _ => {}
    }
    let explicit = args.first().map(PathBuf::from);
    let repo = match open(explicit) {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    remember(&repo.root);
    let workspace = Workspace::load(&repo.layout_path());

    let viewport = app::viewport(&repo.root, &workspace.windows[0]).with_transparent(true);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Shepherd",
        options,
        Box::new(|cc| {
            cc.egui_ctx
                .send_viewport_cmd(eframe::egui::ViewportCommand::Transparent(false));
            Ok(Box::new(app::App::new(
                repo,
                workspace,
                cc.egui_ctx.clone(),
            )))
        }),
    )
}

fn open(explicit: Option<PathBuf>) -> Result<git::Repo, String> {
    if let Some(path) = explicit {
        return git::Repo::open(&path)
            .map_err(|e| format!("{}: not a git repository ({e})", path.display()));
    }
    first_repo(
        std::env::current_dir()
            .ok()
            .into_iter()
            .chain(last_project()),
    )
    .ok_or_else(|| {
        format!(
            "No repository to open: run `shepherd <path>` or open one from the folder it was \
             opened in before ({})",
            dirs::config().map_or_else(
                || "no home directory".to_string(),
                |dir| dir.display().to_string()
            )
        )
    })
}

fn first_repo(candidates: impl IntoIterator<Item = PathBuf>) -> Option<git::Repo> {
    candidates
        .into_iter()
        .find_map(|path| git::Repo::open(&path).ok())
}

fn last_project() -> Option<PathBuf> {
    let path = PathBuf::from(std::fs::read_to_string(dirs::config()?.join("last-project")).ok()?);
    path.is_dir().then_some(path)
}

fn remember(root: &Path) {
    let Some(dir) = dirs::config() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("last-project"), root.to_string_lossy().as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::temp_repo;

    #[test]
    fn opens_the_first_candidate_that_is_a_repository() {
        let (repo, _dir) = temp_repo("open-first-repo", &[("a.txt", "a\n")]);
        let missing = repo.root.join("missing");
        let found = first_repo([missing, repo.root.clone()]).unwrap();
        assert_eq!(found.root, repo.root);
        assert!(first_repo(std::iter::empty()).is_none());
    }
}
