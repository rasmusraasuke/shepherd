use eframe::egui::{self, Color32, RichText, Stroke};

use super::CLIENT_ID;
use super::checks::{Checks, Found, State, short};
use super::failures::Failure;
use crate::ui::status::{self, Kind};
use crate::ui::theme;

const HEIGHT: f32 = 22.0;
const ICON: f32 = 14.0;
const GAP: f32 = 6.0;
const PADDING: f32 = 8.0;
const MARGIN: f32 = 8.0;
const POPUP_WIDTH: f32 = 320.0;
const LOG_WIDTH: f32 = 640.0;
const LOG_HEIGHT: f32 = 220.0;
const COMPACT: f32 = 760.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Icon {
    Status(Kind),
    Run(State),
}

struct Look {
    text: String,
    icon: Option<Icon>,
    failed: bool,
    compact: bool,
}

impl Look {
    fn new(text: impl Into<String>, kind: Option<Kind>) -> Self {
        Self {
            text: text.into(),
            icon: kind.map(Icon::Status),
            failed: matches!(kind, Some(Kind::Error)),
            compact: false,
        }
    }
}

fn paint(ui: &egui::Ui, rect: egui::Rect, icon: Icon) {
    match icon {
        Icon::Status(kind) => status::paint_icon(ui, rect, kind),
        Icon::Run(state) => paint_run(ui, rect, state),
    }
}

fn color(state: State, dark: bool) -> Color32 {
    match (state, dark) {
        (State::Passed, false) => Color32::from_rgb(31, 136, 61),
        (State::Passed, true) => Color32::from_rgb(35, 134, 54),
        (State::Failed, false) => Color32::from_rgb(207, 34, 46),
        (State::Failed, true) => Color32::from_rgb(218, 54, 51),
        (State::Running, false) => Color32::from_rgb(191, 135, 0),
        (State::Running, true) => Color32::from_rgb(210, 153, 34),
    }
}

fn paint_run(ui: &egui::Ui, rect: egui::Rect, state: State) {
    let painter = ui.painter();
    let color = color(state, ui.visuals().dark_mode);
    let center = rect.center();
    let radius = rect.width().min(rect.height()) / 2.0;
    let at = |x: f32, y: f32| center + egui::vec2(x, y) * radius;
    let mark = Stroke::new(radius * 0.25, Color32::WHITE);
    match state {
        State::Passed => {
            painter.circle_filled(center, radius, color);
            painter.line(vec![at(-0.45, 0.0), at(-0.12, 0.33), at(0.45, -0.25)], mark);
        }
        State::Failed => {
            painter.circle_filled(center, radius, color);
            painter.line_segment([at(-0.33, -0.33), at(0.33, 0.33)], mark);
            painter.line_segment([at(0.33, -0.33), at(-0.33, 0.33)], mark);
        }
        State::Running => {
            ui.request_repaint();
            let width = radius * 0.3;
            let radius = radius - width / 2.0;
            painter.circle_stroke(
                center,
                radius,
                Stroke::new(width, color.gamma_multiply(0.35)),
            );
            let start = ui.input(|i| i.time) as f32 * std::f32::consts::TAU;
            let points = (0..=12)
                .map(|i| {
                    let angle = start + i as f32 / 12.0 * std::f32::consts::FRAC_PI_2;
                    center + radius * egui::vec2(angle.cos(), angle.sin())
                })
                .collect();
            painter.add(egui::Shape::line(points, Stroke::new(width, color)));
        }
    }
}

