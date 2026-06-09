//! AI-powered triple extraction from prose.
//!
//! Extracts (subject, predicate, object) triples from natural language text
//! using an LLM (ollama) with structured JSON output. Triples are inserted
//! as entities and edges with `ProvenanceData` marking
//! `extraction_mode: "ai_triple"`.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{AtheneumGraph, EdgeType, ProvenanceData};

/// A single extracted knowledge triple.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractedTriple {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

/// Result of an LLM triple extraction pass.
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    /// Triples extracted from the source text.
    pub triples: Vec<ExtractedTriple>,
    /// The model used for extraction.
    pub model: String,
    /// First 80 characters of source text, for provenance.
    pub source_preview: String,
}

/// Configuration for the triple extraction LLM call.
#[derive(Debug, Clone)]
pub struct ExtractionConfig {
    /// Ollama model name (default: "gemma4:e2b").
    pub model: String,
    /// Ollama base URL (default: "http://localhost:11434").
    pub url: String,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            model: "gemma4:e2b".to_string(),
            url: "http://localhost:11434".to_string(),
        }
    }
}

#[cfg(feature = "neural-embed")]
const EXTRACTION_PROMPT: &str = r#"Extract knowledge triples from the following text. Return a JSON array of objects with keys "subject", "predicate", "object". Each triple should capture a factual relationship. Only include clear, explicit relationships. Return ONLY the JSON array, no other text.

Text:
"#;

/// Extract triples from text using an ollama LLM.
///
/// Sends the text to the configured model with a structured prompt,
/// parses the JSON array response into `ExtractedTriple` values.
#[cfg(feature = "neural-embed")]
pub fn extract_triples(text: &str, config: &ExtractionConfig) -> Result<ExtractionResult> {
    let prompt = format!("{}{}", EXTRACTION_PROMPT, text);

    let response: Value = ureq::post(&format!("{}/api/generate", config.url))
        .send_json(ureq::json!({
            "model": config.model,
            "prompt": prompt,
            "stream": false,
            "format": "json",
            "options": {
                "temperature": 0.1,
                "num_predict": 2048,
            }
        }))?
        .into_json()?;

    let raw_response = response
        .get("response")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing response field from ollama"))?;

    // The model may wrap the array in an object: {"triples": [...]}
    // or return a bare array: [...]
    let triples: Vec<ExtractedTriple> = if raw_response.trim_start().starts_with('[') {
        serde_json::from_str(raw_response)?
    } else {
        let wrapped: Value = serde_json::from_str(raw_response)?;
        if let Some(arr) = wrapped.as_array() {
            serde_json::from_value(wrapped)?
        } else {
            // Try common wrapper keys
            let arr = wrapped
                .get("triples")
                .or_else(|| wrapped.get("results"))
                .or_else(|| wrapped.get("data"))
                .cloned()
                .unwrap_or(wrapped);
            serde_json::from_value(arr)?
        }
    };

    let source_preview = if text.len() > 80 {
        format!("{}...", &text[..80])
    } else {
        text.to_string()
    };

    Ok(ExtractionResult {
        triples,
        model: config.model.clone(),
        source_preview,
    })
}

