use eframe::egui::{self, RichText};

use super::CLIENT_ID;
use super::checks::{Checks, Found, State};
use crate::ui::status::{self, Kind};
use crate::ui::theme;

const HEIGHT: f32 = 22.0;
const ICON: f32 = 12.0;
const GAP: f32 = 6.0;
const PADDING: f32 = 8.0;
const MARGIN: f32 = 8.0;
const POPUP_WIDTH: f32 = 320.0;
const COMPACT: f32 = 760.0;

struct Look {
    text: String,
    kind: Option<Kind>,
    failed: bool,
    compact: bool,
}

impl Look {
    fn new(text: impl Into<String>, kind: Option<Kind>) -> Self {
        Self {
            text: text.into(),
            kind,
            failed: matches!(kind, Some(Kind::Error)),
            compact: false,
        }
    }
}

fn kind(state: State) -> Kind {
    match state {
        State::Passed => Kind::Done,
        State::Running => Kind::Busy,
        State::Failed => Kind::Error,
    }
}

fn label(state: State) -> &'static str {
    match state {
        State::Passed => "Passed",
        State::Running => "Running",
        State::Failed => "Failed",
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
        let icon_width = look.kind.map_or(0.0, |_| ICON);
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
        let popup = egui::Popup::menu(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(POPUP_WIDTH);
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
        if let Some(kind) = look.kind {
            let icon = egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - ICON / 2.0),
                egui::vec2(ICON, ICON),
            );
            status::paint_icon(ui, icon, kind);
            x += ICON + gap;
        }
        if let Some(galley) = galley {
            let at = egui::pos2(x, rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(at, galley, color);
        }
        popup.show(|ui| {
            ui.set_max_width(POPUP_WIDTH);
            self.popup_ui(ui);
        });
        rect.left()
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
                compact: true,
                ..Look::new(label(summary.state), Some(kind(summary.state)))
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
        match &self.found {
            Found::Nothing => {}
            Found::Runs(summary) => {
                ui.weak(format!(
                    "GitHub Actions on {} at {}",
                    self.branch,
                    &summary.commit[..summary.commit.len().min(7)]
                ));
                for run in &summary.runs {
                    ui.horizontal(|ui| {
                        let (icon, _) =
                            ui.allocate_exact_size(egui::vec2(ICON, ICON), egui::Sense::hover());
                        status::paint_icon(ui, icon, kind(run.state));
                        if ui
                            .add(theme::quiet(run.name.as_str()))
                            .on_hover_text(label(run.state))
                            .clicked()
                        {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(&run.url));
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

    #[test]
    fn shows_how_the_workflows_went_and_opens_one_on_github() {
        let mut harness = show(Found::Runs(Summary {
            state: State::Failed,
            commit: "abcdef1234".to_string(),
            runs: vec![
                Run {
                    name: "Build".to_string(),
                    state: State::Passed,
                    url: "https://github.com/o/r/actions/runs/1".to_string(),
                },
                Run {
                    name: "Test".to_string(),
                    state: State::Failed,
                    url: "https://github.com/o/r/actions/runs/2".to_string(),
                },
            ],
        }));
        harness.get_by_label("Failed").click();
        harness.run();
        harness.get_by_label("GitHub Actions on main at abcdef1");
        harness.get_by_label("Build");
        harness.get_by_label("Log in to GitHub");
        harness.get_by_label("Test").click();
        harness.step();
        assert_eq!(opened(&harness), ["https://github.com/o/r/actions/runs/2"]);
    }

    #[test]
    fn asks_to_log_in_when_github_hides_the_repository() {
        let mut harness = show(Found::LogIn);
        harness.get_by_label("Log in to GitHub").click();
        harness.run_steps(2);
        harness.get_by_label("Log in to see GitHub Actions of private repositories.");
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
