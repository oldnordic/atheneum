//! Shared response envelope for the unified tool API: pagination, provenance
//! tagging, staleness signals, and error aggregation, used identically by
//! every dispatch path (atheneum in-process, code-tool subprocess, envoy HTTP).

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 100;
pub const DEFAULT_DEPTH: u32 = 2;
pub const MAX_DEPTH: u32 = 3;

pub const ERR_PROJECT_NOT_FOUND: &str = "PROJECT_NOT_FOUND";
pub const ERR_BACKEND_UNAVAILABLE: &str = "BACKEND_UNAVAILABLE";
pub const ERR_PARSE_ERROR: &str = "PARSE_ERROR";
pub const ERR_TIMEOUT: &str = "TIMEOUT";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Provenance {
    Extracted,
    Inferred,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvelopeError {
    pub backend: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    pub items: Vec<Value>,
    pub limit: usize,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub code_stale: Option<bool>,
    pub knowledge_stale: Option<bool>,
    pub depth_clamped: bool,
    pub errors: Vec<EnvelopeError>,
}

impl Envelope {
    pub fn new(limit: usize) -> Self {
        Self {
            items: Vec::new(),
            limit,
            cursor: None,
            has_more: false,
            code_stale: None,
            knowledge_stale: None,
            depth_clamped: false,
            errors: Vec::new(),
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

pub fn clamp_limit(requested: Option<usize>) -> usize {
    requested.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

pub fn clamp_depth(requested: Option<u32>) -> (u32, bool) {
    let req = requested.unwrap_or(DEFAULT_DEPTH);
    let clamped = req.min(MAX_DEPTH);
    (clamped, clamped != req)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    pub backend: String,
    pub offset: usize,
}

pub fn encode_cursor(cursor: &Cursor) -> String {
    let json = serde_json::to_string(cursor).unwrap_or_default();
    base64::engine::general_purpose::STANDARD.encode(json)
}

pub fn decode_cursor(s: &str) -> Option<Cursor> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(s).ok()?;
    let json = String::from_utf8(bytes).ok()?;
    serde_json::from_str(&json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_limit_defaults_to_20() {
        assert_eq!(clamp_limit(None), 20);
    }

    #[test]
    fn clamp_limit_caps_at_100() {
        assert_eq!(clamp_limit(Some(500)), 100);
    }

    #[test]
    fn clamp_limit_passes_through_valid_value() {
        assert_eq!(clamp_limit(Some(5)), 5);
    }

    #[test]
    fn clamp_limit_floors_zero_to_one() {
        assert_eq!(clamp_limit(Some(0)), 1);
    }

    #[test]
    fn clamp_depth_defaults_to_2_unclamped() {
        assert_eq!(clamp_depth(None), (2, false));
    }

    #[test]
    fn clamp_depth_caps_at_3_and_flags_clamped() {
        assert_eq!(clamp_depth(Some(10)), (3, true));
    }

    #[test]
    fn clamp_depth_passes_through_valid_value_unclamped() {
        assert_eq!(clamp_depth(Some(1)), (1, false));
    }

    #[test]
    fn cursor_round_trips_through_encode_decode() {
        let c = Cursor { backend: "knowledge".to_string(), offset: 42 };
        let encoded = encode_cursor(&c);
        let decoded = decode_cursor(&encoded).expect("cursor should decode");
        assert_eq!(decoded.backend, "knowledge");
        assert_eq!(decoded.offset, 42);
    }

    #[test]
    fn decode_cursor_rejects_garbage() {
        assert!(decode_cursor("not-a-valid-cursor!!!").is_none());
    }

    #[test]
    fn envelope_serializes_with_expected_shape() {
        let mut env = Envelope::new(20);
        env.items.push(serde_json::json!({"name": "foo"}));
        env.has_more = true;
        env.cursor = Some("abc".to_string());
        env.errors.push(EnvelopeError {
            backend: "code".to_string(),
            code: ERR_BACKEND_UNAVAILABLE.to_string(),
            message: "magellan binary not found".to_string(),
        });
        let v = env.to_value();
        assert_eq!(v["limit"], 20);
        assert_eq!(v["has_more"], true);
        assert_eq!(v["cursor"], "abc");
        assert_eq!(v["items"][0]["name"], "foo");
        assert_eq!(v["errors"][0]["code"], ERR_BACKEND_UNAVAILABLE);
    }

    #[test]
    fn provenance_serializes_as_uppercase_tag() {
        assert_eq!(serde_json::to_value(Provenance::Extracted).unwrap(), "EXTRACTED");
        assert_eq!(serde_json::to_value(Provenance::Inferred).unwrap(), "INFERRED");
        assert_eq!(serde_json::to_value(Provenance::Ambiguous).unwrap(), "AMBIGUOUS");
    }
}
