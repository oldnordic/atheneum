use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn json_sha256_hex(value: &Value) -> Result<String> {
    Ok(sha256_hex(&canonical_json_string(value)?))
}

pub(crate) fn canonical_json_string(value: &Value) -> Result<String> {
    let canonical = canonicalize_json(value);
    serde_json::to_string(&canonical)
        .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let ordered: BTreeMap<String, Value> = map
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize_json(value)))
                .collect();
            Value::Object(ordered.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        _ => value.clone(),
    }
}
