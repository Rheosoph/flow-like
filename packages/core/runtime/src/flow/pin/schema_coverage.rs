use flow_like_types::{Value, json::Map};
use std::collections::HashSet;

type Schema = Map<String, Value>;

const MAX_DEPTH: usize = 64;

/// Keywords that never restrict which values a schema admits.
const ANNOTATIONS: &[&str] = &[
    "$schema",
    "$id",
    "$anchor",
    "$comment",
    "$defs",
    "definitions",
    "title",
    "description",
    "default",
    "examples",
    "readOnly",
    "writeOnly",
    "deprecated",
];

/// Keywords whose meaning depends on the schema around them, so an identical keyword on the output
/// side proves nothing. A schema using one is only covered by an identical schema.
const CONTEXTUAL: &[&str] = &[
    "unevaluatedProperties",
    "unevaluatedItems",
    "$dynamicRef",
    "$recursiveRef",
];

/// Keywords compared structurally. Every other assertion the input makes must be repeated verbatim
/// by the output.
const STRUCTURAL: &[&str] = &[
    "$ref",
    "allOf",
    "anyOf",
    "oneOf",
    "type",
    "enum",
    "const",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "prefixItems",
    "minItems",
    "maxItems",
    "uniqueItems",
    "minLength",
    "maxLength",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
];

static ANY: Value = Value::Bool(true);

/// Whether every value the `output` schema admits is also admitted by `input`, so a pin declaring
/// `output` may feed a pin declaring `input`.
///
/// The output has to declare every property the input declares, with a schema that is itself
/// covered, and require at least what the input requires. It may declare more, and titles are
/// ignored: a producer struct with extra fields feeds a consumer that reads a subset of them.
///
/// Conservative: anything that cannot be proven covered is refused. Mirrored in TypeScript by
/// `schemaCovers` in `packages/ui/lib/schema-coverage.ts`.
pub fn schema_covers(output: &str, input: &str) -> bool {
    if output == input {
        return true;
    }
    let (Ok(mut output), Ok(mut input)) = (
        flow_like_types::json::from_str::<Value>(output),
        flow_like_types::json::from_str::<Value>(input),
    ) else {
        return false;
    };
    split_type_unions(&mut output);
    split_type_unions(&mut input);
    Coverage {
        output_root: &output,
        input_root: &input,
        assumed: HashSet::new(),
    }
    .covers(&output, &input, 0)
}

/// Keywords holding one sub-schema that coverage compares structurally.
const SUBSCHEMA_KEYWORDS: &[&str] = &["additionalProperties", "items"];
/// Keywords holding a map of sub-schemas that coverage compares structurally.
const SUBSCHEMA_MAP_KEYWORDS: &[&str] = &["properties", "$defs", "definitions"];
/// Keywords holding a list of sub-schemas that coverage compares structurally.
const SUBSCHEMA_LIST_KEYWORDS: &[&str] = &["allOf", "anyOf", "oneOf"];

