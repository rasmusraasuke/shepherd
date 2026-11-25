use std::sync::mpsc::{self, Receiver};
use std::thread;

use std::collections::HashMap;

use eframe::egui::containers::menu::{MenuButton, MenuConfig};
use eframe::egui::{self, Key, Modifiers, PopupCloseBehavior, RichText};

use crate::agents::{self, Agent, Choice, Model};
use crate::git::{Changes, Operation, Repo, Status};
use crate::review::{self, Agents};
use crate::ui::status::Item;
use crate::ui::theme;

const DRAFT: &str = "Write a commit message for the staged changes, which `git diff --cached` \
shows, and put it into the editor with `shepherd ctl commit-message \"<message>\"`. Don't commit.";

const SPLIT: &str = "The uncommitted changes may mix work of different intents that belong in different \
commits. See them with `shepherd ctl hunks`, which numbers the hunks of every file and the lines of \
every hunk. If they all serve one intent, stage all of them. Otherwise choose one group of changes \
that clearly belongs together and stage only that. Stage with \
`shepherd ctl stage <path>[:<pick>,<pick>...] ...`, naming every file of the group: a pick is a hunk \
`<hunk>`, or some of its lines `<hunk>.<line>` or `<hunk>.<first>-<last>` when a hunk mixes intents, \
with both the removed and the added lines of a changed line; \
without picks the whole file is staged and whatever is not named is unstaged. Check the result with \
`git diff --cached`. Then write a commit message for the staged changes and put it into the editor \
with `shepherd ctl commit-message \"<message>\"`. Don't commit.";

const DONE_SHOWN: f64 = 4.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Commit { push: bool },
    Push,
    Sync,
    Continue,
    Abort,
    Create(String),
    Switch { branch: String, leave: bool },
}

