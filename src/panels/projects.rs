use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use eframe::egui::containers::menu::{MenuButton, MenuConfig};
use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{
    self, Align2, Event, Key, PopupCloseBehavior, Rect, Response, RichText, Sense, TextStyle,
};

use crate::git::Repo;
use crate::projects;
use crate::ui::theme;

const WIDTH: f32 = 300.0;
const MAX_LIST_HEIGHT: f32 = 440.0;
const ACTION_HEIGHT: f32 = 26.0;
const PROJECT_HEIGHT: f32 = 42.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Pick {
    Open,
    New,
}

type Picked = Result<Option<PathBuf>, String>;

pub struct Projects {
    store: Option<PathBuf>,
    root: PathBuf,
    list: Vec<PathBuf>,
    search: String,
    error: Option<String>,
    shown: bool,
    picking: Option<(Pick, Receiver<Picked>)>,
}

impl Projects {
    pub fn new(store: Option<PathBuf>, root: PathBuf) -> Self {
        Self {
            store,
            root,
            list: Vec::new(),
            search: String::new(),
            error: None,
            shown: false,
            picking: None,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        let mut open = self.picked();
        let button = theme::dropdown(RichText::new(projects::name(&self.root)).strong()).truncate();
        let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
        let (response, menu) = MenuButton::from_button(button)
            .config(config)
            .ui(ui, |ui| self.menu_ui(ui));
        theme::caret(ui, &response);
        response.on_hover_text("Projects");
        match menu {
            Some(menu) => open = open.or(menu.inner),
            None => {
                self.shown = false;
                self.search.clear();
            }
        }
        open.filter(|path| *path != self.root)
    }

    fn reload(&mut self) {
        self.list = self
            .store
            .as_deref()
            .map(projects::load)
            .unwrap_or_default();
        self.list.retain(|project| *project != self.root);
        self.list.insert(0, self.root.clone());
    }

    fn matching(&self) -> Vec<PathBuf> {
        let search = self.search.trim().to_lowercase();
        self.list
            .iter()
            .filter(|project| {
                projects::name(project).to_lowercase().contains(&search)
                    || projects::shown_path(project)
                        .to_lowercase()
                        .contains(&search)
            })
            .cloned()
            .collect()
    }

    fn menu_ui(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        if !self.shown {
            self.shown = true;
            self.error = None;
            self.reload();
        }
        ui.set_width(WIDTH);
        ui.spacing_mut().item_spacing.y = 0.0;
        let started = self.search.is_empty() && self.start_search(ui);
        let mut open = None;
        if self.search.is_empty() {
            let busy = self.picking.is_some();
            if action(ui, "New Project…", busy).clicked() {
                self.pick(Pick::New, ui.ctx());
            }
            if action(ui, "Open…", busy).clicked() {
                self.pick(Pick::Open, ui.ctx());
            }
        } else {
            self.search_ui(ui, started);
        }
        if let Some(error) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let matching = self.matching();
        if self.search.is_empty() && matching.is_empty() {
            return None;
        }
        separator(ui);
        if self.search.is_empty() {
            heading(ui, "Recent Projects");
        } else if ui.input(|i| i.key_pressed(Key::Enter)) {
            open = matching.first().cloned();
        }
        let mut forget = None;
        let height = (matching.len().max(1) as f32 * PROJECT_HEIGHT).min(MAX_LIST_HEIGHT);
        egui::ScrollArea::vertical()
            .id_salt("projects")
            .min_scrolled_height(height)
            .max_height(height)
            .show(ui, |ui| {
                if matching.is_empty() {
                    heading(ui, "No projects match");
                }
                for project in &matching {
                    match self.row_ui(ui, project) {
                        Some(Row::Open) => open = Some(project.clone()),
                        Some(Row::Forget) => forget = Some(project.clone()),
                        None => {}
                    }
                }
            });
        if let Some(project) = forget {
            if let Some(store) = &self.store {
                projects::forget(store, &project);
            }
            self.list.retain(|known| *known != project);
        }
        if let Some(project) = &open
            && let Err(error) = self.prepare(Pick::Open, project)
        {
            self.error = Some(error);
            open = None;
        }
        if open.is_some() {
            ui.close();
        }
        open
    }

    fn start_search(&mut self, ui: &egui::Ui) -> bool {
        let typed: String = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|event| match event {
                    Event::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect()
        });
        self.search = typed.trim_start().to_string();
        !self.search.is_empty()
    }

    fn search_ui(&mut self, ui: &mut egui::Ui, started: bool) {
        let output = egui::TextEdit::singleline(&mut self.search)
            .hint_text("Search projects")
            .desired_width(f32::INFINITY)
            .show(ui);
        if started {
            output.response.request_focus();
            let mut state = output.state;
            let end = CCursor::new(self.search.chars().count());
            state.cursor.set_char_range(Some(CCursorRange::one(end)));
            state.store(ui.ctx(), output.response.id);
        }
    }

