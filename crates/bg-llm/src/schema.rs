//! JSON Schema construction and validation.
//!
//! Structured output is *constrained* by the provider, not guaranteed by it: a
//! truncated response, a provider without schema support, or the stub can all
//! produce something that does not fit. Validating locally means a malformed
//! claim is a typed error at the LLM boundary rather than a panic three layers
//! later in the policy engine.
//!
//! Covers the subset the Anthropic structured-output implementation accepts —
//! `type`, `properties`, `required`, `additionalProperties: false`, `items`,
//! `enum`, `anyOf`. Numeric and length constraints are deliberately absent:
//! they are not supported server-side, so enforcing them here would reject
//! output the provider was never asked to constrain.

use serde_json::{json, Value};

// -- construction helpers ---------------------------------------------------

/// An object schema. `additionalProperties: false` is set automatically —
/// structured output requires it on every object.
pub fn object(props: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let mut map = serde_json::Map::new();
    for (k, v) in props {
        map.insert(k.to_string(), v);
    }
    json!({
        "type": "object",
        "properties": Value::Object(map),
        "required": required,
        "additionalProperties": false
    })
}

pub fn string(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}

/// A string with an `x-stub` hint telling the offline provider what kind of
/// plausible text to synthesize. Ignored by real providers.
pub fn string_hinted(desc: &str, stub_hint: &str) -> Value {
    json!({ "type": "string", "description": desc, "x-stub": stub_hint })
}

pub fn number(desc: &str) -> Value {
    json!({ "type": "number", "description": desc })
}

/// A number with an expected range.
///
/// JSON Schema's `minimum`/`maximum` are not supported by structured output, so
/// this is advisory to real providers (the range still reaches the model via
/// `description`) — but the offline provider honours it. Without it the stub
/// emits 0.5–1.0 for everything, and a field that means "0-100" silently
/// truncates to 0, which reads downstream as "nothing was newsworthy" rather
/// than as a stub artefact.
pub fn number_range(desc: &str, lo: f64, hi: f64) -> Value {
    json!({
        "type": "number",
        "description": format!("{desc} (range {lo}-{hi})"),
        "x-stub-min": lo,
        "x-stub-max": hi
    })
}

pub fn integer(desc: &str) -> Value {
    json!({ "type": "integer", "description": desc })
}

/// An integer field that echoes the index of the input it refers to.
///
/// Batched agents match results back to inputs by index, so the offline
/// provider has to emit the element's actual position rather than a random
/// number — otherwise every lookup misses and the batch silently does nothing.
pub fn integer_index(desc: &str) -> Value {
    json!({ "type": "integer", "description": desc, "x-stub": "ordinal" })
}

/// An integer index into a collection of known size (`0..n`).
///
/// Citation indices must land inside the source list. A stub that emits
/// out-of-range indices produces claims with no provenance, which the policy
/// engine correctly refuses to publish — so without this the offline path can
/// never demonstrate a successful publish.
pub fn integer_bounded(desc: &str, n: usize) -> Value {
    json!({
        "type": "integer",
        "description": format!("{desc} (0-{})", n.saturating_sub(1)),
        "x-stub-min": 0.0,
        "x-stub-max": n.saturating_sub(1) as f64
    })
}

/// An enum with a fixed choice for the offline provider.
///
/// The stub should behave like a *competent* model so the happy path is
/// exercised end to end; a uniformly random verdict would mean roughly one
/// offline story in six is marked `refuted` and blocked before it renders.
/// Adversarial inputs belong in explicit tests, where they can be asserted on.
///
/// `stub_value` is named rather than positional so the enum keeps its natural
/// semantic ordering in the prompt — reordering the list to put a convenient
/// value first would nudge real models too.
pub fn enumeration_stub(values: &[&str], desc: &str, stub_value: &str) -> Value {
    debug_assert!(values.contains(&stub_value), "stub value must be one of the variants");
    json!({ "type": "string", "enum": values, "description": desc, "x-stub-enum": stub_value })
}

pub fn boolean(desc: &str) -> Value {
    json!({ "type": "boolean", "description": desc })
}

pub fn array(items: Value, desc: &str) -> Value {
    json!({ "type": "array", "items": items, "description": desc })
}

/// An array whose length is known by the caller — one entry per input.
///
/// Carries an `x-stub-count` hint so the offline provider returns a
/// correctly-sized batch. Without it the stub emits an arbitrary 1-3 elements
/// and every batched agent silently drops most of its input, which looks like a
/// pipeline bug rather than a stub limitation. Real providers ignore the key.
pub fn array_n(items: Value, desc: &str, n: usize) -> Value {
    json!({ "type": "array", "items": items, "description": desc, "x-stub-count": n })
}

pub fn enumeration(values: &[&str], desc: &str) -> Value {
    json!({ "type": "string", "enum": values, "description": desc })
}

/// A nullable value, expressed the way structured output accepts it.
pub fn nullable(inner: Value) -> Value {
    json!({ "anyOf": [inner, { "type": "null" }] })
}

// -- validation -------------------------------------------------------------

/// Validate `value` against `schema`. `Ok(())` means it conforms.
pub fn validate(value: &Value, schema: &Value) -> Result<(), String> {
    check(value, schema, "$")
}

