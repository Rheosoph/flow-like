use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

const IDEOGRAPHIC_FULL_STOP: char = '\u{3002}';

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
