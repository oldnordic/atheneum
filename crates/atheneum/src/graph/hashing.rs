use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn compute_bytes_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn compute_file_sha256(path: &std::path::Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("read file '{}' for hashing failed: {}", path.display(), e))?;
    Ok(compute_bytes_sha256(&bytes))
}

pub(crate) fn sha256_hex(input: &str) -> String {
    compute_bytes_sha256(input.as_bytes())
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

/// Compute a deterministic content hash over a JSON value, excluding volatile fields.
///
/// Volatile fields (timestamps, auto-generated IDs, the hash itself) would cause
/// the same logical content to produce different hashes. This function removes
/// them before hashing so equivalent payloads match regardless of when they were
/// created.
pub(crate) fn content_hash_excluding(value: &Value, volatile_keys: &[&str]) -> Result<String> {
    let mut normalized = value.clone();
    if let Some(obj) = normalized.as_object_mut() {
        for key in volatile_keys {
            obj.remove(*key);
        }
    }
    json_sha256_hex(&normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn content_hash_excluding_removes_volatile_keys() {
        let data = json!({
            "title": "test",
            "content": "hello",
            "timestamp": "2026-06-09T12:00:00Z",
            "sql_id": 42,
            "content_hash": "old_hash"
        });
        let hash_with =
            content_hash_excluding(&data, &["timestamp", "sql_id", "content_hash"]).unwrap();
        let data_no_volatile = json!({
            "title": "test",
            "content": "hello"
        });
        let hash_without = content_hash_excluding(&data_no_volatile, &[]).unwrap();
        assert_eq!(
            hash_with, hash_without,
            "hashes should match after volatile key removal"
        );
    }

    #[test]
    fn content_hash_excluding_is_deterministic() {
        let data1 = json!({"z": 1, "a": 2, "content_hash": "x"});
        let data2 = json!({"a": 2, "z": 1, "content_hash": "y"});
        let h1 = content_hash_excluding(&data1, &["content_hash"]).unwrap();
        let h2 = content_hash_excluding(&data2, &["content_hash"]).unwrap();
        assert_eq!(h1, h2, "key order and hash value should not affect result");
    }

    #[test]
    fn content_hash_excluding_differs_on_content() {
        let data1 = json!({"title": "alpha"});
        let data2 = json!({"title": "beta"});
        let h1 = content_hash_excluding(&data1, &[]).unwrap();
        let h2 = content_hash_excluding(&data2, &[]).unwrap();
        assert_ne!(h1, h2, "different content must produce different hashes");
    }

    #[test]
    fn content_hash_excluding_handles_nested() {
        let data = json!({
            "meta": {"nested": true, "volatile": "remove_me"},
            "content": "hello"
        });
        let hash = content_hash_excluding(&data, &["volatile"]).unwrap();
        // volatile key is nested, not top-level, so it should NOT be removed
        // (only top-level keys are stripped)
        assert!(!hash.is_empty());
    }
}