fn check(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    // anyOf: conform to at least one branch.
    if let Some(branches) = schema.get("anyOf").and_then(|v| v.as_array()) {
        let mut errs = Vec::new();
        for (i, b) in branches.iter().enumerate() {
            match check(value, b, path) {
                Ok(()) => return Ok(()),
                Err(e) => errs.push(format!("[{i}] {e}")),
            }
        }
        return Err(format!("{path}: matched no anyOf branch ({})", errs.join("; ")));
    }

    if let Some(allowed) = schema.get("enum").and_then(|v| v.as_array()) {
        if !allowed.contains(value) {
            return Err(format!("{path}: {value} is not one of {allowed:?}"));
        }
        return Ok(());
    }

    let Some(ty) = schema.get("type").and_then(|v| v.as_str()) else {
        // No type constraint — nothing to check.
        return Ok(());
    };

    match ty {
        "object" => {
            let Some(obj) = value.as_object() else {
                return Err(format!("{path}: expected object, got {}", kind(value)));
            };
            if let Some(req) = schema.get("required").and_then(|v| v.as_array()) {
                for r in req {
                    let Some(name) = r.as_str() else { continue };
                    if !obj.contains_key(name) {
                        return Err(format!("{path}: missing required field `{name}`"));
                    }
                }
            }
            let props = schema.get("properties").and_then(|v| v.as_object());
            if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                if let Some(props) = props {
                    for k in obj.keys() {
                        if !props.contains_key(k) {
                            return Err(format!("{path}: unexpected field `{k}`"));
                        }
                    }
                }
            }
            if let Some(props) = props {
                for (k, sub) in props {
                    if let Some(v) = obj.get(k) {
                        check(v, sub, &format!("{path}.{k}"))?;
                    }
                }
            }
            Ok(())
        }
        "array" => {
            let Some(arr) = value.as_array() else {
                return Err(format!("{path}: expected array, got {}", kind(value)));
            };
            if let Some(items) = schema.get("items") {
                for (i, v) in arr.iter().enumerate() {
                    check(v, items, &format!("{path}[{i}]"))?;
                }
            }
            Ok(())
        }
        "string" => value
            .is_string()
            .then_some(())
            .ok_or_else(|| format!("{path}: expected string, got {}", kind(value))),
        "integer" => {
            // A JSON number that happens to be integral counts; 1.5 does not.
            let ok = value.as_i64().is_some()
                || value.as_f64().is_some_and(|f| f.fract() == 0.0);
            ok.then_some(())
                .ok_or_else(|| format!("{path}: expected integer, got {}", kind(value)))
        }
        "number" => value
            .is_number()
            .then_some(())
            .ok_or_else(|| format!("{path}: expected number, got {}", kind(value))),
        "boolean" => value
            .is_boolean()
            .then_some(())
            .ok_or_else(|| format!("{path}: expected boolean, got {}", kind(value))),
        "null" => value
            .is_null()
            .then_some(())
            .ok_or_else(|| format!("{path}: expected null, got {}", kind(value))),
        other => Err(format!("{path}: unsupported schema type `{other}`")),
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim_schema() -> Value {
        object(
            vec![
                ("text", string("the claim")),
                ("kind", enumeration(&["fact", "figure", "quote", "forecast"], "kind")),
                ("confidence", number("0-1")),
                ("sources", array(integer("index"), "backing source indices")),
            ],
            &["text", "kind", "confidence", "sources"],
        )
    }

    #[test]
    fn a_conforming_object_validates() {
        let v = json!({
            "text": "The exchange froze the funds.",
            "kind": "fact",
            "confidence": 0.9,
            "sources": [0, 2]
        });
        assert!(validate(&v, &claim_schema()).is_ok());
    }

    #[test]
    fn a_missing_required_field_is_caught() {
        let v = json!({"text": "x", "kind": "fact", "confidence": 0.5});
        let err = validate(&v, &claim_schema()).unwrap_err();
        assert!(err.contains("sources"), "{err}");
    }

    #[test]
    fn a_bad_enum_value_is_caught() {
        let v = json!({"text": "x", "kind": "vibes", "confidence": 0.5, "sources": []});
        assert!(validate(&v, &claim_schema()).is_err());
    }

    #[test]
    fn a_wrong_type_is_caught_with_its_path() {
        let v = json!({"text": "x", "kind": "fact", "confidence": "high", "sources": []});
        let err = validate(&v, &claim_schema()).unwrap_err();
        assert!(err.contains("$.confidence"), "{err}");
    }

    #[test]
    fn an_unexpected_field_is_caught() {
        let v = json!({
            "text": "x", "kind": "fact", "confidence": 0.5, "sources": [],
            "editorialised": true
        });
        let err = validate(&v, &claim_schema()).unwrap_err();
        assert!(err.contains("editorialised"), "{err}");
    }

    #[test]
    fn nested_array_items_are_validated_with_index_paths() {
        let schema = array(claim_schema(), "claims");
        let v = json!([
            {"text": "a", "kind": "fact", "confidence": 0.5, "sources": []},
            {"text": "b", "kind": "fact", "confidence": 0.5}
        ]);
        let err = validate(&v, &schema).unwrap_err();
        assert!(err.contains("$[1]"), "{err}");
    }

    #[test]
    fn nullable_accepts_both_the_value_and_null() {
        let schema = nullable(string("maybe"));
        assert!(validate(&json!("hello"), &schema).is_ok());
        assert!(validate(&Value::Null, &schema).is_ok());
        assert!(validate(&json!(42), &schema).is_err());
    }

    #[test]
    fn an_integral_float_counts_as_an_integer() {
        assert!(validate(&json!(3.0), &integer("n")).is_ok());
        assert!(validate(&json!(3.5), &integer("n")).is_err());
    }

    #[test]
    fn object_helper_always_forbids_extra_properties() {
        let s = object(vec![("a", string("a"))], &["a"]);
        assert_eq!(s["additionalProperties"], json!(false));
    }
}
