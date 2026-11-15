use crate::git::{DiffLine, LineKind};

#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub path: String,
    pub lines: Vec<DiffLine>,
    at: u32,
    pub text: String,
    pub sent: bool,
}

fn committed_before(file: &[DiffLine]) -> Vec<u32> {
    let mut before = 0;
    file.iter()
        .map(|line| {
            let at = before;
            before += line.old_no.is_some() as u32;
            at
        })
        .collect()
}

impl Comment {
    pub fn new(path: &str, lines: Vec<DiffLine>, file: &[DiffLine], text: &str) -> Self {
        let start = file.windows(lines.len()).position(|window| window == lines).unwrap_or(0);
        let at = committed_before(file).get(start).copied().unwrap_or(0);
        Self { path: path.to_string(), lines, at, text: text.trim().to_string(), sent: false }
    }

    pub fn follow(&mut self, file: &[DiffLine]) -> bool {
        let same = |a: &DiffLine, b: &DiffLine| a.kind == b.kind && a.text == b.text;
        let before = committed_before(file);
        let was = self.lines[0].number();
        let found = (0..(file.len() + 1).saturating_sub(self.lines.len()))
            .filter(|&start| before[start] == self.at && file[start..].iter().zip(&self.lines).all(|(a, b)| same(a, b)))
            .min_by_key(|&start| file[start].number().abs_diff(was));
        match found {
            Some(start) => {
                self.lines = file[start..start + self.lines.len()].to_vec();
                true
            }
            None => false,
        }
    }

    pub fn last_line(&self) -> &DiffLine {
        self.lines.last().expect("a comment is on at least one line")
    }
}

pub fn quote(path: &str, lines: &[DiffLine], text: &str) -> String {
    let range = |numbers: Vec<u32>| match (numbers.iter().min(), numbers.iter().max()) {
        (Some(first), Some(last)) if first == last => format!("line {first}"),
        (Some(first), Some(last)) => format!("lines {first}-{last}"),
        _ => String::new(),
    };
    let new = range(lines.iter().filter_map(|line| line.new_no).collect());
    let place = if new.is_empty() {
        format!("@{path}, removed {} of the committed version", range(lines.iter().filter_map(|line| line.old_no).collect()))
    } else {
        format!("@{path}, {new}")
    };
    let mut out = format!("{place}\n```diff\n");
    for line in lines {
        let sign = match line.kind {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
        };
        out.push_str(&format!("{sign}{}\n", line.text));
    }
    out.push_str("```\n");
    out.push_str(text.trim());
    out
}

pub fn feedback<'a>(comments: impl IntoIterator<Item = &'a Comment>) -> String {
    let quotes: Vec<String> = comments.into_iter().map(|comment| quote(&comment.path, &comment.lines, &comment.text)).collect();
    format!("Please address these review comments on the uncommitted changes:\n\n{}", quotes.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: LineKind, old_no: Option<u32>, new_no: Option<u32>, text: &str) -> DiffLine {
        DiffLine { kind, old_no, new_no, text: text.to_string() }
    }

    fn context(number: u32, text: &str) -> DiffLine {
        line(LineKind::Context, Some(number), Some(number), text)
    }

    #[test]
    fn follows_its_lines_and_is_gone_when_they_change() {
        let file = [context(1, "a"), context(2, "b"), context(3, "x"), context(4, "a"), context(5, "b")];
        let mut comment = Comment::new("f", file[3..5].to_vec(), &file, " why? ");
        assert_eq!(comment.text, "why?");
        let added = [line(LineKind::Added, None, Some(1), "new"), line(LineKind::Added, None, Some(2), "a"), line(LineKind::Added, None, Some(3), "b")];
        let moved: Vec<DiffLine> = added.into_iter().chain(file.iter().map(|line| DiffLine { new_no: line.new_no.map(|n| n + 3), ..line.clone() })).collect();
        assert!(comment.follow(&moved));
        assert_eq!(comment.last_line().new_no, Some(8));

        let mut changed = moved[..7].to_vec();
        changed.extend([line(LineKind::Removed, Some(5), None, "b"), line(LineKind::Added, None, Some(8), "c")]);
        assert!(!comment.follow(&changed));
    }

    #[test]
    fn follows_added_lines() {
        let file = [line(LineKind::Added, None, Some(1), "a"), line(LineKind::Added, None, Some(2), "b"), line(LineKind::Added, None, Some(3), "a")];
        let mut comment = Comment::new("f", file[2..3].to_vec(), &file, "why?");
        let more = ["a", "new", "b", "a"].iter().enumerate().map(|(i, text)| line(LineKind::Added, None, Some(i as u32 + 1), text)).collect::<Vec<_>>();
        assert!(comment.follow(&more));
        assert_eq!(comment.last_line().new_no, Some(4));
    }

    #[test]
    fn quotes_the_lines_with_where_they_are() {
        let lines = [context(3, "a"), line(LineKind::Removed, Some(4), None, "b"), line(LineKind::Added, None, Some(4), "c")];
        assert_eq!(quote("src/f.rs", &lines, " Use d \n"), "@src/f.rs, lines 3-4\n```diff\n a\n-b\n+c\n```\nUse d");
        assert_eq!(quote("f", &lines[1..2], "Keep it"), "@f, removed line 4 of the committed version\n```diff\n-b\n```\nKeep it");
    }
}
