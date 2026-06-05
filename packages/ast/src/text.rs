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
    } else {
        out
    }
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
    }

    #[test]
    fn quoting_escapes() {
        assert_eq!(quote_string("a\"b\n"), "\"a\\\"b\\n\"");
    }
}
