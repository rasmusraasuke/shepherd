use std::collections::{HashMap, HashSet};

use crate::git::LineKind;

use super::Review;
use super::text::read_lines;

const CONTEXT: usize = 3;

impl Review {
    pub fn hunks_text(&self) -> String {
        if self.files.is_empty() {
            return "No uncommitted changes".to_string();
        }
        let mut out = String::new();
        for file in &self.files {
            let diff = &file.diff;
            out.push_str(&format!("=== {}", diff.display_name()));
            if diff.hunks.is_empty() {
                out.push_str(" (no lines, staged only as a whole)");
            }
            out.push('\n');
            let content = read_lines(&self.repo.root, diff);
            let context = |out: &mut String, from: usize, to: usize| {
                let to = to.min(content.len());
                for line in content.get(from..to).unwrap_or_default() {
                    out.push_str(&format!("{:>4}  {line}\n", ""));
                }
            };
            let mut shown = 0;
            for (number, hunk) in diff.hunks.iter().enumerate() {
                let new = hunk.ranges().1;
                let (start, end) = (new.start as usize - 1, new.end as usize - 1);
                out.push_str(&format!(
                    "--- hunk {} @@ -{},{} +{},{} @@\n",
                    number + 1,
                    hunk.old_start,
                    hunk.old_count,
                    hunk.new_start,
                    hunk.new_count
                ));
                context(&mut out, start.saturating_sub(CONTEXT).max(shown), start);
                for (number, line) in hunk.lines.iter().enumerate() {
                    let sign = match line.kind {
                        LineKind::Context => ' ',
                        LineKind::Added => '+',
                        LineKind::Removed => '-',
                    };
                    out.push_str(&format!("{:>4} {sign}{}\n", number + 1, line.text));
                }
                let next = diff
                    .hunks
                    .get(number + 1)
                    .map_or(usize::MAX, |next| next.ranges().1.start as usize - 1);
                shown = (end + CONTEXT).min(next);
                context(&mut out, end, shown);
            }
        }
        out
    }

    pub fn stage_only(&mut self, places: &[String]) -> Result<String, String> {
        if self.committed() {
            return Err("Only uncommitted changes can be staged".to_string());
        }
        self.wait_for_staging();
        let mut chosen: HashMap<usize, Option<HashSet<(usize, usize)>>> = HashMap::new();
        for place in places {
            let (path, picks) = parse_picks(place);
            let Some(&file) = self.by_path.get(path) else {
                return Err(format!("No changes in {path}"));
            };
            let hunks = &self.files[file].diff.hunks;
            let lines = match picks {
                None => None,
                Some(picks) => {
                    let mut lines = HashSet::new();
                    for (hunk, range) in picks {
                        let Some(found) = hunk.checked_sub(1).and_then(|h| hunks.get(h)) else {
                            return Err(format!(
                                "{path} has no hunk {hunk}, it has {}",
                                hunks.len()
                            ));
                        };
                        let count = found.lines.len();
                        let (first, last) = range.unwrap_or((1, count));
                        if first == 0 || first > last || last > count {
                            return Err(format!(
                                "Hunk {hunk} of {path} has no lines {first}-{last}, it has {count}"
                            ));
                        }
                        lines.extend((first - 1..last).map(|line| (hunk - 1, line)));
                    }
                    Some(lines)
                }
            };
            match (chosen.get_mut(&file), lines) {
                (Some(Some(all)), Some(more)) => all.extend(more),
                (Some(all), None) => *all = None,
                (None, lines) => drop(chosen.insert(file, lines)),
                (Some(None), Some(_)) => {}
            }
        }
        let mut errors = Vec::new();
        for file in 0..self.files.len() {
            let wanted = chosen.get(&file);
            let wants = |h: usize, i: usize| {
                wanted
                    .is_some_and(|lines| lines.as_ref().is_none_or(|lines| lines.contains(&(h, i))))
            };
            let review = &self.files[file];
            let path = review.diff.path.clone();
            if self.has_conflicts(file) {
                if wanted.is_some() {
                    errors.push(format!("{path} has conflicts to resolve first"));
                }
                continue;
            }
            if review.diff.hunks.is_empty() {
                if review.file_staged != wanted.is_some() {
                    self.stage_file(file, wanted.is_some());
                }
            } else {
                let same = review
                    .diff
                    .hunks
                    .iter()
                    .zip(&review.staged)
                    .enumerate()
                    .all(|(h, (hunk, staged))| {
                        hunk.lines
                            .iter()
                            .zip(staged)
                            .enumerate()
                            .all(|(i, (line, &staged))| {
                                line.kind == LineKind::Context || staged == wants(h, i)
                            })
                    });
                if !same {
                    self.stage(file, wants);
                }
            }
            errors.extend(self.error.take().map(|error| format!("{path}: {error}")));
        }
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        let (done, total) = self.progress();
        Ok(format!(
            "Staged {done} of {total} changed lines, the rest is not staged"
        ))
    }
}

type Pick = (usize, Option<(usize, usize)>);

fn parse_picks(place: &str) -> (&str, Option<Vec<Pick>>) {
    let Some((path, picks)) = place.rsplit_once(':') else {
        return (place, None);
    };
    let pick = |pick: &str| -> Option<Pick> {
        let Some((hunk, lines)) = pick.split_once('.') else {
            return Some((pick.parse().ok()?, None));
        };
        let (first, last) = lines.split_once('-').unwrap_or((lines, lines));
        Some((
            hunk.parse().ok()?,
            Some((first.parse().ok()?, last.parse().ok()?)),
        ))
    };
    match picks.split(',').map(|p| pick(p.trim())).collect() {
        Some(picks) => (path, Some(picks)),
        None => (place, None),
    }
}
