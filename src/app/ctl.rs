use eframe::egui;

use super::workspace::Tab;
use super::{Action, App};
use crate::editor::buffer::Pos;
use crate::panels::search::Reveal;
use crate::review::comments;
use crate::terminal::Activity;

const CONTROL_HELP: &str = "\
shepherd ctl: ask the editor what the user sees, or have it show something.

  shepherd ctl context              All of the below in one go
  shepherd ctl selection            The place and the text selected in the file edited last
  shepherd ctl comments             The user's review comments on the uncommitted changes
  shepherd ctl review-state         Which changed lines are reviewed, by file; reviewed means staged
  shepherd ctl terminals            The terminals and what their programs are doing
  shepherd ctl terminal <id>        What a terminal shows
  shepherd ctl open <path>[:<line>[-<line>]]
                                  Open a file, selecting the lines
  shepherd ctl review [<path>]      Show the changes of a file for review, or of the next file to review
  shepherd ctl commit-message <message>
                                  Put a commit message into the editor, for the user to commit with";

pub(super) fn parse_place(place: &str) -> (&str, Option<(usize, usize)>) {
    let Some((path, lines)) = place.rsplit_once(':') else {
        return (place, None);
    };
    let (first, last) = lines.split_once('-').unwrap_or((lines, lines));
    match (first.parse(), last.parse()) {
        (Ok(first), Ok(last)) => (path, Some((first, last))),
        _ => (place, None),
    }
}

impl App {
    pub(super) fn answer(
        &mut self,
        args: &[String],
        ctx: &egui::Context,
    ) -> Result<String, String> {
        let arg = |index: usize| args.get(index).map(String::as_str);
        let content = &self.content;
        match (arg(0), arg(1)) {
            (None | Some("help"), _) => Ok(CONTROL_HELP.to_string()),
            (Some("context"), None) => Ok(self.context()),
            (Some("selection"), None) => Ok(self.selection()),
            (Some("comments"), None) => match content.review.comments() {
                [] => Ok("No review comments".to_string()),
                all => Ok(comments::feedback(all)),
            },
            (Some("review-state"), None) => Ok(self.review_state()),
            (Some("terminals"), None) => Ok(self.terminals()),
            (Some("terminal"), Some(id)) => id
                .parse()
                .ok()
                .and_then(|id| content.terminals.screen(id))
                .ok_or_else(|| format!("No terminal {id}")),
            (Some("open"), Some(place)) => {
                let (path, lines) = parse_place(place);
                if !content.files.iter().any(|file| file == path) {
                    return Err(format!("No file {path} in the project"));
                }
                let (start, end) = match lines {
                    Some((first, last)) => (
                        Pos::new(first.saturating_sub(1), 0),
                        Pos::new(last.saturating_sub(1), usize::MAX),
                    ),
                    None => (Pos::default(), Pos::default()),
                };
                self.perform(
                    Action::Reveal(Reveal {
                        path: path.to_string(),
                        start,
                        end,
                        focus: false,
                    }),
                    ctx,
                );
                Ok(format!("Opened {place}"))
            }
            (Some("agent-session"), Some(id)) => {
                let session = args
                    .get(2)
                    .and_then(|hook| serde_json::from_str::<serde_json::Value>(hook).ok())
                    .and_then(|hook| hook["session_id"].as_str().map(str::to_string))
                    .ok_or("No session_id in the hook input")?;
                let found = id
                    .parse()
                    .is_ok_and(|id| self.workspace.set_session(id, session.clone()));
                if found {
                    Ok(format!("Agent {id} continues in session {session}"))
                } else {
                    Err(format!("No agent {id}"))
                }
            }
            (Some("commit-message"), Some(_)) => {
                self.content.flow.set_message(args[1..].join(" "));
                Ok("The commit message is in the editor, for the user to commit with".to_string())
            }
            (Some("review"), path) => {
                let Some(path) = path
                    .map(str::to_string)
                    .or_else(|| content.review.first_unreviewed())
                else {
                    return Ok("Everything is reviewed".to_string());
                };
                if !content
                    .review
                    .files_progress()
                    .iter()
                    .any(|(file, _)| *file == path)
                {
                    return Err(format!("No changes in {path}"));
                }
                self.perform(
                    Action::Review {
                        path: path.clone(),
                        new_tab: false,
                    },
                    ctx,
                );
                Ok(format!("Showing the review of {path}"))
            }
            _ => Err(format!(
                "Unknown command: {}\n\n{CONTROL_HELP}",
                args.join(" ")
            )),
        }
    }

