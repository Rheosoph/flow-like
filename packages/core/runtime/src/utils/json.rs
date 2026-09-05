use flow_like_types::Value;
use json5;

fn fix_unbalanced(s: &str) -> String {
    let open_braces = s.chars().filter(|&c| c == '{').count();
    let close_braces = s.chars().filter(|&c| c == '}').count();
    let open_brackets = s.chars().filter(|&c| c == '[').count();
    let close_brackets = s.chars().filter(|&c| c == ']').count();

    let mut fixed = s.to_owned();
    for _ in 0..(open_braces.saturating_sub(close_braces)) {
        fixed.push('}');
    }
    for _ in 0..(open_brackets.saturating_sub(close_brackets)) {
        fixed.push(']');
    }
    fixed
}

/// Attempts to extract a JSON fragment from the input string using a stack-based approach.
/// Returns the candidate substring along with any missing closing characters (as a Vec)
/// that were expected but not found.
fn extract_json_fragment(input: &str) -> Option<(String, Vec<char>)> {
    // Locate the first JSON opening character.
    let start = input.find(['{', '['])?;
    let candidate = &input[start..];

    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escape = false;
    let mut string_delim: Option<char> = None;
    let mut last_index = candidate.len();

    // Walk through the candidate substring char by char.
    for (i, c) in candidate.char_indices() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            if c == '\\' {
                escape = true;
                continue;
            }
            if Some(c) == string_delim {
                in_string = false;
                string_delim = None;
            }
        } else {
            if c == '"' || c == '\'' {
                in_string = true;
                string_delim = Some(c);
                continue;
            }
            if c == '{' {
                stack.push('}');
            } else if c == '[' {
                stack.push(']');
            } else if c == '}' || c == ']' {
                if let Some(expected) = stack.pop() {
                    if c != expected {
                        // Mismatch detected. We break early.
                        last_index = i + c.len_utf8();
                        break;
                    }
                } else {
                    // Unmatched closing found; break.
                    last_index = i + c.len_utf8();
                    break;
                }
            }
            // If the stack is empty, we assume the JSON fragment is complete.
            if stack.is_empty() {
                last_index = i + c.len_utf8();
                break;
            }
        }
    }

    let fragment = candidate[..last_index].to_string();

    Some((fragment, stack))
}

/// Tries multiple strategies to parse a malformed JSON string into a serde_json::Value.
pub fn parse_malformed_json(input: &str) -> flow_like_types::Result<Value> {
    // 1. Direct parse attempt.
    if let Ok(val) = flow_like_types::json::from_str(input) {
        return Ok(val);
    }

    // 2. Try to extract a candidate JSON fragment.
    if let Some((mut candidate, missing_stack)) = extract_json_fragment(input) {
        // Append missing closing characters (if any) in the correct order.
        if !missing_stack.is_empty() {
            let missing: String = missing_stack.iter().rev().collect();
            candidate.push_str(&missing);
        }

        // Trim any trailing garbage beyond the last valid JSON delimiter.
        let last_valid_index = candidate
            .rfind(['}', ']'])
            .map(|idx| idx + 1)
            .unwrap_or(candidate.len());
        let candidate = &candidate[..last_valid_index];

        // 3. Try strict parsing first.
        if let Ok(val) = flow_like_types::json::from_str(candidate) {
            return Ok(val);
        }
        // 4. Then try the lenient json5 parser.
        if let Ok(val) = json5::from_str(candidate) {
            return Ok(val);
        }

        // 5. As an extra heuristic, try to fix any unbalanced delimiters in the candidate.
        let fixed_candidate = fix_unbalanced(candidate);
        if let Ok(val) = flow_like_types::json::from_str(&fixed_candidate) {
            return Ok(val);
        }
        if let Ok(val) = json5::from_str(&fixed_candidate) {
            return Ok(val);
        }
    }

    // 6. Fallback: remove any non-JSON prefix and apply a basic fix.
    if let Some(start) = input.find(['{', '[']) {
        let trimmed = &input[start..];
        let fixed = fix_unbalanced(trimmed);
        if let Ok(val) = json5::from_str(&fixed) {
            return Ok(val);
        }
    }

    flow_like_types::bail!("Failed to parse JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_json_test_prefix() {
        let input = "Some extraneous text before the JSON {\"name\": \"Alice\", \"age\": 30";
        let result = parse_malformed_json(input).unwrap();
        let expected = flow_like_types::json::json!({"name": "Alice", "age": 30});
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_json_test_array() {
        let input = "prefix [1, 2, 3, 4, 5] some trailing text";
        let result = parse_malformed_json(input).unwrap();
        let expected = flow_like_types::json::json!([1, 2, 3, 4, 5]);
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_json_test_invalid() {
        let input = "random text {invalid json here";
        let result = parse_malformed_json(input);
        assert!(result.is_err());
    }

    #[test]
    fn malformed_json_test_nested() {
        let input = "noise {\"nested\": {\"key\": \"value\", \"list\": [1, 2, 3]";
        let result = parse_malformed_json(input).unwrap();
        let expected =
            flow_like_types::json::json!({"nested": {"key": "value", "list": [1, 2, 3]}});
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_json_test_trailing_commas() {
        let input = r#"{"items": [1, 2, 3,], "more": {"a": 1, "b": 2,},}"#;
        let result = parse_malformed_json(input).unwrap();
        let expected = flow_like_types::json::json!({"items": [1, 2, 3], "more": {"a": 1, "b": 2}});
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_json_test_deeply_nested_unbalanced() {
        let input = r#"{"level1": {"level2": {"level3": [1, 2, {"deep": "value""#;
        let result = parse_malformed_json(input).unwrap();
        let expected = flow_like_types::json::json!({"level1": {"level2": {"level3": [1, 2, {"deep": "value"}]}}});
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_json_test_json5_features() {
        let input = r#"{
            // This is a comment
            key: 'single quoted string',
            trailing_comma: true,
        }"#;
        let result = parse_malformed_json(input).unwrap();
        let expected =
            flow_like_types::json::json!({"key": "single quoted string", "trailing_comma": true});
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_json_test_empty_structures() {
        let input = "garbage {} more garbage";
        let result = parse_malformed_json(input).unwrap();
        let expected = flow_like_types::json::json!({});
        assert_eq!(result, expected);

        let input = "garbage [] more garbage";
        let result = parse_malformed_json(input).unwrap();
        let expected = flow_like_types::json::json!([]);
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_json_test_special_characters() {
        let input = r#"{"unicode": "こんにちは世界", "escaped": "line1\nline2"}"#;
        let result = parse_malformed_json(input).unwrap();
        let expected =
            flow_like_types::json::json!({"unicode": "こんにちは世界", "escaped": "line1\nline2"});
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_json_test_primitive_values() {
        let input = "42";
        let result = parse_malformed_json(input).unwrap();
        let expected = flow_like_types::json::json!(42);
        assert_eq!(result, expected);

        let input = "true";
        let result = parse_malformed_json(input).unwrap();
        let expected = flow_like_types::json::json!(true);
        assert_eq!(result, expected);
    }
}
