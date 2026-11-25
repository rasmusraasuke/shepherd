use eframe::egui::{self, Color32, RichText, Stroke};

use super::CLIENT_ID;
use super::checks::{Checks, Found, Run, State, short};
use super::failures::{Failure, Job};
use crate::ui::status::{self, Kind};
use crate::ui::theme;

const HEIGHT: f32 = 22.0;
const ICON: f32 = 14.0;
const STEP_ICON: f32 = 10.0;
const ROW: f32 = 26.0;
const STEP_ROW: f32 = 21.0;
const LEVEL: f32 = 20.0;
const STEP_TEXT: f32 = 12.5;
const LOG_TEXT: f32 = 11.5;
const GAP: f32 = 6.0;
const PADDING: f32 = 8.0;
const MARGIN: f32 = 8.0;
const POPUP_WIDTH: f32 = 320.0;
const LOG_WIDTH: f32 = 640.0;
const LOG_HEIGHT: f32 = 220.0;
const COMPACT: f32 = 760.0;
const LIST_HEIGHT: f32 = 0.7;

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
        (State::Waiting | State::Skipped, false) => Color32::from_gray(140),
        (State::Waiting | State::Skipped, true) => Color32::from_gray(125),
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
        State::Waiting => {
            painter.circle_stroke(center, radius * 0.85, Stroke::new(radius * 0.25, color));
        }
        State::Skipped => {
            let stroke = Stroke::new(radius * 0.25, color);
            painter.circle_stroke(center, radius * 0.85, stroke);
            painter.line_segment([at(-0.55, 0.55), at(0.55, -0.55)], stroke);
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
                ..Look::new(summary.label(), None)
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
                ui.horizontal(|ui| {
                    ui.add_space(PADDING);
                    ui.strong("GitHub Actions");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(PADDING);
                        ui.weak(format!("{} · {}", self.branch, short(&summary.commit)));
                    });
                });
                ui.add_space(4.0);
                let height = ui.ctx().content_rect().height() * LIST_HEIGHT;
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let mut first = true;
                        for (index, run) in summary.runs.iter().enumerate() {
                            if index > 0 {
                                ui.add_space(6.0);
                            }
                            if let Some(failures) = self.run_ui(ui, run, &mut first) {
                                prompt = Some(self.prompt(&summary.commit, run, &failures));
                            }
                        }
                    });
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

impl Checks {
    fn run_ui(&self, ui: &mut egui::Ui, run: &Run, first: &mut bool) -> Option<Vec<Failure>> {
        let mut send = None;
        let response = row(ui, 0, run.state, RichText::new(&run.name).strong(), true);
        if !run.failures.is_empty() {
            let size = egui::vec2(132.0, 20.0);
            let at = egui::Rect::from_min_size(
                egui::pos2(
                    response.rect.right() - size.x - 4.0,
                    response.rect.center().y - size.y / 2.0,
                ),
                size,
            );
            if ui
                .put(at, theme::primary(ui, "Solve with agent"))
                .on_hover_text("Start the coding agent on fixing every failed step")
                .clicked()
            {
                send = Some(run.failures.clone());
            }
        }
        if response.on_hover_text(run.label()).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(&run.url));
        }
        if run.state == State::Passed {
            return send;
        }
        for job in &run.jobs {
            let failure = run.failures.iter().find(|failure| failure.url == job.url);
            if job_ui(ui, job, failure, first, self.logged_in) {
                send = failure.cloned().map(|failure| vec![failure]);
            }
        }
        send
    }
}

fn row(
    ui: &mut egui::Ui,
    depth: usize,
    state: State,
    text: RichText,
    clickable: bool,
) -> egui::Response {
    let step = depth > 1;
    let (height, icon) = if step {
        (STEP_ROW, STEP_ICON)
    } else {
        (ROW, ICON)
    };
    let (sense, role) = if clickable {
        (egui::Sense::click(), egui::Role::Button)
    } else {
        (egui::Sense::hover(), egui::Role::Label)
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), sense);
    let label = text.text().to_string();
    response.widget_info(|| egui::WidgetInfo::labeled(role, true, &label));
    if clickable && response.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, theme::palette(ui.visuals().dark_mode).hover);
    }
    let center = egui::pos2(
        rect.left() + PADDING + depth as f32 * LEVEL + ICON / 2.0,
        rect.center().y,
    );
    paint_run(
        ui,
        egui::Rect::from_center_size(center, egui::vec2(icon, icon)),
        state,
    );
    let left = center.x + ICON / 2.0 + GAP;
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        rect.right() - PADDING - left,
        egui::TextStyle::Body,
    );
    let at = egui::pos2(left, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, ui.visuals().text_color());
    response
}

fn step_text(ui: &egui::Ui, name: &str, state: State) -> RichText {
    let text = RichText::new(name).size(STEP_TEXT);
    match state {
        State::Waiting | State::Skipped => text.color(ui.visuals().weak_text_color()),
        State::Failed => text.color(color(state, ui.visuals().dark_mode)),
        State::Running => text.strong(),
        State::Passed => text,
    }
}

fn job_ui(
    ui: &mut egui::Ui,
    job: &Job,
    failure: Option<&Failure>,
    first: &mut bool,
    logged_in: bool,
) -> bool {
    let mut send = false;
    let mut log = |ui: &mut egui::Ui, depth: usize, toggle: bool| {
        let Some(failure) = failure else {
            return;
        };
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.scope_id().with(&failure.url),
            std::mem::take(first),
        );
        if toggle {
            state.toggle(ui);
        }
        state.show_body_unindented(ui, |ui| {
            send = failure_ui(ui, failure, depth, logged_in);
        });
    };
    let response = row(ui, 1, job.state, RichText::new(&job.name), true);
    if response.on_hover_text(job.state.label()).clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(&job.url));
    }
    if !matches!(job.state, State::Running | State::Failed) {
        return false;
    }
    let failed = failure.and_then(|failure| failure.step.as_deref());
    if failed.is_none() {
        log(ui, 2, false);
    }
    for step in &job.steps {
        let text = step_text(ui, &step.name, step.state);
        let clickable = failed == Some(step.name.as_str());
        let response = row(ui, 2, step.state, text, clickable).on_hover_text(step.state.label());
        if clickable {
            log(ui, 3, response.clicked());
        }
    }
    send
}

