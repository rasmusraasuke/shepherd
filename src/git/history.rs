use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{Changes, DIFF, EMPTY_TREE, Repo, git};

const LOG: usize = 300;

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct CommitRange {
    pub base: String,
    pub head: String,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub author: String,
    pub time: String,
    pub subject: String,
    pub timestamp: u64,
}

impl Commit {
    pub fn recency(&self, times: &[u64]) -> f32 {
        if times.len() < 2 {
            return 1.0;
        }
        let rank = times.partition_point(|&time| time < self.timestamp);
        rank as f32 / (times.len() - 1) as f32
    }
}

pub fn commit_times(lines: &[Commit]) -> Vec<u64> {
    let mut times: Vec<u64> = lines
        .iter()
        .filter(|commit| !commit.short.is_empty())
        .map(|commit| commit.timestamp)
        .collect();
    times.sort_unstable();
    times.dedup();
    times
}

fn commits(out: &str) -> Vec<Commit> {
    out.lines()
        .filter_map(|line| {
            let mut fields = line.split('\u{1f}');
            Some(Commit {
                hash: fields.next()?.to_string(),
                short: fields.next()?.to_string(),
                author: fields.next()?.to_string(),
                time: fields.next()?.to_string(),
                subject: fields.next()?.to_string(),
                timestamp: 0,
            })
        })
        .collect()
}

impl Repo {
    pub fn log(&self) -> Result<Vec<Commit>, String> {
        if self.base() == EMPTY_TREE {
            return Ok(Vec::new());
        }
        let limit = LOG.to_string();
        git(
            &self.root,
            &[
                "log",
                "--max-count",
                &limit,
                "--format=%H\u{1f}%h\u{1f}%an\u{1f}%ar\u{1f}%s",
            ],
        )
        .map(|out| commits(&out))
    }

    pub fn file_log(&self, path: &str) -> Result<Vec<Commit>, String> {
        if self.base() == EMPTY_TREE {
            return Ok(Vec::new());
        }
        let limit = LOG.to_string();
        git(
            &self.root,
            &[
                "log",
                "--follow",
                "--max-count",
                &limit,
                "--format=%H\u{1f}%h\u{1f}%an\u{1f}%ar\u{1f}%s",
                "--",
                path,
            ],
        )
        .map(|out| commits(&out))
    }

    pub fn commit_range(&self, head: &str) -> Result<CommitRange, String> {
        let parents = git(
            &self.root,
            &["rev-list", "--parents", "--max-count", "1", head],
        )?;
        let base = parents
            .split_whitespace()
            .nth(1)
            .unwrap_or(EMPTY_TREE)
            .to_string();
        Ok(CommitRange {
            base,
            head: head.to_string(),
        })
    }

    pub fn range_changes(&self, range: &CommitRange) -> Result<Changes, String> {
        let span = format!("{}..{}", range.base, range.head);
        let args = [DIFF, &[span.as_str()]].concat();
        let diff = git(&self.root, &args)?;
        Ok(Changes {
            staged: diff.clone(),
            diff,
            ..Changes::default()
        })
    }

    pub fn content_at(&self, revision: &str, path: &str) -> Option<String> {
        git(&self.root, &["show", &format!("{revision}:{path}")]).ok()
    }

    pub fn blame_lines(&self, path: &str) -> Result<Vec<Commit>, String> {
        let out = git(&self.root, &["blame", "--porcelain", "--", path])?;
        let mut known: HashMap<String, Commit> = HashMap::new();
        let mut current = String::new();
        let mut lines: Vec<Option<Commit>> = Vec::new();
        let mut pending = None;
        for line in out.lines() {
            if line.starts_with('\t') {
                if let Some(at) = pending.take()
                    && let Some(commit) = known.get(&current).cloned()
                {
                    if lines.len() <= at {
                        lines.resize(at + 1, None);
                    }
                    lines[at] = Some(commit);
                }
                continue;
            }
            let mut fields = line.split(' ');
            let first = fields.next().unwrap_or_default();
            if first.len() >= 7 && first.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                let Some(final_no) = fields.nth(1).and_then(|n| n.parse::<usize>().ok()) else {
                    continue;
                };
                current = first.to_string();
                pending = Some(final_no - 1);
                if lines.len() < final_no {
                    lines.resize(final_no, None);
                }
                if !known.contains_key(first) {
                    known.insert(
                        first.to_string(),
                        if first.bytes().all(|byte| byte == b'0') {
                            uncommitted()
                        } else {
                            Commit {
                                hash: first.to_string(),
                                short: first[..first.len().min(7)].to_string(),
                                author: String::new(),
                                time: String::new(),
                                subject: String::new(),
                                timestamp: 0,
                            }
                        },
                    );
                }
            } else if let Some((key, value)) = line.split_once(' ')
                && let Some(commit) = known.get_mut(&current)
                && !commit.short.is_empty()
            {
                match key {
                    "author" => commit.author = value.to_string(),
                    "author-time" => {
                        commit.timestamp = value.parse().unwrap_or(0);
                        commit.time = relative_time(commit.timestamp);
                    }
                    "summary" => commit.subject = value.to_string(),
                    _ => {}
                }
            }
        }
        Ok(lines
            .into_iter()
            .map(|line| line.unwrap_or_else(uncommitted))
            .collect())
    }
}

