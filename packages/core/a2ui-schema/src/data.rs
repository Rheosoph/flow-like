use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_value, to_value};
use std::collections::HashMap;

use super::BoundValue;

/// Data model entry
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DataEntry {
    pub key: String,
    pub value: Value,
}

impl DataEntry {
    pub fn new(key: impl Into<String>, value: Value) -> Self {
        Self {
            key: key.into(),
            value,
        }
    }
}

/// Data model for a surface - holds the reactive data
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DataModel {
    data: HashMap<String, Value>,
}

impl DataModel {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    pub fn from_entries(entries: Vec<DataEntry>) -> Self {
        let mut model = Self::new();
        for entry in entries {
            model.set(&entry.key, entry.value);
        }
        model
    }

    pub fn get(&self, path: &str) -> Option<&Value> {
        if path.is_empty() || path == "/" {
            return None;
        }

        let path = path.strip_prefix('/').unwrap_or(path);
        let parts: Vec<&str> = path.split('/').collect();

        if parts.is_empty() {
            return None;
        }

        let root_key = parts[0];
        let root_value = self.data.get(root_key)?;

        if parts.len() == 1 {
            return Some(root_value);
        }

        Self::traverse_path(root_value, &parts[1..])
    }

    pub fn get_typed<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Option<T> {
        self.get(path).and_then(|v| from_value(v.clone()).ok())
    }

    pub fn set(&mut self, path: &str, value: Value) {
        if path.is_empty() || path == "/" {
            return;
        }

        let path = path.strip_prefix('/').unwrap_or(path);
        let parts: Vec<&str> = path.split('/').collect();

        if parts.is_empty() {
            return;
        }

        if parts.len() == 1 {
            self.data.insert(parts[0].to_string(), value);
            return;
        }

        let root_key = parts[0].to_string();
        let root_value = self
            .data
            .entry(root_key)
            .or_insert_with(|| Value::Object(Default::default()));
        Self::set_nested(root_value, &parts[1..], value);
    }

    pub fn set_typed<T: Serialize>(&mut self, path: &str, value: T) {
        if let Ok(v) = to_value(value) {
            self.set(path, v);
        }
    }

    pub fn remove(&mut self, path: &str) -> Option<Value> {
        if path.is_empty() || path == "/" {
            return None;
        }

        let path = path.strip_prefix('/').unwrap_or(path);
        let parts: Vec<&str> = path.split('/').collect();

        if parts.is_empty() {
            return None;
        }

        if parts.len() == 1 {
            return self.data.remove(parts[0]);
        }

        let root_key = parts[0];
        let root_value = self.data.get_mut(root_key)?;
        Self::remove_nested(root_value, &parts[1..])
    }

    pub fn resolve_bound_value(&self, bound: &BoundValue) -> Value {
        match bound {
            BoundValue::LiteralString { value } => Value::String(value.clone()),
            BoundValue::LiteralNumber { value } => Value::Number(
                serde_json::Number::from_f64(*value).unwrap_or(serde_json::Number::from(0)),
            ),
            BoundValue::LiteralBool { value } => Value::Bool(*value),
            BoundValue::LiteralOptions { value } => {
                serde_json::to_value(value).unwrap_or(Value::Null)
            }
            BoundValue::LiteralJson { value } => serde_json::from_str(value).unwrap_or(Value::Null),
            BoundValue::PathBinding(pb) => self.get(&pb.path).cloned().unwrap_or_else(|| match &pb
                .default_value
            {
                Some(super::PathDefault::String(s)) => Value::String(s.clone()),
                Some(super::PathDefault::Number(n)) => Value::Number(
                    serde_json::Number::from_f64(*n).unwrap_or(serde_json::Number::from(0)),
                ),
                Some(super::PathDefault::Bool(b)) => Value::Bool(*b),
                None => Value::Null,
            }),
        }
    }

    pub fn to_entries(&self) -> Vec<DataEntry> {
        self.data
            .iter()
            .map(|(k, v)| DataEntry::new(k.clone(), v.clone()))
            .collect()
    }

    pub fn merge(&mut self, other: DataModel) {
        for (key, value) in other.data {
            self.data.insert(key, value);
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.data.keys()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn traverse_path<'a>(value: &'a Value, parts: &[&str]) -> Option<&'a Value> {
        if parts.is_empty() {
            return Some(value);
        }

        match value {
            Value::Object(map) => {
                let next = map.get(parts[0])?;
                Self::traverse_path(next, &parts[1..])
            }
            Value::Array(arr) => {
                let index: usize = parts[0].parse().ok()?;
                let next = arr.get(index)?;
                Self::traverse_path(next, &parts[1..])
            }
            _ => None,
        }
    }

