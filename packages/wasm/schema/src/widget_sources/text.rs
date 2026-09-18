use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

const IDEOGRAPHIC_FULL_STOP: char = '\u{3002}';
const REASON_PUNCTUATION: &[char] = &[',', '.', ':', ';', '(', ')', '-', '\'', '/', '&'];
const MAX_MARKS_IN_A_ROW: usize = 2;

/// `NFKC(s).toLowerCase()` with U+3002 between ASCII alphanumerics read as `.`.
pub(super) fn fold(text: &str) -> String {
    let lowered: Vec<char> = text
        .nfkc()
        .collect::<String>()
        .to_lowercase()
        .chars()
        .collect();
    lowered
        .iter()
        .enumerate()
        .map(|(index, &c)| {
            let between_alphanumerics = index > 0
                && lowered[index - 1].is_ascii_alphanumeric()
                && lowered
                    .get(index + 1)
                    .is_some_and(char::is_ascii_alphanumeric);
            if c == IDEOGRAPHIC_FULL_STOP && between_alphanumerics {
                '.'
            } else {
                c
            }
        })
        .collect()
}

/// Reason rule 8 (§14.2.4) with the TLD test injected.
pub(super) fn contains_address(text: &str, is_icann_tld: impl Fn(&str) -> bool) -> bool {
    let folded = fold(text);
    if folded.contains("://") || folded.contains('@') || folded.contains("www.") {
        return true;
    }
    folded
        .split(|c: char| !(is_letter_or_mark(c) || c.is_numeric() || c == '-' || c == '.'))
        .any(|token| {
            let token = token.trim_matches('.');
            let mut labels = token.rsplit('.');
            let last = labels.next().unwrap_or_default();
            labels.next().is_some()
                && last.chars().count() >= 2
                && (!last.is_ascii() || is_icann_tld(last))
        })
}

fn is_letter_or_mark(c: char) -> bool {
    c.is_alphabetic() || is_combining_mark(c)
}

/// Reason rule 3 (§14.2.4): the character allowlist.
pub(super) fn has_only_allowed_characters(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut marks_in_a_row = 0;
    chars.iter().enumerate().all(|(index, &c)| {
        if is_combining_mark(c) {
            marks_in_a_row += 1;
            return marks_in_a_row <= MAX_MARKS_IN_A_ROW;
        }
        marks_in_a_row = 0;
        if c.is_alphabetic() || c.is_numeric() || c == ' ' || REASON_PUNCTUATION.contains(&c) {
            return true;
        }
        let non_ascii_letter_or_mark = |neighbour: Option<&char>| {
            neighbour.is_some_and(|n| !n.is_ascii() && is_letter_or_mark(*n))
        };
        matches!(c, '\u{200C}' | '\u{200D}')
            && index > 0
            && non_ascii_letter_or_mark(chars.get(index - 1))
            && non_ascii_letter_or_mark(chars.get(index + 1))
    })
}

/// Reason rule 7 (§14.2.4): a letter run mixing ASCII and non-Latin letters.
pub(super) fn has_mixed_script(text: &str) -> bool {
    let latin_extended = |c: char| matches!(c, '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}');
    text.split(|c: char| !is_letter_or_mark(c)).any(|run| {
        let letters = || run.chars().filter(|c| c.is_alphabetic());
        letters().any(|c| c.is_ascii()) && letters().any(|c| !c.is_ascii() && !latin_extended(c))
    })
}
