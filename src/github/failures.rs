use serde::Deserialize;

use super::checks::{State, state};
use super::get;

const LOG_LINES: usize = 60;

const ERROR: &str = "##[error]";

const GROUP: &str = "##[group]";

const END_GROUP: &str = "##[endgroup]";

#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub job: String,
    pub step: Option<String>,
    pub url: String,
    pub log: Option<String>,
}

impl Failure {
    pub fn title(&self) -> String {
        match &self.step {
            Some(step) => format!("{} › {step}", self.job),
            None => self.job.clone(),
        }
    }

    pub fn describe(&self) -> String {
        match &self.log {
            Some(log) => format!("{} ({}):\n```\n{log}\n```", self.title(), self.url),
            None => format!("{} ({})", self.title(), self.url),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub name: String,
    pub state: State,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub name: String,
    pub state: State,
    pub url: String,
    pub steps: Vec<Step>,
    pub(super) id: u64,
}

impl Job {
    pub fn current(&self) -> Option<&Step> {
        self.steps.iter().find(|step| step.state == State::Running)
    }

    pub fn failure(&self) -> Option<Failure> {
        (self.state == State::Failed).then(|| Failure {
            job: self.name.clone(),
            step: self
                .steps
                .iter()
                .find(|step| step.state == State::Failed)
                .map(|step| step.name.clone()),
            url: self.url.clone(),
            log: None,
        })
    }
}

#[derive(Deserialize)]
struct ApiJobs {
    jobs: Vec<ApiJob>,
}

#[derive(Deserialize)]
struct ApiJob {
    id: u64,
    name: String,
    status: Option<String>,
    conclusion: Option<String>,
    html_url: String,
    #[serde(default)]
    steps: Vec<ApiStep>,
}

#[derive(Deserialize)]
struct ApiStep {
    name: String,
    status: Option<String>,
    conclusion: Option<String>,
}

pub fn fetch(repository: &str, run: u64, token: Option<&str>) -> Result<Vec<Job>, String> {
    let response = get(
        &format!("/repos/{repository}/actions/runs/{run}/jobs"),
        &[("per_page", "100")],
        token,
    )?;
    if response.status != 200 {
        return Err(format!("GitHub answered with {}", response.status));
    }
    jobs(&response.body)
}

fn jobs(json: &str) -> Result<Vec<Job>, String> {
    let api: ApiJobs =
        serde_json::from_str(json).map_err(|e| format!("Cannot read the jobs: {e}"))?;
    Ok(api
        .jobs
        .into_iter()
        .map(|job| Job {
            state: state(job.status.as_deref(), job.conclusion.as_deref()),
            steps: job
                .steps
                .into_iter()
                .map(|step| Step {
                    state: state(step.status.as_deref(), step.conclusion.as_deref()),
                    name: step.name,
                })
                .collect(),
            name: job.name,
            url: job.html_url,
            id: job.id,
        })
        .collect())
}

pub fn log(repository: &str, job: u64, token: &str) -> Option<String> {
    let response = get(
        &format!("/repos/{repository}/actions/jobs/{job}/logs"),
        &[],
        Some(token),
    )
    .ok()?;
    (response.status == 200)
        .then(|| excerpt(&response.body))
        .flatten()
}

fn excerpt(log: &str) -> Option<String> {
    let lines: Vec<String> = log.lines().map(clean).collect();
    let first = lines.iter().position(|line| line.starts_with(ERROR))?;
    let start = lines[..first]
        .iter()
        .rposition(|line| line.starts_with(END_GROUP) || line.starts_with(GROUP))
        .map_or(0, |at| at + 1);
    let next = lines[first..]
        .iter()
        .position(|line| line.starts_with(GROUP))
        .map_or(lines.len(), |at| first + at);
    let end = lines[..next]
        .iter()
        .rposition(|line| line.starts_with(ERROR))?;
    let shown = &lines[start.max((end + 1).saturating_sub(LOG_LINES))..=end];
    Some(
        shown
            .iter()
            .map(|line| line.strip_prefix(ERROR).unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn clean(line: &str) -> String {
    let line = line.trim_start_matches('\u{feff}');
    let line = match line.split_once(' ') {
        Some((stamp, rest)) if is_stamp(stamp) => rest,
        _ if is_stamp(line) => "",
        _ => line,
    };
    let mut text = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            text.push(c);
        }
    }
    text
}

fn is_stamp(word: &str) -> bool {
    word.len() >= 20
        && word.ends_with('Z')
        && word.as_bytes()[4] == b'-'
        && word.as_bytes()[10] == b'T'
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\u{feff}2026-11-25T20:00:00.0000001Z ##[group]Run actions/checkout@v4
2026-11-25T20:00:00.0000002Z with:
2026-11-25T20:00:00.0000003Z ##[endgroup]
2026-11-25T20:00:01.0000000Z Checked out
2026-11-25T20:00:02.0000000Z ##[group]Run cargo clippy -- -D warnings
2026-11-25T20:00:02.0000001Z cargo clippy -- -D warnings
2026-11-25T20:00:02.0000002Z shell: /usr/bin/bash -e {0}
2026-11-25T20:00:02.0000003Z ##[endgroup]
2026-11-25T20:00:03.0000000Z \u{1b}[1m\u{1b}[91merror\u{1b}[0m: unused variable: `x`
2026-11-25T20:00:03.0000001Z  --> src/main.rs:3:9
2026-11-25T20:00:03.0000002Z
2026-11-25T20:00:04.0000000Z ##[error]Process completed with exit code 101.
2026-11-25T20:00:05.0000000Z ##[group]Post Run actions/checkout@v4
2026-11-25T20:00:05.0000001Z Cleaning up
2026-11-25T20:00:05.0000002Z ##[endgroup]";

    #[test]
    fn keeps_what_the_failed_step_printed() {
        assert_eq!(
            excerpt(LOG).as_deref(),
            Some(
                "error: unused variable: `x`\n --> src/main.rs:3:9\n\nProcess completed with exit code 101."
            )
        );
    }

    #[test]
    fn keeps_only_the_end_of_a_long_step() {
        let mut log = "##[group]Run tests\n##[endgroup]\n".to_string();
        for line in 0..200 {
            log.push_str(&format!("line {line}\n"));
        }
        log.push_str("##[error]Process completed with exit code 1.\n");
        let excerpt = excerpt(&log).unwrap();
        assert_eq!(excerpt.lines().count(), LOG_LINES);
        assert!(excerpt.starts_with("line 141\n"));
    }

    #[test]
    fn finds_nothing_in_a_log_without_errors() {
        assert_eq!(excerpt("##[group]Run tests\n##[endgroup]\nok\n"), None);
    }

    #[test]
    fn reads_the_jobs_with_their_steps_and_what_failed() {
        let json = r#"{"total_count":4,"jobs":[
            {"id":1,"name":"test","status":"completed","conclusion":"failure","html_url":"https://github.com/o/r/actions/runs/9/job/1",
             "steps":[{"name":"Set up job","status":"completed","conclusion":"success"},{"name":"Warnings","status":"completed","conclusion":"failure"},{"name":"Tests","status":"completed","conclusion":"skipped"}]},
            {"id":2,"name":"linux","status":"in_progress","conclusion":null,"html_url":"https://github.com/o/r/actions/runs/9/job/2",
             "steps":[{"name":"Build","status":"completed","conclusion":"success"},{"name":"Tests","status":"in_progress","conclusion":null},{"name":"Upload","status":"queued","conclusion":null}]},
            {"id":3,"name":"mac","status":"completed","conclusion":"timed_out","html_url":"https://github.com/o/r/actions/runs/9/job/3"},
            {"id":4,"name":"windows","status":"queued","conclusion":null,"html_url":"https://github.com/o/r/actions/runs/9/job/4"}
        ]}"#;
        let jobs = jobs(json).unwrap();
        assert_eq!(
            jobs.iter()
                .map(|job| (job.id, job.name.as_str(), job.state))
                .collect::<Vec<_>>(),
            vec![
                (1, "test", State::Failed),
                (2, "linux", State::Running),
                (3, "mac", State::Failed),
                (4, "windows", State::Waiting)
            ]
        );
        assert_eq!(
            jobs[1]
                .steps
                .iter()
                .map(|step| step.state)
                .collect::<Vec<_>>(),
            vec![State::Passed, State::Running, State::Waiting]
        );
        assert_eq!(jobs[1].current().unwrap().name, "Tests");
        assert_eq!(jobs[0].steps[2].state, State::Skipped);
        assert_eq!(
            jobs.iter()
                .filter_map(|job| job.failure().map(|failure| failure.title()))
                .collect::<Vec<_>>(),
            vec!["test › Warnings".to_string(), "mac".to_string()]
        );
        assert_eq!(
            jobs[0].failure().unwrap().url,
            "https://github.com/o/r/actions/runs/9/job/1"
        );
    }
}
