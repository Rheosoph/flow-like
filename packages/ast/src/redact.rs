//! Privacy redaction for FlowScript sources that leave the machine they were authored on.
//!
//! A failed apply is only diagnosable with the source that failed, but that source is board
//! content: declared variable defaults hold whatever the user typed into the variables panel, and
//! long call literals hold system prompts, SQL, URLs and pasted data. This module strips both
//! before the source is stored, keeping the shape an admin actually needs — which nodes were
//! called, with which argument keys, in which structure.
//!
//! Two rules, deliberately dumb so they hold on input that does not parse (the common case here):
//!
//! 1. **Declared values are dropped.** A `const`/`let` declaration, typed or not, loses an
//!    initializer that is a literal value — a string, number, boolean, `null`, template, or an
//!    object/array built only from those — because that is what a variable default is. An
//!    initializer that computes something (`const call = httpFetch({ … })`,
//!    `const id: string = ctx.id`, `const key = "…".format(…)`) is program, not data: it stays so
//!    the failed program remains diagnosable, and rule 2 still applies inside it.
//! 2. **Long literals are generalized.** A string literal over [`MAX_LITERAL_CHARS`] becomes
//!    `"<str:N>"`, a template literal `` `<tpl:N>` ``. Short ones — enum values, pin names, format
//!    keys, element refs — are kept verbatim, because they are what makes the source readable. A
//!    template literal spanning several lines is always generalized: multi-line templates are how
//!    prompts are written.
//!
//! Line numbers are preserved so a parser diagnostic's `line:col` still points into the redacted
//! text: a dropped multi-line initializer leaves its lines behind empty rather than closing the gap.

use serde::{Deserialize, Serialize};

use crate::parse::{is_ident_continue, is_ident_start};

/// String literals up to this many characters survive verbatim. Sized to keep enum values, pin
/// names, routes, format strings and element refs while catching prompts, SQL and pasted data.
pub const MAX_LITERAL_CHARS: usize = 64;

/// Upper bound on the redacted source. A board renders to a few hundred lines; this only bounds a
/// pathological paste.
pub const MAX_SOURCE_CHARS: usize = 64_000;

const TRUNCATION_MARKER: &str = "\n// … truncated";

/// A FlowScript source with every declared value and long literal removed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedFlowScript {
    pub text: String,
    /// Declarations whose initializer was dropped.
    pub dropped_values: usize,
    /// String literals replaced by a `"<str:N>"` placeholder.
    pub redacted_literals: usize,
    /// Whether the source was cut at [`MAX_SOURCE_CHARS`], by this pass or an earlier one.
    pub truncated: bool,
}

