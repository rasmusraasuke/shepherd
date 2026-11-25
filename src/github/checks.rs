use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use serde::Deserialize;

use super::login::{Event, Login};
use super::{forget_token, get, token};
use crate::git::DETACHED;

const WATCHING: Duration = Duration::from_secs(15);

const RESTING: Duration = Duration::from_secs(60);

const WATCHING_ANONYMOUSLY: Duration = Duration::from_secs(120);

const RESTING_ANONYMOUSLY: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Passed,
    Running,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub name: String,
    pub state: State,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub state: State,
    pub commit: String,
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Found {
    #[default]
    Nothing,
    Runs(Summary),
    LogIn,
    NoAccess,
}

struct Fetched {
    found: Found,
    logged_in: bool,
}

pub struct Checks {
    pub(super) repository: Option<String>,
    pub(super) branch: String,
    pub(super) found: Found,
    pub(super) logged_in: bool,
    pub(super) login: Option<Login>,
    pub(super) error: Option<String>,
    head: String,
    fetching: Option<Receiver<Result<Fetched, String>>>,
    next: Instant,
}

impl Checks {
    pub fn new(repository: Option<String>) -> Self {
        Self {
            repository,
            branch: String::new(),
            found: Found::Nothing,
            logged_in: false,
            login: None,
            error: None,
            head: String::new(),
            fetching: None,
            next: Instant::now(),
        }
    }

    pub fn set_commit(&mut self, branch: &str, head: &str) {
        if branch != self.branch {
            self.branch = branch.to_string();
            self.found = Found::Nothing;
            self.fetching = None;
            self.refresh();
        }
        if head != self.head {
            self.head = head.to_string();
            self.refresh();
        }
    }

    pub fn refresh(&mut self) {
        self.next = Instant::now();
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        self.follow_login(ctx);
        if let Some(fetched) = self.fetching.as_ref().and_then(|r| r.try_recv().ok()) {
            self.fetching = None;
            match fetched {
                Ok(fetched) => {
                    self.found = fetched.found;
                    self.logged_in = fetched.logged_in;
                }
                Err(error) => eprintln!("cannot check GitHub Actions: {error}"),
            }
            self.next = Instant::now() + self.interval();
        }
        if self.branch.is_empty() || self.branch == DETACHED || self.fetching.is_some() {
            return;
        }
        let now = Instant::now();
        if now < self.next {
            ctx.request_repaint_after(self.next - now);
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let branch = self.branch.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = sender.send(fetch(&repository, &branch));
            ctx.request_repaint();
        });
        self.fetching = Some(receiver);
    }

    fn interval(&self) -> Duration {
        let watching = match &self.found {
            Found::Runs(summary) => summary.state == State::Running || summary.commit != self.head,
            _ => false,
        };
        match (self.logged_in, watching) {
            (true, true) => WATCHING,
            (true, false) => RESTING,
            (false, true) => WATCHING_ANONYMOUSLY,
            (false, false) => RESTING_ANONYMOUSLY,
        }
    }

    pub(super) fn log_in(&mut self, ctx: &egui::Context) {
        self.error = None;
        self.login = Some(Login::start(ctx.clone()));
    }

    pub(super) fn log_out(&mut self) {
        forget_token();
        self.logged_in = false;
        self.login = None;
        self.found = Found::Nothing;
        self.fetching = None;
        self.refresh();
    }

    fn follow_login(&mut self, ctx: &egui::Context) {
        let Some(event) = self.login.as_mut().and_then(Login::next) else {
            return;
        };
        match event {
            Event::Code { code, url } => {
                ctx.copy_text(code);
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            Event::Done(result) => {
                self.login = None;
                self.error = result.err();
                self.refresh();
            }
        }
    }
}

fn fetch(repository: &str, branch: &str) -> Result<Fetched, String> {
    let token = token();
    let response = get(
        &format!("/repos/{repository}/actions/runs"),
        &[("branch", branch), ("per_page", "30")],
        token.as_deref(),
    )?;
    let logged_in = token.is_some();
    let found = match response.status {
        200 => summary(&response.body)?.map_or(Found::Nothing, Found::Runs),
        401 if logged_in => {
            forget_token();
            return Ok(Fetched {
                found: Found::LogIn,
                logged_in: false,
            });
        }
        401 | 404 if !logged_in => Found::LogIn,
        404 => Found::NoAccess,
        status => return Err(format!("GitHub answered with {status}")),
    };
    Ok(Fetched { found, logged_in })
}

#[derive(Deserialize)]
struct ApiRuns {
    workflow_runs: Vec<ApiRun>,
}

#[derive(Deserialize)]
struct ApiRun {
    name: Option<String>,
    workflow_id: u64,
    head_sha: String,
    status: Option<String>,
    conclusion: Option<String>,
    html_url: String,
}