    fn context(&self) -> String {
        let tabs: Vec<String> = self
            .workspace
            .windows
            .iter()
            .flat_map(|window| window.tabs())
            .map(|tab| match tab {
                Tab::Review(path) => {
                    format!("- review of {}", path.as_deref().unwrap_or("no file yet"))
                }
                Tab::CommitReview { path, label, .. } => format!(
                    "- review of {} in commit {label}",
                    path.as_deref().unwrap_or("no file yet")
                ),
                Tab::File(path) => format!("- file {path}"),
                Tab::Terminal(id) => format!("- terminal {id}"),
                Tab::Agent { id, agent, .. } => format!("- agent {id}: {}", agent.name()),
            })
            .collect();
        let comments = self.content.review.comments().len();
        [
            format!(
                "Repository: {} (branch {})",
                self.content.root.display(),
                self.repo.branch()
            ),
            format!("Open tabs:\n{}", tabs.join("\n")),
            format!("Selection: {}", self.selection()),
            self.review_state(),
            format!(
                "Review comments: {comments}{}",
                if comments > 0 {
                    " (`shepherd ctl comments` shows them)"
                } else {
                    ""
                }
            ),
            self.terminals(),
            self.agents_state(),
        ]
        .join("\n\n")
    }

    fn selection(&self) -> String {
        match self.content.open_files.selection() {
            None => "nothing, no file was edited yet".to_string(),
            Some(selection) => {
                let place = |pos: Pos| format!("{}:{}", pos.line + 1, pos.col + 1);
                let mut out = format!("{}:{}", selection.path, place(selection.start));
                if selection.end != selection.start {
                    out.push_str(&format!(" to {}\n{}", place(selection.end), selection.text));
                }
                out
            }
        }
    }

    fn review_state(&self) -> String {
        let review = &self.content.review;
        let (done, total) = review.progress();
        let files: Vec<String> = review
            .files_progress()
            .into_iter()
            .map(|(path, (done, total))| format!("- {path}: {done}/{total}"))
            .collect();
        let files = if files.is_empty() {
            "No uncommitted changes".to_string()
        } else {
            files.join("\n")
        };
        format!(
            "Review: {done}/{total} changed lines reviewed. Reviewed lines are staged in git, the others are not.\n{files}"
        )
    }

    fn state_of(&self, id: u64, title: String) -> String {
        let activity = match self.content.terminals.activity(id) {
            Some(Activity::Working) => "working",
            Some(Activity::Waiting) => "stopped, waiting",
            Some(Activity::Idle) => "idle",
            Some(Activity::Ended) => "ended",
            None => "not started",
        };
        format!("{title} ({activity})")
    }

    fn tabs_of(&self, agent: bool) -> Vec<(u64, String)> {
        self.workspace
            .windows
            .iter()
            .flat_map(|window| window.tabs())
            .filter_map(|tab| match tab {
                Tab::Agent {
                    id, agent: held, ..
                } if agent => Some((*id, held.name().to_string())),
                Tab::Terminal(id) if !agent => Some((*id, "Terminal".to_string())),
                _ => None,
            })
            .collect()
    }

    fn terminals(&self) -> String {
        let lines: Vec<String> = self
            .tabs_of(false)
            .into_iter()
            .map(|(id, fallback)| {
                let title = self.content.terminals.title(id).unwrap_or(fallback);
                format!("- terminal {id}: {}", self.state_of(id, title))
            })
            .collect();
        if lines.is_empty() {
            "Terminals: none".to_string()
        } else {
            format!(
                "Terminals (`shepherd ctl terminal <id>` shows one):\n{}",
                lines.join("\n")
            )
        }
    }

    fn agents_state(&self) -> String {
        let lines: Vec<String> = self
            .tabs_of(true)
            .into_iter()
            .map(|(id, fallback)| {
                let title = self.content.terminals.title(id).unwrap_or(fallback);
                format!("- agent {id}: {}", self.state_of(id, title))
            })
            .collect();
        if lines.is_empty() {
            "Agents: none".to_string()
        } else {
            format!(
                "Agents (`shepherd ctl terminal <id>` shows one):\n{}",
                lines.join("\n")
            )
        }
    }
}
