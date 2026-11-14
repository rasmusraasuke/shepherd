mod app;
mod buffer;
mod code;
mod editor;
mod file_tree;
mod file_view;
mod git;
mod highlight;
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

    // Windows can be see-through only when the main one is created so: the title of a dragged
    // tab needs it. The main window itself is made solid again, or its title bar is see-through.
    let viewport = app::viewport(&repo.root, &workspace.windows[0]).with_transparent(true);
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "editor",
        options,
        Box::new(|cc| {
            cc.egui_ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Transparent(false));
            Ok(Box::new(app::App::new(repo, workspace, cc.egui_ctx.clone())))
        }),
    )
}
