mod app;
mod code;
mod file_tree;
mod file_view;
mod git;
mod quick_open;
mod review;
#[cfg(test)]
mod testing;
mod watcher;
mod workspace;

use std::path::PathBuf;

use workspace::Workspace;

fn main() -> eframe::Result {
    let path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let repo = match git::Repo::open(&path) {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("{}: not a git repository ({e})", path.display());
            std::process::exit(1);
        }
    };
    let workspace = Workspace::load(&repo.layout_path());

    let options = eframe::NativeOptions { viewport: app::viewport(&repo.root, &workspace.windows[0]), ..Default::default() };
    eframe::run_native("editor", options, Box::new(|cc| Ok(Box::new(app::App::new(repo, workspace, cc.egui_ctx.clone())))))
}
