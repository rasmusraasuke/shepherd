mod app;
mod buffer;
mod code;
mod comments;
mod control;
mod editor;
mod file_tree;
mod file_view;
mod git;
mod highlight;
mod line_diff;
mod quick_open;
mod review;
mod search;
mod terminal;
mod terminal_text;
#[cfg(test)]
mod testing;
mod watcher;
mod word_diff;
mod workspace;

use std::path::PathBuf;

use workspace::Workspace;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "ctl") {
        std::process::exit(control::run_command(&args[2..]));
    }
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let repo = match git::Repo::open(&path) {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("{}: not a git repository ({e})", path.display());
            std::process::exit(1);
        }
    };
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