/// Rewrite every `{type: [A, B, …], ..rest}` into `{anyOf: [{type: A, ..rest}, {type: B, ..rest}]}`.
///
/// A `type` list is a union, but coverage only splits explicit `anyOf`/`oneOf` outputs. Without
/// this, serde's `Option<Vec<T>>` (`{type: ["array", "null"], items: …}`) is never covered by the
/// `anyOf: [{type: array, …}, {type: null}]` the very same field projects to through FlowScript.
/// Annotations — `$defs` among them — stay on the outer object, so `$ref`s keep resolving. Both
/// roots are rewritten, so keywords compared verbatim still compare like with like; only
/// structurally compared sub-schemas are visited.
fn split_type_unions(schema: &mut Value) {
    match schema {
        Value::Object(fields) => {
            for (key, value) in fields.iter_mut() {
                let key = key.as_str();
                if SUBSCHEMA_KEYWORDS.contains(&key) && value.is_object() {
                    split_type_unions(value);
                } else if SUBSCHEMA_MAP_KEYWORDS.contains(&key)
                    && let Value::Object(members) = value
                {
                    members.values_mut().for_each(split_type_unions);
                } else if SUBSCHEMA_LIST_KEYWORDS.contains(&key)
                    && let Value::Array(members) = value
                {
                    members.iter_mut().for_each(split_type_unions);
                }
            }
            let Some(Value::Array(kinds)) = fields.get("type") else {
                return;
            };
            if kinds.len() < 2 {
                return;
            }
            let kinds = kinds.clone();
            let (annotations, rest): (Schema, Schema) = std::mem::take(fields)
                .into_iter()
                .filter(|(key, _)| key != "type")
                .partition(|(key, _)| ANNOTATIONS.contains(&key.as_str()));
            *fields = annotations;
            let variants = kinds
                .into_iter()
                .map(|kind| {
                    let mut variant = rest.clone();
                    variant.insert("type".to_string(), kind);
                    Value::Object(variant)
                })
                .collect();
            fields.insert("anyOf".to_string(), Value::Array(variants));
        }
        _ => {}
    }
}

struct Coverage<'a> {
    output_root: &'a Value,
    input_root: &'a Value,
    assumed: HashSet<(usize, usize)>,
}

impl<'a> Coverage<'a> {
    fn covers(&mut self, output_schema: &'a Value, input_schema: &'a Value, depth: usize) -> bool {
        let input = match input_schema {
            Value::Bool(admits) => return *admits || matches!(output_schema, Value::Bool(false)),
            Value::Object(input) => input,
            _ => return false,
        };
        if !constrains(input) {
            return true;
        }
        let output = match output_schema {
            Value::Bool(admits) => return !admits,
            Value::Object(output) => output,
            _ => return false,
        };
        if depth > MAX_DEPTH {
            return false;
        }
        // Only a recursive `$ref` can bring a pair back onto the stack; assuming it holds is what
        // lets two recursive types be compared at all.
        let pair = (
            output_schema as *const Value as usize,
            input_schema as *const Value as usize,
        );
        if !self.assumed.insert(pair) {
            return true;
        }
        let covered = self.covers_output(output_schema, output, input_schema, input, depth + 1);
        self.assumed.remove(&pair);
        covered
    }

    /// The output admits the intersection of its `$ref`, its `allOf` members and its own keywords,
    /// and the union of its `anyOf`/`oneOf` variants, so proving any one of those covered suffices.
    fn covers_output(
        &mut self,
        output_schema: &'a Value,
        output: &'a Schema,
        input_schema: &'a Value,
        input: &'a Schema,
        depth: usize,
    ) -> bool {
        if let Some(Some(target)) = reference(self.output_root, output)
            && self.covers(target, input_schema, depth)
        {
            return true;
        }
        if members(output, "allOf")
            .iter()
            .any(|member| self.covers(member, input_schema, depth))
        {
            return true;
        }
        for key in ["anyOf", "oneOf"] {
            if !members(output, key).is_empty()
                && members(output, key)
                    .iter()
                    .all(|variant| self.covers(variant, input_schema, depth))
            {
                return true;
            }
        }
        self.covers_input(output_schema, output, input, depth)
    }

    /// The input admits only the intersection of its `$ref`, its `allOf` members, one of its
    /// `anyOf`/`oneOf` variants and its own keywords, so each must hold. `oneOf` is read as `anyOf`:
    /// pin schemas describe serde enums, whose variants do not overlap.
    fn covers_input(
        &mut self,
        output_schema: &'a Value,
        output: &'a Schema,
        input: &'a Schema,
        depth: usize,
    ) -> bool {
        match reference(self.input_root, input) {
            Some(Some(target)) if !self.covers(output_schema, target, depth) => return false,
            Some(None) => return false,
            _ => {}
        }
        if !members(input, "allOf")
            .iter()
            .all(|member| self.covers(output_schema, member, depth))
        {
            return false;
        }
        for key in ["anyOf", "oneOf"] {
            if input.contains_key(key)
                && !members(input, key)
                    .iter()
                    .any(|variant| self.covers(output_schema, variant, depth))
            {
                return false;
            }
        }
        self.covers_keywords(output, input, depth)
    }

