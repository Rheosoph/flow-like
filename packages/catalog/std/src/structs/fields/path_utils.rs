use flow_like_types::{Result, Value, anyhow};

#[derive(Debug, Clone)]
enum PathSegment {
    Field(String),
    ArrayIndex(usize),
}

fn parse_path(path: &str) -> Vec<PathSegment> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !current.is_empty() {
                    segments.push(PathSegment::Field(current.clone()));
                    current.clear();
                }
            }
            '[' => {
                if !current.is_empty() {
                    segments.push(PathSegment::Field(current.clone()));
                    current.clear();
                }

                let mut index_str = String::new();
                while let Some(&next_ch) = chars.peek() {
                    if next_ch == ']' {
                        chars.next();
                        break;
                    }
                    index_str.push(chars.next().unwrap());
                }

                if let Ok(index) = index_str.parse::<usize>() {
                    segments.push(PathSegment::ArrayIndex(index));
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        segments.push(PathSegment::Field(current));
    }

    segments
}

pub fn get_value_by_path(value: &Value, path: &str) -> Option<Value> {
    let segments = parse_path(path);
    let mut current = value.clone();

    for segment in segments {
        match segment {
            PathSegment::Field(field_name) => {
                let obj = current.as_object()?;
                current = obj.get(&field_name)?.clone();
            }
            PathSegment::ArrayIndex(index) => {
                let arr = current.as_array()?;
                current = arr.get(index)?.clone();
            }
        }
    }

    Some(current)
}

pub fn has_value_by_path(value: &Value, path: &str) -> bool {
    get_value_by_path(value, path).is_some()
}

pub fn set_value_by_path(value: &mut Value, path: &str, new_value: Value) -> Result<()> {
    let segments = parse_path(path);

    if segments.is_empty() {
        *value = new_value;
        return Ok(());
    }

    let mut current = value;
    let last_index = segments.len() - 1;

    for (i, segment) in segments.iter().enumerate() {
        if i == last_index {
            match segment {
                PathSegment::Field(field_name) => {
                    if let Some(obj) = current.as_object_mut() {
                        obj.insert(field_name.clone(), new_value);
                        return Ok(());
                    } else {
                        return Err(anyhow!("Cannot set field '{}' on non-object", field_name));
                    }
                }
                PathSegment::ArrayIndex(index) => {
                    if let Some(arr) = current.as_array_mut() {
                        if *index < arr.len() {
                            arr[*index] = new_value;
                            return Ok(());
                        } else {
                            return Err(anyhow!("Array index {} out of bounds", index));
                        }
                    } else {
                        return Err(anyhow!("Cannot index non-array"));
                    }
                }
            }
        } else {
            match segment {
                PathSegment::Field(field_name) => {
                    if let Some(obj) = current.as_object_mut() {
                        if !obj.contains_key(field_name) {
                            obj.insert(field_name.clone(), Value::Object(Default::default()));
                        }
                        current = obj.get_mut(field_name).unwrap();
                    } else {
                        return Err(anyhow!(
                            "Cannot traverse field '{}' on non-object",
                            field_name
                        ));
                    }
                }
                PathSegment::ArrayIndex(index) => {
                    if let Some(arr) = current.as_array_mut() {
                        if *index < arr.len() {
                            current = &mut arr[*index];
                        } else {
                            return Err(anyhow!("Array index {} out of bounds", index));
                        }
                    } else {
                        return Err(anyhow!("Cannot index non-array"));
                    }
                }
            }
        }
    }

    Ok(())
}