impl ApiRun {
    fn state(&self) -> State {
        match (self.status.as_deref(), self.conclusion.as_deref()) {
            (Some("completed"), Some("success" | "skipped" | "neutral")) => State::Passed,
            (Some("completed"), _) => State::Failed,
            _ => State::Running,
        }
    }
}

fn summary(json: &str) -> Result<Option<Summary>, String> {
    let api: ApiRuns =
        serde_json::from_str(json).map_err(|e| format!("Cannot read the workflow runs: {e}"))?;
    let Some(commit) = api.workflow_runs.first().map(|run| run.head_sha.clone()) else {
        return Ok(None);
    };
    let mut workflows = HashSet::new();
    let runs: Vec<Run> = api
        .workflow_runs
        .iter()
        .filter(|run| run.head_sha == commit && workflows.insert(run.workflow_id))
        .map(|run| Run {
            name: run.name.clone().unwrap_or_else(|| "Workflow".to_string()),
            state: run.state(),
            url: run.html_url.clone(),
        })
        .collect();
    let any = |state| runs.iter().any(|run| run.state == state);
    let state = if any(State::Failed) {
        State::Failed
    } else if any(State::Running) {
        State::Running
    } else {
        State::Passed
    };
    Ok(Some(Summary {
        state,
        commit,
        runs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: u64, sha: &str, status: &str, conclusion: Option<&str>) -> String {
        let conclusion = conclusion.map_or("null".to_string(), |c| format!("\"{c}\""));
        format!(
            r#"{{"name":"Workflow {id}","workflow_id":{id},"head_sha":"{sha}","status":"{status}","conclusion":{conclusion},"html_url":"https://github.com/o/r/actions/runs/{id}{sha}"}}"#
        )
    }

    fn runs(runs: &[String]) -> String {
        format!(
            r#"{{"total_count":{},"workflow_runs":[{}]}}"#,
            runs.len(),
            runs.join(",")
        )
    }

    fn state_of(list: &[String]) -> State {
        summary(&runs(list)).unwrap().unwrap().state
    }

    #[test]
    fn sums_up_the_workflows_of_the_latest_commit() {
        let json = runs(&[
            run(1, "new", "in_progress", None),
            run(2, "new", "completed", Some("success")),
            run(1, "old", "completed", Some("failure")),
        ]);
        let summary = summary(&json).unwrap().unwrap();
        assert_eq!(summary.commit, "new");
        assert_eq!(summary.state, State::Running);
        assert_eq!(
            summary
                .runs
                .iter()
                .map(|run| (run.name.as_str(), run.state))
                .collect::<Vec<_>>(),
            vec![
                ("Workflow 1", State::Running),
                ("Workflow 2", State::Passed)
            ]
        );
        assert_eq!(
            summary.runs[0].url,
            "https://github.com/o/r/actions/runs/1new"
        );
    }

    #[test]
    fn passes_only_when_every_workflow_passed() {
        assert_eq!(
            state_of(&[
                run(1, "a", "completed", Some("success")),
                run(2, "a", "completed", Some("skipped")),
            ]),
            State::Passed
        );
        assert_eq!(
            state_of(&[
                run(1, "a", "queued", None),
                run(2, "a", "completed", Some("timed_out")),
            ]),
            State::Failed
        );
        assert_eq!(
            state_of(&[
                run(1, "a", "completed", Some("success")),
                run(2, "a", "waiting", None),
            ]),
            State::Running
        );
    }

    #[test]
    fn a_rerun_replaces_the_earlier_run_of_the_same_workflow() {
        assert_eq!(
            state_of(&[
                run(1, "a", "completed", Some("success")),
                run(1, "a", "completed", Some("failure")),
            ]),
            State::Passed
        );
    }

    #[test]
    fn finds_nothing_on_a_branch_without_runs() {
        assert_eq!(summary(&runs(&[])).unwrap(), None);
    }

    #[test]
    fn watches_closely_while_runs_go_or_the_pushed_commit_has_none_yet() {
        let mut checks = Checks::new(Some("o/r".to_string()));
        checks.set_commit("main", "a");
        let summary = |state| {
            Found::Runs(Summary {
                state,
                commit: "a".to_string(),
                runs: Vec::new(),
            })
        };
        checks.found = summary(State::Passed);
        assert_eq!(checks.interval(), RESTING_ANONYMOUSLY);
        checks.logged_in = true;
        assert_eq!(checks.interval(), RESTING);
        checks.found = summary(State::Running);
        assert_eq!(checks.interval(), WATCHING);
        checks.found = summary(State::Passed);
        checks.set_commit("main", "b");
        assert_eq!(checks.interval(), WATCHING);
    }

    #[test]
    fn forgets_what_it_found_when_the_branch_changes() {
        let mut checks = Checks::new(Some("o/r".to_string()));
        checks.set_commit("main", "a");
        checks.found = Found::LogIn;
        checks.set_commit("main", "b");
        assert_eq!(checks.found, Found::LogIn);
        checks.set_commit("feature", "b");
        assert_eq!(checks.found, Found::Nothing);
    }
}