/// Redact `src` for storage. Never fails and never rejects: malformed input is redacted too.
///
/// Idempotent in the sense that matters — re-running it on its own output changes nothing, so a
/// client that already redacted locally is not penalized by the server redacting again.
pub fn redact_flowscript(src: &str) -> RedactedFlowScript {
    // A client that cut the source already sends it with the marker. Re-measuring the shorter text
    // would store a cut capture as a complete one.
    let (src, already_truncated) = match src.strip_suffix(TRUNCATION_MARKER) {
        Some(body) => (body, true),
        None => (src, false),
    };
    let mut out = String::with_capacity(src.len());
    let mut dropped_values = 0usize;
    let mut redacted_literals = 0usize;
    // Byte offset in `src` where a dropped initializer that runs over several lines ends.
    let mut dropped_until: Option<usize> = None;
    // A template literal opened on an earlier line and not yet closed.
    let mut open_template = false;
    let mut line_start = 0usize;

    for (index, full_line) in src.split('\n').enumerate() {
        let line_end = line_start + full_line.len();
        line_start = line_end + 1;
        if index > 0 {
            out.push('\n');
        }

        let mut line = full_line;
        if let Some(end) = dropped_until {
            if end > line_end {
                continue;
            }
            dropped_until = None;
            line = &src[end..line_end];
        }

        if open_template {
            let Some(end) = template_close_offset(line) else {
                continue;
            };
            out.push('`');
            line = &line[end + 1..];
        }

        let (code, comment) = line.split_at(comment_start(line));
        let mut kept = code;
        if let Some(eq) = declaration_value_span(code) {
            let value_start = line_end - line.len() + eq + 1;
            if let Some(len) = literal_initializer_len(&src[value_start..]) {
                dropped_values += 1;
                kept = code[..eq].trim_end();
                if value_start + len > line_end {
                    dropped_until = Some(value_start + len);
                }
            }
        }
        let dropped = kept.len() < code.len();

        let (redacted, unclosed_template) = redact_literals(kept, &mut redacted_literals);
        open_template = unclosed_template;
        out.push_str(&redacted);
        if !comment.is_empty() {
            if !redacted.is_empty() && dropped {
                out.push(' ');
            }
            out.push_str(comment);
        }
    }

    let truncated = already_truncated || out.chars().count() > MAX_SOURCE_CHARS;
    if truncated {
        truncate_to_whole_lines(&mut out);
        out.push_str(TRUNCATION_MARKER);
    }

    RedactedFlowScript {
        text: out,
        dropped_values,
        redacted_literals,
        truncated,
    }
}

/// Cut `text` to at most [`MAX_SOURCE_CHARS`], back to the last whole line, so a second pass sees
/// the same lines this one emitted.
fn truncate_to_whole_lines(text: &mut String) {
    if let Some((limit, _)) = text.char_indices().nth(MAX_SOURCE_CHARS) {
        let cut = text[..limit].rfind('\n').unwrap_or(limit);
        text.truncate(cut);
    }
}

/// Whether `ch` opens a string or template literal whose contents are not code.
fn is_quote(ch: char) -> bool {
    matches!(ch, '"' | '\'' | '`')
}

/// Byte offset of the unescaped backtick that closes a template literal continued from an
/// earlier line, if it closes on this one.
fn template_close_offset(line: &str) -> Option<usize> {
    let mut escaped = false;
    for (offset, ch) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '`' {
            return Some(offset);
        }
    }
    None
}

/// Byte offset of the `//` that starts a comment, or the line length when there is none.
fn comment_start(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut in_string: Option<u8> = None;
    let mut escaped = false;
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i];
        if let Some(quote) = in_string {
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == quote {
                in_string = None;
            }
        } else if matches!(ch, b'"' | b'\'' | b'`') {
            in_string = Some(ch);
        } else if ch == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return i;
        }
        i += 1;
    }
    line.len()
}

/// Byte offset of the `=` that opens a declaration's initializer, if this line declares one.
fn declaration_value_span(code: &str) -> Option<usize> {
    let keyword_start = skip_whitespace(code, 0);
    let keyword_end = read_identifier(code, keyword_start);
    if !matches!(&code[keyword_start..keyword_end], "const" | "let") {
        return None;
    }

    let name_start = skip_whitespace(code, keyword_end);
    let name_end = read_identifier(code, name_start);
    if name_end == name_start {
        return None;
    }

    let after_name = skip_whitespace(code, name_end);
    match code[after_name..].chars().next() {
        Some(':') => assignment_offset(code, after_name + 1),
        Some('=') if !code[after_name + 1..].starts_with('=') => Some(after_name),
        _ => None,
    }
}