pub fn remove_value_by_path(value: &mut Value, path: &str) -> Result<Option<Value>> {
    let segments = parse_path(path);

    if segments.is_empty() {
        return Err(anyhow!("Cannot remove root value"));
    }

    let mut current = value;
    let last_index = segments.len() - 1;

    for (i, segment) in segments.iter().enumerate() {
        if i == last_index {
            match segment {
                PathSegment::Field(field_name) => {
                    if let Some(obj) = current.as_object_mut() {
                        return Ok(obj.remove(field_name));
                    } else {
                        return Err(anyhow!(
                            "Cannot remove field '{}' from non-object",
                            field_name
                        ));
                    }
                }
                PathSegment::ArrayIndex(index) => {
                    if let Some(arr) = current.as_array_mut() {
                        if *index < arr.len() {
                            return Ok(Some(arr.remove(*index)));
                        } else {
                            return Err(anyhow!("Array index {} out of bounds", index));
                        }
                    } else {
                        return Err(anyhow!("Cannot index non-array"));
                    }
                }
            }
        } else {
            match segment {
                PathSegment::Field(field_name) => {
                    if let Some(obj) = current.as_object_mut() {
                        if let Some(next) = obj.get_mut(field_name) {
                            current = next;
                        } else {
                            return Err(anyhow!("Field '{}' not found", field_name));
                        }
                    } else {
                        return Err(anyhow!(
                            "Cannot traverse field '{}' on non-object",
                            field_name
                        ));
                    }
                }
                PathSegment::ArrayIndex(index) => {
                    if let Some(arr) = current.as_array_mut() {
                        if *index < arr.len() {
                            current = &mut arr[*index];
                        } else {
                            return Err(anyhow!("Array index {} out of bounds", index));
                        }
                    } else {
                        return Err(anyhow!("Cannot index non-array"));
                    }
                }
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    #[test]
    fn test_simple_field_access() {
        let data = json!({
            "name": "Alice",
            "age": 30
        });

        assert_eq!(get_value_by_path(&data, "name"), Some(json!("Alice")));
        assert_eq!(get_value_by_path(&data, "age"), Some(json!(30)));
    }

    #[test]
    fn test_nested_field_access() {
        let data = json!({
            "message": {
                "content": "Hi"
            }
        });

        assert_eq!(
            get_value_by_path(&data, "message.content"),
            Some(json!("Hi"))
        );
    }

    #[test]
    fn test_array_access() {
        let data = json!({
            "items": [1, 2, 3, 4]
        });

        assert_eq!(get_value_by_path(&data, "items[0]"), Some(json!(1)));
        assert_eq!(get_value_by_path(&data, "items[2]"), Some(json!(3)));
    }

    #[test]
    fn test_mixed_access() {
        let data = json!({
            "users": [
                {"name": "Alice", "age": 30},
                {"name": "Bob", "age": 25}
            ]
        });

        assert_eq!(
            get_value_by_path(&data, "users[0].name"),
            Some(json!("Alice"))
        );
        assert_eq!(get_value_by_path(&data, "users[1].age"), Some(json!(25)));
    }

    #[test]
    fn test_set_simple_field() {
        let mut data = json!({
            "name": "Alice"
        });

        set_value_by_path(&mut data, "name", json!("Bob")).unwrap();
        assert_eq!(data["name"], json!("Bob"));
    }

    #[test]
    fn test_set_nested_field() {
        let mut data = json!({
            "message": {
                "content": "Hi"
            }
        });

        set_value_by_path(&mut data, "message.content", json!("Hello")).unwrap();
        assert_eq!(data["message"]["content"], json!("Hello"));
    }

    #[test]
    fn test_set_array_element() {
        let mut data = json!({
            "items": [1, 2, 3]
        });

        set_value_by_path(&mut data, "items[1]", json!(99)).unwrap();
        assert_eq!(data["items"][1], json!(99));
    }

    #[test]
    fn test_has_value() {
        let data = json!({
            "message": {
                "content": "Hi"
            },
            "items": [1, 2, 3]
        });

        assert!(has_value_by_path(&data, "message.content"));
        assert!(has_value_by_path(&data, "items[0]"));
        assert!(!has_value_by_path(&data, "message.missing"));
        assert!(!has_value_by_path(&data, "items[10]"));
    }

    #[test]
    fn test_remove_simple_field() {
        let mut data = json!({
            "name": "Alice",
            "age": 30
        });

        let removed = remove_value_by_path(&mut data, "name").unwrap();
        assert_eq!(removed, Some(json!("Alice")));
        assert!(!has_value_by_path(&data, "name"));
        assert!(has_value_by_path(&data, "age"));
    }

    #[test]
    fn test_remove_nested_field() {
        let mut data = json!({
            "message": {
                "content": "Hi",
                "sender": "Bob"
            }
        });

        let removed = remove_value_by_path(&mut data, "message.content").unwrap();
        assert_eq!(removed, Some(json!("Hi")));
        assert!(!has_value_by_path(&data, "message.content"));
        assert!(has_value_by_path(&data, "message.sender"));
    }

    #[test]
    fn test_remove_array_element() {
        let mut data = json!({
            "items": [1, 2, 3]
        });

        let removed = remove_value_by_path(&mut data, "items[1]").unwrap();
        assert_eq!(removed, Some(json!(2)));
        assert_eq!(data["items"], json!([1, 3]));
    }

    #[test]
    fn test_remove_missing_field() {
        let mut data = json!({
            "name": "Alice"
        });

        let removed = remove_value_by_path(&mut data, "missing").unwrap();
        assert_eq!(removed, None);
    }
}
