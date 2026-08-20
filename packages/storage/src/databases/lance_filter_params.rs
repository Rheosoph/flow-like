//! Parameter binding for LanceDB filter strings — the `only_if` predicate behind every
//! vector search, count and delete.
//!
//! LanceDB has no placeholder binding. `only_if` takes a SQL string that
//! `lance_datafusion::planner::Planner` parses, and that parser has no placeholder arm at
//! all: a `$name` reaching it fails as "not supported SQL in lance". So a value cannot be
//! bound by the engine the way `ctx.sql` binds one (see [`super::sql_params`]) — it has to
//! be substituted into the filter before Lance is handed the text.
//!
//! Substitution happens on the token stream, and neither on the raw text nor on a
//! re-rendered AST:
//!
//! * Placeholders are found with the tokenizer Lance itself parses with, so what this
//!   module calls a parameter is exactly what Lance would. A `$` inside a string literal,
//!   a backtick-quoted column or a comment is a `$`, not a placeholder, to both.
//! * Only the placeholder's own source span is replaced; every other byte of the filter is
//!   copied through untouched. Re-rendering the whole predicate would rewrite the author's
//!   own literals as a side effect of binding a value.
//! * The replacement is rendered here, escaping unconditionally. sqlparser's `Display` for
//!   a string literal is deliberately NOT used: it skips doubling a quote that follows a
//!   backslash, and the Lance dialect does not treat a backslash as an escape, so a value
//!   of `x\' OR true --` would render as a literal that closes itself and leaves `OR true`
//!   standing in the predicate.
//!
//! The result is that a parameter can only ever be the literal it sits in. It still cannot
//! be a column or a table name — those are identifiers, and no amount of binding makes a
//! caller-authored identifier safe.

use datafusion::sql::sqlparser::dialect::{Dialect, GenericDialect};
use datafusion::sql::sqlparser::keywords::Keyword;
use datafusion::sql::sqlparser::tokenizer::{Location, Token, Tokenizer};
use flow_like_types::{Result, Value, anyhow};
use std::any::TypeId;
use std::collections::HashMap;

use super::sql_params::{MAX_QUERY_PARAMS, resolve_declared};

/// The dialect Lance tokenizes a filter with.
///
/// Mirrors `lance_datafusion::sql::LanceDialect` method for method: a `GenericDialect` that
/// delegates only these three, so that everything else — unicode string literals, backslash
/// escapes — falls back to the trait defaults rather than to `GenericDialect`'s overrides.
/// Copying the delegation set exactly is the point: a filter has to tokenize here the way it
/// will tokenize inside Lance, or this module and the engine disagree about where a value
/// ends.
#[derive(Debug, Default)]
struct LanceFilterDialect(GenericDialect);

impl Dialect for LanceFilterDialect {
    fn dialect(&self) -> TypeId {
        self.0.dialect()
    }

    fn is_identifier_start(&self, ch: char) -> bool {
        self.0.is_identifier_start(ch)
    }

    fn is_identifier_part(&self, ch: char) -> bool {
        self.0.is_identifier_part(ch)
    }

    fn is_delimited_identifier_start(&self, ch: char) -> bool {
        ch == '`'
    }
}

/// One `$name` in the filter text, with the span it occupies and whether it stands directly
/// inside an `IN (…)` list — the only position where a value may expand to more than one
/// literal.
struct Occurrence {
    name: String,
    start: Location,
    end: Location,
    in_list: bool,
}

fn scan(filter: &str) -> Result<Vec<Occurrence>> {
    let dialect = LanceFilterDialect::default();
    let tokens = Tokenizer::new(&dialect, filter)
        .tokenize_with_location()
        .map_err(|error| anyhow!("Could not read the filter: {error}"))?;

    let mut occurrences: Vec<Occurrence> = Vec::new();
    let mut distinct: Vec<&str> = Vec::new();
    let mut previous: Option<Token> = None;
    let mut before_previous: Option<Token> = None;

    for token in &tokens {
        if matches!(token.token, Token::Whitespace(_)) {
            continue;
        }

        if let Token::Placeholder(raw) = &token.token {
            // `?` and `?N` tokenize as placeholders but carry no name, so no value can ever
            // be addressed to them.
            let Some(name) = raw.strip_prefix('$') else {
                return Err(anyhow!(
                    "Unsupported placeholder '{raw}' in the filter. Use a named placeholder like $customer_id, or a numbered one like $1"
                ));
            };
            if name.is_empty() {
                return Err(anyhow!(
                    "Found a bare '$' in the filter. Use a named placeholder like $customer_id, or a numbered one like $1"
                ));
            }

            if !distinct.contains(&name) {
                distinct.push(name);
            }

            occurrences.push(Occurrence {
                name: name.to_string(),
                start: token.span.start,
                end: token.span.end,
                in_list: matches!(previous, Some(Token::LParen))
                    && matches!(&before_previous, Some(Token::Word(word)) if word.keyword == Keyword::IN),
            });
        }

        before_previous = previous.take();
        previous = Some(token.token.clone());
    }

    if distinct.len() > MAX_QUERY_PARAMS {
        return Err(anyhow!(
            "The filter declares {} parameters, more than the {} supported in one predicate",
            distinct.len(),
            MAX_QUERY_PARAMS
        ));
    }

    Ok(occurrences)
}