fn failure_ui(ui: &mut egui::Ui, failure: &Failure, depth: usize, logged_in: bool) -> bool {
    let indent = PADDING + (depth - 1) as f32 * LEVEL + ICON + GAP;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add_space(4.0);
            match &failure.log {
                Some(log) => {
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .stroke(Stroke::new(
                            1.0,
                            theme::palette(ui.visuals().dark_mode).line,
                        ))
                        .corner_radius(6)
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width() - PADDING);
                            egui::ScrollArea::both()
                                .id_salt(&failure.url)
                                .max_height(LOG_HEIGHT)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(log).monospace().size(LOG_TEXT),
                                        )
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
            let send = ui
                .horizontal(|ui| {
                    let send = ui
                        .add(theme::primary(ui, "Send to agent"))
                        .on_hover_text("Start the coding agent on fixing this")
                        .clicked();
                    if ui.add(theme::quiet("Open on GitHub")).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&failure.url));
                    }
                    send
                })
                .inner;
            ui.add_space(6.0);
            send
        })
        .inner
    })
    .inner
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use super::super::checks::Summary;
    use super::super::failures::Step;
    use super::*;

    fn show(found: Found) -> Harness<'static, Checks> {
        let mut checks = Checks::new(Some("o/r".to_string()));
        checks.set_commit("main", "abcdef1234");
        checks.found = found;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
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
            jobs: failures
                .iter()
                .map(|failure| Job {
                    name: failure.job.clone(),
                    state: State::Failed,
                    url: failure.url.clone(),
                    steps: vec![
                        step("Checkout", State::Passed),
                        step("Warnings", State::Failed),
                        step("Tests", State::Skipped),
                    ],
                    id: 7,
                })
                .collect(),
            failures,
            finished: state != State::Running,
            id,
            attempt: 1,
            with_logs: false,
        }
    }

    fn step(name: &str, state: State) -> Step {
        Step {
            name: name.to_string(),
            state,
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
        harness.get_by_label("GitHub Actions");
        harness.get_by_label("main · abcdef1");
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
        harness.get_by_label("Warnings");
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
            url: "https://github.com/o/r/actions/runs/2/job/8".to_string(),
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
    fn shows_the_steps_of_a_failed_job_with_the_failed_step() {
        let mut harness = show(failed(vec![failure(Some("error: unused"))]));
        open_popup(&mut harness);
        harness.get_by_label("Checkout");
        harness.get_by_label("Warnings");
        harness.get_by_label("Tests");
        harness.get_by_label("test").click();
        harness.step();
        assert_eq!(
            opened(&harness),
            ["https://github.com/o/r/actions/runs/2/job/7"]
        );
    }

    #[test]
    fn shows_what_failed_and_what_still_runs() {
        let mut test = run(
            "Test",
            State::Failed,
            2,
            vec![failure(Some("error: unused"))],
        );
        test.finished = false;
        let job = |id: u64, name: &str, state: State, steps: Vec<Step>| Job {
            name: name.to_string(),
            state,
            url: format!("https://github.com/o/r/actions/runs/2/job/{id}"),
            steps,
            id,
        };
        test.jobs.extend([
            job(
                8,
                "linux",
                State::Running,
                vec![
                    step("Checkout", State::Passed),
                    step("Compile", State::Running),
                    step("Upload", State::Waiting),
                ],
            ),
            job(
                9,
                "lint",
                State::Passed,
                vec![step("Clippy", State::Passed)],
            ),
            job(10, "windows", State::Waiting, Vec::new()),
        ]);
        let mut harness = show(Found::Runs(Summary {
            state: State::Failed,
            commit: "abcdef1234".to_string(),
            runs: vec![run("Build", State::Running, 1, Vec::new()), test],
        }));
        harness.get_by_label("Failed, still running").click();
        harness.run_steps(2);
        harness.get_by_label("Warnings");
        harness.get_by_label("error: unused");
        for shown in ["linux", "Compile", "Upload", "lint", "windows"] {
            harness.get_by_label(shown);
        }
        assert!(harness.query_by_label("Clippy").is_none());
    }

    #[test]
    fn folds_the_log_away_by_clicking_the_failed_step() {
        let log = "error: unused";
        let mut harness = show(failed(vec![failure(Some(log))]));
        open_popup(&mut harness);
        harness.get_by_label(log);
        harness.get_by_label("Warnings").click();
        harness.run();
        assert!(harness.query_by_label(log).is_none());
        harness.get_by_label("Warnings").click();
        harness.run();
        harness.get_by_label(log);
    }

    #[test]
    fn only_the_failed_step_can_be_clicked() {
        let mut harness = show(failed(vec![failure(Some("error: unused"))]));
        open_popup(&mut harness);
        harness.get_by_role_and_label(egui::accesskit::Role::Button, "Warnings");
        for step in ["Checkout", "Tests"] {
            harness.get_by_role_and_label(egui::accesskit::Role::Label, step);
            assert!(
                harness
                    .query_by_role_and_label(egui::accesskit::Role::Button, step)
                    .is_none()
            );
        }
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
        harness.get_by_label("Warnings");
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
