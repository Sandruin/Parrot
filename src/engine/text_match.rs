use std::ops::Range;

use crate::model::Rect;
use crate::platform::{OcrLine, OcrWord};

/// Finds `needle` inside the recognised lines and returns the union of the word boxes it covers,
/// in the pixel coordinates of the analysed image. Prefers the earliest line, then the earliest match.
pub fn find_text(lines: &[OcrLine], needle: &str, case_sensitive: bool) -> Option<Rect> {
    let needle = normalize(needle, case_sensitive);
    if needle.is_empty() {
        return None;
    }
    lines.iter().find_map(|line| find_in_line(line, &needle, case_sensitive))
}

fn find_in_line(line: &OcrLine, needle: &str, case_sensitive: bool) -> Option<Rect> {
    let haystack = normalize(&line.text, case_sensitive);
    let spans = word_spans(&haystack, &line.words, case_sensitive);
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(needle) {
        let start = from + offset;
        let end = start + needle.len();
        let covered = spans
            .iter()
            .filter(|(span, _)| span.start < end && span.end > start)
            .map(|(_, rect)| *rect)
            .filter(|rect| rect.w > 0 && rect.h > 0)
            .reduce(union);
        if let Some(rect) = covered {
            return Some(rect);
        }
        from = start + haystack[start..].chars().next().map_or(1, char::len_utf8);
    }
    None
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

    fn save_as() -> Vec<OcrLine> {
        vec![
            line("File Edit", &[("File", Rect::new(0, 0, 20, 10)), ("Edit", Rect::new(24, 0, 20, 10))]),
            line("Save As", &[("Save", Rect::new(0, 20, 30, 12)), ("As", Rect::new(36, 20, 14, 12))]),
        ]
    }

    #[test]
    fn a_single_word_returns_its_own_box() {
        assert_eq!(find_text(&save_as(), "Edit", true), Some(Rect::new(24, 0, 20, 10)));
    }

    #[test]
    fn a_phrase_unions_the_boxes_of_both_words() {
        assert_eq!(find_text(&save_as(), "Save As", true), Some(Rect::new(0, 20, 50, 12)));
    }

    #[test]
    fn matching_folds_case_unless_asked_otherwise() {
        assert_eq!(find_text(&save_as(), "sAvE aS", false), Some(Rect::new(0, 20, 50, 12)));
        assert_eq!(find_text(&save_as(), "sAvE aS", true), None);
    }

    #[test]
    fn a_missing_or_empty_needle_returns_none() {
        assert_eq!(find_text(&save_as(), "Print", false), None);
        assert_eq!(find_text(&save_as(), "", false), None);
        assert_eq!(find_text(&save_as(), "   ", false), None);
        assert_eq!(find_text(&[], "Save", false), None);
    }

    #[test]
    fn a_partial_span_over_two_words_unions_both() {
        assert_eq!(find_text(&save_as(), "ve A", true), Some(Rect::new(0, 20, 50, 12)));
        assert_eq!(find_text(&save_as(), "av", true), Some(Rect::new(0, 20, 30, 12)));
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
        assert_eq!(find_text(&lines, "open recent", false), Some(Rect::new(2, 4, 48, 9)));
        assert_eq!(find_text(&lines, "recent   file", false), Some(Rect::new(24, 4, 46, 9)));
    }

    #[test]
    fn the_earliest_line_and_match_win() {
        let lines = vec![
            line("go go", &[("go", Rect::new(0, 0, 10, 8)), ("go", Rect::new(14, 0, 10, 8))]),
            line("go", &[("go", Rect::new(0, 20, 10, 8))]),
        ];
        assert_eq!(find_text(&lines, "go", false), Some(Rect::new(0, 0, 10, 8)));
    }

    #[test]
    fn a_line_without_usable_boxes_is_skipped() {
        let lines = vec![
            line("ok", &[("ok", Rect::new(5, 5, 0, 0))]),
            line("ok", &[("ok", Rect::new(9, 30, 12, 8))]),
        ];
        assert_eq!(find_text(&lines, "ok", false), Some(Rect::new(9, 30, 12, 8)));
    }
}
