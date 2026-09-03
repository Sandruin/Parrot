use std::ops::Range;

use anyhow::{Context, Result};
use regex::{Regex, RegexBuilder};

use crate::model::{Rect, TextMatch};
use crate::platform::{OcrLine, OcrWord};

/// The search text of a `WaitForText` or `ClickOnText` action, compiled once before polling starts.
#[derive(Debug)]
pub enum TextNeedle {
    Contains { needle: String, case_sensitive: bool },
    Regex(Regex),
}

impl TextNeedle {
    /// Compiles `text` for `match_mode`; an invalid regular expression is reported as an error.
    pub fn new(text: &str, match_mode: TextMatch, case_sensitive: bool) -> Result<Self> {
        match match_mode {
            TextMatch::Contains => {
                Ok(TextNeedle::Contains { needle: normalize(text, case_sensitive), case_sensitive })
            }
            TextMatch::Regex => RegexBuilder::new(text)
                .case_insensitive(!case_sensitive)
                .build()
                .map(TextNeedle::Regex)
                .with_context(|| format!("invalid regular expression {text:?}")),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            TextNeedle::Contains { needle, .. } => needle.is_empty(),
            TextNeedle::Regex(_) => false,
        }
    }

    /// Whether the recognised line keeps its case; a regex folds case through its own flag instead.
    fn keeps_case(&self) -> bool {
        match self {
            TextNeedle::Contains { case_sensitive, .. } => *case_sensitive,
            TextNeedle::Regex(_) => true,
        }
    }
}

/// Finds `needle` inside the recognised lines and returns the union of the word boxes it covers,
/// in the pixel coordinates of the analysed image. Prefers the earliest line, then the earliest match.
pub fn find_text(lines: &[OcrLine], needle: &TextNeedle) -> Option<Rect> {
    if needle.is_empty() {
        return None;
    }
    lines.iter().find_map(|line| find_in_line(line, needle))
}

fn find_in_line(line: &OcrLine, needle: &TextNeedle) -> Option<Rect> {
    let keeps_case = needle.keeps_case();
    let haystack = normalize(&line.text, keeps_case);
    let spans = word_spans(&haystack, &line.words, keeps_case);
    match needle {
        TextNeedle::Contains { needle, .. } => {
            let mut from = 0;
            while let Some(offset) = haystack[from..].find(needle.as_str()) {
                let start = from + offset;
                if let Some(rect) = covered_box(&spans, start..start + needle.len()) {
                    return Some(rect);
                }
                from = start + haystack[start..].chars().next().map_or(1, char::len_utf8);
            }
            None
        }
        TextNeedle::Regex(re) => re.find_iter(&haystack).find_map(|m| covered_box(&spans, m.range())),
    }
}

/// Union of the word boxes overlapping `range`, skipping boxes without area.
fn covered_box(spans: &[(Range<usize>, Rect)], range: Range<usize>) -> Option<Rect> {
    spans
        .iter()
        .filter(|(span, _)| span.start < range.end && span.end > range.start)
        .map(|(_, rect)| *rect)
        .filter(|rect| rect.w > 0 && rect.h > 0)
        .reduce(union)
}

/// Byte ranges of `words` inside `haystack`, resolved in order so a repeated word keeps its place.
fn word_spans(haystack: &str, words: &[OcrWord], case_sensitive: bool) -> Vec<(Range<usize>, Rect)> {
    let mut spans = Vec::with_capacity(words.len());
    let mut cursor = 0;
    for word in words {
        let text = normalize(&word.text, case_sensitive);
        if text.is_empty() {
            continue;
        }
        let Some(offset) = haystack[cursor..].find(&text) else {
            continue;
        };
        let start = cursor + offset;
        cursor = start + text.len();
        spans.push((start..cursor, word.rect));
    }
    spans
}

/// Collapses whitespace runs into single spaces, trims, and case folds unless `case_sensitive`.
fn normalize(text: &str, case_sensitive: bool) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if case_sensitive { joined } else { joined.to_lowercase() }
}

fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect::new(x, y, a.right().max(b.right()) - x, a.bottom().max(b.bottom()) - y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, words: &[(&str, Rect)]) -> OcrLine {
        OcrLine {
            text: text.into(),
            words: words.iter().map(|(t, rect)| OcrWord { text: (*t).into(), rect: *rect }).collect(),
        }
    }

    fn contains(text: &str, case_sensitive: bool) -> TextNeedle {
        TextNeedle::new(text, TextMatch::Contains, case_sensitive).unwrap()
    }

    fn regex(text: &str, case_sensitive: bool) -> TextNeedle {
        TextNeedle::new(text, TextMatch::Regex, case_sensitive).unwrap()
    }

    fn save_as() -> Vec<OcrLine> {
        vec![
            line("File Edit", &[("File", Rect::new(0, 0, 20, 10)), ("Edit", Rect::new(24, 0, 20, 10))]),
            line("Save As", &[("Save", Rect::new(0, 20, 30, 12)), ("As", Rect::new(36, 20, 14, 12))]),
        ]
    }

    fn progress() -> Vec<OcrLine> {
        vec![
            line(
                "Copying files",
                &[("Copying", Rect::new(0, 0, 34, 10)), ("files", Rect::new(38, 0, 22, 10))],
            ),
            line(
                "Done 1024 items",
                &[
                    ("Done", Rect::new(0, 20, 20, 10)),
                    ("1024", Rect::new(24, 20, 22, 10)),
                    ("items", Rect::new(50, 20, 24, 10)),
                ],
            ),
        ]
    }

    #[test]
    fn a_single_word_returns_its_own_box() {
        assert_eq!(find_text(&save_as(), &contains("Edit", true)), Some(Rect::new(24, 0, 20, 10)));
    }

    #[test]
    fn a_phrase_unions_the_boxes_of_both_words() {
        assert_eq!(find_text(&save_as(), &contains("Save As", true)), Some(Rect::new(0, 20, 50, 12)));
    }

    #[test]
    fn matching_folds_case_unless_asked_otherwise() {
        assert_eq!(find_text(&save_as(), &contains("sAvE aS", false)), Some(Rect::new(0, 20, 50, 12)));
        assert_eq!(find_text(&save_as(), &contains("sAvE aS", true)), None);
    }

    #[test]
    fn a_missing_or_empty_needle_returns_none() {
        assert_eq!(find_text(&save_as(), &contains("Print", false)), None);
        assert_eq!(find_text(&save_as(), &contains("", false)), None);
        assert_eq!(find_text(&save_as(), &contains("   ", false)), None);
        assert_eq!(find_text(&[], &contains("Save", false)), None);
    }

    #[test]
    fn a_partial_span_over_two_words_unions_both() {
        assert_eq!(find_text(&save_as(), &contains("ve A", true)), Some(Rect::new(0, 20, 50, 12)));
        assert_eq!(find_text(&save_as(), &contains("av", true)), Some(Rect::new(0, 20, 30, 12)));
    }

    #[test]
    fn irregular_spacing_in_the_line_is_normalised() {
        let lines = vec![line(
            "  Open \t recent   file \n",
            &[
                ("Open", Rect::new(2, 4, 18, 9)),
                ("recent", Rect::new(24, 4, 26, 9)),
                ("file", Rect::new(54, 4, 16, 9)),
            ],
        )];
        assert_eq!(find_text(&lines, &contains("open recent", false)), Some(Rect::new(2, 4, 48, 9)));
        assert_eq!(find_text(&lines, &contains("recent   file", false)), Some(Rect::new(24, 4, 46, 9)));
    }

    #[test]
    fn the_earliest_line_and_match_win() {
        let lines = vec![
            line("go go", &[("go", Rect::new(0, 0, 10, 8)), ("go", Rect::new(14, 0, 10, 8))]),
            line("go", &[("go", Rect::new(0, 20, 10, 8))]),
        ];
        assert_eq!(find_text(&lines, &contains("go", false)), Some(Rect::new(0, 0, 10, 8)));
        assert_eq!(find_text(&lines, &regex("go", false)), Some(Rect::new(0, 0, 10, 8)));
    }

    #[test]
    fn a_line_without_usable_boxes_is_skipped() {
        let lines = vec![
            line("ok", &[("ok", Rect::new(5, 5, 0, 0))]),
            line("ok", &[("ok", Rect::new(9, 30, 12, 8))]),
        ];
        assert_eq!(find_text(&lines, &contains("ok", false)), Some(Rect::new(9, 30, 12, 8)));
        assert_eq!(find_text(&lines, &regex("ok", false)), Some(Rect::new(9, 30, 12, 8)));
    }

    #[test]
    fn a_regex_finds_a_number_pattern() {
        assert_eq!(find_text(&progress(), &regex(r"\d{3,}", true)), Some(Rect::new(24, 20, 22, 10)));
        assert_eq!(find_text(&progress(), &regex(r"\d{5,}", true)), None);
    }

    #[test]
    fn a_regex_spanning_two_words_unions_both_boxes() {
        assert_eq!(find_text(&progress(), &regex(r"\d+ items", true)), Some(Rect::new(24, 20, 50, 10)));
    }

    #[test]
    fn regex_anchors_apply_to_the_normalised_line() {
        assert_eq!(find_text(&progress(), &regex("^Done", true)), Some(Rect::new(0, 20, 20, 10)));
        assert_eq!(find_text(&progress(), &regex("items$", true)), Some(Rect::new(50, 20, 24, 10)));
        assert_eq!(find_text(&progress(), &regex("^items", true)), None);
        assert_eq!(find_text(&progress(), &regex("Done$", true)), None);
    }

    #[test]
    fn a_regex_folds_case_unless_asked_otherwise() {
        assert_eq!(find_text(&progress(), &regex("COPYING", false)), Some(Rect::new(0, 0, 34, 10)));
        assert_eq!(find_text(&progress(), &regex("COPYING", true)), None);
        assert_eq!(find_text(&progress(), &regex("(?i)COPYING", true)), Some(Rect::new(0, 0, 34, 10)));
    }

    #[test]
    fn an_invalid_regex_reports_the_pattern_and_the_regex_message() {
        let error = TextNeedle::new("(unclosed", TextMatch::Regex, false).unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains("(unclosed"), "{text}");
        assert!(text.contains("unclosed group"), "{text}");
        assert!(TextNeedle::new("(unclosed", TextMatch::Contains, false).is_ok());
    }
}
