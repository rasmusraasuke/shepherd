use std::ops::Range;

/// Above this many pairs of differing words the words of a line are not matched one by one,
/// to keep very long lines quick; everything between the common start and end is then changed.
const MAX_PAIRS: usize = 250_000;

pub type Ranges = Vec<Range<usize>>;

/// The byte ranges that changed between two versions of a line, in the old and in the new one.
/// Lines with less than half of their words in common get none: marking nearly all of a line
/// only gets in the way.
pub fn changed_ranges(old: &str, new: &str) -> Option<(Ranges, Ranges)> {
    let (a, b) = (words(old), words(new));
    let (a_kept, b_kept) = common(&a, &b, old, new);
    let kept_size: usize = a.iter().zip(&a_kept).filter(|(word, kept)| **kept && !is_space(&old[(*word).clone()])).map(|(word, _)| word.len()).sum();
    let size = |text: &str| text.chars().filter(|c| !c.is_whitespace()).map(char::len_utf8).sum::<usize>();
    if kept_size * 2 < size(old).max(size(new)) {
        return None;
    }
    Some((changed(&a, a_kept, old), changed(&b, b_kept, new)))
}

/// Splits a line into runs of letters and digits, runs of spaces, and single other characters.
fn words(text: &str) -> Vec<Range<usize>> {
    let class = |c: char| if c.is_alphanumeric() || c == '_' { 1 } else if c.is_whitespace() { 2 } else { 0 };
    let mut words: Vec<Range<usize>> = Vec::new();
    let mut previous = None;
    for (at, c) in text.char_indices() {
        let end = at + c.len_utf8();
        match words.last_mut() {
            Some(word) if previous == Some(class(c)) && class(c) != 0 => word.end = end,
            _ => words.push(at..end),
        }
        previous = Some(class(c));
    }
    words
}

fn is_space(word: &str) -> bool {
    word.chars().all(char::is_whitespace)
}

/// Which words of each line are kept in the other: a longest common sequence of them.
fn common(a: &[Range<usize>], b: &[Range<usize>], old: &str, new: &str) -> (Vec<bool>, Vec<bool>) {
    let same = |i: usize, j: usize| old[a[i].clone()] == new[b[j].clone()];
    let mut start = 0;
    while start < a.len().min(b.len()) && same(start, start) {
        start += 1;
    }
    let mut end = 0;
    while end < a.len().min(b.len()) - start && same(a.len() - 1 - end, b.len() - 1 - end) {
        end += 1;
    }
    let (mut a_kept, mut b_kept) = (vec![false; a.len()], vec![false; b.len()]);
    for i in (0..start).chain(a.len() - end..a.len()) {
        a_kept[i] = true;
    }
    for j in (0..start).chain(b.len() - end..b.len()) {
        b_kept[j] = true;
    }
    let (n, m) = (a.len() - start - end, b.len() - start - end);
    if n * m > MAX_PAIRS {
        return (a_kept, b_kept);
    }
    // lengths[i][j]: the longest common sequence of the middle words from i and from j on
    let mut lengths = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lengths[i][j] = if same(start + i, start + j) { lengths[i + 1][j + 1] + 1 } else { lengths[i + 1][j].max(lengths[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if same(start + i, start + j) {
            a_kept[start + i] = true;
            b_kept[start + j] = true;
            (i, j) = (i + 1, j + 1);
        } else if lengths[i + 1][j] >= lengths[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    (a_kept, b_kept)
}

/// The ranges of the words not kept, joined where only spaces are between them.
fn changed(words: &[Range<usize>], mut kept: Vec<bool>, text: &str) -> Ranges {
    for i in 1..words.len().saturating_sub(1) {
        if is_space(&text[words[i].clone()]) && !kept[i - 1] && !kept[i + 1] {
            kept[i] = false;
        }
    }
    let mut ranges = Ranges::new();
    for (word, kept) in words.iter().zip(kept) {
        if kept {
            continue;
        }
        match ranges.last_mut() {
            Some(last) if last.end == word.start => last.end = word.end,
            _ => ranges.push(word.clone()),
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The changed parts of each line.
    fn changes(old: &str, new: &str) -> Option<(Vec<String>, Vec<String>)> {
        let (a, b) = changed_ranges(old, new)?;
        Some((a.into_iter().map(|r| old[r].to_string()).collect(), b.into_iter().map(|r| new[r].to_string()).collect()))
    }

    #[test]
    fn finds_the_words_that_changed() {
        assert_eq!(changes("val total = price * count", "val total = price * amount"), Some((vec!["count".into()], vec!["amount".into()])));
        assert_eq!(changes("call(a)", "call(a, b)"), Some((vec![], vec![", b".into()])));
        assert_eq!(changes("    return x", "        return x"), Some((vec!["    ".into()], vec!["        ".into()])));
        assert_eq!(changes("fn öäü(x: u8) {}", "fn öäü(y: u8) {}"), Some((vec!["x".into()], vec!["y".into()])));
    }

    #[test]
    fn joins_changed_words_with_the_spaces_between_them() {
        assert_eq!(changes("let a = one two three;", "let a = four five three;"), Some((vec!["one two".into()], vec!["four five".into()])));
    }

    #[test]
    fn leaves_out_lines_with_too_little_in_common() {
        assert_eq!(changes("val a = 1", "println(\"something else\")"), None);
    }

    #[test]
    fn handles_long_lines_by_their_common_start_and_end() {
        let old = format!("start {} end", "a ".repeat(1000));
        let new = format!("start {} end", "b ".repeat(1000));
        assert_eq!(changes(&old, &new), None);
        let new = old.replacen("a ", "x ", 1);
        assert_eq!(changes(&old, &new), Some((vec!["a".into()], vec!["x".into()])));
    }
}
