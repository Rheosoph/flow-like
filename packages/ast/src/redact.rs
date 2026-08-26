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
//! 1. **Declared values are dropped.** A declaration that carries a type annotation
//!    (`const goal: string = "…"`, `let label: string = ""`) is a variable, and its initializer is
//!    removed. A declaration without one (`const call = httpFetch({ … })`) binds a node result and
//!    is the program itself, so it is kept.
//! 2. **Long literals are generalized.** A string literal over [`MAX_LITERAL_CHARS`] becomes
//!    `"<str:N>"`, a template literal `` `<tpl:N>` ``. Short ones — enum values, pin names, format
//!    keys, element refs — are kept verbatim, because they are what makes the source readable. A
//!    template literal spanning several lines is always generalized: multi-line templates are how
//!    prompts are written.
//!
//! Line numbers are preserved so a parser diagnostic's `line:col` still points into the redacted
//! text: a dropped multi-line initializer leaves its lines behind empty rather than closing the gap.

use serde::{Deserialize, Serialize};

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
    /// Whether the result hit [`MAX_SOURCE_CHARS`].
    pub truncated: bool,
}

/// Redact `src` for storage. Never fails and never rejects: malformed input is redacted too.
///
/// Idempotent in the sense that matters — re-running it on its own output changes nothing, so a
/// client that already redacted locally is not penalized by the server redacting again.
pub fn redact_flowscript(src: &str) -> RedactedFlowScript {
    let mut out = String::with_capacity(src.len());
    let mut dropped_values = 0usize;
    let mut redacted_literals = 0usize;
    // Depth still owed by a dropped initializer that opened brackets and did not close them.
    let mut dropping: Option<usize> = None;
    // A template literal opened on an earlier line and not yet closed.
    let mut open_template = false;

    for (index, line) in src.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }

        let line = if open_template {
            match template_close_offset(line) {
                Some(end) => {
                    open_template = false;
                    out.push('`');
                    &line[end + 1..]
                }
                None => continue,
            }
        } else {
            line
        };

        let line = if let Some(depth) = dropping {
            match resume_dropped_initializer(line, depth) {
                Continuation::StillOpen(depth) => {
                    dropping = Some(depth);
                    continue;
                }
                Continuation::Closed(rest) => {
                    dropping = None;
                    rest
                }
            }
        } else {
            line
        };

        let comment_at = comment_start(line);
        let (code, comment) = line.split_at(comment_at);

        let (code, cut) = match declaration_value_span(code) {
            Some(eq) => {
                dropped_values += 1;
                let kept = code[..eq].trim_end();
                (kept, Some(&code[eq..]))
            }
            None => (code, None),
        };

        if let Some(cut) = cut {
            let depth = trailing_depth(cut);
            if depth > 0 {
                dropping = Some(depth);
            }
        }

        let (redacted, unclosed_template) = redact_literals(code, &mut redacted_literals);
        open_template = unclosed_template;
        out.push_str(&redacted);
        if !comment.is_empty() {
            if !redacted.is_empty() && cut.is_some() {
                out.push(' ');
            }
            out.push_str(comment);
        }
    }

    let truncated = out.chars().count() > MAX_SOURCE_CHARS;
    if truncated {
        out = out.chars().take(MAX_SOURCE_CHARS).collect();
        out.push_str(TRUNCATION_MARKER);
    }

    RedactedFlowScript {
        text: out,
        dropped_values,
        redacted_literals,
        truncated,
    }
}

enum Continuation<'a> {
    StillOpen(usize),
    Closed(&'a str),
}