    fn row_ui(&self, ui: &mut egui::Ui, project: &Path) -> Option<Row> {
        let name = projects::name(project);
        let (rect, response) = row(ui, PROJECT_HEIGHT, &name);
        let current = project == self.root;
        let missing = !project.is_dir();
        let mut path = projects::shown_path(project);
        if missing {
            path.push_str(" · missing");
        }
        let left = rect.left() + 8.0;
        let painter = ui
            .painter()
            .with_clip_rect(rect.shrink2(egui::vec2(0.0, 1.0)));
        painter.text(
            egui::pos2(left, rect.top() + 4.0),
            Align2::LEFT_TOP,
            &name,
            TextStyle::Body.resolve(ui.style()),
            if current {
                theme::palette(ui.visuals().dark_mode).accent
            } else if missing {
                ui.visuals().weak_text_color()
            } else {
                ui.visuals().strong_text_color()
            },
        );
        painter.text(
            egui::pos2(left, rect.top() + 23.0),
            Align2::LEFT_TOP,
            path,
            TextStyle::Small.resolve(ui.style()),
            ui.visuals().weak_text_color(),
        );
        if !current && ui.rect_contains_pointer(rect) {
            let close = Rect::from_center_size(
                egui::pos2(rect.right() - 16.0, rect.center().y),
                egui::vec2(22.0, 22.0),
            );
            let remove = ui.interact(close, response.id.with("forget"), Sense::click());
            remove.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Forget {name}"))
            });
            if remove.hovered() {
                ui.painter()
                    .rect_filled(close, 4.0, ui.visuals().widgets.active.weak_bg_fill);
            }
            ui.painter().text(
                close.center(),
                Align2::CENTER_CENTER,
                "✕",
                TextStyle::Small.resolve(ui.style()),
                ui.visuals().text_color(),
            );
            if remove.on_hover_text("Remove from the projects").clicked() {
                return Some(Row::Forget);
            }
        }
        response.clicked().then_some(Row::Open)
    }

    fn pick(&mut self, pick: Pick, ctx: &egui::Context) {
        let parent = self.root.parent().unwrap_or(&self.root);
        let location = format!("default location (POSIX file \"{}\")", quoted(parent));
        let script = match pick {
            Pick::Open => format!("choose folder with prompt \"Open a project\" {location}"),
            Pick::New => format!(
                "choose file name with prompt \"Create a project\" default name \"project\" {location}"
            ),
        };
        let script = format!("POSIX path of ({script})");
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let picked = Command::new("osascript")
                .args(["-e", &script])
                .output()
                .map_err(|e| format!("Cannot ask for a folder: {e}"))
                .map(|out| {
                    let path = String::from_utf8_lossy(&out.stdout);
                    let path = path.trim().trim_end_matches('/');
                    (out.status.success() && !path.is_empty()).then(|| PathBuf::from(path))
                });
            let _ = sender.send(picked);
            ctx.request_repaint();
        });
        self.picking = Some((pick, receiver));
    }

    fn picked(&mut self) -> Option<PathBuf> {
        let (pick, receiver) = self.picking.as_ref()?;
        let picked = receiver.try_recv().ok()?;
        let pick = *pick;
        self.picking = None;
        let path = match picked {
            Ok(path) => path?,
            Err(error) => {
                self.error = Some(error);
                return None;
            }
        };
        match self.prepare(pick, &path) {
            Ok(()) => Some(path),
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }

    fn prepare(&self, pick: Pick, path: &Path) -> Result<(), String> {
        match pick {
            Pick::Open => Repo::open(path)
                .map(|_| ())
                .map_err(|_| format!("{} is not a git repository", projects::shown_path(path))),
            Pick::New => Repo::init(path).map(|_| ()),
        }
    }

    pub fn fail(&mut self, error: String) {
        self.error = Some(error);
    }
}

enum Row {
    Open,
    Forget,
}

fn row(ui: &mut egui::Ui, height: f32, label: &str) -> (Rect, Response) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    if ui.is_enabled() && ui.rect_contains_pointer(rect) {
        ui.painter()
            .rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    (rect, response)
}

fn action(ui: &mut egui::Ui, label: &str, busy: bool) -> Response {
    ui.add_enabled_ui(!busy, |ui| {
        let (rect, response) = row(ui, ACTION_HEIGHT, label);
        ui.painter().text(
            egui::pos2(rect.left() + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            TextStyle::Body.resolve(ui.style()),
            if busy {
                ui.visuals().weak_text_color()
            } else {
                ui.visuals().text_color()
            },
        );
        response
    })
    .inner
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(
            RichText::new(text)
                .strong()
                .color(ui.visuals().weak_text_color()),
        );
    });
    ui.add_space(4.0);
}

fn separator(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);
}

