use std::collections::BTreeMap;

use crate::errors::TgError;

/// Parse a dot-path key into segments.
/// The first segment must start with `x-`.
pub fn parse_dot_path(key: &str) -> Result<Vec<String>, TgError> {
    let segments: Vec<String> = key.split('.').map(|s| s.to_string()).collect();
    if segments.is_empty() || segments[0].is_empty() {
        return Err(TgError::InvalidInput(
            "Extension key cannot be empty".to_string(),
        ));
    }
    if !segments[0].starts_with("x-") {
        return Err(TgError::InvalidInput(format!(
            "Extension key must start with 'x-', got '{}'",
            segments[0]
        )));
    }
    for seg in &segments {
        if seg.is_empty() {
            return Err(TgError::InvalidInput(format!(
                "Extension key has empty segment: '{}'",
                key
            )));
        }
    }
    Ok(segments)
}

/// Parse a value string into a serde_json::Value.
/// Try JSON parse first (numbers, booleans, objects, arrays, null),
/// then fall back to string literal.
pub fn parse_value(value: &str) -> serde_json::Value {
    // Try JSON parse first
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(value) {
        // Only accept structured JSON types, not plain strings
        // (strings would always succeed since any input is a valid JSON string if quoted)
        match &v {
            serde_json::Value::Number(_)
            | serde_json::Value::Bool(_)
            | serde_json::Value::Null
            | serde_json::Value::Array(_)
            | serde_json::Value::Object(_) => return v,
            serde_json::Value::String(_) => {
                // Only return if the input was actually a quoted JSON string
                if value.starts_with('"') {
                    return v;
                }
                // Otherwise fall through to string literal
            }
        }
    }
    // Fallback: treat as string literal
    serde_json::Value::String(value.to_string())
}

/// Parse a `key=value` pair from a `--set` flag.
/// Returns (dot-path segments, Option<Value>). None value means delete.
pub fn parse_set_flag(input: &str) -> Result<(Vec<String>, Option<serde_json::Value>), TgError> {
    let Some(eq_pos) = input.find('=') else {
        return Err(TgError::InvalidInput(format!(
            "Invalid --set format '{}': expected KEY=VALUE or KEY= (to delete)",
            input
        )));
    };
    let key = &input[..eq_pos];
    let value = &input[eq_pos + 1..];
    let segments = parse_dot_path(key)?;
    if value.is_empty() {
        Ok((segments, None)) // delete
    } else {
        Ok((segments, Some(parse_value(value))))
    }
}

/// Set a value at a dot-path in the extensions map.
/// Creates intermediate objects as needed. Overwrites non-object values.
pub fn set_nested(
    extensions: &mut BTreeMap<String, serde_json::Value>,
    segments: &[String],
    value: serde_json::Value,
) {
    assert!(!segments.is_empty());
    if segments.len() == 1 {
        extensions.insert(segments[0].clone(), value);
        return;
    }

    // Navigate/create intermediate objects
    let root_key = &segments[0];
    let entry = extensions
        .entry(root_key.clone())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

    // If existing value is not an object, overwrite with empty object
    if !entry.is_object() {
        *entry = serde_json::Value::Object(serde_json::Map::new());
    }

    let mut current = entry;
    for seg in &segments[1..segments.len() - 1] {
        // Ensure current is an object and navigate into it
        if !current.is_object() {
            *current = serde_json::Value::Object(serde_json::Map::new());
        }
        current = current
            .as_object_mut()
            .unwrap()
            .entry(seg.clone())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if !current.is_object() {
            *current = serde_json::Value::Object(serde_json::Map::new());
        }
    }

    // Set the leaf value
    current
        .as_object_mut()
        .unwrap()
        .insert(segments.last().unwrap().clone(), value);
}

/// Delete a key at a dot-path and recursively clean up empty parent objects.
pub fn delete_nested(
    extensions: &mut BTreeMap<String, serde_json::Value>,
    segments: &[String],
) {
    assert!(!segments.is_empty());
    if segments.len() == 1 {
        extensions.remove(&segments[0]);
        return;
    }

    // Navigate to the parent of the target key
    let root_key = &segments[0];
    let Some(root_val) = extensions.get_mut(root_key) else {
        return; // nothing to delete
    };

    if delete_in_value(root_val, &segments[1..]) {
        // Root value became empty, remove it
        extensions.remove(root_key);
    }
}

/// Recursively delete within a Value::Object. Returns true if the value should be removed
/// (because it's now an empty object).
fn delete_in_value(value: &mut serde_json::Value, segments: &[String]) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false; // not an object, nothing to delete
    };

    if segments.len() == 1 {
        obj.remove(&segments[0]);
        return obj.is_empty();
    }

    let key = &segments[0];
    let Some(child) = obj.get_mut(key) else {
        return false; // path doesn't exist
    };

    if delete_in_value(child, &segments[1..]) {
        obj.remove(key);
        return obj.is_empty();
    }

    false
}