    fn covers_keywords(&mut self, output: &'a Schema, input: &'a Schema, depth: usize) -> bool {
        if CONTEXTUAL.iter().any(|key| input.contains_key(*key)) {
            return output == input && !input.values().any(mentions_reference);
        }

        let output_types = types(output);
        if let Some(input_types) = types(input) {
            let Some(output_types) = &output_types else {
                return false;
            };
            if !output_types.iter().all(|kind| {
                input_types.contains(kind)
                    || (*kind == "integer" && input_types.contains(&"number"))
            }) {
                return false;
            }
        }

        for key in ["enum", "const"] {
            let Some(admitted) = input.get(key) else {
                continue;
            };
            let admitted = match (key, admitted) {
                ("enum", Value::Array(values)) => values.iter().collect::<Vec<_>>(),
                ("enum", _) => return false,
                _ => vec![admitted],
            };
            let Some(produced) = literals(output) else {
                return false;
            };
            if !produced.iter().all(|value| admitted.contains(value)) {
                return false;
            }
        }

        if could_be(&output_types, "object") && !self.covers_object(output, input, depth) {
            return false;
        }
        if could_be(&output_types, "array") && !self.covers_array(output, input, depth) {
            return false;
        }
        if could_be(&output_types, "string")
            && !(at_least(output, input, "minLength") && at_most(output, input, "maxLength"))
        {
            return false;
        }
        if could_be(&output_types, "number") && !numeric_bounds(output, input) {
            return false;
        }

        input.iter().all(|(key, value)| {
            ANNOTATIONS.contains(&key.as_str())
                || STRUCTURAL.contains(&key.as_str())
                || verbatim(output.get(key), Some(value))
        })
    }

    fn covers_object(&mut self, output: &'a Schema, input: &'a Schema, depth: usize) -> bool {
        let output_required = strings(output, "required");
        if !strings(input, "required")
            .iter()
            .all(|name| output_required.contains(name))
        {
            return false;
        }

        let output_properties = output.get("properties").and_then(Value::as_object);
        let input_properties = input.get("properties").and_then(Value::as_object);
        for (name, admitted) in input_properties.into_iter().flatten() {
            let Some(produced) = output_properties.and_then(|properties| properties.get(name))
            else {
                return false;
            };
            if !self.covers(produced, admitted, depth) {
                return false;
            }
        }

        let Some(extra) = input
            .get("additionalProperties")
            .filter(|extra| **extra != Value::Bool(true))
        else {
            return true;
        };
        if output.contains_key("patternProperties") {
            return false;
        }
        let undeclared = output_properties
            .into_iter()
            .flatten()
            .filter(|(name, _)| input_properties.is_none_or(|input| !input.contains_key(*name)));
        for (_, produced) in undeclared {
            if !self.covers(produced, extra, depth) {
                return false;
            }
        }
        self.covers(
            output.get("additionalProperties").unwrap_or(&ANY),
            extra,
            depth,
        )
    }

    fn covers_array(&mut self, output: &'a Schema, input: &'a Schema, depth: usize) -> bool {
        if !verbatim(output.get("prefixItems"), input.get("prefixItems")) {
            return false;
        }
        match input.get("items") {
            None => {}
            Some(Value::Array(_)) => {
                if !verbatim(output.get("items"), input.get("items")) {
                    return false;
                }
            }
            Some(admitted) => match output.get("items") {
                Some(Value::Array(_)) => return false,
                produced => {
                    if !self.covers(produced.unwrap_or(&ANY), admitted, depth) {
                        return false;
                    }
                }
            },
        }
        at_least(output, input, "minItems")
            && at_most(output, input, "maxItems")
            && (input.get("uniqueItems") != Some(&Value::Bool(true))
                || output.get("uniqueItems") == Some(&Value::Bool(true)))
    }
}

