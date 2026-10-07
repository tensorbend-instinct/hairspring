//! Conservative schema-guided argument coercion (Hermes `arg_coercion`
//! idea). Coerces string-typed values to the type the tool schema declares
//! only when the conversion is exact and lossless. Unknown fields,
//! non-matching strings and already-correct values pass through untouched.
use serde_json::Value;

fn coerce_value(schema: &Value, v: &Value, n: &mut u32) -> Value {
    let ty = schema.get("type").and_then(Value::as_str).unwrap_or("");
    match (ty, v) {
        ("integer", Value::String(s)) => {
            if let Ok(i) = s.parse::<i64>() {
                if i.to_string() == *s {
                    *n += 1;
                    return Value::from(i);
                }
            }
            v.clone()
        }
        ("number", Value::String(s)) => {
            if let Ok(f) = s.parse::<f64>() {
                if f.is_finite() && !s.starts_with('+') && s.trim() == s {
                    if let Some(num) = serde_json::Number::from_f64(f) {
                        *n += 1;
                        return Value::Number(num);
                    }
                }
            }
            v.clone()
        }
        ("boolean", Value::String(s)) => match s.as_str() {
            "true" => {
                *n += 1;
                Value::Bool(true)
            }
            "false" => {
                *n += 1;
                Value::Bool(false)
            }
            _ => v.clone(),
        },
        ("array", Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(parsed @ Value::Array(_)) => {
                *n += 1;
                coerce_value(schema, &parsed, n)
            }
            _ => v.clone(),
        },
        ("object", Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(parsed @ Value::Object(_)) => {
                *n += 1;
                coerce_value(schema, &parsed, n)
            }
            _ => v.clone(),
        },
        ("array", Value::Array(items)) => match schema.get("items") {
            Some(is) => Value::Array(items.iter().map(|i| coerce_value(is, i, n)).collect()),
            None => v.clone(),
        },
        ("object", Value::Object(map)) => match schema.get("properties").and_then(Value::as_object) {
            Some(props) => Value::Object(
                map.iter()
                    .map(|(k, val)| {
                        let nv = props.get(k).map_or_else(|| val.clone(), |ps| coerce_value(ps, val, n));
                        (k.clone(), nv)
                    })
                    .collect(),
            ),
            None => v.clone(),
        },
        _ => v.clone(),
    }
}

/// Coerce `args` against the tool's parameter `schema`; returns the new
/// args and how many values were coerced.
#[must_use]
pub fn coerce(schema: &Value, args: &Value) -> (Value, u32) {
    let mut n = 0;
    let out = coerce_value(schema, args, &mut n);
    (out, n)
}

/// Find a tool's parameter schema in an OpenAI-shaped tools array.
#[must_use]
pub fn schema_for<'a>(tools: &'a Value, name: &str) -> Option<&'a Value> {
    tools.as_array()?.iter().find_map(|t| {
        let f = t.get("function")?;
        (f.get("name")?.as_str()? == name).then(|| f.get("parameters")).flatten()
    })
}