/// Ingest extracted triples into the graph as entities and edges.
///
/// For each triple (subject, predicate, object):
/// 1. Upsert subject as an entity (kind="Concept")
/// 2. Upsert object as an entity (kind="Concept")
/// 3. Insert edge from subject to object with predicate as edge type
///    and provenance marking extraction_mode="ai_triple"
///
/// Returns a vector of (subject_id, edge_id, object_id) for each triple.
pub fn ingest_triples(
    graph: &AtheneumGraph,
    result: &ExtractionResult,
    project_id: Option<&str>,
) -> Result<Vec<(i64, i64, i64)>> {
    let mut inserted = Vec::with_capacity(result.triples.len());

    for triple in &result.triples {
        // Upsert subject entity
        let subject_data = if let Some(pid) = project_id {
            serde_json::json!({"project_id": pid})
        } else {
            serde_json::json!({})
        };
        let subject_id = graph.upsert_concept(&triple.subject, &subject_data)?;

        // Upsert object entity
        let object_data = if let Some(pid) = project_id {
            serde_json::json!({"project_id": pid})
        } else {
            serde_json::json!({})
        };
        let object_id = graph.upsert_concept(&triple.object, &object_data)?;

        // Insert edge with provenance
        let provenance = ProvenanceData::new("ai_triple_extraction")
            .with_actor(&format!("ollama:{}", result.model))
            .with_extraction_mode("ai_triple")
            .with_source_text(&result.source_preview)
            .to_value();

        let edge_data = serde_json::json!({
            "predicate": triple.predicate,
            "provenance": provenance,
        });

        let edge_id = graph.insert_edge(subject_id, object_id, EdgeType::RelatedTo, edge_data)?;

        inserted.push((subject_id, edge_id, object_id));
    }

    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracted_triple_serialization_roundtrip() {
        let triple = ExtractedTriple {
            subject: "Rust".to_string(),
            predicate: "has_feature".to_string(),
            object: "ownership".to_string(),
        };
        let json = serde_json::to_string(&triple).unwrap();
        let back: ExtractedTriple = serde_json::from_str(&json).unwrap();
        assert_eq!(triple, back);
    }

    #[test]
    fn extracted_triple_parses_from_json_array() {
        let json = r#"[{"subject":"Rust","predicate":"has","object":"ownership"},{"subject":"ownership","predicate":"prevents","object":"data races"}]"#;
        let triples: Vec<ExtractedTriple> = serde_json::from_str(json).unwrap();
        assert_eq!(triples.len(), 2);
        assert_eq!(triples[0].subject, "Rust");
        assert_eq!(triples[1].object, "data races");
    }

    #[test]
    fn extraction_result_source_preview_truncates() {
        let text = "A".repeat(200);
        let result = ExtractionResult {
            triples: vec![],
            model: "test".to_string(),
            source_preview: if text.len() > 80 {
                format!("{}...", &text[..80])
            } else {
                text.clone()
            },
        };
        assert!(result.source_preview.len() <= 83); // 80 + "..."
        assert!(result.source_preview.ends_with("..."));
    }

    #[test]
    fn extraction_config_default_uses_gemma4() {
        let config = ExtractionConfig::default();
        assert_eq!(config.model, "gemma4:e2b");
        assert_eq!(config.url, "http://localhost:11434");
    }

    #[test]
    fn ingest_triples_creates_entities_and_edges() {
        let graph = AtheneumGraph::open_in_memory().unwrap();
        let result = ExtractionResult {
            triples: vec![
                ExtractedTriple {
                    subject: "Rust".to_string(),
                    predicate: "has_feature".to_string(),
                    object: "ownership".to_string(),
                },
                ExtractedTriple {
                    subject: "ownership".to_string(),
                    predicate: "prevents".to_string(),
                    object: "data races".to_string(),
                },
            ],
            model: "test-model".to_string(),
            source_preview: "Test text...".to_string(),
        };

        let inserted = ingest_triples(&graph, &result, Some("test")).unwrap();
        assert_eq!(inserted.len(), 2);

        // Verify first triple: subject -> edge -> object
        let (s1, e1, o1) = inserted[0];
        assert!(s1 > 0);
        assert!(e1 > 0);
        assert!(o1 > 0);

        // Verify entities exist
        let subject = graph.get_entity(s1).unwrap();
        assert_eq!(subject.name, "Rust");
        let object = graph.get_entity(o1).unwrap();
        assert_eq!(object.name, "ownership");

        // Verify edge exists with provenance
        let edge = graph.get_edge(e1).unwrap();
        assert_eq!(edge.edge_type, "related_to");
        assert_eq!(edge.data["predicate"], "has_feature");
        let prov = edge.data.get("provenance").unwrap();
        assert_eq!(prov["method"], "ai_triple_extraction");
        assert_eq!(prov["extraction_mode"], "ai_triple");
    }
}
