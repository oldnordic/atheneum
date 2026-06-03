use anyhow::Result;
use serde_json::json;
use sqlitegraph::hnsw::{DistanceMetric, HnswConfigBuilder};
use sqlitegraph::GraphEntity;
use std::collections::HashSet;

use super::{AtheneumGraph, EntityType, SearchResult};

const SEARCH_INDEX_NAME: &str = "discoveries";

fn embed_text_for_entity(entity: &GraphEntity) -> String {
    let mut parts = vec![entity.kind.clone(), entity.name.clone()];
    for key in [
        "target",
        "agent",
        "discovery_type",
        "file",
        "file_path",
        "summary",
        "signature",
        "title",
        "path",
        "body",
        "kind",
    ] {
        if let Some(value) = entity.data.get(key).and_then(|v| v.as_str()) {
            parts.push(value.to_string());
        }
    }
    if let Some(items) = entity.data.get("wikilinks").and_then(|v| v.as_array()) {
        for item in items {
            if let Some(value) = item.as_str() {
                parts.push(value.to_string());
            }
        }
    }
    parts.join(" ")
}

fn query_tokens(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| seen.insert(t.clone()))
        .collect()
}

fn lexical_token_score(entity: &GraphEntity, tokens: &[String]) -> f32 {
    if tokens.is_empty() {
        return 0.0;
    }
    let text = embed_text_for_entity(entity).to_ascii_lowercase();
    let matched = tokens.iter().filter(|token| text.contains(*token)).count();
    matched as f32 / tokens.len() as f32
}

fn search_config(dim: usize) -> Result<sqlitegraph::hnsw::HnswConfig> {
    HnswConfigBuilder::new()
        .dimension(dim)
        .distance_metric(DistanceMetric::Cosine)
        .build()
        .map_err(|e| anyhow::anyhow!("HNSW config build failed: {}", e))
}

impl AtheneumGraph {
    /// Ensure the HNSW index exists. Creates it lazily on first use.
    fn ensure_search_index(&self) -> Result<()> {
        let existing = self
            .inner
            .list_hnsw_indexes()
            .map_err(|e| anyhow::anyhow!("list_hnsw_indexes failed: {}", e))?;
        if existing.iter().any(|n| n == SEARCH_INDEX_NAME) {
            return Ok(());
        }
        let config = search_config(self.embedder.dimension())?;
        let _guard = self
            .inner
            .hnsw_index(SEARCH_INDEX_NAME, config)
            .map_err(|e| anyhow::anyhow!("hnsw_index create failed: {}", e))?;
        Ok(())
    }

    /// Add a single entity's vector to the existing HNSW index.
    pub(super) fn add_entity_to_search_index(&self, entity: &GraphEntity) -> Result<()> {
        self.ensure_search_index()?;
        let text = embed_text_for_entity(entity);
        let vector = self.embedder.embed(&text)?;
        let entity_id = entity.id;
        self.inner
            .get_hnsw_index_mut(SEARCH_INDEX_NAME, move |idx| {
                idx.insert_vector(&vector, Some(json!({"entity_id": entity_id})))
            })
            .map_err(|e| anyhow::anyhow!("get_hnsw_index_mut failed: {}", e))?
            .map_err(|e| anyhow::anyhow!("insert_vector failed: {}", e))?;
        Ok(())
    }

    /// Full rebuild of the HNSW index (still useful for manual reindexing).
    pub fn build_search_index(&self) -> Result<()> {
        let _ = self.inner.delete_hnsw_index(SEARCH_INDEX_NAME);
        self.ensure_search_index()?;
        for kind in [EntityType::Discovery.as_str(), "WikiPage", "JournalSection"] {
            for entity in self.entities_by_kind(kind)? {
                self.add_entity_to_search_index(&entity)?;
            }
        }
        Ok(())
    }

    /// Search discoveries using a hash-projected bag-of-tokens index (HNSW).
    ///
    /// Finds entities that share tokens with `query`. This is **lexical similarity**,
    /// not semantic/neural similarity — synonyms with no token overlap will not match.
    /// For true semantic search, embeddings from a language model would be needed.
    pub fn lexical_search(
        &self,
        query: &str,
        k: usize,
        project_id: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        self.ensure_search_index()?;
        let query_vec = self.embedder.embed(query)?;
        let fetch_k = if project_id.is_some() { k * 4 } else { k };

        let hits = self
            .inner
            .get_hnsw_index_ref(SEARCH_INDEX_NAME, |idx| idx.search(&query_vec, fetch_k))
            .map_err(|e| anyhow::anyhow!("search index lookup failed: {}", e))?
            .map_err(|e| anyhow::anyhow!("hnsw search failed: {}", e))?;

        let mut results = Vec::with_capacity(hits.len());
        let mut seen_entities = HashSet::new();
        for (vector_id, score) in hits {
            let metadata = self
                .inner
                .get_hnsw_index_ref(SEARCH_INDEX_NAME, |idx| {
                    idx.get_vector(vector_id).ok().flatten()
                })
                .map_err(|e| anyhow::anyhow!("get_vector failed: {}", e))?;
            let Some((_vec, meta)) = metadata else {
                continue;
            };
            let Some(entity_id) = meta.get("entity_id").and_then(|v| v.as_i64()) else {
                continue;
            };

            let entity = match self.get_entity(entity_id) {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !seen_entities.insert(entity.id) {
                continue;
            }

            if let Some(pid) = project_id {
                let entity_project = entity
                    .data
                    .get("project_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if entity_project != pid {
                    continue;
                }
            }

            results.push(SearchResult {
                id: entity.id,
                name: entity.name,
                kind: entity.kind,
                score,
                data: entity.data,
            });

            if results.len() >= k {
                break;
            }
        }

        if results.len() < k {
            let tokens = query_tokens(query);
            let mut fallback = Vec::new();
            for kind in [EntityType::Discovery.as_str(), "WikiPage", "JournalSection"] {
                for entity in self.entities_by_kind(kind)? {
                    if seen_entities.contains(&entity.id) {
                        continue;
                    }
                    if let Some(pid) = project_id {
                        let entity_project = entity
                            .data
                            .get("project_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if entity_project != pid {
                            continue;
                        }
                    }
                    let score = lexical_token_score(&entity, &tokens);
                    if score > 0.0 {
                        fallback.push((entity, score));
                    }
                }
            }
            fallback.sort_by(|(left, left_score), (right, right_score)| {
                right_score
                    .partial_cmp(left_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.name.cmp(&right.name))
            });
            for (entity, score) in fallback {
                if !seen_entities.insert(entity.id) {
                    continue;
                }
                results.push(SearchResult {
                    id: entity.id,
                    name: entity.name,
                    kind: entity.kind,
                    score,
                    data: entity.data,
                });
                if results.len() >= k {
                    break;
                }
            }
        }
        Ok(results)
    }
}
