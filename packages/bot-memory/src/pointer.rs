//! Model-facing pointers into the history: `#4812` (a message), `#4800–#4830` (a range),
//! `#4812.t1` (a tool result) and `#4812.ui2` (a UI block).

use std::fmt;
use std::ops::Range;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::model::Seq;

/// A part of one message: its n-th tool result or n-th UI block, counted from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum PartRef {
    Tool(u16),
    Ui(u16),
}

impl PartRef {
    /// `t1` or `ui2`.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        let (make, digits): (fn(u16) -> Self, &str) = match value.strip_prefix("ui") {
            Some(rest) => (Self::Ui, rest),
            None => (Self::Tool, value.strip_prefix('t')?),
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        digits
            .parse::<u16>()
            .ok()
            .filter(|index| *index >= 1)
            .map(make)
    }

    pub fn index(self) -> u16 {
        match self {
            Self::Tool(index) | Self::Ui(index) => index,
        }
    }
}

impl fmt::Display for PartRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Tool(index) => write!(f, "t{index}"),
            Self::Ui(index) => write!(f, "ui{index}"),
        }
    }
}

impl From<PartRef> for String {
    fn from(part: PartRef) -> Self {
        part.to_string()
    }
}

impl TryFrom<String> for PartRef {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value).ok_or_else(|| format!("'{value}' is not a part such as t1 or ui2"))
    }
}

/// A position in the history the model can cite and the tools can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Pointer {
    Message(Seq),
    Range { from: Seq, to: Seq },
    Part { seq: Seq, part: PartRef },
}

/// Why a pointer string was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerError(String);

impl fmt::Display for PointerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "'{}' is not a history pointer such as #4812, #4800–#4830, #4812.t1 or #4812.ui2",
            self.0
        )
    }
}

impl std::error::Error for PointerError {}

const RANGE_SEPARATORS: [&str; 4] = ["–", "—", "..", "-"];

fn leading_number(text: &str) -> Option<(u64, usize)> {
    let len = text.bytes().take_while(u8::is_ascii_digit).count();
    if len == 0 {
        return None;
    }
    text[..len].parse().ok().map(|value| (value, len))
}

fn ends_word(text: &str, at: usize) -> bool {
    text[at..]
        .chars()
        .next()
        .is_none_or(|next| !(next.is_alphanumeric() || next == '_'))
}

fn parse_at(text: &str, start: usize) -> Option<(Pointer, usize)> {
    let digits_at = start + text[start..].strip_prefix('#').map(|_| 1)?;
    let (seq, len) = leading_number(&text[digits_at..])?;
    let after_seq = digits_at + len;
    let rest = &text[after_seq..];

    for (prefix, make) in [
        (".ui", PartRef::Ui as fn(u16) -> PartRef),
        (".t", PartRef::Tool),
    ] {
        let Some(after) = rest.strip_prefix(prefix) else {
            continue;
        };
        let Some((index, index_len)) = leading_number(after) else {
            continue;
        };
        let end = after_seq + prefix.len() + index_len;
        if let Ok(index) = u16::try_from(index)
            && index >= 1
            && ends_word(text, end)
        {
            return Some((
                Pointer::Part {
                    seq,
                    part: make(index),
                },
                end,
            ));
        }
    }

    for separator in RANGE_SEPARATORS {
        let Some(after) = rest.strip_prefix(separator) else {
            continue;
        };
        let (after, hash) = match after.strip_prefix('#') {
            Some(after) => (after, 1),
            None => (after, 0),
        };
        let Some((to, to_len)) = leading_number(after) else {
            continue;
        };
        let end = after_seq + separator.len() + hash + to_len;
        if to >= seq && ends_word(text, end) {
            return Some((Pointer::range(seq, to), end));
        }
    }

    ends_word(text, after_seq).then_some((Pointer::Message(seq), after_seq))
}

impl Pointer {
    /// A range, or a single message when both ends are the same.
    pub fn range(from: Seq, to: Seq) -> Self {
        if from == to {
            Self::Message(from)
        } else {
            Self::Range {
                from: from.min(to),
                to: from.max(to),
            }
        }
    }

    pub fn parse(value: &str) -> Result<Self, PointerError> {
        let trimmed = value.trim();
        match parse_at(trimmed, 0) {
            Some((pointer, end)) if end == trimmed.len() => Ok(pointer),
            _ => Err(PointerError(value.chars().take(80).collect())),
        }
    }

    pub fn first_seq(self) -> Seq {
        match self {
            Self::Message(seq) | Self::Part { seq, .. } => seq,
            Self::Range { from, .. } => from,
        }
    }

    pub fn last_seq(self) -> Seq {
        match self {
            Self::Message(seq) | Self::Part { seq, .. } => seq,
            Self::Range { to, .. } => to,
        }
    }