fn uncommitted() -> Commit {
    Commit {
        hash: String::new(),
        short: String::new(),
        author: String::new(),
        time: String::new(),
        subject: String::new(),
        timestamp: 0,
    }
}

fn relative_time(seconds: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let elapsed = now.saturating_sub(seconds);
    let (count, unit) = match elapsed {
        0..=59 => return "just now".to_string(),
        60..=3599 => (elapsed / 60, "minute"),
        3600..=86_399 => (elapsed / 3600, "hour"),
        86_400..=2_591_999 => (elapsed / 86_400, "day"),
        2_592_000..=31_535_999 => (elapsed / 2_592_000, "month"),
        _ => (elapsed / 31_536_000, "year"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{commit_all, temp_repo};

    fn commit(repo: &Repo, dir: &crate::testing::TempDir, path: &str, text: &str, message: &str) {
        dir.write(path, text);
        repo.stage_file(path, true).unwrap();
        repo.commit(message).unwrap();
    }

    #[test]
    fn lists_the_commits_of_the_branch_newest_first() {
        let (repo, dir) = temp_repo("history-log", &[("a.txt", "a\n")]);
        commit(&repo, &dir, "a.txt", "one\n", "first");
        commit(&repo, &dir, "a.txt", "two\n", "second");
        let log = repo.log().unwrap();
        assert_eq!(
            log.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(),
            ["second", "first"]
        );
        assert_eq!(log[0].short.len(), 7);
        assert!(log.iter().all(|c| c.author == "Test"));
    }

    #[test]
    fn shows_the_changes_of_a_commit() {
        let (repo, dir) = temp_repo("history-commit", &[("a.txt", "a\n")]);
        commit(&repo, &dir, "a.txt", "one\n", "first");
        commit(&repo, &dir, "a.txt", "two\n", "second");
        commit(&repo, &dir, "b.txt", "b\n", "third");
        let log = repo.log().unwrap();

        let single = repo.commit_range(&log[0].hash).unwrap();
        let changes = repo.range_changes(&single).unwrap();
        let files = crate::git::parse_diff(&changes.diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "b.txt");
        assert!(changes.staged.contains("b.txt"));
        assert_eq!(repo.content_at(&single.head, "b.txt").unwrap(), "b\n");
    }

    #[test]
    fn recency_ranks_the_commits_of_a_file_from_oldest_to_newest() {
        let commit = |timestamp: u64| Commit {
            hash: "a".repeat(40),
            short: "aaaaaaa".to_string(),
            author: "Test".to_string(),
            time: String::new(),
            subject: String::new(),
            timestamp,
        };
        let lines = [commit(300), commit(100), commit(100_000), commit(300)];
        let times = commit_times(&lines);
        assert_eq!(times, vec![100, 300, 100_000]);
        assert_eq!(lines[1].recency(&times), 0.0);
        assert_eq!(lines[0].recency(&times), 0.5);
        assert_eq!(lines[2].recency(&times), 1.0);
        assert_eq!(commit(5).recency(&[5]), 1.0);
    }

    #[test]
    fn follows_a_file_through_the_commits_that_changed_it() {
        let (repo, dir) = temp_repo("history-file", &[("a.txt", "a\n"), ("b.txt", "b\n")]);
        commit(&repo, &dir, "a.txt", "one\n", "first a");
        commit(&repo, &dir, "b.txt", "one\n", "first b");
        commit(&repo, &dir, "a.txt", "two\n", "second a");
        let log = repo.file_log("a.txt").unwrap();
        assert_eq!(
            log.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(),
            ["second a", "first a"]
        );
    }

    #[test]
    fn blames_every_line_of_a_file() {
        let (repo, dir) = temp_repo("history-blame", &[("a.txt", "one\ntwo\n")]);
        commit_all(&repo);
        commit(&repo, &dir, "a.txt", "one\nchanged\n", "change two");
        let lines = repo.blame_lines("a.txt").unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].subject, "test");
        assert_eq!(lines[0].author, "Test");
        assert!(!lines[0].time.is_empty());
        assert!(lines[0].timestamp > 0);
        assert_eq!(lines[1].subject, "change two");
        assert_eq!(lines[1].author, "Test");
        assert!(!lines[1].time.is_empty());

        dir.write("a.txt", "one\nchanged\nnew\n");
        let lines = repo.blame_lines("a.txt").unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines[2].short.is_empty());
    }
}