impl Checks {
    pub fn ui(&mut self, ui: &mut egui::Ui, bar: egui::Rect) -> f32 {
        let Some(look) = self.look() else {
            return bar.right();
        };
        let compact = look.compact && ui.ctx().content_rect().width() < COMPACT;
        let color = if look.failed {
            ui.visuals().error_fg_color
        } else {
            ui.visuals().weak_text_color()
        };
        let galley = (!compact).then(|| {
            let font = egui::TextStyle::Body.resolve(ui.style());
            ui.painter().layout_no_wrap(look.text.clone(), font, color)
        });
        let icon_width = look.icon.map_or(0.0, |_| ICON);
        let text_width = galley.as_ref().map_or(0.0, |galley| galley.size().x);
        let gap = if icon_width > 0.0 && text_width > 0.0 {
            GAP
        } else {
            0.0
        };
        let width = PADDING * 2.0 + icon_width + gap + text_width;
        let rect = egui::Rect::from_min_size(
            egui::pos2(bar.right() - MARGIN - width, bar.center().y - HEIGHT / 2.0),
            egui::vec2(width, HEIGHT),
        );
        let response = ui.interact(
            rect,
            ui.scope_id().with("github checks"),
            egui::Sense::click(),
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &look.text));
        let popup_width = self.popup_width(ui.ctx());
        let popup = egui::Popup::menu(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(popup_width);
        if !popup.is_open() {
            response
                .clone()
                .on_hover_text(format!("GitHub Actions on {}", self.branch));
        }
        if response.hovered() || popup.is_open() {
            ui.painter()
                .rect_filled(rect, 5.0, theme::palette(ui.visuals().dark_mode).hover);
        }
        let mut x = rect.left() + PADDING;
        if let Some(icon) = look.icon {
            let area = egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - ICON / 2.0),
                egui::vec2(ICON, ICON),
            );
            paint(ui, area, icon);
            x += ICON + gap;
        }
        if let Some(galley) = galley {
            let at = egui::pos2(x, rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(at, galley, color);
        }
        popup.show(|ui| {
            ui.set_max_width(popup_width);
            self.popup_ui(ui);
        });
        rect.left()
    }

    fn popup_width(&self, ctx: &egui::Context) -> f32 {
        let failed = match &self.found {
            Found::Runs(summary) => summary.runs.iter().any(|run| !run.failures.is_empty()),
            _ => false,
        };
        if failed {
            (ctx.content_rect().width() - 2.0 * MARGIN).clamp(POPUP_WIDTH, LOG_WIDTH)
        } else {
            POPUP_WIDTH
        }
    }

    fn look(&self) -> Option<Look> {
        self.repository.as_ref()?;
        if let Some(login) = &self.login {
            let text = match &login.code {
                Some((code, _)) => format!("Enter {code} on GitHub"),
                None => "Logging in to GitHub…".to_string(),
            };
            return Some(Look::new(text, Some(Kind::Busy)));
        }
        if self.error.is_some() {
            return Some(Look::new("GitHub login failed", Some(Kind::Error)));
        }
        match &self.found {
            Found::Nothing => None,
            Found::Runs(summary) => Some(Look {
                icon: Some(Icon::Run(summary.state)),
                failed: summary.state == State::Failed,
                compact: true,
                ..Look::new(summary.state.label(), None)
            }),
            Found::LogIn => Some(Look::new("Log in to GitHub", None)),
            Found::NoAccess => Some(Look::new("No access to GitHub Actions", Some(Kind::Error))),
        }
    }

    fn popup_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(login) = &self.login {
            match &login.code {
                Some((code, url)) => {
                    let url = url.clone();
                    ui.label("Enter this code on GitHub. It is copied already.");
                    ui.label(RichText::new(code).monospace().size(18.0).strong());
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary(ui, "Open GitHub")).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                        }
                        if ui.add(theme::quiet("Cancel")).clicked() {
                            self.login = None;
                        }
                    });
                }
                None => {
                    ui.label("Asking GitHub for a login code…");
                }
            }
            return;
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
            if ui.add(theme::quiet("Dismiss")).clicked() {
                self.error = None;
            }
            return;
        }
        let mut prompt = None;
        match &self.found {
            Found::Nothing => {}
            Found::Runs(summary) => {
                ui.weak(format!(
                    "GitHub Actions on {} at {}",
                    self.branch,
                    short(&summary.commit)
                ));
                for run in &summary.runs {
                    ui.horizontal(|ui| {
                        let (icon, _) =
                            ui.allocate_exact_size(egui::vec2(ICON, ICON), egui::Sense::hover());
                        paint_run(ui, icon, run.state);
                        if ui
                            .add(theme::quiet(run.name.as_str()))
                            .on_hover_text(run.state.label())
                            .clicked()
                        {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(&run.url));
                        }
                        if !run.failures.is_empty() {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(theme::primary(ui, "Solve with agent"))
                                        .on_hover_text(
                                            "Start the coding agent on fixing every failed step",
                                        )
                                        .clicked()
                                    {
                                        prompt =
                                            Some(self.prompt(&summary.commit, run, &run.failures));
                                    }
                                },
                            );
                        }
                    });
                    ui.indent(&run.url, |ui| {
                        for (index, failure) in run.failures.iter().enumerate() {
                            if failure_ui(ui, failure, index == 0, self.logged_in) {
                                prompt = Some(self.prompt(
                                    &summary.commit,
                                    run,
                                    std::slice::from_ref(failure),
                                ));
                            }
                        }
                    });
                }
            }
            Found::LogIn => {
                ui.label("Log in to see GitHub Actions of private repositories.");
            }
            Found::NoAccess => {
                ui.label(
                    "GitHub does not show this repository to Shepherd. \
                     An owner of its organization may have to approve Shepherd first.",
                );
                if ui.add(theme::quiet("Ask for access")).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                        "https://github.com/settings/connections/applications/{CLIENT_ID}"
                    )));
                }
            }
        }
        if prompt.is_some() {
            self.prompt = prompt;
            ui.close();
        }
        ui.separator();
        if self.logged_in {
            if ui.add(theme::quiet("Log out of GitHub")).clicked() {
                self.log_out();
            }
        } else if ui.add(theme::primary(ui, "Log in to GitHub")).clicked() {
            self.log_in(ui.ctx());
        }
    }
}

