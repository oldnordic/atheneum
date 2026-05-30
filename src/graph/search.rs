use anyhow::Result;
use serde_json::json;
use sqlitegraph::hnsw::{DistanceMetric, HnswConfigBuilder};
use sqlitegraph::GraphEntity;
use std::hash::{Hash, Hasher};

use super::{AtheneumGraph, EntityType, SearchResult};

const SEARCH_INDEX_NAME: &str = "discoveries";
const SEARCH_EMBED_DIM: usize = 128;

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
        "kind",
    ] {
        if let Some(value) = entity.data.get(key).and_then(|v| v.as_str()) {
            parts.push(value.to_string());
        }
    }
    parts.join(" ")
}

fn hash_embed(text: &str, dim: usize) -> Vec<f32> {
    let mut vector = vec![0.0_f32; dim];
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        token.to_ascii_lowercase().hash(&mut hasher);
        let bucket = (hasher.finish() as usize) % dim;
        vector[bucket] += 1.0;
    }
    let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vector {
            *v /= norm;
        }
    }
    vector
}

impl AtheneumGraph {
    pub fn build_search_index(&self) -> Result<()> {
        let _ = self.inner.delete_hnsw_index(SEARCH_INDEX_NAME);

        let config = HnswConfigBuilder::new()
            .dimension(SEARCH_EMBED_DIM)
            .distance_metric(DistanceMetric::Cosine)
            .build()
            .map_err(|e| anyhow::anyhow!("HNSW config build failed: {}", e))?;

        {
            let _guard = self
                .inner
                .hnsw_index(SEARCH_INDEX_NAME, config)
                .map_err(|e| anyhow::anyhow!("hnsw_index create failed: {}", e))?;
        }

        let discoveries = self.entities_by_kind(EntityType::Discovery.as_str())?;
        for entity in discoveries {
            let text = embed_text_for_entity(&entity);
            let vector = hash_embed(&text, SEARCH_EMBED_DIM);
            let entity_id = entity.id;
            self.inner
                .get_hnsw_index_mut(SEARCH_INDEX_NAME, move |idx| {
                    idx.insert_vector(&vector, Some(json!({"entity_id": entity_id})))
                })
                .map_err(|e| anyhow::anyhow!("get_hnsw_index_mut failed: {}", e))?
                .map_err(|e| anyhow::anyhow!("insert_vector failed: {}", e))?;
        }

        Ok(())
    }

    pub fn semantic_search(
        &self,
        query: &str,
        k: usize,
        project_id: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let query_vec = hash_embed(query, SEARCH_EMBED_DIM);
        let fetch_k = if project_id.is_some() { k * 4 } else { k };

        let hits = self
            .inner
            .get_hnsw_index_ref(SEARCH_INDEX_NAME, |idx| idx.search(&query_vec, fetch_k))
            .map_err(|e| anyhow::anyhow!("search index lookup failed: {}", e))?
            .map_err(|e| anyhow::anyhow!("hnsw search failed: {}", e))?;

        let mut results = Vec::with_capacity(hits.len());
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
        Ok(results)
    }
}