fn constrains(schema: &Schema) -> bool {
    schema
        .keys()
        .any(|key| !ANNOTATIONS.contains(&key.as_str()))
}

/// `None` without a `$ref`, `Some(None)` for one that does not point into the document.
fn reference<'a>(root: &'a Value, schema: &Schema) -> Option<Option<&'a Value>> {
    let reference = schema.get("$ref")?;
    Some(
        reference
            .as_str()
            .and_then(|reference| reference.strip_prefix('#'))
            .and_then(|pointer| root.pointer(pointer)),
    )
}

fn members<'a>(schema: &'a Schema, key: &str) -> &'a [Value] {
    schema
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn types(schema: &Schema) -> Option<Vec<&str>> {
    match schema.get("type")? {
        Value::String(kind) => Some(vec![kind.as_str()]),
        Value::Array(kinds) => Some(kinds.iter().filter_map(Value::as_str).collect()),
        _ => Some(Vec::new()),
    }
}

fn could_be(types: &Option<Vec<&str>>, kind: &str) -> bool {
    types.as_ref().is_none_or(|types| {
        types.contains(&kind) || (kind == "number" && types.contains(&"integer"))
    })
}

fn literals(schema: &Schema) -> Option<Vec<&Value>> {
    if let Some(value) = schema.get("const") {
        return Some(vec![value]);
    }
    schema
        .get("enum")
        .and_then(Value::as_array)
        .map(|values| values.iter().collect())
}

fn strings<'a>(schema: &'a Schema, key: &str) -> Vec<&'a str> {
    members(schema, key)
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

fn at_least(output: &Schema, input: &Schema, key: &str) -> bool {
    let Some(limit) = input.get(key) else {
        return true;
    };
    let produced = output.get(key).map_or(Some(0.0), Value::as_f64);
    limit
        .as_f64()
        .zip(produced)
        .is_some_and(|(limit, produced)| produced >= limit)
}

fn at_most(output: &Schema, input: &Schema, key: &str) -> bool {
    let Some(limit) = input.get(key) else {
        return true;
    };
    limit
        .as_f64()
        .zip(output.get(key).and_then(Value::as_f64))
        .is_some_and(|(limit, produced)| produced <= limit)
}

fn numeric_bounds(output: &Schema, input: &Schema) -> bool {
    let number = |key: &str| output.get(key).and_then(Value::as_f64);
    let (floor, strict_floor) = (number("minimum"), number("exclusiveMinimum"));
    let (ceiling, strict_ceiling) = (number("maximum"), number("exclusiveMaximum"));
    let bounded = |key: &str, holds: &dyn Fn(f64) -> bool| {
        input
            .get(key)
            .is_none_or(|limit| limit.as_f64().is_some_and(holds))
    };
    bounded("minimum", &|limit| {
        floor.is_some_and(|floor| floor >= limit)
            || strict_floor.is_some_and(|floor| floor >= limit)
    }) && bounded("exclusiveMinimum", &|limit| {
        strict_floor.is_some_and(|floor| floor >= limit) || floor.is_some_and(|floor| floor > limit)
    }) && bounded("maximum", &|limit| {
        ceiling.is_some_and(|ceiling| ceiling <= limit)
            || strict_ceiling.is_some_and(|ceiling| ceiling <= limit)
    }) && bounded("exclusiveMaximum", &|limit| {
        strict_ceiling.is_some_and(|ceiling| ceiling <= limit)
            || ceiling.is_some_and(|ceiling| ceiling < limit)
    })
}

/// Identical keyword values, provided neither side leans on a `$ref`: the two schemas resolve
/// references against different documents, so identical text can name different shapes.
fn verbatim(output: Option<&Value>, input: Option<&Value>) -> bool {
    output == input && !input.is_some_and(mentions_reference)
}