    fn set_nested(value: &mut Value, parts: &[&str], new_value: Value) {
        if parts.is_empty() {
            return;
        }

        if parts.len() == 1 {
            match value {
                Value::Object(map) => {
                    map.insert(parts[0].to_string(), new_value);
                }
                Value::Array(arr) => {
                    if let Ok(index) = parts[0].parse::<usize>() {
                        if index < arr.len() {
                            arr[index] = new_value;
                        } else {
                            while arr.len() < index {
                                arr.push(Value::Null);
                            }
                            arr.push(new_value);
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        match value {
            Value::Object(map) => {
                let next = map
                    .entry(parts[0].to_string())
                    .or_insert_with(|| Value::Object(Default::default()));
                Self::set_nested(next, &parts[1..], new_value);
            }
            Value::Array(arr) => {
                if let Ok(index) = parts[0].parse::<usize>() {
                    while arr.len() <= index {
                        arr.push(Value::Object(Default::default()));
                    }
                    Self::set_nested(&mut arr[index], &parts[1..], new_value);
                }
            }
            _ => {}
        }
    }

    fn remove_nested(value: &mut Value, parts: &[&str]) -> Option<Value> {
        if parts.is_empty() {
            return None;
        }

        if parts.len() == 1 {
            match value {
                Value::Object(map) => map.remove(parts[0]),
                Value::Array(arr) => {
                    let index: usize = parts[0].parse().ok()?;
                    if index < arr.len() {
                        Some(arr.remove(index))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else {
            match value {
                Value::Object(map) => {
                    let next = map.get_mut(parts[0])?;
                    Self::remove_nested(next, &parts[1..])
                }
                Value::Array(arr) => {
                    let index: usize = parts[0].parse().ok()?;
                    let next = arr.get_mut(index)?;
                    Self::remove_nested(next, &parts[1..])
                }
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_simple_path() {
        let mut model = DataModel::new();
        model.set("/name", json!("Alice"));

        assert_eq!(model.get("/name"), Some(&json!("Alice")));
        assert_eq!(model.get("name"), Some(&json!("Alice")));
    }

    #[test]
    fn test_get_nested_path() {
        let mut model = DataModel::new();
        model.set("/user", json!({"profile": {"name": "Bob"}}));

        assert_eq!(model.get("/user/profile/name"), Some(&json!("Bob")));
    }

    #[test]
    fn test_set_nested_path() {
        let mut model = DataModel::new();
        model.set("/user/profile/name", json!("Charlie"));

        assert_eq!(model.get("/user/profile/name"), Some(&json!("Charlie")));
    }

    #[test]
    fn test_resolve_bound_value() {
        let mut model = DataModel::new();
        model.set("/greeting", json!("Hello"));

        assert_eq!(
            model.resolve_bound_value(&BoundValue::path("/greeting")),
            json!("Hello")
        );
        assert_eq!(
            model.resolve_bound_value(&BoundValue::literal_string("World")),
            json!("World")
        );
    }

    #[test]
    fn test_bound_value_serde() {
        use super::super::{PathBinding, PathDefault};

        let lit_str = BoundValue::literal_string("test");
        let lit_num = BoundValue::literal_number(42.0);
        let path_only = BoundValue::path("/data");
        let path_with_default = BoundValue::PathBinding(PathBinding {
            path: "/value".to_string(),
            default_value: Some(PathDefault::String("preview".to_string())),
        });

        let lit_str_json = serde_json::to_string(&lit_str).unwrap();
        let lit_num_json = serde_json::to_string(&lit_num).unwrap();
        let path_only_json = serde_json::to_string(&path_only).unwrap();
        let path_default_json = serde_json::to_string(&path_with_default).unwrap();

        println!("literalString: {}", lit_str_json);
        println!("literalNumber: {}", lit_num_json);
        println!("path only: {}", path_only_json);
        println!("path with default: {}", path_default_json);

        assert_eq!(lit_str_json, r#"{"literalString":"test"}"#);
        assert_eq!(lit_num_json, r#"{"literalNumber":42.0}"#);
        assert_eq!(path_only_json, r#"{"path":"/data"}"#);
    }
}
