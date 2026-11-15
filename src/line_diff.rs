use crate::git::{DiffLine, Hunk, LineKind};

const MAX_DIFFERENCES: usize = 2000;

pub fn diff<A: AsRef<str>, B: AsRef<str>>(old: &[A], new: &[B]) -> Vec<Hunk> {
    let same = |i: usize, j: usize| old[i].as_ref() == new[j].as_ref();
    let mut start = 0;
    while start < old.len().min(new.len()) && same(start, start) {
        start += 1;
    }
    let mut end = 0;
    while end < (old.len() - start).min(new.len() - start)
        && same(old.len() - 1 - end, new.len() - 1 - end)
    {
        end += 1;
    }
    let (n, m) = (old.len() - start - end, new.len() - start - end);
    let mut pairs = matching(n, m, |i, j| same(start + i, start + j)).unwrap_or_default();
    pairs.push((n, m));

    let mut hunks = Vec::new();
    let (mut i, mut j) = (0, 0);
    for (next_i, next_j) in pairs {
        if next_i > i || next_j > j {
            let (i0, j0) = (start + i, start + j);
            let mut lines: Vec<DiffLine> = (i0..start + next_i)
                .map(|at| DiffLine {
                    kind: LineKind::Removed,
                    old_no: Some(at as u32 + 1),
                    new_no: None,
                    text: old[at].as_ref().to_string(),
                })
                .collect();
            lines.extend((j0..start + next_j).map(|at| DiffLine {
                kind: LineKind::Added,
                old_no: None,
                new_no: Some(at as u32 + 1),
                text: new[at].as_ref().to_string(),
            }));
            let first =
                |at: usize, count: usize| if count == 0 { at as u32 } else { at as u32 + 1 };
            hunks.push(Hunk {
                old_start: first(i0, next_i - i),
                new_start: first(j0, next_j - j),
                lines,
            });
        }
        (i, j) = (next_i + 1, next_j + 1);
    }
    hunks
}

fn matching(
    n: usize,
    m: usize,
    same: impl Fn(usize, usize) -> bool,
) -> Option<Vec<(usize, usize)>> {
    let (n, m) = (n as isize, m as isize);
    let max = (n + m).min(MAX_DIFFERENCES as isize);
    let offset = max + 1;
    let mut reach = vec![0isize; 2 * offset as usize + 1];
    let mut trace = Vec::new();
    for d in 0..=max {
        trace.push(reach[(offset - d - 1) as usize..=(offset + d + 1) as usize].to_vec());
        for k in (-d..=d).step_by(2) {
            let at = |k: isize| reach[(k + offset) as usize];
            let mut x = if k == -d || (k != d && at(k - 1) < at(k + 1)) {
                at(k + 1)
            } else {
                at(k - 1) + 1
            };
            let mut y = x - k;
            while x < n && y < m && same(x as usize, y as usize) {
                (x, y) = (x + 1, y + 1);
            }
            reach[(k + offset) as usize] = x;
            if x >= n && y >= m {
                return Some(backtrack(&trace, n, m));
            }
        }
    }
    None
}

fn backtrack(trace: &[Vec<isize>], n: isize, m: isize) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    let (mut x, mut y) = (n, m);
    for (d, before) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let at = |k: isize| before[(k + d + 1) as usize];
        let k = x - y;
        let previous_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let previous_x = at(previous_k);
        let previous_y = previous_x - previous_k;
        while x > previous_x && y > previous_y {
            (x, y) = (x - 1, y - 1);
            pairs.push((x as usize, y as usize));
        }
        (x, y) = (previous_x, previous_y);
    }
    pairs.reverse();
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(hunks: &[Hunk]) -> Vec<String> {
        hunks
            .iter()
            .map(|hunk| {
                let lines: Vec<String> = hunk
                    .lines
                    .iter()
                    .map(|line| {
                        format!(
                            "{}{}",
                            if line.kind == LineKind::Added {
                                "+"
                            } else {
                                "-"
                            },
                            line.text
                        )
                    })
                    .collect();
                format!("{} {} {}", hunk.old_start, hunk.new_start, lines.join(" "))
            })
            .collect()
    }

    #[test]
    fn finds_the_changed_lines_as_git_does() {
        assert!(diff(&["a", "b"], &["a", "b"]).is_empty());
        assert_eq!(
            shown(&diff(&["a", "b", "c", "d"], &["a", "B", "c", "d", "e"])),
            ["2 2 -b +B", "4 5 +e"]
        );
        assert_eq!(
            shown(&diff(&["a", "b", "c"], &["c", "a", "b"])),
            ["0 1 +c", "3 3 -c"]
        );
        assert_eq!(shown(&diff::<&str, &str>(&[], &["x"])), ["0 1 +x"]);
        assert_eq!(shown(&diff::<&str, &str>(&["x", "y"], &[])), ["1 0 -x -y"]);
        assert_eq!(
            shown(&diff(&["a", "x", "b", "y", "c"], &["a", "b", "c"])),
            ["2 1 -x", "4 2 -y"]
        );
    }

    #[test]
    fn numbers_the_lines_of_both_texts() {
        let old: Vec<String> = (0..300).map(|n| format!("{}", n % 7)).collect();
        let new: Vec<String> = (0..280).map(|n| format!("{}", (n * 3) % 11)).collect();
        let hunks = diff(&old, &new);
        let lines = hunks.iter().flat_map(|hunk| &hunk.lines);
        for line in lines {
            match line.kind {
                LineKind::Removed => assert_eq!(old[line.old_no.unwrap() as usize - 1], line.text),
                _ => assert_eq!(new[line.new_no.unwrap() as usize - 1], line.text),
            }
        }
        let numbers = |kind: LineKind| {
            hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .filter(|line| line.kind == kind)
                .filter_map(|line| line.old_no.or(line.new_no))
                .collect::<Vec<u32>>()
        };
        let (removed, added) = (numbers(LineKind::Removed), numbers(LineKind::Added));
        let kept = |lines: &[String], changed: &[u32]| {
            lines
                .iter()
                .enumerate()
                .filter(|(at, _)| !changed.contains(&(*at as u32 + 1)))
                .map(|(_, line)| line.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(kept(&old, &removed), kept(&new, &added));
    }
}
