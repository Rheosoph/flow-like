//! Key resolution and typed comparison shared by every key-aware array node.

use crate::structs::fields::path_utils::{PathSegment, parse_path};
use crate::utils::datetime::util::{from_value, from_value_strict};
use flow_like::flow::{node::Node, pin::PinOptions, variable::VariableType};
use flow_like_types::{Value, json::json};
use std::cmp::Ordering;

pub const COMPARE_MODES: [&str; 7] = [
    "Auto",
    "Number",
    "Text",
    "Text (Natural)",
    "Text (Ignore Case)",
    "Date",
    "Boolean",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CompareMode {
    Auto,
    Number,
    Text,
    Natural,
    IgnoreCase,
    Date,
    Boolean,
}

impl CompareMode {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Number" => CompareMode::Number,
            "Text" => CompareMode::Text,
            "Text (Natural)" => CompareMode::Natural,
            "Text (Ignore Case)" => CompareMode::IgnoreCase,
            "Date" => CompareMode::Date,
            "Boolean" => CompareMode::Boolean,
            _ => CompareMode::Auto,
        }
    }

    /// The comparator a scalar array implies when the user left it on Auto.
    pub fn from_element_type(data_type: &VariableType) -> Self {
        match data_type {
            VariableType::Date => CompareMode::Date,
            VariableType::Integer | VariableType::Float | VariableType::Byte => CompareMode::Number,
            VariableType::Boolean => CompareMode::Boolean,
            VariableType::String => CompareMode::Auto,
            _ => CompareMode::Auto,
        }
    }
}

/// Resolves a field path against a value, sharing its grammar with
/// `struct_get` / `struct_set`: `customer.address.city`, `orders[0].total`, and
/// `items.0.price` all work. An empty path is the value itself. A path that does
/// not exist is `None`, which the null policy then places — distinct from a path
/// that exists and holds null.
pub fn resolve_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.trim();
    if path.is_empty() {
        return Some(value);
    }

    let mut current = value;
    for segment in parse_path(path) {
        current = match segment {
            PathSegment::ArrayIndex(index) => current.as_array()?.get(index)?,
            PathSegment::Field(name) => match current {
                Value::Object(map) => map.get(&name)?,
                // `items.0.price` — a bare number reads as an index on an array.
                Value::Array(items) => items.get(name.parse::<usize>().ok()?)?,
                _ => return None,
            },
        };
    }

    Some(current)
}

/// A resolved, comparable key. Variants are ordered so that mixed arrays still
/// sort deterministically instead of depending on element order.
pub enum SortKey {
    Boolean(bool),
    Number(f64),
    Date(i64),
    Text(String),
    Natural(String),
}

impl SortKey {
    fn rank(&self) -> u8 {
        match self {
            SortKey::Boolean(_) => 0,
            SortKey::Number(_) => 1,
            SortKey::Date(_) => 2,
            SortKey::Text(_) | SortKey::Natural(_) => 3,
        }
    }

    pub fn compare(&self, other: &SortKey) -> Ordering {
        match (self, other) {
            (SortKey::Boolean(left), SortKey::Boolean(right)) => left.cmp(right),
            (SortKey::Number(left), SortKey::Number(right)) => left.total_cmp(right),
            (SortKey::Date(left), SortKey::Date(right)) => left.cmp(right),
            (SortKey::Text(left), SortKey::Text(right)) => left.cmp(right),
            (SortKey::Natural(left), SortKey::Natural(right)) => natural_compare(left, right),
            (left, right) => left.rank().cmp(&right.rank()),
        }
    }
}

fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Turns a value into a comparable key, or `None` when it cannot participate —
/// a missing field, an explicit null, or a value the chosen mode cannot read.
pub fn sort_key(mode: CompareMode, value: Option<&Value>) -> Option<SortKey> {
    let value = value?;
    if value.is_null() {
        return None;
    }

    match mode {
        CompareMode::Number => as_number(value).map(SortKey::Number),
        CompareMode::Text => Some(SortKey::Text(as_text(value))),
        CompareMode::Natural => Some(SortKey::Natural(as_text(value))),
        CompareMode::IgnoreCase => Some(SortKey::Text(as_text(value).to_lowercase())),
        CompareMode::Boolean => value.as_bool().map(SortKey::Boolean),
        CompareMode::Date => from_value(value).map(|date| SortKey::Date(date.timestamp_millis())),
        CompareMode::Auto => Some(match value {
            Value::Bool(flag) => SortKey::Boolean(*flag),
            Value::Number(number) => SortKey::Number(number.as_f64().unwrap_or(f64::NAN)),
            _ => match from_value_strict(value) {
                Some(date) => SortKey::Date(date.timestamp_millis()),
                None => SortKey::Text(as_text(value)),
            },
        }),
    }
}

/// Orders two optional keys. Missing keys never interleave with present ones,
/// and the direction only flips the present values — "Missing Last" means last
/// in both directions, which is what every database does.
pub fn compare_keys(
    left: Option<&SortKey>,
    right: Option<&SortKey>,
    nulls_first: bool,
    descending: bool,
) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => {
            let ordering = left.compare(right);
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        }
        (None, None) => Ordering::Equal,
        (None, Some(_)) if nulls_first => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) if nulls_first => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
    }
}

/// "item2" before "item10": digit runs compare as numbers, everything else as text.
pub fn natural_compare(left: &str, right: &str) -> Ordering {
    let mut left = left.chars().peekable();
    let mut right = right.chars().peekable();

    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left_char), Some(right_char)) => {
                if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
                    let left_run = take_digits(&mut left);
                    let right_run = take_digits(&mut right);
                    let ordering = left_run
                        .trim_start_matches('0')
                        .len()
                        .cmp(&right_run.trim_start_matches('0').len())
                        .then_with(|| {
                            left_run
                                .trim_start_matches('0')
                                .cmp(right_run.trim_start_matches('0'))
                        });
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                } else {
                    let ordering = left_char.cmp(&right_char);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                    left.next();
                    right.next();
                }
            }
        }
    }
}

fn take_digits(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut run = String::new();
    while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
        run.push(chars.next().unwrap_or_default());
    }
    run
}

/// The key pin only makes sense when the elements can hold fields. Showing it on
/// a `String[]` is noise; hiding it on a `Struct[]` makes the node useless.
pub fn key_applies(element_type: &VariableType) -> bool {
    matches!(element_type, VariableType::Struct | VariableType::Generic)
}

/// Adds or removes the key pin without ever recreating an existing one — pin ids
/// are minted randomly, so a rebuild on every `on_update` would keep the board
/// dirty forever. A wired pin is never removed.
pub fn sync_key_pin(node: &mut Node, element_type: &VariableType, description: &str) {
    let needed = key_applies(element_type);
    let existing = node.get_pin_by_name("key").is_some();

    if needed == existing {
        return;
    }

    if needed {
        node.add_input_pin("key", "Key", description, VariableType::String)
            .set_default_value(Some(json!("")));
        return;
    }

    if node
        .get_pin_by_name("key")
        .is_some_and(|pin| pin.depends_on.is_empty() && pin.connected_to.is_empty())
    {
        node.pins.retain(|_, pin| pin.name != "key");
    }
}

pub const KEY_DESCRIPTION: &str = "Field to read from each element, dot notation for nested fields (customer.address.city). Empty uses the element itself";

pub fn add_compare_pins(node: &mut Node) {
    node.add_input_pin(
        "compare",
        "Compare As",
        "How the key values are ordered. Auto reads each value and falls back to text",
        VariableType::String,
    )
    .set_default_value(Some(json!("Auto")))
    .set_options(
        PinOptions::new()
            .set_valid_values(COMPARE_MODES.iter().map(|mode| mode.to_string()).collect())
            .build(),
    );
}

pub fn add_nulls_pin(node: &mut Node) {
    node.add_input_pin(
        "nulls",
        "Missing Values",
        "Where elements without a key value end up",
        VariableType::String,
    )
    .set_default_value(Some(json!("Last")))
    .set_options(
        PinOptions::new()
            .set_valid_values(vec!["Last".to_string(), "First".to_string()])
            .build(),
    );
}
