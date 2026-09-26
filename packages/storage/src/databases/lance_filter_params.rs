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

use anyhow::{Result, anyhow};
use serde_json::Value;
use sqlparser::dialect::{Dialect, GenericDialect};
use sqlparser::keywords::Keyword;
use sqlparser::tokenizer::{Location, Token, Tokenizer};
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
    in_wkt_call: bool,
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
                in_wkt_call: matches!(previous, Some(Token::LParen))
                    && matches!(&before_previous, Some(Token::Word(word))
                        if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("st_geomfromtext")),
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
        bound.push_str(&render(
            &occurrence.name,
            value,
            occurrence.in_list,
            occurrence.in_wkt_call,
        )?);
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

fn render(name: &str, value: &Value, in_list: bool, in_wkt_call: bool) -> Result<String> {
    if value.is_object() {
        return render_geometry(name, value, in_wkt_call);
    }
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

/// A GeoJSON geometry, as a Geometry pin supplies it, binds as WKT through Lance's
/// `ST_GeomFromText`, or as the bare WKT literal when the placeholder already is that
/// call's argument.
#[cfg(feature = "database-runtime")]
fn render_geometry(name: &str, value: &Value, in_wkt_call: bool) -> Result<String> {
    if !crate::geometry::names_geometry_kind(value) {
        return Err(object_parameter_error(name));
    }
    let wkt = flow_like_geometry::to_wkt(value)
        .map_err(|error| anyhow!("Parameter ${name} is not a valid geometry: {error}"))?;
    let literal = sql_string_literal(&wkt);
    Ok(if in_wkt_call {
        literal
    } else {
        format!("ST_GeomFromText({literal})")
    })
}

#[cfg(not(feature = "database-runtime"))]
fn render_geometry(name: &str, _value: &Value, _in_wkt_call: bool) -> Result<String> {
    Err(object_parameter_error(name))
}

fn object_parameter_error(name: &str) -> anyhow::Error {
    anyhow!(
        "Parameter ${name} holds an object that is not a GeoJSON geometry; bind the individual fields as separate parameters"
    )
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
        Value::Object(_) => return Err(object_parameter_error(name)),
    })
}

const CONVERSE_RELATIONS: [(&str, &str); 4] = [
    ("st_contains", "ST_Within"),
    ("st_within", "ST_Contains"),
    ("st_covers", "ST_CoveredBy"),
    ("st_coveredby", "ST_Covers"),
];

/// Rewrites `ST_Contains(<constant>, <column expression>)`, and likewise `ST_Within`,
/// `ST_Covers` and `ST_CoveredBy`, into the converse relation with its arguments swapped.
///
/// Lance evaluates spatial filters with geodatafusion, which relates a constant first
/// argument to a column in reverse and so answers the converse question; the column-first
/// form is evaluated correctly and still uses an RTree index. Only the function name and the
/// two argument spans are rewritten; every other byte of the filter is kept. A first argument
/// that might reference a column is left alone.
pub fn orient_spatial_relations(filter: &str) -> Result<String> {
    let lowered = filter.to_ascii_lowercase();
    if !CONVERSE_RELATIONS
        .iter()
        .any(|(name, _)| lowered.contains(name))
    {
        return Ok(filter.to_string());
    }

    let dialect = LanceFilterDialect::default();
    let tokens: Vec<_> = Tokenizer::new(&dialect, filter)
        .tokenize_with_location()
        .map_err(|error| anyhow!("Could not read the filter: {error}"))?
        .into_iter()
        .filter(|token| !matches!(token.token, Token::Whitespace(_)))
        .collect();
    let offsets = byte_offsets(filter);
    let at = |location| byte_offset(&offsets, location, filter);

    let mut oriented = String::with_capacity(filter.len());
    let mut cursor = 0usize;
    let mut index = 0usize;
    while index < tokens.len() {
        let converse = match &tokens[index].token {
            Token::Word(word) if word.quote_style.is_none() => CONVERSE_RELATIONS
                .iter()
                .find(|(name, _)| word.value.eq_ignore_ascii_case(name))
                .map(|(_, converse)| *converse),
            _ => None,
        };
        let arguments = converse.and_then(|_| two_arguments(&tokens, index + 1));
        let (Some(converse), Some((comma, close))) = (converse, arguments) else {
            index += 1;
            continue;
        };
        let (first, second) = (&tokens[index + 2..comma], &tokens[comma + 1..close]);
        if references_column(first) || !references_column(second) {
            index = close + 1;
            continue;
        }

        let first = at(first[0].span.start)?..at(first[first.len() - 1].span.end)?;
        let second = at(second[0].span.start)?..at(second[second.len() - 1].span.end)?;
        oriented.push_str(&filter[cursor..at(tokens[index].span.start)?]);
        oriented.push_str(converse);
        oriented.push_str(&filter[at(tokens[index].span.end)?..first.start]);
        oriented.push_str(&filter[second.clone()]);
        oriented.push_str(&filter[first.end..second.start]);
        oriented.push_str(&filter[first]);
        cursor = second.end;
        index = close + 1;
    }
    oriented.push_str(&filter[cursor..]);

    Ok(oriented)
}