/// Distinct placeholder names declared by `filter`, without the leading `$`, ordered by
/// first appearance. A placeholder repeated in the predicate is reported once — it resolves
/// to one value, bound at every occurrence.
///
/// Errors when the filter cannot be tokenized (a half-typed predicate, typically) or uses a
/// placeholder form that cannot be addressed by name. Callers deriving pins should treat an
/// error as "leave the current pins alone" rather than as a reason to drop them.
pub fn declared_placeholders(filter: &str) -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    for occurrence in scan(filter)? {
        if !names.contains(&occurrence.name) {
            names.push(occurrence.name);
        }
    }
    Ok(names)
}

/// The values for exactly the placeholders `filter` declares, ordered by first appearance.
/// See [`super::sql_params::resolve_query_params`], of which this is the Lance-dialect twin.
pub fn resolve_filter_params(filter: &str, supplied: &Value) -> Result<Vec<(String, Value)>> {
    resolve_declared("The filter", declared_placeholders(filter)?, supplied)
}

/// Substitutes every `$placeholder` in `filter` with the literal form of its value.
///
/// A filter that declares no placeholders is returned unchanged, so the call is safe to make
/// unconditionally — including on the predicates that were already hand-written before this
/// existed.
pub fn bind_filter_params(filter: &str, params: &[(String, Value)]) -> Result<String> {
    let occurrences = scan(filter)?;
    if occurrences.is_empty() {
        return Ok(filter.to_string());
    }

    let supplied: HashMap<&str, &Value> = params
        .iter()
        .map(|(name, value)| (name.as_str(), value))
        .collect();
    let offsets = byte_offsets(filter);

    let mut bound = String::with_capacity(filter.len());
    let mut cursor = 0usize;
    for occurrence in &occurrences {
        let Some(value) = supplied.get(occurrence.name.as_str()) else {
            return Err(anyhow!(
                "The filter declares ${} with no value supplied. Connect the matching parameter pin, or supply the name in the parameters object.",
                occurrence.name
            ));
        };

        let start = byte_offset(&offsets, occurrence.start, filter)?;
        let end = byte_offset(&offsets, occurrence.end, filter)?;
        bound.push_str(&filter[cursor..start]);
        bound.push_str(&render(&occurrence.name, value, occurrence.in_list)?);
        cursor = end;
    }
    bound.push_str(&filter[cursor..]);

    Ok(bound)
}

/// Byte offset of every `(line, column)` the tokenizer can report, including the one past
/// the last character. Columns count characters, not bytes, so a filter holding non-ASCII
/// text cannot be indexed by column directly.
fn byte_offsets(filter: &str) -> HashMap<(u64, u64), usize> {
    let mut offsets = HashMap::with_capacity(filter.len() + 1);
    let (mut line, mut column) = (1u64, 1u64);
    for (index, character) in filter.char_indices() {
        offsets.insert((line, column), index);
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    offsets.insert((line, column), filter.len());
    offsets
}

fn byte_offset(
    offsets: &HashMap<(u64, u64), usize>,
    location: Location,
    filter: &str,
) -> Result<usize> {
    offsets
        .get(&(location.line, location.column))
        .copied()
        .ok_or_else(|| {
            anyhow!(
                "Could not locate line {} column {} in the filter '{filter}'",
                location.line,
                location.column
            )
        })
}

fn render(name: &str, value: &Value, in_list: bool) -> Result<String> {
    let Value::Array(items) = value else {
        return render_scalar(name, value);
    };

    if !in_list {
        return Err(anyhow!(
            "Parameter ${name} is a list, which can only be bound directly inside an IN (...) list — write `column IN (${name})`"
        ));
    }

    // An empty set matches nothing, and `IN ()` is not a predicate Lance can parse. `IN
    // (NULL)` is: every comparison against it is unknown, so no row survives the filter.
    if items.is_empty() {
        return Ok("NULL".to_string());
    }

    let elements = items
        .iter()
        .map(|item| render_scalar(name, item))
        .collect::<Result<Vec<_>>>()?;
    Ok(elements.join(", "))
}

fn render_scalar(name: &str, value: &Value) -> Result<String> {
    Ok(match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(value) => value.to_string(),
        // A JSON number's text is already a SQL numeric literal, and cannot hold anything
        // else. A negative one binds as unary minus over a literal, which Lance folds.
        Value::Number(number) => number.to_string(),
        Value::String(text) => sql_string_literal(text),
        Value::Array(_) => {
            return Err(anyhow!(
                "Parameter ${name} holds a nested list, which has no filter literal"
            ));
        }
        Value::Object(_) => {
            return Err(anyhow!(
                "Parameter ${name} holds an object; bind the individual fields as separate parameters"
            ));
        }
    })
}