fn quoted(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::temp_repo;
    use crate::testing::TempDir;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    struct Shown {
        projects: Projects,
        opened: Option<PathBuf>,
    }

    fn show(root: &Path, store: PathBuf) -> Harness<'static, Shown> {
        let shown = Shown {
            projects: Projects::new(Some(store), root.to_path_buf()),
            opened: None,
        };
        let mut harness = Harness::builder()
            .with_size(egui::vec2(500.0, 600.0))
            .build_ui_state(
                |ui, shown: &mut Shown| {
                    if let Some(path) = shown.projects.ui(ui) {
                        shown.opened = Some(path);
                    }
                },
                shown,
            );
        crate::ui::theme::apply(&harness.ctx, false);
        harness.run();
        harness
    }

    fn type_text(harness: &mut Harness<'_, Shown>, text: &str) {
        harness.event(egui::Event::Text(text.to_string()));
        harness.run();
    }

    #[test]
    fn lists_the_other_projects_and_searches_them_once_typing_starts() {
        let (shop, _shop_dir) = temp_repo("projects-shop", &[]);
        let (blog, _blog_dir) = temp_repo("projects-blog", &[]);
        let dir = TempDir::new("projects-store");
        let store = dir.path().join("projects");
        projects::remember(&store, Path::new("/nowhere/notes"));
        projects::remember(&store, &blog.root);
        projects::remember(&store, &shop.root);
        let shop_name = projects::name(&shop.root);
        let blog_name = projects::name(&blog.root);

        let mut harness = show(&shop.root, store);
        harness.get_by_label(&shop_name).click();
        harness.run();
        harness.get_by_label("New Project…");
        harness.get_by_label("Open…");
        harness.get_by_label("Recent Projects");
        harness.get_by_label(&blog_name);
        harness.get_by_label("notes");
        assert_eq!(harness.query_all_by_label(&shop_name).count(), 2);
        let rows: Vec<_> = [&shop_name, &blog_name, &"notes".to_string()]
            .map(|name| {
                harness
                    .query_all_by_label(name)
                    .last()
                    .unwrap()
                    .rect()
                    .top()
            })
            .into();
        assert!(rows.is_sorted());
        assert!(harness.query_by_role(egui::Role::TextInput).is_none());

        type_text(&mut harness, "b");
        harness.get_by_role(egui::Role::TextInput);
        assert_eq!(harness.state().projects.search, "b");
        assert!(harness.query_by_label("New Project…").is_none());
        type_text(&mut harness, "log");
        assert_eq!(harness.state().projects.search, "blog");
        assert!(harness.query_by_label("notes").is_none());
        harness.get_by_label(&blog_name);

        type_text(&mut harness, "xx");
        harness.get_by_label("No projects match");

        harness.state_mut().projects.search.clear();
        harness.run();
        harness.get_by_label("New Project…");
        type_text(&mut harness, "nowhere");
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(harness.state().opened.is_none());
        harness.get_by_label("/nowhere/notes is not a git repository");
    }

    #[test]
    fn lists_the_current_project_that_cannot_be_forgotten() {
        let (shop, _shop_dir) = temp_repo("projects-alone", &[]);
        let dir = TempDir::new("projects-alone-store");
        let name = projects::name(&shop.root);

        let mut harness = show(&shop.root, dir.path().join("projects"));
        harness.get_by_label(&name).click();
        harness.run();
        harness.get_by_label("Recent Projects");
        let row = harness.query_all_by_label(&name).last().unwrap();
        row.hover();
        harness.run();
        assert!(harness.query_by_label(&format!("Forget {name}")).is_none());
        harness.query_all_by_label(&name).last().unwrap().click();
        harness.run();
        assert!(harness.state().opened.is_none());
    }

    #[test]
    fn forgets_a_project_from_the_cross_on_hover() {
        let (shop, _shop_dir) = temp_repo("projects-forget-shop", &[]);
        let dir = TempDir::new("projects-forget");
        let store = dir.path().join("projects");
        projects::remember(&store, Path::new("/nowhere/notes"));
        projects::remember(&store, &shop.root);

        let mut harness = show(&shop.root, store.clone());
        harness.get_by_label(&projects::name(&shop.root)).click();
        harness.run();
        assert!(harness.query_by_label("Forget notes").is_none());
        harness.get_by_label("notes").hover();
        harness.run();
        assert!(
            harness
                .query_by_label(&format!("Forget {}", projects::name(&shop.root)))
                .is_none()
        );
        harness.get_by_label("Forget notes").click();
        harness.run();
        assert!(harness.query_by_label("notes").is_none());
        assert_eq!(projects::load(&store), std::slice::from_ref(&shop.root));
    }

    #[test]
    fn opens_the_project_that_was_clicked() {
        let (shop, _shop_dir) = temp_repo("projects-open-shop", &[]);
        let (blog, _blog_dir) = temp_repo("projects-open-blog", &[]);
        let dir = TempDir::new("projects-open");
        let store = dir.path().join("projects");
        projects::remember(&store, &blog.root);

        let mut harness = show(&shop.root, store);
        harness.get_by_label(&projects::name(&shop.root)).click();
        harness.run();
        harness.get_by_label(&projects::name(&blog.root)).click();
        harness.run();
        assert_eq!(harness.state().opened, Some(blog.root.clone()));
    }

    #[test]
    fn creates_a_new_project_as_a_git_repository() {
        let dir = TempDir::new("projects-new");
        let path = dir.path().join("fresh");
        let projects = Projects::new(None, dir.path().to_path_buf());
        projects.prepare(Pick::New, &path).unwrap();
        assert!(Repo::open(&path).is_ok());
        assert!(projects.prepare(Pick::Open, dir.path()).is_err());
    }
}