/// Offset of the first top-level `=` at or after `from`, stopping at a statement terminator.
/// What precedes it is a type annotation, so a `>` or `<` before it closes a generic
/// (`Map<string, int>= …`) rather than forming a comparison.
fn assignment_offset(code: &str, from: usize) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    let mut in_string: Option<u8> = None;
    let mut escaped = false;
    let mut i = from;

    while i < bytes.len() {
        let ch = bytes[i];
        if let Some(quote) = in_string {
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == quote {
                in_string = None;
            }
            i += 1;
            continue;
        }
        match ch {
            b'"' | b'\'' | b'`' => in_string = Some(ch),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b';' if depth == 0 => return None,
            b'=' if depth == 0 => {
                let previous = i.checked_sub(1).map(|p| bytes[p]);
                let next = bytes.get(i + 1).copied();
                let comparison = matches!(previous, Some(b'=' | b'!')) || next == Some(b'=');
                if !comparison {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Byte length of the initializer that starts `rest` when it is a literal value — a string,
/// number, boolean, `null` or template, or an object/array built only from those, with bare or
/// quoted keys — or `None` when it computes something: a name, a call, an operator, a member
/// access. It runs to the first line break, `;` or comment outside brackets, across lines while
/// brackets or a template stay open. One the source cuts off counts as a literal: what is there
/// cannot be told apart from a value.
fn literal_initializer_len(rest: &str) -> Option<usize> {
    let mut brackets: Vec<char> = Vec::new();
    let mut chars = rest.char_indices().peekable();

    while let Some((offset, ch)) = chars.next() {
        match ch {
            '\n' | ';' if brackets.is_empty() => return Some(offset),
            '/' if chars.peek().is_some_and(|(_, next)| *next == '/') => {
                if brackets.is_empty() {
                    return Some(offset);
                }
                while chars.next_if(|(_, ch)| *ch != '\n').is_some() {}
            }
            '`' => {
                if !skip_template(&mut chars) {
                    return Some(rest.len());
                }
            }
            '"' | '\'' => skip_quoted(&mut chars, ch),
            '{' | '[' | '(' => brackets.push(ch),
            '}' | ']' | ')' => {
                if brackets.pop().is_none() {
                    return Some(offset);
                }
            }
            '-' | '+' => {
                if !signs_a_number(&mut chars) {
                    return None;
                }
            }
            ch if starts_number(ch, chars.peek()) => skip_number(&mut chars, ch),
            ch if is_ident_start(ch) => {
                let end = read_identifier(rest, offset);
                while chars.next_if(|(next, _)| *next < end).is_some() {}
                if !is_literal_word(rest, offset..end, brackets.last()) {
                    return None;
                }
            }
            ch if matches!(ch, ',' | ':' | ';') || ch.is_whitespace() => {}
            _ => return None,
        }
    }
    Some(rest.len())
}

type Chars<'a> = std::iter::Peekable<std::str::CharIndices<'a>>;

/// Consume a template literal after its opening backtick; `false` when the source ends first.
fn skip_template(chars: &mut Chars<'_>) -> bool {
    let mut escaped = false;
    chars.any(|(_, ch)| {
        let closes = !escaped && ch == '`';
        escaped = !escaped && ch == '\\';
        closes
    })
}

/// Consume a string literal after its opening `quote`, up to the closing quote or the end of the
/// line: like everywhere in this module, a plain string does not run on to the next line.
fn skip_quoted(chars: &mut Chars<'_>, quote: char) {
    let mut escaped = false;
    while let Some((_, ch)) = chars.next_if(|(_, ch)| *ch != '\n') {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            break;
        }
    }
}

/// Whether the `-`/`+` just consumed is a number's sign rather than arithmetic.
fn signs_a_number(chars: &mut Chars<'_>) -> bool {
    while chars.next_if(|(_, ch)| *ch == ' ' || *ch == '\t').is_some() {}
    chars
        .peek()
        .is_some_and(|(_, ch)| ch.is_ascii_digit() || *ch == '.')
}

fn starts_number(ch: char, next: Option<&(usize, char)>) -> bool {
    ch.is_ascii_digit() || (ch == '.' && next.is_some_and(|(_, next)| next.is_ascii_digit()))
}

/// Consume the rest of a number: digits, `.`, `_`, radix and exponent letters, an exponent's sign.
fn skip_number(chars: &mut Chars<'_>, first: char) {
    let mut previous = first;
    while let Some((_, ch)) = chars.next_if(|(_, ch)| {
        ch.is_ascii_alphanumeric()
            || matches!(ch, '.' | '_')
            || (matches!(ch, '+' | '-') && matches!(previous, 'e' | 'E'))
    }) {
        previous = ch;
    }
}

/// Whether the name at `rest[word]` leaves an initializer literal: `true`, `false`, `null`, or an
/// object key (`{ notes: [] }`). Any other name refers to something.
fn is_literal_word(rest: &str, word: std::ops::Range<usize>, innermost: Option<&char>) -> bool {
    let after = &rest[skip_whitespace(rest, word.end)..];
    matches!(&rest[word], "true" | "false" | "null")
        || (innermost == Some(&'{') && after.starts_with(':') && !after.starts_with("::"))
}

/// Replace the contents of over-long string literals with a `"<str:N>"` placeholder and of
/// over-long template literals with `` `<tpl:N>` ``. Returns the redacted code and whether a
/// template literal opened on this line without closing, so the caller drops the lines it
/// continues on.
fn redact_literals(code: &str, redacted: &mut usize) -> (String, bool) {
    if !code.chars().any(is_quote) {
        return (code.to_string(), false);
    }

    let mut out = String::with_capacity(code.len());
    let mut chars = code.char_indices();

    while let Some((start, ch)) = chars.next() {
        if !is_quote(ch) {
            out.push(ch);
            continue;
        }
        let quote = ch;

        let mut inner = 0usize;
        let mut escaped = false;
        let mut end = None;
        for (offset, ch) in chars.by_ref() {
            if escaped {
                escaped = false;
                inner += 1;
                continue;
            }
            match ch {
                '\\' => {
                    escaped = true;
                    inner += 1;
                }
                ch if ch == quote => {
                    end = Some(offset);
                    break;
                }
                _ => inner += 1,
            }
        }

        let Some(end) = end else {
            *redacted += 1;
            if quote == '`' {
                // A template literal continues on the next lines; those are dropped whole.
                out.push_str("`<tpl>");
                return (out, true);
            }
            // Unterminated literal: the source is malformed, so drop the rest of the line wholesale
            // rather than let raw text through on a technicality.
            out.push_str(&format!("\"<str:{inner}>"));
            return (out, false);
        };

        if inner > MAX_LITERAL_CHARS {
            *redacted += 1;
            if quote == '`' {
                out.push_str(&format!("`<tpl:{inner}>`"));
            } else {
                out.push_str(&format!("\"<str:{inner}>\""));
            }
        } else {
            out.push_str(&code[start..=end]);
        }
    }
    (out, false)
}

fn skip_whitespace(code: &str, from: usize) -> usize {
    code[from..]
        .find(|ch: char| !ch.is_whitespace())
        .map_or(code.len(), |offset| from + offset)
}

/// End of the identifier starting at byte `start` (or `start` when there is none), by the
/// lexer's own rules: a name like `databaseErnährung` is one identifier, not `databaseErn`.
fn read_identifier(code: &str, start: usize) -> usize {
    let mut chars = code[start..].char_indices();
    if !chars.next().is_some_and(|(_, ch)| is_ident_start(ch)) {
        return start;
    }
    chars
        .find(|(_, ch)| !is_ident_continue(*ch))
        .map_or(code.len(), |(offset, _)| start + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_declarations_lose_their_value() {
        let redacted = redact_flowscript("const goal: string = \"ship the thing\"");
        assert_eq!(redacted.text, "const goal: string");
        assert_eq!(redacted.dropped_values, 1);
    }

    #[test]
    fn untyped_bindings_keep_their_call() {
        let source = "const call = httpFetch({ request: makeRequest({ method: \"GET\" }) })";
        let redacted = redact_flowscript(source);
        assert_eq!(redacted.text, source);
        assert_eq!(redacted.dropped_values, 0);
    }

    #[test]
    fn inferred_scalar_defaults_are_redacted_without_losing_node_calls() {
        for value in [
            "\"short-token\"",
            "'short-token'",
            "'https://secret/{token}'",
            "'it\\'s secret'",
            "42",
            "-1.5",
            "true",
            "false",
            "null",
        ] {
            let source = format!(
                "const setting = {value} //@v:setting\nconst result = httpFetch({{ method: \"GET\" }})"
            );
            let redacted = redact_flowscript(&source);
            assert_eq!(
                redacted.text,
                "const setting //@v:setting\nconst result = httpFetch({ method: \"GET\" })"
            );
            assert_eq!(redacted.dropped_values, 1);
            assert_eq!(redacted.text.lines().count(), source.lines().count());
            assert_eq!(redact_flowscript(&redacted.text).text, redacted.text);
        }
        for source in ["const result = trueValue()", "const result = nullValue()"] {
            assert_eq!(redact_flowscript(source).text, source);
        }
    }

    #[test]
    fn typed_declarations_keep_an_initializer_that_computes() {
        let long = "q".repeat(MAX_LITERAL_CHARS + 1);
        for (source, expected) in [
            (
                "    const rows: Row[] = readRows()   //@n:READ_ROWS".to_string(),
                "    const rows: Row[] = readRows()   //@n:READ_ROWS".to_string(),
            ),
            (
                "    const requestedDraftId: string = actionContext.draftId".to_string(),
                "    const requestedDraftId: string = actionContext.draftId".to_string(),
            ),
            (
                "    const payload: Struct = { id: ctx.id, label: \"x\" }".to_string(),
                "    const payload: Struct = { id: ctx.id, label: \"x\" }".to_string(),
            ),
            (
                format!("    const query: string = \"{long}\".format({{ id: id }})"),
                format!(
                    "    const query: string = \"<str:{}>\".format({{ id: id }})",
                    MAX_LITERAL_CHARS + 1
                ),
            ),
        ] {
            let redacted = redact_flowscript(&source);
            assert_eq!(redacted.text, expected);
            assert_eq!(redacted.dropped_values, 0, "{source}");
            assert_eq!(redact_flowscript(&redacted.text).text, redacted.text);
        }
    }

    #[test]
    fn literal_objects_and_arrays_are_declared_values_whether_typed_or_not() {
        for source in [
            "const cfg: Struct = { notes: [], count: 0, nested: { ok: true, ratio: -0.5 } }",
            "const o = {\"k\": 1, \"secret\": \"short\"}",
            "const xs = [1, 2, \"three\"]",
            "    let rows = []",
            "const origin: geometry<Point> = {\"type\":\"Point\",\"coordinates\":[13.405,52.52]}",
            "const m: Map<string, int>= {\"a\": 1}",
            "const big = 1e-5",
            "const prompt = `Hello ${name}`",
        ] {
            let redacted = redact_flowscript(source);
            assert_eq!(redacted.dropped_values, 1, "{source} -> {}", redacted.text);
            assert!(
                !redacted.text.contains('='),
                "{source} kept its value: {}",
                redacted.text
            );
        }
    }

    #[test]
    fn untyped_expressions_that_start_like_a_literal_keep_their_initializer() {
        for source in [
            "    const hash4 = \"momo-draft:{draftId}\".format({ draftId: id }).sha256()",
            "    let x = -a + b",
            "    const n = 1 + count",
            "    const label = \"#\" + name",
            "    const size = \"abc\".length",
            "    const parts = [first, \"second\"]",
            "    const pick = flag ? \"a\" : \"b\"",
        ] {
            let redacted = redact_flowscript(source);
            assert_eq!(redacted.text, source);
            assert_eq!(redacted.dropped_values, 0, "{source}");
        }
    }

    #[test]
    fn non_ascii_names_are_declarations_too() {
        for (source, expected) in [
            (
                "const databaseErnährung: NodeDBConnection = {\"cache_key\":\"\"}",
                "const databaseErnährung: NodeDBConnection",
            ),
            ("const größe = 5 //@v:size", "const größe //@v:size"),
            ("    let ñandú = \"privat\"", "    let ñandú"),
        ] {
            let redacted = redact_flowscript(source);
            assert_eq!(redacted.text, expected);
            assert_eq!(redacted.dropped_values, 1);
        }
        let source = "constÄ = \"value\"";
        assert_eq!(redact_flowscript(source).text, source);
    }

    #[test]
    fn single_quoted_node_literals_follow_the_existing_redaction_policy() {
        let short = "const result = httpFetch({ method: 'GET', url: 'https://example.com' })";
        assert_eq!(redact_flowscript(short).text, short);
        let long = "x".repeat(MAX_LITERAL_CHARS + 1);
        let redacted = redact_flowscript(&format!("httpFetch({{ body: '{long}' }})"));
        assert!(!redacted.text.contains(&long));
        assert_eq!(redacted.redacted_literals, 1);
        assert_eq!(redact_flowscript(&redacted.text).text, redacted.text);
    }

    #[test]
    fn locals_are_variables_too() {
        let redacted = redact_flowscript("    let label: string = \"internal name\"");
        assert_eq!(redacted.text, "    let label: string");
    }

    #[test]
    fn anchors_survive_a_dropped_value() {
        let redacted = redact_flowscript("const goal: string = \"secret\" //@v:abc123");
        assert_eq!(redacted.text, "const goal: string //@v:abc123");
    }

    #[test]
    fn short_literals_are_kept_and_long_ones_generalized() {
        let long = "x".repeat(MAX_LITERAL_CHARS + 1);
        let redacted = redact_flowscript(&format!(
            "httpMakeRequest({{ method: \"GET\", url: \"{long}\" }})"
        ));
        assert_eq!(
            redacted.text,
            format!(
                "httpMakeRequest({{ method: \"GET\", url: \"<str:{}>\" }})",
                MAX_LITERAL_CHARS + 1
            )
        );
        assert_eq!(redacted.redacted_literals, 1);
    }

    #[test]
    fn a_literal_exactly_at_the_limit_survives() {
        let value = "y".repeat(MAX_LITERAL_CHARS);
        let source = format!("call({{ text: \"{value}\" }})");
        assert_eq!(redact_flowscript(&source).text, source);
    }

    #[test]
    fn comment_markers_inside_literals_are_not_comments() {
        let source = "call({ url: \"https://a.example/b\" })";
        assert_eq!(redact_flowscript(source).text, source);
    }

    #[test]
    fn multi_line_values_are_dropped_whole_and_keep_the_line_count() {
        let source = "const cfg: Struct = {\n  \"key\": \"value\",\n}\ncall({ a: 1 })";
        let redacted = redact_flowscript(source);
        assert_eq!(redacted.text, "const cfg: Struct\n\n\ncall({ a: 1 })");
        assert_eq!(redacted.dropped_values, 1);

        let source = "const cfg = { // defaults\n  \"key\": \"value\"\n}; call({ a: 1 })";
        let redacted = redact_flowscript(source);
        assert_eq!(redacted.text, "const cfg // defaults\n\n; call({ a: 1 })");
    }

    #[test]
    fn a_multi_line_expression_keeps_every_line() {
        let source =
            "const result = httpFetch({\n  method: \"GET\",\n  url: base,\n})\ncall({ a: 1 })";
        assert_eq!(redact_flowscript(source).text, source);
    }

    #[test]
    fn comparisons_are_not_mistaken_for_assignment() {
        let source = "if (source.type == \"Search\") {";
        assert_eq!(redact_flowscript(source).text, source);
        assert_eq!(redact_flowscript(source).dropped_values, 0);
        let source = "const same == other";
        assert_eq!(redact_flowscript(source).text, source);
    }

    #[test]
    fn reassignment_of_a_variable_is_not_a_declaration() {
        let source = "reportEntry = push2.arrayOut";
        assert_eq!(redact_flowscript(source).text, source);
    }

    #[test]
    fn redaction_is_idempotent() {
        let source = format!(
            "const goal: string = \"{}\"\ncall({{ q: \"{}\" }})",
            "a".repeat(200),
            "b".repeat(200)
        );
        let once = redact_flowscript(&source);
        let twice = redact_flowscript(&once.text);
        assert_eq!(once.text, twice.text);
        // Nothing is left to strip on the second pass, which is what idempotent means here.
        assert_eq!(twice.dropped_values, 0);
        assert_eq!(twice.redacted_literals, 0);
    }

    #[test]
    fn unterminated_literals_never_leak_the_rest_of_the_line() {
        let redacted = redact_flowscript("call({ text: \"oops unterminated data");
        assert_eq!(redacted.text, "call({ text: \"<str:22>");
        assert_eq!(redacted.redacted_literals, 1);
        let redacted = redact_flowscript("const cfg = { \"key\": \"oops\nnext: 1\n");
        assert_eq!(redacted.text, "const cfg\n\n");
    }

    #[test]
    fn oversized_sources_are_truncated_at_a_line_boundary() {
        let source = "call({ a: 1 })\n".repeat(MAX_SOURCE_CHARS);
        let redacted = redact_flowscript(&source);
        assert!(redacted.truncated);
        let body = redacted
            .text
            .strip_suffix(TRUNCATION_MARKER)
            .expect("a cut source ends with the marker");
        assert!(body.chars().count() <= MAX_SOURCE_CHARS);
        assert!(body.lines().all(|line| line == "call({ a: 1 })"));
    }

    /// The I088 capture was cut on the desktop but stored as `truncated = false`: the server
    /// re-measured the already shortened text.
    #[test]
    fn a_source_cut_by_an_earlier_pass_stays_marked_truncated() {
        let source = "call({ a: 1 })\n".repeat(MAX_SOURCE_CHARS);
        let client = redact_flowscript(&source);
        let server = redact_flowscript(&client.text);
        assert!(server.truncated);
        assert_eq!(server.text, client.text);

        let older_client =
            format!("call({{ a: 1 }})\nconst setting = \"token\"{TRUNCATION_MARKER}");
        let server = redact_flowscript(&older_client);
        assert!(server.truncated);
        assert_eq!(
            server.text,
            format!("call({{ a: 1 }})\nconst setting{TRUNCATION_MARKER}")
        );
        assert!(!redact_flowscript("call({ a: 1 })").truncated);
    }

    #[test]
    fn namespace_method_use_and_destructuring_forms_pass_through() {
        for source in [
            "use ai::ml::*, string::*",
            "const t = string::trim({ string: s })",
            "const { text, usage: u } = ai::invoke({ model: m })",
            "    let n = s.contains(\"?\", { ignoreCase: true })",
            "    items.push(x)   //@n:abc",
        ] {
            let redacted = redact_flowscript(source);
            assert_eq!(redacted.text, source);
            assert_eq!(redacted.dropped_values, 0);
        }
    }

    #[test]
    fn module_blocks_pass_through_untouched() {
        // Redaction is line-based and keys off `const`/`let`, so a module header carries no value
        // to drop; only the declarations nested inside one are affected.
        let source = "module checkout {   //@l:mod1\n    function helper(): (out: string) {\n        let label: string = \"internal\"\n    }\n}";
        let redacted = redact_flowscript(source);
        assert_eq!(
            redacted.text,
            "module checkout {   //@l:mod1\n    function helper(): (out: string) {\n        let label: string\n    }\n}"
        );
        assert_eq!(redacted.dropped_values, 1);
    }

    #[test]
    fn detached_blocks_are_redacted_like_an_event_body() {
        // A `detached` container has no node behind it, so it carries no value to drop and no
        // anchor of its own; the statements inside it are redacted exactly as an event body's are.
        let prompt = "p".repeat(MAX_LITERAL_CHARS + 1);
        let source = format!(
            "detached {{\n    let note: string = \"internal only\"   //@n:orphan1\n    const answer = ai::invoke({{ model: \"gpt-4\", prompt: \"{prompt}\" }})   //@n:orphan2\n}}\n\ndetached {{\n    log::info({{ message: \"keep me\" }})   //@n:orphan3\n}}"
        );
        let redacted = redact_flowscript(&source);
        assert_eq!(
            redacted.text,
            format!(
                "detached {{\n    let note: string //@n:orphan1\n    const answer = ai::invoke({{ model: \"gpt-4\", prompt: \"<str:{}>\" }})   //@n:orphan2\n}}\n\ndetached {{\n    log::info({{ message: \"keep me\" }})   //@n:orphan3\n}}",
                MAX_LITERAL_CHARS + 1
            )
        );
        assert_eq!(redacted.dropped_values, 1);
        assert_eq!(redacted.redacted_literals, 1);
        assert_eq!(redacted.text.lines().count(), source.lines().count());
        assert_eq!(redact_flowscript(&redacted.text).text, redacted.text);
    }

    #[test]
    fn short_template_literals_survive_and_long_ones_generalize() {
        let source = "call({ text: `hello ${name}` })";
        let redacted = redact_flowscript(source);
        assert_eq!(redacted.text, source);
        assert_eq!(redacted.redacted_literals, 0);

        let long = "p".repeat(MAX_LITERAL_CHARS + 1);
        let redacted = redact_flowscript(&format!("call({{ text: `{long} ${{x}}` }})"));
        assert_eq!(
            redacted.text,
            format!("call({{ text: `<tpl:{}>` }})", MAX_LITERAL_CHARS + 6)
        );
        assert_eq!(redacted.redacted_literals, 1);
    }

    #[test]
    fn multi_line_template_literals_are_dropped_whole_and_keep_the_line_count() {
        let source = "call({ prompt: `You are a helpful\nassistant for ${user}.\nAnswer briefly.` })\ncall({ a: \"x\" })";
        let redacted = redact_flowscript(source);
        assert_eq!(
            redacted.text,
            "call({ prompt: `<tpl>\n\n` })\ncall({ a: \"x\" })"
        );
        assert_eq!(redacted.redacted_literals, 1);
        assert_eq!(redacted.text.lines().count(), source.lines().count());
        assert_eq!(redact_flowscript(&redacted.text).text, redacted.text);

        let source = "const m = `You are a helpful\nassistant for ${user}.\nAnswer briefly.`\ncall({ a: \"x\" })";
        let redacted = redact_flowscript(source);
        assert_eq!(redacted.text, "const m\n\n\ncall({ a: \"x\" })");
        assert_eq!(redacted.dropped_values, 1);
        assert_eq!(redacted.text.lines().count(), source.lines().count());
    }

    #[test]
    fn comment_markers_and_braces_inside_template_literals_are_template_text() {
        let source = "call({ url: `https://a.example/${id}` })";
        assert_eq!(redact_flowscript(source).text, source);
        let source = "const cfg: string = `{ // not a comment`";
        assert_eq!(redact_flowscript(source).text, "const cfg: string");
    }

    #[test]
    fn line_numbers_are_stable_so_diagnostics_still_point_at_the_right_line() {
        let source = "const a: string = \"one\"\nconst b: int = 2\ncall({ x: 1 })";
        let redacted = redact_flowscript(source);
        assert_eq!(redacted.text.lines().count(), source.lines().count());
        assert_eq!(redacted.text.lines().nth(2), Some("call({ x: 1 })"));
    }
}