fn mentions_reference(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            CONTEXTUAL
                .iter()
                .chain(&["$ref"])
                .any(|key| fields.contains_key(*key))
                || fields.values().any(mentions_reference)
        }
        Value::Array(values) => values.iter().any(mentions_reference),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::schema_covers;
    use flow_like_types::json::json;
    use schemars::{JsonSchema, schema_for};

    fn covers(output: flow_like_types::Value, input: flow_like_types::Value) -> bool {
        schema_covers(&output.to_string(), &input.to_string())
    }

    fn schema<T: JsonSchema>() -> flow_like_types::Value {
        flow_like_types::json::to_value(schema_for!(T)).unwrap()
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct Address {
        street: String,
        city: String,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct Customer {
        id: u64,
        name: String,
        email: Option<String>,
        address: Address,
        tags: Vec<String>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct CustomerName {
        name: String,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct CustomerCity {
        id: u64,
        address: CityOnly,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct CityOnly {
        city: String,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct OptionalContact {
        email: Option<String>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct RequiredContact {
        email: String,
    }

    #[derive(JsonSchema)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct StrictName {
        name: String,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct Tree {
        label: String,
        children: Vec<Tree>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct LabelledTree {
        label: String,
        weight: f64,
        children: Vec<LabelledTree>,
    }

    #[test]
    fn a_producer_with_more_fields_covers_a_consumer_reading_a_subset() {
        assert!(covers(schema::<Customer>(), schema::<CustomerName>()));
        assert!(covers(schema::<Customer>(), schema::<CustomerCity>()));
        assert!(covers(schema::<LabelledTree>(), schema::<Tree>()));
    }

    #[test]
    fn a_consumer_asking_for_more_than_the_producer_declares_is_refused() {
        assert!(!covers(schema::<CustomerName>(), schema::<Customer>()));
        assert!(!covers(schema::<Tree>(), schema::<LabelledTree>()));
        assert!(!covers(
            json!({"type":"object","properties":{"sub":{"type":"string"}}}),
            json!({"type":"object","properties":{"count":{"type":"number"}}}),
        ));
    }

    #[test]
    fn an_optional_field_covers_only_an_optional_one() {
        assert!(covers(
            schema::<RequiredContact>(),
            schema::<OptionalContact>()
        ));
        assert!(!covers(
            schema::<OptionalContact>(),
            schema::<RequiredContact>()
        ));
    }

    #[test]
    fn field_types_must_be_covered_too() {
        assert!(covers(
            json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}),
            json!({"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}),
        ));
        assert!(!covers(
            json!({"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}),
            json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}),
        ));
        assert!(!covers(
            json!({"type":"object","properties":{"n":{"type":"integer","format":"int64"}}}),
            json!({"type":"object","properties":{"n":{"type":"integer","format":"int32"}}}),
        ));
    }

    #[test]
    fn a_consumer_denying_unknown_fields_is_covered_only_by_the_same_fields() {
        assert!(!covers(schema::<Customer>(), schema::<StrictName>()));
        assert!(covers(schema::<StrictName>(), schema::<CustomerName>()));
        assert!(covers(
            json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false}),
            schema::<StrictName>(),
        ));
    }

    #[test]
    fn map_values_are_compared_covariantly() {
        let map =
            |value: flow_like_types::Value| json!({"type":"object","additionalProperties":value});
        assert!(covers(
            map(json!({"type":"integer"})),
            map(json!({"type":"number"}))
        ));
        assert!(!covers(
            map(json!({"type":"number"})),
            map(json!({"type":"integer"}))
        ));
        assert!(!covers(
            json!({"type":"object","properties":{"a":{"type":"string"}}}),
            map(json!({"type":"integer"}))
        ));
    }

    #[test]
    fn enums_and_unions_are_covered_by_their_subsets() {
        assert!(covers(
            json!({"type":"string","enum":["a"]}),
            json!({"type":"string","enum":["a","b"]}),
        ));
        assert!(!covers(
            json!({"type":"string","enum":["a","c"]}),
            json!({"type":"string","enum":["a","b"]}),
        ));
        assert!(!covers(
            json!({"type":"string"}),
            json!({"type":"string","enum":["a"]})
        ));
        assert!(covers(
            json!({"type":"string"}),
            json!({"type":["string","null"]})
        ));
        assert!(!covers(
            json!({"type":["string","null"]}),
            json!({"type":"string"})
        ));
        assert!(covers(
            json!({"anyOf":[{"type":"string"},{"type":"null"}]}),
            json!({"anyOf":[{"type":"null"},{"type":"string"}]}),
        ));
    }

    #[test]
    fn numeric_and_length_bounds_must_be_at_least_as_tight() {
        assert!(covers(
            json!({"type":"integer","minimum":1}),
            json!({"type":"integer","minimum":0})
        ));
        assert!(!covers(
            json!({"type":"integer"}),
            json!({"type":"integer","minimum":0})
        ));
        assert!(covers(
            json!({"type":"array","items":{"type":"string"},"minItems":2}),
            json!({"type":"array","items":{"type":"string"},"minItems":1}),
        ));
        assert!(!covers(
            json!({"type":"string"}),
            json!({"type":"string","maxLength":4})
        ));
    }

    #[test]
    fn identical_text_behind_a_ref_is_not_trusted_across_documents() {
        let output = json!({
            "type":"object",
            "properties":{"inner":{"not":{"$ref":"#/$defs/X"}}},
            "$defs":{"X":{"type":"string"}},
        });
        let input = json!({
            "type":"object",
            "properties":{"inner":{"not":{"$ref":"#/$defs/X"}}},
            "$defs":{"X":{"type":"integer"}},
        });
        assert!(!covers(output, input));
    }

    #[test]
    fn unknown_input_assertions_must_be_repeated_by_the_output() {
        assert!(covers(
            json!({"type":"string","pattern":"^a"}),
            json!({"type":"string","pattern":"^a"})
        ));
        assert!(!covers(
            json!({"type":"string"}),
            json!({"type":"string","pattern":"^a"})
        ));
        assert!(!covers(
            json!({"type":"object"}),
            json!({"not":{"type":"object"}})
        ));
    }

    #[test]
    fn unparseable_schemas_only_cover_themselves() {
        assert!(schema_covers("not json", "not json"));
        assert!(!schema_covers("not json", r#"{"type":"object"}"#));
        assert!(!schema_covers(r#"{"type":"object"}"#, "not json"));
    }

    #[test]
    fn a_bit_does_not_cover_a_cached_embedding_model() {
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct CachedEmbeddingModel {
            cache_key: String,
            model_type: crate::bit::BitTypes,
        }
        assert!(!covers(
            schema::<crate::bit::Bit>(),
            schema::<CachedEmbeddingModel>()
        ));
    }

    /// serde renders `Option<Vec<u8>>` as a type list; FlowScript interfaces project the same
    /// field as `anyOf`. A variable typed with the rendered `Bit` interface must accept a `Bit`.
    #[test]
    fn a_type_list_output_is_covered_by_the_equivalent_any_of() {
        let nullable_bytes = json!({"type": ["array", "null"], "items": {"type": "integer"}});
        let projection = json!({"anyOf": [
            {"type": "array", "items": {"type": "integer"}},
            {"type": "null"}
        ]});
        assert!(covers(nullable_bytes.clone(), projection));
        assert!(!covers(
            nullable_bytes,
            json!({"type": "array", "items": {"type": "integer"}})
        ));
        assert!(covers(
            json!({"type": ["string", "null"], "enum": ["a", null]}),
            json!({"type": ["string", "null"]})
        ));
        // Verbatim-compared keywords are left exactly as written.
        let tuple = json!({"type": "array", "prefixItems": [{"type": "string"}, {"type": ["integer", "null"]}]});
        let mut described = tuple.clone();
        described["description"] = json!("a pair");
        assert!(covers(tuple, described));
    }
}