fn failure_ui(ui: &mut egui::Ui, failure: &Failure, open: bool, logged_in: bool) -> bool {
    let title = RichText::new(failure.title()).color(ui.visuals().error_fg_color);
    egui::CollapsingHeader::new(title)
        .id_salt(&failure.url)
        .default_open(open)
        .show(ui, |ui| {
            match &failure.log {
                Some(log) => {
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .corner_radius(4)
                        .inner_margin(6)
                        .show(ui, |ui| {
                            egui::ScrollArea::both()
                                .max_height(LOG_HEIGHT)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(RichText::new(log).monospace().size(11.0))
                                            .extend(),
                                    );
                                });
                        });
                }
                None if logged_in => {
                    ui.weak("GitHub has no log of what the step printed.");
                }
                None => {
                    ui.weak("Log in to GitHub to see what the step printed.");
                }
            }
            ui.horizontal(|ui| {
                let send = ui
                    .add(theme::primary(ui, "Send to agent"))
                    .on_hover_text("Start the coding agent on fixing this")
                    .clicked();
                if ui.add(theme::quiet("Open on GitHub")).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&failure.url));
                }
                send
            })
            .inner
        })
        .body_returned
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use super::super::checks::{Run, Summary};
    use super::*;

    fn show(found: Found) -> Harness<'static, Checks> {
        let mut checks = Checks::new(Some("o/r".to_string()));
        checks.set_commit("main", "abcdef1234");
        checks.found = found;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 300.0))
            .with_context_setup(|ctx| theme::apply(ctx, false))
            .build_ui_state(
                |ui, checks: &mut Checks| {
                    let bar =
                        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 30.0));
                    checks.ui(ui, bar);
                },
                checks,
            );
        harness.run_steps(2);
        harness
    }

    fn opened(harness: &Harness<'_, Checks>) -> Vec<String> {
        harness
            .output()
            .platform_output
            .commands
            .iter()
            .filter_map(|command| match command {
                egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                _ => None,
            })
            .collect()
    }

    fn run(name: &str, state: State, id: u64, failures: Vec<Failure>) -> Run {
        Run {
            name: name.to_string(),
            state,
            url: format!("https://github.com/o/r/actions/runs/{id}"),
            failures,
            id,
            attempt: 1,
            with_logs: false,
        }
    }

    fn failure(log: Option<&str>) -> Failure {
        Failure {
            job: "test".to_string(),
            step: Some("Warnings".to_string()),
            url: "https://github.com/o/r/actions/runs/2/job/7".to_string(),
            log: log.map(str::to_string),
        }
    }

    fn failed(failures: Vec<Failure>) -> Found {
        Found::Runs(Summary {
            state: State::Failed,
            commit: "abcdef1234".to_string(),
            runs: vec![
                run("Build", State::Passed, 1, Vec::new()),
                run("Test", State::Failed, 2, failures),
            ],
        })
    }

    fn open_popup(harness: &mut Harness<'_, Checks>) {
        harness.get_by_label("Failed").click();
        harness.run();
    }

    #[test]
    fn shows_how_the_workflows_went_and_opens_one_on_github() {
        let mut harness = show(failed(Vec::new()));
        open_popup(&mut harness);
        harness.get_by_label("GitHub Actions on main at abcdef1");
        harness.get_by_label("Build");
        harness.get_by_label("Log in to GitHub");
        harness.get_by_label("Test").click();
        harness.step();
        assert_eq!(opened(&harness), ["https://github.com/o/r/actions/runs/2"]);
    }

    #[test]
    fn shows_the_failed_step_with_the_end_of_its_log_and_sends_it_to_an_agent() {
        let log = "error: unused variable: `x`\nProcess completed with exit code 101.";
        let mut harness = show(failed(vec![failure(Some(log))]));
        open_popup(&mut harness);
        harness.get_by_label("test › Warnings");
        harness.get_by_label(log);
        harness.get_by_label("Open on GitHub").click();
        harness.step();
        assert_eq!(
            opened(&harness),
            ["https://github.com/o/r/actions/runs/2/job/7"]
        );
        harness.get_by_label("Send to agent").click();
        harness.run();
        let prompt = harness.state_mut().take_prompt().unwrap();
        assert!(
            prompt.starts_with("The GitHub Actions workflow \"Test\" failed on branch main"),
            "{prompt}"
        );
        assert!(prompt.contains(log), "{prompt}");
        assert!(harness.query_by_label("Send to agent").is_none());
    }

    #[test]
    fn solves_every_failure_of_a_workflow_with_an_agent() {
        let linux = Failure {
            job: "linux".to_string(),
            ..failure(Some("error: linux"))
        };
        let mut harness = show(failed(vec![failure(Some("error: mac")), linux]));
        open_popup(&mut harness);
        harness.get_by_label("Solve with agent").click();
        harness.run();
        let prompt = harness.state_mut().take_prompt().unwrap();
        assert!(prompt.contains("In test › Warnings"), "{prompt}");
        assert!(prompt.contains("error: mac"), "{prompt}");
        assert!(prompt.contains("In linux › Warnings"), "{prompt}");
        assert!(prompt.contains("error: linux"), "{prompt}");
        assert!(harness.query_by_label("Solve with agent").is_none());
    }

    #[test]
    fn offers_to_solve_only_failed_workflows() {
        let mut harness = show(failed(Vec::new()));
        open_popup(&mut harness);
        assert!(harness.query_by_label("Solve with agent").is_none());
    }

    #[test]
    fn asks_to_log_in_to_see_the_log() {
        let mut harness = show(failed(vec![failure(None)]));
        open_popup(&mut harness);
        harness.get_by_label("test › Warnings");
        harness.get_by_label("Log in to GitHub to see what the step printed.");
        harness.get_by_label("Send to agent");
    }

    #[test]
    fn asks_to_log_in_when_github_hides_the_repository() {
        let mut harness = show(Found::LogIn);
        harness.get_by_label("Log in to GitHub").click();
        harness.run_steps(2);
        harness.get_by_label("Log in to see GitHub Actions of private repositories.");
    }

    #[test]
    fn shows_runs_with_the_icons_of_github() {
        let mut checks = Checks::new(Some("o/r".to_string()));
        for (state, failed) in [
            (State::Running, false),
            (State::Passed, false),
            (State::Failed, true),
        ] {
            checks.found = Found::Runs(Summary {
                state,
                commit: "abcdef1234".to_string(),
                runs: Vec::new(),
            });
            let look = checks.look().unwrap();
            assert_eq!(look.icon, Some(Icon::Run(state)));
            assert_eq!(look.failed, failed);
        }
    }

    #[test]
    fn shows_nothing_outside_github_or_without_runs() {
        let harness = show(Found::Nothing);
        assert!(harness.query_by_role(egui::Role::Button).is_none());
        let mut checks = Checks::new(None);
        checks.found = Found::LogIn;
        assert!(checks.look().is_none());
    }
}