/// Positions of the separating comma and the closing parenthesis when `tokens[open]` opens
/// a call with exactly two non-empty arguments.
fn two_arguments(
    tokens: &[sqlparser::tokenizer::TokenWithSpan],
    open: usize,
) -> Option<(usize, usize)> {
    if !matches!(tokens.get(open)?.token, Token::LParen) {
        return None;
    }
    let mut depth = 0usize;
    let mut comma = None;
    for (position, token) in tokens.iter().enumerate().skip(open + 1) {
        match token.token {
            Token::LParen => depth += 1,
            Token::RParen if depth > 0 => depth -= 1,
            Token::RParen => {
                let comma = comma?;
                return (comma > open + 1 && position > comma + 1).then_some((comma, position));
            }
            Token::Comma if depth == 0 => {
                if comma.is_some() {
                    return None;
                }
                comma = Some(position);
            }
            _ => {}
        }
    }
    None
}

/// Whether an argument may read a column: any word that is neither a function name nor a
/// NULL/TRUE/FALSE literal counts, so an unrecognized form is treated as a column.
fn references_column(tokens: &[sqlparser::tokenizer::TokenWithSpan]) -> bool {
    tokens.iter().enumerate().any(|(position, token)| {
        let Token::Word(word) = &token.token else {
            return false;
        };
        let is_call = word.quote_style.is_none()
            && matches!(
                tokens.get(position + 1).map(|next| &next.token),
                Some(Token::LParen)
            );
        !is_call && !matches!(word.keyword, Keyword::NULL | Keyword::TRUE | Keyword::FALSE)
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
    use serde_json::json;

    fn bind(filter: &str, params: Value) -> Result<String> {
        let resolved = resolve_filter_params(filter, &params)?;
        bind_filter_params(filter, &resolved)
    }

    #[test]
    fn constant_first_relations_are_rewritten_column_first() {
        let area = "ST_GeomFromText('POLYGON ((0 0,4 0,4 4,0 4,0 0))')";
        for (filter, expected) in [
            (
                format!("kind = 'a' AND ST_Contains({area}, geometry) -- st_contains"),
                format!("kind = 'a' AND ST_Within(geometry, {area}) -- st_contains"),
            ),
            (
                format!("st_within({area},geometry)"),
                format!("ST_Contains(geometry,{area})"),
            ),
            (
                format!("ST_Covers({area}, `geo col`) OR ST_CoveredBy({area}, geo)"),
                format!("ST_CoveredBy(`geo col`, {area}) OR ST_Covers(geo, {area})"),
            ),
        ] {
            assert_eq!(
                orient_spatial_relations(&filter).expect("orients"),
                expected
            );
        }
    }

    #[test]
    fn relations_that_already_evaluate_correctly_are_left_alone() {
        let area = "ST_GeomFromText('POINT (1 2)')";
        for filter in [
            format!("ST_Contains(geometry, {area})"),
            format!("ST_Contains(ST_Buffer(geometry, 1), {area})"),
            format!("ST_Contains({area}, {area})"),
            format!("ST_Intersects({area}, geometry)"),
            format!("ST_Contains({area}, geometry, 1)"),
            "note = 'ST_Contains(a, b)'".to_string(),
            "`st_contains`(a, b)".to_string(),
        ] {
            assert_eq!(orient_spatial_relations(&filter).expect("orients"), filter);
        }
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

    #[cfg(feature = "database-runtime")]
    #[test]
    fn geometry_parameters_bind_as_wkt_geometries() {
        let area = json!({"type": "Polygon", "coordinates": [[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]]]});
        let wkt = flow_like_geometry::to_wkt(&area).expect("valid");
        assert_eq!(
            bind("ST_Intersects(geometry, $area)", json!({"area": area})).expect("binds"),
            format!("ST_Intersects(geometry, ST_GeomFromText('{wkt}'))")
        );
        assert_eq!(
            bind(
                "ST_Within(geometry, st_geomfromtext( $area ))",
                json!({"area": area})
            )
            .expect("binds"),
            format!("ST_Within(geometry, st_geomfromtext( '{wkt}' ))")
        );

        let error = bind(
            "ST_Intersects(geometry, $area)",
            json!({"area": {"type": "Point", "coordinates": [500.0, 0.0]}}),
        )
        .expect_err("outside WGS 84");
        assert!(
            error.to_string().contains("not a valid geometry"),
            "{error}"
        );
    }
    /// The single invariant everything else rests on: whatever the value was, the filter Lance
    /// tokenizes carries it back exactly, as one literal.
    fn round_trip(value: &str) -> String {
        let bound = bind("id = $id", json!({ "id": value })).expect("binds");
        let dialect = LanceFilterDialect::default();
        let literals: Vec<String> = Tokenizer::new(&dialect, &bound)
            .tokenize()
            .unwrap_or_else(|error| panic!("{bound:?} does not tokenize: {error}"))
            .into_iter()
            .filter_map(|token| match token {
                Token::SingleQuotedString(value) => Some(value),
                _ => None,
            })
            .collect();
        assert_eq!(
            literals.len(),
            1,
            "{bound:?} did not hold exactly one literal"
        );
        literals.into_iter().next().expect("literal")
    }

    #[test]
    fn adversarial_values_round_trip_as_one_literal() {
        for value in [
            "plain",
            "o'brien",
            "' OR true --",
            "'; DROP TABLE t; --",
            // Already-doubled quotes: sqlparser's Display would pass these through as-is and
            // Lance would read them back as a single quote, silently changing the value.
            "a''b",
            // A backslash is an ordinary character in this dialect, in every position.
            "x\\'",
            "x\\",
            "back`tick",
            "double\"quote",
            "new\nline",
            "ünïcøde 🎈",
            "",
        ] {
            assert_eq!(round_trip(value), value, "value did not survive binding");
        }
    }

    #[test]
    fn lance_double_equals_is_left_alone() {
        // Lance accepts `==` by rewriting its own token stream. Binding never re-renders the
        // predicate, so the operator reaches Lance exactly as written.
        assert_eq!(
            bind("id == $id", json!({ "id": "x" })).expect("binds"),
            "id == 'x'"
        );
    }

    #[test]
    fn an_in_list_binds_without_surrounding_whitespace() {
        assert_eq!(
            bind("id IN($ids)", json!({ "ids": [1, 2] })).expect("binds"),
            "id IN(1, 2)"
        );
        assert_eq!(
            bind("id IN (\n  $ids\n)", json!({ "ids": ["a"] })).expect("binds"),
            "id IN (\n  'a'\n)"
        );
    }

    #[test]
    fn a_repeated_placeholder_binds_at_every_occurrence() {
        assert_eq!(
            bind("a = $q OR b = $q OR c = $q", json!({ "q": "x'y" })).expect("binds"),
            "a = 'x''y' OR b = 'x''y' OR c = 'x''y'"
        );
    }

    #[test]
    fn a_double_quoted_string_is_data_to_this_dialect() {
        // The opposite of DataFusion, where `"col"` is an identifier. Either way the `$` inside
        // is not a placeholder — but only this tokenizer can say so for a filter.
        assert!(
            declared_placeholders("note = \"$id\"")
                .expect("scans")
                .is_empty()
        );
    }

    #[test]
    fn declaring_more_parameters_than_the_cap_is_rejected() {
        let filter = (0..=MAX_QUERY_PARAMS)
            .map(|index| format!("c{index} = $p{index}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let error = declared_placeholders(&filter).expect_err("rejected");
        assert!(error.to_string().contains("more than the"), "{error}");
    }

    #[test]
    fn numbers_keep_their_json_form() {
        assert_eq!(
            bind(
                "a = $big AND b = $small AND c = $neg",
                json!({ "big": 9_007_199_254_740_993i64, "small": 0.125, "neg": -42 })
            )
            .expect("binds"),
            "a = 9007199254740993 AND b = 0.125 AND c = -42"
        );
    }

    #[test]
    fn a_nested_list_has_no_literal_form() {
        let error = bind("id IN ($ids)", json!({ "ids": [["a"]] })).expect_err("rejected");
        assert!(error.to_string().contains("nested list"), "{error}");
    }
}