/// Apply a list of `--set` operations to an extensions map.
/// Operations are applied left-to-right sequentially.
pub fn apply_sets(
    extensions: &mut BTreeMap<String, serde_json::Value>,
    sets: &[String],
) -> Result<(), TgError> {
    for set_str in sets {
        let (segments, value) = parse_set_flag(set_str)?;
        match value {
            Some(v) => set_nested(extensions, &segments, v),
            None => delete_nested(extensions, &segments),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dot_path_simple() {
        let segs = parse_dot_path("x-foo").unwrap();
        assert_eq!(segs, vec!["x-foo"]);
    }

    #[test]
    fn parse_dot_path_nested() {
        let segs = parse_dot_path("x-foo.bar.baz").unwrap();
        assert_eq!(segs, vec!["x-foo", "bar", "baz"]);
    }

    #[test]
    fn parse_dot_path_rejects_non_x_prefix() {
        assert!(parse_dot_path("foo").is_err());
        assert!(parse_dot_path("priority").is_err());
    }

    #[test]
    fn parse_dot_path_rejects_empty() {
        assert!(parse_dot_path("").is_err());
        assert!(parse_dot_path("x-foo..bar").is_err());
    }

    #[test]
    fn parse_value_number() {
        assert_eq!(parse_value("42"), serde_json::json!(42));
        assert_eq!(parse_value("3.14"), serde_json::json!(3.14));
    }

    #[test]
    fn parse_value_boolean() {
        assert_eq!(parse_value("true"), serde_json::json!(true));
        assert_eq!(parse_value("false"), serde_json::json!(false));
    }

    #[test]
    fn parse_value_null() {
        assert_eq!(parse_value("null"), serde_json::Value::Null);
    }

    #[test]
    fn parse_value_object() {
        let v = parse_value(r#"{"key":"val"}"#);
        assert!(v.is_object());
    }

    #[test]
    fn parse_value_array() {
        let v = parse_value("[1,2,3]");
        assert!(v.is_array());
    }

    #[test]
    fn parse_value_string_fallback() {
        assert_eq!(parse_value("hello"), serde_json::json!("hello"));
        assert_eq!(parse_value("some text"), serde_json::json!("some text"));
    }

    #[test]
    fn set_nested_simple() {
        let mut ext = BTreeMap::new();
        set_nested(&mut ext, &["x-foo".into()], serde_json::json!(42));
        assert_eq!(ext["x-foo"], serde_json::json!(42));
    }

    #[test]
    fn set_nested_creates_intermediates() {
        let mut ext = BTreeMap::new();
        set_nested(
            &mut ext,
            &["x-foo".into(), "bar".into()],
            serde_json::json!(1),
        );
        assert_eq!(ext["x-foo"], serde_json::json!({"bar": 1}));
    }

    #[test]
    fn set_nested_deep() {
        let mut ext = BTreeMap::new();
        set_nested(
            &mut ext,
            &["x-foo".into(), "bar".into(), "baz".into()],
            serde_json::json!("deep"),
        );
        assert_eq!(ext["x-foo"], serde_json::json!({"bar": {"baz": "deep"}}));
    }

    #[test]
    fn set_nested_overwrites_non_object() {
        let mut ext = BTreeMap::new();
        ext.insert("x-foo".into(), serde_json::json!("hello"));
        set_nested(
            &mut ext,
            &["x-foo".into(), "bar".into()],
            serde_json::json!(1),
        );
        assert_eq!(ext["x-foo"], serde_json::json!({"bar": 1}));
    }

    #[test]
    fn delete_nested_simple() {
        let mut ext = BTreeMap::new();
        ext.insert("x-foo".into(), serde_json::json!(42));
        delete_nested(&mut ext, &["x-foo".into()]);
        assert!(!ext.contains_key("x-foo"));
    }

    #[test]
    fn delete_nested_with_cleanup() {
        let mut ext = BTreeMap::new();
        ext.insert("x-foo".into(), serde_json::json!({"bar": 1}));
        delete_nested(&mut ext, &["x-foo".into(), "bar".into()]);
        // Parent should be cleaned up since it's now empty
        assert!(!ext.contains_key("x-foo"));
    }

    #[test]
    fn delete_nested_partial_cleanup() {
        let mut ext = BTreeMap::new();
        ext.insert(
            "x-foo".into(),
            serde_json::json!({"bar": 1, "baz": 2}),
        );
        delete_nested(&mut ext, &["x-foo".into(), "bar".into()]);
        // Parent still has "baz", so not cleaned up
        assert_eq!(ext["x-foo"], serde_json::json!({"baz": 2}));
    }

    #[test]
    fn parse_set_flag_set_value() {
        let (segs, val) = parse_set_flag("x-foo.bar=42").unwrap();
        assert_eq!(segs, vec!["x-foo", "bar"]);
        assert_eq!(val, Some(serde_json::json!(42)));
    }

    #[test]
    fn parse_set_flag_delete() {
        let (segs, val) = parse_set_flag("x-foo.bar=").unwrap();
        assert_eq!(segs, vec!["x-foo", "bar"]);
        assert_eq!(val, None);
    }

    #[test]
    fn parse_set_flag_rejects_no_equals() {
        assert!(parse_set_flag("x-foo").is_err());
    }

    #[test]
    fn apply_sets_multiple() {
        let mut ext = BTreeMap::new();
        let sets = vec!["x-a=1".to_string(), "x-a.b=2".to_string()];
        apply_sets(&mut ext, &sets).unwrap();
        // Second set overwrites first (x-a was 1, now becomes {"b": 2})
        assert_eq!(ext["x-a"], serde_json::json!({"b": 2}));
    }

    #[test]
    fn x_prefix_validation_in_set() {
        let mut ext = BTreeMap::new();
        let result = apply_sets(&mut ext, &["priority=5".to_string()]);
        assert!(result.is_err());
    }
}