impl Op {
    fn label(&self) -> &'static str {
        match self {
            Op::Commit { .. } => "Committing…",
            Op::Push => "Pushing…",
            Op::Sync => "Syncing…",
            Op::Continue => "Continuing…",
            Op::Abort => "Aborting…",
            Op::Create(_) | Op::Switch { .. } => "Switching…",
        }
    }

    fn done(&self) -> String {
        match self {
            Op::Commit { .. } => "Committed".to_string(),
            Op::Push => "Pushed".to_string(),
            Op::Sync => "Synced".to_string(),
            Op::Continue => "Continued".to_string(),
            Op::Abort => "Aborted".to_string(),
            Op::Create(branch) | Op::Switch { branch, .. } => format!("Switched to {branch}"),
        }
    }

    fn run(&self, repo: &Repo, message: &str) -> Result<(), String> {
        match self {
            Op::Commit { .. } => repo.commit(message),
            Op::Push => repo.push(),
            Op::Sync => repo.sync(),
            Op::Continue => repo.continue_operation(),
            Op::Abort => repo.abort_operation(),
            Op::Create(name) => repo.create_branch(name),
            Op::Switch { branch, leave } => repo.switch(branch, *leave),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Run(Op),
    Draft { text: String },
    Mark(bool),
}

pub struct Flow {
    repo: Repo,
    status: Status,
    staged: bool,
    changed: bool,
    conflicts: bool,
    operation: Option<Operation>,
    message: String,
    new_branch: String,
    leave: bool,
    drafting: Option<&'static str>,
    running: Option<(Op, Receiver<Result<(), String>>)>,
    done: Option<(String, f64)>,
    error: Option<String>,
}

impl Flow {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            status: Status::default(),
            staged: false,
            changed: false,
            conflicts: false,
            operation: None,
            message: String::new(),
            new_branch: String::new(),
            leave: false,
            drafting: None,
            running: None,
            done: None,
            error: None,
        }
    }

    pub fn update(&mut self, status: Status, changes: &Changes) {
        self.status = status;
        self.staged = !changes.staged.is_empty();
        self.changed = !changes.diff.is_empty();
        self.conflicts = !changes.conflicts.is_empty();
        self.operation = changes.operation;
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn set_message(&mut self, message: String) {
        self.message = message;
        self.drafting = None;
    }

    pub fn fail(&mut self, error: String) {
        self.error = Some(error);
        self.drafting = None;
    }

    pub fn run(&mut self, op: Op, ctx: &egui::Context) {
        let (sender, result) = mpsc::channel();
        let (repo, message) = (self.repo.clone(), self.message.trim().to_string());
        let (job, ctx) = (op.clone(), ctx.clone());
        thread::spawn(move || {
            let _ = sender.send(job.run(&repo, &message));
            ctx.request_repaint();
        });
        self.error = None;
        self.done = None;
        self.running = Some((op, result));
    }

    pub fn finished(&mut self, ctx: &egui::Context) -> bool {
        let Some((op, result)) = &self.running else {
            return false;
        };
        let Ok(result) = result.try_recv() else {
            return false;
        };
        let op = op.clone();
        self.running = None;
        self.error = result.err();
        if self.error.is_none() {
            self.done = Some((op.done(), ctx.input(|i| i.time)));
            if let Op::Commit { push } = op {
                self.message.clear();
                if push {
                    self.run(Op::Push, ctx);
                }
            }
        }
        true
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        progress: (usize, usize),
        agents: &Agents,
        found: &[Agent],
        models: &HashMap<Agent, Vec<Model>>,
        choice: &mut Choice,
    ) -> Option<Outcome> {
        let op = self.operation_ui(ui);
        ui.add_space(4.0);
        let commit = egui::Panel::bottom("commit")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(egui::Margin {
                        top: 6,
                        ..Default::default()
                    }),
            )
            .show(ui, |ui| {
                self.commit_ui(ui, progress, agents, found, models, choice)
            })
            .inner;
        op.map(Outcome::Run).or(commit)
    }

    fn operation_ui(&mut self, ui: &mut egui::Ui) -> Option<Op> {
        let busy = self.running.is_some();
        let mut op = None;
        if let Some(operation) = self.operation {
            ui.horizontal(|ui| {
                ui.weak(operation.label());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if operation.can_abort()
                        && ui
                            .add_enabled(!busy, theme::quiet("Abort"))
                            .on_hover_text("Take the operation back")
                            .clicked()
                    {
                        op = Some(Op::Abort);
                    }
                    if operation.can_continue()
                        && ui
                            .add_enabled(!busy && !self.conflicts, theme::primary(ui, "Continue"))
                            .on_hover_text("Finish the operation")
                            .on_disabled_hover_text("Resolve and mark the conflicts first")
                            .clicked()
                    {
                        op = Some(Op::Continue);
                    }
                });
            });
        }
        op
    }

    fn sync_buttons(&mut self, ui: &mut egui::Ui) -> Option<Op> {
        let busy = self.running.is_some();
        let Status {
            upstream,
            ahead,
            behind,
            ..
        } = self.status;
        let (sync, push) = if upstream {
            (format!("↓{behind}"), format!("↑{ahead}"))
        } else {
            ("↓".to_string(), "↑".to_string())
        };
        let pushing = if upstream {
            "Push the commits of the branch"
        } else {
            "Push the branch, which isn't on a remote yet"
        };
        let mut op = None;
        if ui
            .add_enabled(!busy, theme::quiet(sync))
            .on_hover_text("Fetch, and pull the commits there are to pull")
            .clicked()
        {
            op = Some(Op::Sync);
        }
        if ui
            .add_enabled(!busy && (ahead > 0 || !upstream), theme::quiet(push))
            .on_hover_text(pushing)
            .clicked()
        {
            op = Some(Op::Push);
        }
        op
    }

    pub fn branch_ui(&mut self, ui: &mut egui::Ui) -> Option<Op> {
        let branch =
            theme::dropdown(RichText::new(self.status.branch.as_str()).strong()).truncate();
        let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
        let (response, menu) = MenuButton::from_button(branch)
            .config(config)
            .ui(ui, |ui| self.branches_ui(ui));
        theme::caret(ui, &response);
        response.on_hover_text("Branches");
        let op = menu.and_then(|menu| menu.inner);
        if self.operation.is_some() {
            return op;
        }
        self.sync_buttons(ui).or(op)
    }

    pub fn activity(&mut self, ctx: &egui::Context) -> Option<Item> {
        if let Some(error) = &self.error {
            return Some(Item::error(error.as_str()).dismissable());
        }
        if let Some((op, _)) = &self.running {
            return Some(Item::busy(op.label()));
        }
        let (text, at) = self.done.as_ref()?;
        let left = DONE_SHOWN - (ctx.input(|i| i.time) - at);
        if left <= 0.0 {
            self.done = None;
            return None;
        }
        ctx.request_repaint_after_secs(left as f32);
        Some(Item::done(text.as_str()))
    }

    pub fn dismiss(&mut self) {
        self.error = None;
    }

    fn branches_ui(&mut self, ui: &mut egui::Ui) -> Option<Op> {
        let mut op = None;
        let edit = egui::TextEdit::singleline(&mut self.new_branch)
            .hint_text("New branch")
            .desired_width(200.0);
        let entered = ui.add(edit).lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let name = self.new_branch.trim().replace(' ', "-");
        let create = ui.add_enabled(!name.is_empty(), egui::Button::new("Create branch"));
        if !name.is_empty() && (create.clicked() || entered) {
            op = Some(Op::Create(name));
        }
        let current = &self.status.branch;
        let others: Vec<&String> = self
            .status
            .branches
            .iter()
            .filter(|branch| *branch != current)
            .collect();
        if !others.is_empty() {
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for branch in others {
                        let label = if self.status.left.contains(branch) {
                            format!("{branch} (changes left there)")
                        } else {
                            branch.clone()
                        };
                        if ui.button(label).clicked() {
                            op = Some(Op::Switch {
                                branch: branch.clone(),
                                leave: self.leave && self.changed,
                            });
                        }
                    }
                });
            if self.changed {
                ui.separator();
                ui.checkbox(&mut self.leave, format!("Leave the changes on {current}"))
                    .on_hover_text(
                        "Without this the uncommitted changes are taken along. \
                         Changes left on a branch come back when it is switched to.",
                    );
            }
        }
        if op.is_some() {
            self.new_branch.clear();
            ui.close();
        }
        op
    }

    fn commit_ui(
        &mut self,
        ui: &mut egui::Ui,
        (done, total): (usize, usize),
        agents: &Agents,
        found: &[Agent],
        models: &HashMap<Agent, Vec<Model>>,
        choice: &mut Choice,
    ) -> Option<Outcome> {
        let busy = self.running.is_some();
        let mut outcome = None;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if total == 0 {
                return;
            }
            ui.weak(format!("{done}/{total} reviewed"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if done > 0
                    && ui
                        .add(theme::quiet("None").small())
                        .on_hover_text("Mark all changes as not reviewed")
                        .clicked()
                {
                    outcome = Some(Outcome::Mark(false));
                }
                if done < total
                    && ui
                        .add(theme::quiet("All").small())
                        .on_hover_text("Mark all changes as reviewed")
                        .clicked()
                {
                    outcome = Some(Outcome::Mark(true));
                }
            });
        });
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                agents::choice_menu(ui, found, models, choice);
                for (label, text, enabled, busy) in [
                    (
                        "Draft",
                        DRAFT,
                        self.staged,
                        "An agent is writing the message…",
                    ),
                    (
                        "Split",
                        SPLIT,
                        self.changed && !self.conflicts,
                        "An agent is choosing the changes of one commit…",
                    ),
                ] {
                    let busy = (self.drafting == Some(label)).then_some(busy);
                    let enabled = enabled && self.drafting.is_none();
                    if review::new_agent_button(ui, label, enabled, busy, agents) {
                        self.drafting = Some(label);
                        outcome = Some(Outcome::Draft {
                            text: text.to_string(),
                        });
                    }
                }
            });
        });
        let id = egui::Id::unique("commit message");
        let focused = ui.memory(|m| m.has_focus(id));
        let key = |ui: &egui::Ui, modifiers| {
            focused && ui.input_mut(|i| i.consume_key(modifiers, Key::Enter))
        };
        let push_key = key(ui, Modifiers::COMMAND | Modifiers::SHIFT);
        let commit_key = key(ui, Modifiers::COMMAND);
        ui.add(
            egui::TextEdit::multiline(&mut self.message)
                .id(id)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text(if self.drafting.is_some() {
                    "The message of the agent will show up here"
                } else {
                    "Commit message"
                }),
        );
        let ready = self.staged && !self.message.trim().is_empty() && !busy;
        let missing = if self.staged {
            "Write a commit message first"
        } else {
            "Mark changes as reviewed to commit them"
        };
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (label, keys, push, pressed) in [
                    ("Commit and push", "Cmd+Shift+Enter", true, push_key),
                    ("Commit", "Cmd+Enter", false, commit_key),
                ] {
                    let (button, width) = if push {
                        (egui::Button::new(label), 0.0)
                    } else {
                        (theme::primary(ui, label), ui.available_width())
                    };
                    let button = ui
                        .add_enabled(ready, button.min_size(egui::vec2(width, 26.0)))
                        .on_hover_text(keys)
                        .on_disabled_hover_text(missing);
                    if button.clicked() || (ready && pressed) {
                        outcome = Some(Outcome::Run(Op::Commit { push }));
                    }
                }
            });
        });
        ui.add_space(10.0);
        outcome
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};

    use super::*;
    use crate::git::tests::{commit_all, conflict_repo, temp_repo, with_remote};
    use crate::testing::TempDir;
    use crate::ui::status;

    struct Shown {
        flow: Flow,
        asked: Option<Outcome>,
        choice: Choice,
    }

    fn show(name: &str) -> (Harness<'static, Shown>, TempDir) {
        let (repo, dir) = temp_repo(name, &[("a.txt", "a\n")]);
        commit_all(&repo);
        let shown = Shown {
            flow: Flow::new(repo),
            asked: None,
            choice: Choice::default(),
        };
        let ui = |ui: &mut egui::Ui, shown: &mut Shown| render(ui, shown, (1, 2));
        let mut harness = Harness::builder()
            .with_size(egui::vec2(260.0, 500.0))
            .with_context_setup(|ctx| theme::apply(ctx, false))
            .build_ui_state(ui, shown);
        refresh(&mut harness.state_mut().flow);
        harness.run();
        (harness, dir)
    }

    fn render(ui: &mut egui::Ui, shown: &mut Shown, progress: (usize, usize)) {
        let agents = Agents {
            open: vec![(1, "Claude Code".to_string())],
            new: Some("Claude Code".to_string()),
        };
        let found = [Agent::Claude];
        let models = HashMap::new();
        let items: Vec<Item> = shown.flow.activity(ui.ctx()).into_iter().collect();
        let (area, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
        if status::ui(ui, &items, area).is_some() {
            shown.flow.dismiss();
        }
        let branch = shown.flow.branch_ui(ui).map(Outcome::Run);
        match branch.or(shown
            .flow
            .ui(ui, progress, &agents, &found, &models, &mut shown.choice))
        {
            Some(Outcome::Run(op)) => shown.flow.run(op, ui.ctx()),
            Some(asked) => shown.asked = Some(asked),
            None => {}
        }
        if shown.flow.finished(ui.ctx()) {
            refresh(&mut shown.flow);
        }
    }

    fn refresh(flow: &mut Flow) {
        let changes = flow.repo.changes().unwrap();
        flow.update(flow.repo.status(), &changes);
    }

    fn type_into(harness: &mut Harness<'_, Shown>, role: egui::Role, text: &str) {
        harness.get_by_role(role).click();
        harness.run();
        harness.get_by_role(role).type_text(text);
    }

    fn wait_for(harness: &mut Harness<'_, Shown>, label: &str, shown: bool) {
        for _ in 0..400 {
            harness.step();
            if harness.query_by_label(label).is_some() == shown {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!(
            "{label:?} shown isn't {shown}: {:?}",
            harness.state().flow.error
        );
    }

    #[test]
    fn continues_a_merge_once_the_conflicts_are_resolved() {
        let (repo, dir) = conflict_repo("flow-conflict");
        let shown = Shown {
            flow: Flow::new(repo),
            asked: None,
            choice: Choice::default(),
        };
        let ui = |ui: &mut egui::Ui, shown: &mut Shown| render(ui, shown, (0, 0));
        let mut harness = Harness::builder()
            .with_size(egui::vec2(260.0, 500.0))
            .with_context_setup(|ctx| theme::apply(ctx, false))
            .build_ui_state(ui, shown);
        refresh(&mut harness.state_mut().flow);
        harness.run();
        harness.get_by_label("Merging");
        assert!(
            harness
                .get_by_label("Continue")
                .accesskit_node()
                .is_disabled()
        );

        dir.write("a.txt", "resolved\n");
        harness.state().flow.repo.stage_file("a.txt", true).unwrap();
        refresh(&mut harness.state_mut().flow);
        harness.run();
        assert!(
            !harness
                .get_by_label("Continue")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("Continue").click();
        wait_for(&mut harness, "Continued", true);
        assert_eq!(harness.state().flow.operation, None);
    }

    #[test]
    fn keeps_draft_and_its_agent_menu_above_the_commit_message() {
        let (mut harness, _dir) = show("flow-draft-above");
        let draft = harness.get_by_label("Draft").rect();
        let message = harness
            .get_by_role(egui::Role::MultilineTextInput)
            .rect()
            .top();
        assert!(draft.bottom() <= message);
        assert!(harness.query_by_label("Claude Code").is_none());
        assert!(harness.query_by_label("Default model").is_none());
        harness
            .get_by_label("Harness and model of the agent")
            .hover();
        harness.run();
        let menu =
            harness.get_by_role_and_label(egui::Role::Button, "Harness and model of the agent");
        assert!(menu.rect().left() >= draft.right());
        menu.click();
        harness.run();
        harness.get_by_label("Harness: Claude Code").click();
        harness.run();
        harness.get_by_label("Claude Code").click();
        harness.run();
        assert_eq!(harness.state().choice.agent, Some(Agent::Claude));
    }

    #[test]
    fn splits_the_changes_with_an_agent_next_to_draft() {
        let (mut harness, dir) = show("flow-split");
        assert!(harness.get_by_label("Split").accesskit_node().is_disabled());
        dir.write("a.txt", "changed\n");
        refresh(&mut harness.state_mut().flow);
        harness.run();
        assert!(harness.get_by_label("Draft").accesskit_node().is_disabled());
        assert!(
            harness.get_by_label("Split").rect().right()
                <= harness.get_by_label("Draft").rect().left()
        );
        harness.get_by_label("Split").click();
        harness.run_steps(4);
        assert!(matches!(
            &harness.state().asked,
            Some(Outcome::Draft { text }) if text.contains("shepherd ctl stage")
                && text.contains("shepherd ctl commit-message")
        ));
        assert!(harness.get_by_label("Split").accesskit_node().is_disabled());
        assert!(harness.get_by_label("Draft").accesskit_node().is_disabled());
        harness.state_mut().flow.set_message("Change a".to_string());
        harness.run();
        assert!(!harness.get_by_label("Split").accesskit_node().is_disabled());
    }

    #[test]
    fn commits_what_is_staged_with_the_message_and_pushes_it() {
        let (mut harness, dir) = show("flow-commit");
        let repo = harness.state().flow.repo.clone();
        let _remote = with_remote(&repo, "flow-commit-remote");
        dir.write("a.txt", "changed\n");
        dir.write("b.txt", "new\n");
        refresh(&mut harness.state_mut().flow);
        harness.run();
        assert!(
            harness
                .get_by_label("Commit")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("All").click();
        harness.run();
        assert_eq!(harness.state().asked, Some(Outcome::Mark(true)));

        repo.stage_file("a.txt", true).unwrap();
        refresh(&mut harness.state_mut().flow);
        harness.run();
        harness.get_by_label("Draft").click();
        harness.run_steps(4);
        assert!(matches!(
            &harness.state().asked,
            Some(Outcome::Draft { text }) if text.contains("shepherd ctl commit-message")
        ));
        assert!(harness.get_by_label("Draft").accesskit_node().is_disabled());
        assert!(harness.query_by_label("Cancel").is_none());
        harness.state_mut().flow.set_message("Change a".to_string());
        harness.run();
        assert!(!harness.get_by_label("Draft").accesskit_node().is_disabled());
        harness.get_by_label("Commit and push").click();
        wait_for(&mut harness, "↑0", true);
        assert_eq!(repo.changes().unwrap().untracked, ["b.txt"]);
        assert!(repo.changes().unwrap().staged.is_empty());
        assert!(harness.state().flow.message.is_empty());
    }

    #[test]
    fn pushes_the_commits_of_the_branch_from_the_top() {
        let (mut harness, dir) = show("flow-push");
        let repo = harness.state().flow.repo.clone();
        let _remote = with_remote(&repo, "flow-push-remote");
        harness.get_by_label("↑").click();
        wait_for(&mut harness, "Pushing…", true);
        wait_for(&mut harness, "Pushed", true);
        wait_for(&mut harness, "↑0", true);
        dir.write("a.txt", "changed\n");
        commit_all(&repo);
        refresh(&mut harness.state_mut().flow);
        harness.run();
        harness.get_by_label("↑1").click();
        wait_for(&mut harness, "↑0", true);
        assert_eq!(repo.status().ahead, 0);
    }

    #[test]
    fn creates_a_branch_and_switches_back_leaving_the_changes() {
        let (mut harness, dir) = show("flow-branches");
        let repo = harness.state().flow.repo.clone();
        let main = repo.branch();
        harness.get_by_label(main.as_str()).click();
        harness.run();
        type_into(&mut harness, egui::Role::TextInput, "my feature");
        harness.run();
        harness.get_by_label("Create branch").click();
        wait_for(&mut harness, "my-feature", true);
        assert!(harness.query_by_label("Create branch").is_none());

        dir.write("a.txt", "changed\n");
        refresh(&mut harness.state_mut().flow);
        harness.get_by_label("my-feature").click();
        harness.run();
        harness
            .get_by_label("Leave the changes on my-feature")
            .click();
        harness.run();
        harness.get_by_label(main.as_str()).click();
        wait_for(&mut harness, "my-feature", false);
        assert_eq!(repo.branch(), main);
        assert!(repo.changes().unwrap().diff.is_empty());
        harness.get_by_label(main.as_str()).click();
        harness.run();
        harness.get_by_label("my-feature (changes left there)");
    }
}
