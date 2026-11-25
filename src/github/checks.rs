use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use serde::Deserialize;

use super::failures::{self, Failure};
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

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Passed => "Passed",
            State::Running => "Running",
            State::Failed => "Failed",
        }
    }
}

pub(super) fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub name: String,
    pub state: State,
    pub url: String,
    pub failures: Vec<Failure>,
    pub(super) id: u64,
    pub(super) attempt: u32,
    pub(super) with_logs: bool,
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
    pub(crate) prompt: Option<String>,
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
            prompt: None,
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

    pub fn take_prompt(&mut self) -> Option<String> {
        self.prompt.take()
    }

    pub fn report(&self) -> String {
        let Some(repository) = &self.repository else {
            return "The origin of the repository is not on GitHub".to_string();
        };
        match &self.found {
            Found::Nothing => format!(
                "No GitHub Actions runs of {repository} on branch {}",
                self.branch
            ),
            Found::LogIn => format!(
                "GitHub shows the runs of {repository} only after the user logs in to GitHub \
                 from the title bar"
            ),
            Found::NoAccess => format!(
                "GitHub does not show {repository} to Shepherd until an owner of its \
                 organization approves Shepherd"
            ),
            Found::Runs(summary) => {
                let mut lines = vec![format!(
                    "GitHub Actions of {repository} on branch {} at commit {}: {}",
                    self.branch,
                    short(&summary.commit),
                    summary.state.label()
                )];
                if summary.commit != self.head {
                    lines.push(format!(
                        "The local commit {} has no runs on GitHub yet",
                        short(&self.head)
                    ));
                }
                for run in &summary.runs {
                    lines.push(format!(
                        "- {}: {} ({})",
                        run.name,
                        run.state.label(),
                        run.url
                    ));
                    for failure in &run.failures {
                        lines.push(format!("  Failed in {}", failure.describe()));
                    }
                }
                lines.join("\n")
            }
        }
    }

    pub(super) fn prompt(&self, commit: &str, run: &Run, failures: &[Failure]) -> String {
        let failures: Vec<String> = failures
            .iter()
            .map(|failure| format!("In {}", failure.describe()))
            .collect();
        format!(
            "The GitHub Actions workflow \"{}\" failed on branch {} at commit {}.\n\n{}\n\n\
             Find out why, fix it and check the fix locally. \
             `shepherd ctl ci` shows how every workflow of that commit went.",
            run.name,
            self.branch,
            short(commit),
            failures.join("\n\n")
        )
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
        let known = match &self.found {
            Found::Runs(summary) => summary.runs.clone(),
            _ => Vec::new(),
        };
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = sender.send(fetch(&repository, &branch, &known));
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

fn fetch(repository: &str, branch: &str, known: &[Run]) -> Result<Fetched, String> {
    let token = token();
    let logged_in = token.is_some();
    let response = get(
        &format!("/repos/{repository}/actions/runs"),
        &[("branch", branch), ("per_page", "30")],
        token.as_deref(),
    )?;
    let found = match response.status {
        200 => match summary(&response.body)? {
            Some(mut summary) => {
                explain(&mut summary, known, logged_in, |run| {
                    failures::fetch(repository, run, token.as_deref())
                });
                Found::Runs(summary)
            }
            None => Found::Nothing,
        },
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

fn explain(
    summary: &mut Summary,
    known: &[Run],
    logged_in: bool,
    fetch: impl Fn(u64) -> Result<Vec<Failure>, String>,
) {
    for run in summary
        .runs
        .iter_mut()
        .filter(|run| run.state == State::Failed)
    {
        let seen = known.iter().find(|seen| {
            seen.id == run.id && seen.attempt == run.attempt && (seen.with_logs || !logged_in)
        });
        match seen {
            Some(seen) => {
                run.failures = seen.failures.clone();
                run.with_logs = seen.with_logs;
            }
            None => match fetch(run.id) {
                Ok(failures) => {
                    run.failures = failures;
                    run.with_logs = logged_in;
                }
                Err(error) => eprintln!("cannot find what failed in GitHub Actions: {error}"),
            },
        }
    }
}

#[derive(Deserialize)]
struct ApiRuns {
    workflow_runs: Vec<ApiRun>,
}

#[derive(Deserialize)]
struct ApiRun {
    id: u64,
    #[serde(default = "first_attempt")]
    run_attempt: u32,
    name: Option<String>,
    workflow_id: u64,
    head_sha: String,
    status: Option<String>,
    conclusion: Option<String>,
    html_url: String,
}

fn first_attempt() -> u32 {
    1
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
            failures: Vec::new(),
            id: run.id,
            attempt: run.run_attempt,
            with_logs: false,
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
            r#"{{"id":{id},"name":"Workflow {id}","workflow_id":{id},"head_sha":"{sha}","status":"{status}","conclusion":{conclusion},"html_url":"https://github.com/o/r/actions/runs/{id}{sha}"}}"#
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

    fn failure(log: Option<&str>) -> Failure {
        Failure {
            job: "test".to_string(),
            step: Some("Warnings".to_string()),
            url: "https://github.com/o/r/actions/runs/1/job/7".to_string(),
            log: log.map(str::to_string),
        }
    }

    fn explained(known: &[Run], logged_in: bool) -> (Summary, Vec<u64>) {
        let mut summary = summary(&runs(&[
            run(1, "a", "completed", Some("failure")),
            run(2, "a", "completed", Some("success")),
        ]))
        .unwrap()
        .unwrap();
        let asked = std::cell::RefCell::new(Vec::new());
        explain(&mut summary, known, logged_in, |run| {
            asked.borrow_mut().push(run);
            Ok(vec![failure(logged_in.then_some("error: unused"))])
        });
        (summary, asked.into_inner())
    }

    #[test]
    fn asks_what_failed_once_for_each_failed_run() {
        let (first, asked) = explained(&[], false);
        assert_eq!(asked, [1]);
        assert_eq!(first.runs[0].failures, [failure(None)]);
        assert!(first.runs[1].failures.is_empty());
        let (again, asked) = explained(&first.runs, false);
        assert!(asked.is_empty());
        assert_eq!(again.runs[0].failures, [failure(None)]);
    }

    #[test]
    fn asks_again_for_the_logs_after_logging_in() {
        let (anonymous, _) = explained(&[], false);
        let (logged_in, asked) = explained(&anonymous.runs, true);
        assert_eq!(asked, [1]);
        assert_eq!(logged_in.runs[0].failures, [failure(Some("error: unused"))]);
        let (logged_out, asked) = explained(&logged_in.runs, false);
        assert!(asked.is_empty());
        assert_eq!(
            logged_out.runs[0].failures,
            [failure(Some("error: unused"))]
        );
    }

    fn reported(found: Found, head: &str) -> Checks {
        let mut checks = Checks::new(Some("o/r".to_string()));
        checks.set_commit("main", head);
        checks.found = found;
        checks
    }

    fn failed_summary() -> Summary {
        let mut summary = summary(&runs(&[
            run(1, "abcdef1234", "completed", Some("failure")),
            run(2, "abcdef1234", "completed", Some("success")),
        ]))
        .unwrap()
        .unwrap();
        summary.runs[0].failures = vec![failure(Some("error: unused"))];
        summary
    }

    #[test]
    fn reports_how_the_workflows_went_with_what_failed() {
        let checks = reported(Found::Runs(failed_summary()), "abcdef1234");
        assert_eq!(
            checks.report(),
            "GitHub Actions of o/r on branch main at commit abcdef1: Failed\n\
             - Workflow 1: Failed (https://github.com/o/r/actions/runs/1abcdef1234)\n  \
             Failed in test › Warnings (https://github.com/o/r/actions/runs/1/job/7):\n\
             ```\nerror: unused\n```\n\
             - Workflow 2: Passed (https://github.com/o/r/actions/runs/2abcdef1234)"
        );
        let ahead = reported(Found::Runs(failed_summary()), "9999999999");
        assert!(
            ahead
                .report()
                .contains("The local commit 9999999 has no runs on GitHub yet")
        );
    }

    #[test]
    fn reports_why_there_is_nothing_to_show() {
        assert_eq!(
            Checks::new(None).report(),
            "The origin of the repository is not on GitHub"
        );
        assert_eq!(
            reported(Found::Nothing, "a").report(),
            "No GitHub Actions runs of o/r on branch main"
        );
        assert!(reported(Found::LogIn, "a").report().contains("logs in"));
        assert!(reported(Found::NoAccess, "a").report().contains("approves"));
    }

    #[test]
    fn asks_the_agent_to_fix_the_failed_step() {
        let summary = failed_summary();
        let checks = reported(Found::Runs(summary.clone()), "abcdef1234");
        let run = &summary.runs[0];
        assert_eq!(
            checks.prompt(&summary.commit, run, &run.failures),
            "The GitHub Actions workflow \"Workflow 1\" failed on branch main at commit abcdef1.\n\n\
             In test › Warnings (https://github.com/o/r/actions/runs/1/job/7):\n\
             ```\nerror: unused\n```\n\n\
             Find out why, fix it and check the fix locally. \
             `shepherd ctl ci` shows how every workflow of that commit went."
        );
        let both = [failure(None), failure(Some("error: unused"))];
        let prompt = checks.prompt(&summary.commit, run, &both);
        assert!(
            prompt.contains(
                "In test › Warnings (https://github.com/o/r/actions/runs/1/job/7)\n\n\
                 In test › Warnings (https://github.com/o/r/actions/runs/1/job/7):"
            ),
            "{prompt}"
        );
    }
}