    pub fn covers(self, seq: Seq) -> bool {
        (self.first_seq()..=self.last_seq()).contains(&seq)
    }
}

impl fmt::Display for Pointer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Message(seq) => write!(f, "#{seq}"),
            Self::Range { from, to } => write!(f, "#{from}–#{to}"),
            Self::Part { seq, part } => write!(f, "#{seq}.{part}"),
        }
    }
}

impl FromStr for Pointer {
    type Err = PointerError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl From<Pointer> for String {
    fn from(pointer: Pointer) -> Self {
        pointer.to_string()
    }
}

impl TryFrom<String> for Pointer {
    type Error = PointerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

/// Every pointer cited in `text`, in order, with its byte range. A `#` inside a word (`C#1`,
/// `&#123;`) or followed by letters (`#1a2b3c`) is not a pointer.
pub fn scan_pointers(text: &str) -> Vec<(Range<usize>, Pointer)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find('#') {
        let start = from + offset;
        let inside_word = text[..start].chars().next_back().is_some_and(|previous| {
            previous.is_alphanumeric() || previous == '&' || previous == '_'
        });
        match (!inside_word).then(|| parse_at(text, start)).flatten() {
            Some((pointer, end)) => {
                found.push((start..end, pointer));
                from = end;
            }
            None => from = start + 1,
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointers_parse_and_format_canonically() {
        let cases = [
            ("#4812", Pointer::Message(4812), "#4812"),
            (
                "#4800–#4830",
                Pointer::Range {
                    from: 4800,
                    to: 4830,
                },
                "#4800–#4830",
            ),
            (
                "#4800-#4830",
                Pointer::Range {
                    from: 4800,
                    to: 4830,
                },
                "#4800–#4830",
            ),
            (
                "#4800—4830",
                Pointer::Range {
                    from: 4800,
                    to: 4830,
                },
                "#4800–#4830",
            ),
            (
                "#4800..#4830",
                Pointer::Range {
                    from: 4800,
                    to: 4830,
                },
                "#4800–#4830",
            ),
            (
                "  #4812.t1 ",
                Pointer::Part {
                    seq: 4812,
                    part: PartRef::Tool(1),
                },
                "#4812.t1",
            ),
            (
                "#4812.ui2",
                Pointer::Part {
                    seq: 4812,
                    part: PartRef::Ui(2),
                },
                "#4812.ui2",
            ),
            ("#7–#7", Pointer::Message(7), "#7"),
        ];
        for (input, expected, formatted) in cases {
            let pointer = Pointer::parse(input).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(pointer, expected, "{input}");
            assert_eq!(pointer.to_string(), formatted);
            assert_eq!(Pointer::parse(&pointer.to_string()), Ok(pointer));
        }
    }

    #[test]
    fn malformed_pointers_are_refused() {
        for input in [
            "",
            "#",
            "4812",
            "#abc",
            "#4812.t0",
            "#4812.t",
            "#4812.x1",
            "#4830–#4800",
            "#12a",
            "#4812.t1x",
            "#1-",
            "#99999999999999999999999",
        ] {
            let error = Pointer::parse(input).unwrap_err();
            assert!(
                error.to_string().contains("not a history pointer"),
                "{input}"
            );
        }
    }

    #[test]
    fn ranges_and_parts_know_what_they_cover() {
        let range = Pointer::parse("#10–#20").unwrap();
        assert!(range.covers(10) && range.covers(20) && !range.covers(21));
        assert_eq!((range.first_seq(), range.last_seq()), (10, 20));
        let part = Pointer::parse("#15.t2").unwrap();
        assert!(part.covers(15) && !part.covers(16));
        assert_eq!(PartRef::parse("ui3"), Some(PartRef::Ui(3)));
        assert_eq!(PartRef::parse("t0"), None);
        assert_eq!(serde_json::to_value(PartRef::Tool(4)).unwrap(), "t4");
        let decoded: Pointer = serde_json::from_value(serde_json::json!("#9.ui1")).unwrap();
        assert_eq!(
            decoded,
            Pointer::Part {
                seq: 9,
                part: PartRef::Ui(1)
            }
        );
    }

    #[test]
    fn citations_are_found_in_prose_but_not_inside_words_or_colours() {
        let text = "Moved the launch (#4810, #4812.t1). See #4800–#4805; colour #1a2b3c, C#7, &#123; and #99.";
        let found: Vec<String> = scan_pointers(text)
            .into_iter()
            .map(|(range, pointer)| {
                assert_eq!(Pointer::parse(&text[range]).unwrap(), pointer);
                pointer.to_string()
            })
            .collect();
        assert_eq!(found, ["#4810", "#4812.t1", "#4800–#4805", "#99"]);
    }
}
