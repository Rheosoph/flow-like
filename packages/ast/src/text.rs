//! Pure text utilities for FlowScript (no board/catalog dependency).

/// Convert a catalog identifier (`snake_case`, `namespaced::name`, `kebab-case`) into a
/// JS-flavoured `camelCase` display name.
pub fn to_camel_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut upcoming_upper = false;
    let mut first = true;
    for ch in input.chars() {
        if ch.is_alphanumeric() {
            if first {
                out.push(ch.to_ascii_lowercase());
                first = false;
            } else if upcoming_upper {
                out.extend(ch.to_uppercase());
            } else {
                out.push(ch);
            }
            upcoming_upper = false;
        } else if !first {
            // Any separator (`_`, `-`, `:`, `/`, space) triggers the next char to upper.
            upcoming_upper = true;
        }
    }
    if out.is_empty() {
        "node".to_string()
    } else if out.chars().next().is_some_and(|c| c.is_numeric()) {
        // A digit-leading identifier lexes as a number and breaks the whole document.
        // Both lowering and reconcile camelize through here, so names stay in sync.
        format!("_{out}")
    } else {
        out
    }
}

/// Whether `s` lexes as a single FlowScript identifier (mirrors the lexer's rules).
pub fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// Quote and escape a string as a FlowScript double-quoted literal.
pub fn quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Emitted as escapes so every escape the lexer accepts is one the renderer can
            // produce; a raw 0x08/0x0C in rendered source is invisible in an editor. `'` is
            // deliberately NOT escaped — the fixtures carry many bare apostrophes.
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_variants() {
        assert_eq!(
            to_camel_case("ai_generative_find_model"),
            "aiGenerativeFindModel"
        );
        assert_eq!(to_camel_case("namespaced::name"), "namespacedName");
        assert_eq!(to_camel_case("kebab-case-thing"), "kebabCaseThing");
        assert_eq!(to_camel_case("Already Spaced"), "alreadySpaced");
        assert_eq!(to_camel_case(""), "node");
        assert_eq!(to_camel_case("2fa enabled"), "_2faEnabled");
    }

    #[test]
    fn quoting_escapes() {
        assert_eq!(quote_string("a\"b\n"), "\"a\\\"b\\n\"");
    }
}
