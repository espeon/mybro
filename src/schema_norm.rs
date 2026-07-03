use serde_json::{Map, Value};

// ── Tool schema normalization (spec §10) ─────────────────────────────────────

/// Fast-path guard: scan for $defs, definitions, or $ref at any depth.
/// Bail on first hit.  If none found, skip normalization entirely.
fn needs_normalization(tools: &Value) -> bool {
    if let Some(arr) = tools.as_array() {
        for tool in arr {
            if let Some(params) = tool.get("function").and_then(|f| f.get("parameters")) {
                if has_ref_or_defs(params) {
                    return true;
                }
            }
        }
    }
    false
}

fn has_ref_or_defs(node: &Value) -> bool {
    match node {
        Value::Object(obj) => {
            if obj.contains_key("$ref") || obj.contains_key("$defs") || obj.contains_key("definitions") {
                return true;
            }
            for (_, v) in obj {
                if has_ref_or_defs(v) {
                    return true;
                }
            }
            false
        }
        Value::Array(arr) => arr.iter().any(has_ref_or_defs),
        _ => false,
    }
}

/// Entry point: normalize all tool schemas in place (spec §10.1).
pub fn normalize_tool_schemas(tools: &mut Value) {
    if !needs_normalization(tools) {
        return;
    }
    if let Some(arr) = tools.as_array_mut() {
        for tool in arr.iter_mut() {
            if let Some(params) = tool
                .get_mut("function")
                .and_then(|f| f.get_mut("parameters"))
            {
                let mut defs: HashMap<&str, &Value> = std::collections::HashMap::new();
                normalize_schema(params, &mut defs, 12);
            }
        }
    }
}

use std::collections::HashMap;

/// Recursive schema normalization (spec §10.2).
fn normalize_schema(node: &mut Value, defs: &mut HashMap<&str, &Value>, depth: usize) {
    if depth == 0 {
        return;
    }

    let Some(obj) = node.as_object_mut() else {
        return;
    };

    // Merge local definitions/​$defs into defs
    let local_keys: Vec<String> = obj
        .get("definitions")
        .and_then(|v| v.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let local_defs_keys: Vec<String> = obj
        .get("$defs")
        .and_then(|v| v.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();

    // Save references to local defs before we recurse
    let mut local_defs: Vec<(String, Value)> = Vec::new();
    if let Some(d) = obj.get("definitions").and_then(|v| v.as_object()) {
        for (k, v) in d {
            local_defs.push((k.clone(), v.clone()));
        }
    }
    if let Some(d) = obj.get("$defs").and_then(|v| v.as_object()) {
        for (k, v) in d {
            local_defs.push((k.clone(), v.clone()));
        }
    }

    // Check if this is exactly { "$ref": "..." }
    if obj.len() == 1 {
        if let Some(ref_str) = obj.get("$ref").and_then(|v| v.as_str()) {
            if let Some(resolved) = resolve_ref(ref_str, &local_defs, defs) {
                let mut cloned = resolved.clone();
                let mut new_defs: HashMap<&str, &Value> = HashMap::new();
                // We need to re-merge the resolved value's defs
                normalize_schema(&mut cloned, &mut new_defs, depth - 1);
                *node = cloned;
                return;
            }
        }
    }

    // Recurse into every value, skipping definitions, $defs, nullable
    let keys: Vec<String> = obj.keys().cloned().collect();
    for key in keys {
        if key == "definitions" || key == "$defs" || key == "nullable" {
            continue;
        }
        if let Some(v) = obj.get_mut(&key) {
            normalize_schema(v, defs, depth - 1);
        }
    }

    // Post-passes
    let obj = node.as_object_mut().unwrap();

    // simplify nullable combinators
    for key in ["anyOf", "oneOf"] {
        if obj.contains_key(key) {
            simplify_nullable_combinator(obj, key);
        }
    }

    // normalize type field
    normalize_type_field(obj);

    // normalize enum field
    normalize_enum_field(obj);

    // remove const: null
    if obj.get("const").map(|v| v.is_null()).unwrap_or(false) {
        obj.remove("const");
    }
}

fn resolve_ref(
    ref_str: &str,
    local_defs: &[(String, Value)],
    defs: &HashMap<&str, &Value>,
) -> Option<Value> {
    // #/definitions/x or #/$defs/x
    let name = ref_str
        .strip_prefix("#/definitions/")
        .or_else(|| ref_str.strip_prefix("#/$defs/"))?;
    // Check local defs first
    for (k, v) in local_defs {
        if k == name {
            return Some(v.clone());
        }
    }
    // Then the passed-in defs
    defs.get(name).map(|v| (*v).clone())
}

/// Filter null-schemas out of anyOf/oneOf arrays (spec §10.3).
fn simplify_nullable_combinator(obj: &mut Map<String, Value>, key: &str) {
    if let Some(Value::Array(arr)) = obj.get_mut(key) {
        arr.retain(|s| !is_null_schema(s));
        match arr.len() {
            0 => {
                obj.remove(key);
            }
            1 => {
                // Inline the single remaining schema
                let inlined = arr[0].clone();
                if let Value::Object(inlined_obj) = inlined {
                    obj.remove(key);
                    for (k, v) in inlined_obj {
                        obj.insert(k, v);
                    }
                }
            }
            _ => {}
        }
    }
}

fn is_null_schema(s: &Value) -> bool {
    match s {
        Value::Object(o) => {
            (o.len() == 1 && o.get("type").map(|v| v == "null").unwrap_or(false))
                || (o.len() == 1 && o.get("const").map(|v| v.is_null()).unwrap_or(false))
                || (o.len() == 1
                    && o.get("enum")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len() == 1 && a[0].is_null())
                        .unwrap_or(false))
        }
        _ => false,
    }
}

/// Normalize type field: if array, drop "null" and empty strings (spec §10.4).
fn normalize_type_field(obj: &mut Map<String, Value>) {
    if let Some(Value::Array(arr)) = obj.get_mut("type") {
        arr.retain(|v| {
            if v.is_null() {
                false
            } else if let Some(s) = v.as_str() {
                !s.is_empty() && s != "null"
            } else {
                true
            }
        });
        match arr.len() {
            0 => {
                obj.remove("type");
            }
            1 => {
                let first = arr[0].clone();
                obj.insert("type".to_string(), first);
            }
            _ => {}
        }
    }
}

/// Normalize enum field: drop nulls, dedupe (spec §10.5).
fn normalize_enum_field(obj: &mut Map<String, Value>) {
    if let Some(Value::Array(arr)) = obj.get_mut("enum") {
        arr.retain(|v| !v.is_null());
        // dedupe by canonical JSON
        let mut seen: Vec<String> = Vec::new();
        arr.retain(|v| {
            let key = serde_json::to_string(v).unwrap_or_default();
            if seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        });
        if arr.is_empty() {
            obj.remove("enum");
        }
    }
}