/// Whether `ch` opens a string or template literal whose contents are not code.
fn is_quote(ch: char) -> bool {
    ch == '"' || ch == '`'
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

/// Consume a continued initializer, returning what is left of the line once its brackets balance.
fn resume_dropped_initializer(line: &str, depth: usize) -> Continuation<'_> {
    let mut depth = depth;
    let mut in_string: Option<char> = None;
    let mut escaped = false;

    for (offset, ch) in line.char_indices() {
        if let Some(quote) = in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                in_string = None;
            }
            continue;
        }
        match ch {
            ch if is_quote(ch) => in_string = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return Continuation::Closed(&line[offset + ch.len_utf8()..]);
                }
            }
            _ => {}
        }
    }
    Continuation::StillOpen(depth)
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
        } else if ch == b'"' || ch == b'`' {
            in_string = Some(ch);
        } else if ch == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return i;
        }
        i += 1;
    }
    line.len()
}

/// Byte offset of the `=` that opens a *typed* declaration's initializer, if this line is one.
///
/// Typed is the whole discrimination: `const x: T = …` declares a variable and carries a value,
/// `const x = call(…)` binds a node result and carries the program.
fn declaration_value_span(code: &str) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut i = skip_whitespace(bytes, 0);

    let keyword_end = read_identifier(bytes, i);
    let keyword = &code[i..keyword_end];
    if keyword != "const" && keyword != "let" {
        return None;
    }

    i = skip_whitespace(bytes, keyword_end);
    let name_end = read_identifier(bytes, i);
    if name_end == i {
        return None;
    }

    i = skip_whitespace(bytes, name_end);
    if bytes.get(i) != Some(&b':') {
        return None;
    }

    assignment_offset(code, i + 1)
}

/// Offset of the first top-level `=` at or after `from`, stopping at a statement terminator.
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
            b'"' | b'`' => in_string = Some(ch),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b';' if depth == 0 => return None,
            b'=' if depth == 0 => {
                let previous = i.checked_sub(1).map(|p| bytes[p]);
                let next = bytes.get(i + 1).copied();
                let comparison =
                    matches!(previous, Some(b'=' | b'!' | b'<' | b'>')) || next == Some(b'=');
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

/// Bracket depth left open by `text`, used to know whether a dropped value continues onto the
/// next line.
fn trailing_depth(text: &str) -> usize {
    match resume_dropped_initializer(text, 1) {
        // The synthetic depth of 1 stands in for the statement itself, so anything that closes it
        // was already balanced within the line.
        Continuation::StillOpen(depth) => depth - 1,
        Continuation::Closed(_) => 0,
    }
}

/// Replace the contents of over-long string literals with a `"<str:N>"` placeholder and of
/// over-long template literals with `` `<tpl:N>` ``. Returns the redacted code and whether a
/// template literal opened on this line without closing, so the caller drops the lines it
/// continues on.
fn redact_literals(code: &str, redacted: &mut usize) -> (String, bool) {
    if !code.contains('"') && !code.contains('`') {
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

fn skip_whitespace(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn read_identifier(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
    {
        i += 1;
    }
    i
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
    }

    #[test]
    fn comparisons_are_not_mistaken_for_assignment() {
        let source = "if (source.type == \"Search\") {";
        assert_eq!(redact_flowscript(source).text, source);
        assert_eq!(redact_flowscript(source).dropped_values, 0);
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
    }

    #[test]
    fn oversized_sources_are_truncated() {
        let source = "call({ a: 1 })\n".repeat(MAX_SOURCE_CHARS);
        let redacted = redact_flowscript(&source);
        assert!(redacted.truncated);
        assert_eq!(
            redacted.text.chars().count(),
            MAX_SOURCE_CHARS + TRUNCATION_MARKER.chars().count()
        );
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
        let source = "const m = `You are a helpful\nassistant for ${user}.\nAnswer briefly.`\ncall({ a: \"x\" })";
        let redacted = redact_flowscript(source);
        assert_eq!(redacted.text, "const m = `<tpl>\n\n`\ncall({ a: \"x\" })");
        assert_eq!(redacted.redacted_literals, 1);
        assert_eq!(redacted.text.lines().count(), source.lines().count());
        assert_eq!(redact_flowscript(&redacted.text).text, redacted.text);
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