/// A single-quoted literal, doubling every quote unconditionally.
///
/// Complete for this dialect precisely because Lance does not honour backslash escapes:
/// doubling is the only way to write a quote, so it is the only sequence that needs
/// escaping and no other character can end the literal early.
fn sql_string_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    fn bind(filter: &str, params: Value) -> Result<String> {
        let resolved = resolve_filter_params(filter, &params)?;
        bind_filter_params(filter, &resolved)
    }

    #[test]
    fn a_filter_without_placeholders_is_returned_verbatim() {
        let filter = "`user id` = 'o''brien' AND rank > 3 -- $notaparam\n";
        assert_eq!(bind(filter, json!({})).expect("binds"), filter);
    }

    #[test]
    fn placeholders_are_declared_in_order_and_deduplicated() {
        assert_eq!(
            declared_placeholders("id = $id OR parent = $id AND rank > $min").expect("scans"),
            vec!["id".to_string(), "min".to_string()]
        );
    }

    #[test]
    fn a_dollar_inside_a_literal_or_a_quoted_column_is_not_a_placeholder() {
        assert!(
            declared_placeholders("note = '$id' AND `col$id` > 1 /* $id */")
                .expect("scans")
                .is_empty()
        );
    }

    #[test]
    fn a_quote_in_a_value_cannot_close_its_literal() {
        assert_eq!(
            bind("id = $id", json!({"id": "o'brien"})).expect("binds"),
            "id = 'o''brien'"
        );
        assert_eq!(
            bind("id = $id", json!({"id": "' OR true --"})).expect("binds"),
            "id = ''' OR true --'"
        );
    }

    /// The case that rules out re-rendering the filter through sqlparser's `Display`: it
    /// leaves a backslash-quote pair alone, and this dialect reads the backslash as an
    /// ordinary character, so the literal would end at that quote.
    #[test]
    fn a_backslash_before_a_quote_does_not_escape_it() {
        let bound = bind("id = $id", json!({"id": "x\\' OR true --"})).expect("binds");
        assert_eq!(bound, "id = 'x\\'' OR true --'");

        // What matters is that the value is still one literal to the tokenizer that Lance
        // parses with, and that the injected tail is part of it.
        assert!(declared_placeholders(&bound).expect("scans").is_empty());
        let dialect = LanceFilterDialect::default();
        let literals: Vec<String> = Tokenizer::new(&dialect, &bound)
            .tokenize()
            .expect("tokenizes")
            .into_iter()
            .filter_map(|token| match token {
                Token::SingleQuotedString(value) => Some(value),
                _ => None,
            })
            .collect();
        assert_eq!(literals, vec!["x\\' OR true --".to_string()]);
    }

    #[test]
    fn only_the_placeholder_span_is_rewritten() {
        assert_eq!(
            bind(
                "`Full Name` = 'a''b' AND id = $id AND rank > 3",
                json!({"id": "x"})
            )
            .expect("binds"),
            "`Full Name` = 'a''b' AND id = 'x' AND rank > 3"
        );
    }

    #[test]
    fn spans_survive_multibyte_text_and_newlines() {
        assert_eq!(
            bind("name = 'züge'\n  AND id = $id", json!({"id": "ä"})).expect("binds"),
            "name = 'züge'\n  AND id = 'ä'"
        );
    }

    #[test]
    fn scalars_bind_by_type() {
        assert_eq!(
            bind(
                "a = $s AND b = $n AND c = $f AND d = $t AND e = $z",
                json!({"s": "x", "n": -5, "f": 1.5, "t": true, "z": null})
            )
            .expect("binds"),
            "a = 'x' AND b = -5 AND c = 1.5 AND d = true AND e = NULL"
        );
    }

    #[test]
    fn a_list_expands_inside_an_in_list() {
        assert_eq!(
            bind("id IN ($ids)", json!({"ids": ["a", "b'c"]})).expect("binds"),
            "id IN ('a', 'b''c')"
        );
        assert_eq!(
            bind("id NOT IN ($ids)", json!({"ids": [1, 2]})).expect("binds"),
            "id NOT IN (1, 2)"
        );
    }

    #[test]
    fn an_empty_list_matches_nothing() {
        assert_eq!(
            bind("id IN ($ids)", json!({"ids": []})).expect("binds"),
            "id IN (NULL)"
        );
    }

    #[test]
    fn a_list_outside_an_in_list_is_rejected() {
        let error = bind("id = $ids", json!({"ids": ["a"]})).expect_err("rejected");
        assert!(error.to_string().contains("IN (...)"), "{error}");
    }

    #[test]
    fn a_missing_value_is_named() {
        let error = bind("id = $id AND rank > $min", json!({"id": "x"})).expect_err("rejected");
        assert!(error.to_string().contains("$min"), "{error}");
    }

    #[test]
    fn an_unnamed_placeholder_is_rejected_with_the_fix() {
        let error = declared_placeholders("id = ?").expect_err("rejected");
        assert!(error.to_string().contains("$customer_id"), "{error}");
    }

    #[test]
    fn objects_have_no_literal_form() {
        let error = bind("id = $id", json!({"id": {"a": 1}})).expect_err("rejected");
        assert!(error.to_string().contains("separate parameters"), "{error}");
    }
}
