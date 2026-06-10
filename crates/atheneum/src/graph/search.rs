use anyhow::Result;
#[cfg(feature = "semantic-search")]
use serde_json::json;
use sqlitegraph::GraphEntity;
use std::collections::HashSet;

#[cfg(feature = "semantic-search")]
use sqlitegraph::hnsw::{DistanceMetric, HnswConfigBuilder};

use super::cache::{CacheDomain, QueryCacheKey, QueryCacheValue};
use super::{AtheneumGraph, SearchResult};

#[cfg(feature = "semantic-search")]
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

#[cfg(feature = "semantic-search")]
fn search_config(dim: usize) -> Result<sqlitegraph::hnsw::HnswConfig> {
    HnswConfigBuilder::new()
        .dimension(dim)
        .distance_metric(DistanceMetric::Cosine)
        .build()
        .map_err(|e| anyhow::anyhow!("HNSW config build failed: {}", e))
}

impl AtheneumGraph {
    pub(super) fn merge_exact_match_candidates(
        &self,
        mut candidate_matches: Vec<SearchResult>,
        exact_matches: &[GraphEntity],
        k: usize,
    ) -> Vec<SearchResult> {
        for entity in exact_matches {
            if candidate_matches
                .iter()
                .any(|candidate| candidate.id == entity.id)
            {
                continue;
            }
            candidate_matches.push(SearchResult {
                id: entity.id,
                name: entity.name.clone(),
                kind: entity.kind.clone(),
                score: 1.0,
                data: entity.data.clone(),
            });
        }
        candidate_matches.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.name.cmp(&right.name))
        });
        if candidate_matches.len() > k {
            candidate_matches.truncate(k);
        }
        candidate_matches
    }

    #[cfg(feature = "semantic-search")]
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
        {
            let _guard = self
                .inner
                .hnsw_index_persistent(SEARCH_INDEX_NAME, config)
                .map_err(|e| anyhow::anyhow!("hnsw_index_persistent create failed: {}", e))?;
        }
        for entity in self.all_entities()? {
            let text = embed_text_for_entity(&entity);
            let vector = self.embedder.embed(&text)?;
            let entity_id = entity.id;
            let _ = self
                .inner
                .get_hnsw_index_mut(SEARCH_INDEX_NAME, move |idx| {
                    idx.insert_vector(&vector, Some(json!({"entity_id": entity_id})))
                });
        }
        Ok(())
    }

    /// Add a single entity's vector to the existing HNSW index.
    /// No-op when `semantic-search` feature is disabled.
    pub(super) fn add_entity_to_search_index(&self, _entity: &GraphEntity) -> Result<()> {
        #[cfg(feature = "semantic-search")]
        {
            self.ensure_search_index()?;
            let text = embed_text_for_entity(_entity);
            let vector = self.embedder.embed(&text)?;
            let entity_id = _entity.id;
            self.inner
                .get_hnsw_index_mut(SEARCH_INDEX_NAME, move |idx| {
                    idx.insert_vector(&vector, Some(json!({"entity_id": entity_id})))
                })
                .map_err(|e| anyhow::anyhow!("get_hnsw_index_mut failed: {}", e))?
                .map_err(|e| anyhow::anyhow!("insert_vector failed: {}", e))?;
        }
        Ok(())
    }

    /// Full rebuild of the HNSW index.
    /// No-op when `semantic-search` feature is disabled.
    pub fn build_search_index(&self) -> Result<()> {
        #[cfg(feature = "semantic-search")]
        {
            let _ = self.inner.delete_hnsw_index(SEARCH_INDEX_NAME);
            self.ensure_search_index()?;
        }
        Ok(())
    }

    #[cfg(feature = "semantic-search")]
    fn try_hnsw_search(
        &self,
        query_vec: &[f32],
        k: usize,
        project_id: Option<&str>,
        entity_kind: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let fetch_k = if project_id.is_some() || entity_kind.is_some() {
            k * 4
        } else {
            k
        };

        let hits = self
            .inner
            .get_hnsw_index_ref(SEARCH_INDEX_NAME, |idx| idx.search(query_vec, fetch_k))
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

            if let Some(kind) = entity_kind {
                if entity.kind != kind {
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

    fn fallback_lexical_search(
        &self,
        query: &str,
        k: usize,
        project_id: Option<&str>,
        entity_kind: Option<&str>,
        mut results: Vec<SearchResult>,
    ) -> Result<Vec<SearchResult>> {
        let tokens = query_tokens(query);
        let mut seen_entities: HashSet<i64> = results.iter().map(|r| r.id).collect();
        let mut fallback = Vec::new();
        for entity in self.all_entities()? {
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
            if let Some(kind) = entity_kind {
                if entity.kind != kind {
                    continue;
                }
            }
            let score = lexical_token_score(&entity, &tokens);
            if score > 0.0 {
                fallback.push((entity, score));
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

        Ok(results)
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
        entity_kind: Option<&str>,
        max_tokens: Option<usize>,
    ) -> Result<Vec<SearchResult>> {
        self.runtime.record_search_query();
        let cache_key = QueryCacheKey::LexicalSearch {
            query: query.to_string(),
            k,
            project_id: project_id.map(str::to_string),
            entity_kind: entity_kind.map(str::to_string),
            max_tokens,
        };
        if let Some(QueryCacheValue::SearchResults(results)) =
            self.runtime.cache_get(&cache_key, CacheDomain::Search)
        {
            return Ok(results);
        }

        #[cfg(feature = "semantic-search")]
        let results = {
            let query_vec = self.embedder.embed(query)?;
            let hnsw_results = match self.ensure_search_index() {
                Ok(()) => match self.try_hnsw_search(&query_vec, k, project_id, entity_kind) {
                    Ok(results) => Some(results),
                    Err(first_err) => {
                        eprintln!("[atheneum] search index warning: {}", first_err);
                        match self.build_search_index() {
                            Ok(()) => {
                                match self.try_hnsw_search(&query_vec, k, project_id, entity_kind) {
                                    Ok(results) => Some(results),
                                    Err(second_err) => {
                                        eprintln!(
                                            "[atheneum] search index rebuild warning: {}",
                                            second_err
                                        );
                                        None
                                    }
                                }
                            }
                            Err(rebuild_err) => {
                                eprintln!(
                                    "[atheneum] search index rebuild failed: {}",
                                    rebuild_err
                                );
                                None
                            }
                        }
                    }
                },
                Err(err) => {
                    eprintln!("[atheneum] search index unavailable: {}", err);
                    None
                }
            };

            let results = hnsw_results.unwrap_or_default();
            if results.len() >= k {
                self.runtime.record_hnsw_hit();
                results
            } else {
                if !results.is_empty() {
                    self.runtime.record_hnsw_hit();
                }
                self.runtime.record_hnsw_fallback_scan();
                self.fallback_lexical_search(query, k, project_id, entity_kind, results)?
            }
        };

        #[cfg(not(feature = "semantic-search"))]
        let results =
            self.fallback_lexical_search(query, k, project_id, entity_kind, Vec::new())?;

        let results = if let Some(max_tokens) = max_tokens {
            let mut budget = max_tokens;
            let mut kept = Vec::new();
            for result in results {
                let cost =
                    (result.name.len() + result.kind.len() + result.data.to_string().len() + 20)
                        / 4;
                if cost <= budget {
                    kept.push(result);
                    budget = budget.saturating_sub(cost);
                } else {
                    break;
                }
            }
            kept
        } else {
            results
        };

        self.runtime.cache_store(
            cache_key,
            CacheDomain::Search,
            QueryCacheValue::SearchResults(results.clone()),
        );
        Ok(results)
    }

    /// Return ranked existing entities for a fuzzy identifier without mutating the graph.
    pub fn preview_entity_candidates(
        &self,
        query: &str,
        k: usize,
        project_id: Option<&str>,
        entity_kind: Option<&str>,
        min_score: f32,
    ) -> Result<Vec<SearchResult>> {
        let mut results = self.lexical_search(query, k, project_id, entity_kind, None)?;
        results.retain(|result| result.score >= min_score);
        results.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(results)
    }

    /// Find the top-k entities most similar to a given name using vector search.
    ///
    /// This is the entity-disambiguation entry point: given a candidate name,
    /// return the most similar existing entities from the graph, ranked by
    /// vector similarity score.
    pub fn get_similar(
        &self,
        name: &str,
        top_k: usize,
        project_id: Option<&str>,
        entity_kind: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        self.lexical_search(name, top_k, project_id, entity_kind, None)
    }

    /// Resolve a name to a single best-matching entity above a confidence threshold.
    ///
    /// Returns a `DisambiguationResult` with the resolved entity (if confidence
    /// is met), all candidates for inspection, and the threshold used. Callers
    /// can use `result.is_resolved()` to check if resolution succeeded, and
    /// inspect `candidates` for alternatives.
    pub fn resolve(
        &self,
        name: &str,
        min_confidence: f32,
        project_id: Option<&str>,
        entity_kind: Option<&str>,
    ) -> Result<super::DisambiguationResult> {
        let candidates = self.get_similar(name, 10, project_id, entity_kind)?;
        let resolved = candidates.first().and_then(|top| {
            if top.score >= min_confidence {
                Some(top.clone())
            } else {
                None
            }
        });
        Ok(super::DisambiguationResult {
            resolved,
            candidates,
            min_confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_graph_with_entities() -> AtheneumGraph {
        let graph = AtheneumGraph::open_in_memory().unwrap();
        graph
            .store_memory(
                "rust-ownership",
                "Rust ownership rules",
                "memory",
                1.0,
                Some("test"),
                None,
            )
            .unwrap();
        graph
            .store_memory(
                "rust-borrowing",
                "Rust borrowing and lifetimes",
                "memory",
                0.9,
                Some("test"),
                None,
            )
            .unwrap();
        graph
            .insert_event(
                "Ownership transfer semantics",
                serde_json::json!({"scope": "discovery", "project_id": "test"}),
            )
            .unwrap();
        graph
    }

    #[test]
    fn get_similar_returns_ranked_results() {
        let graph = make_graph_with_entities();
        let results = graph.get_similar("Rust ownership", 5, None, None).unwrap();
        assert!(!results.is_empty(), "should find similar entities");
        assert!(
            results[0].score > 0.0,
            "top result should have positive score"
        );
    }

    #[test]
    fn get_similar_filters_by_kind() {
        let graph = make_graph_with_entities();
        let memory_only = graph
            .get_similar("Rust ownership", 5, None, Some("Memory"))
            .unwrap();
        assert!(
            memory_only.iter().all(|r| r.kind == "Memory"),
            "all results should be Memory kind"
        );
    }

    #[test]
    fn get_similar_filters_by_project() {
        let graph = make_graph_with_entities();
        let in_project = graph.get_similar("Rust", 5, Some("test"), None).unwrap();
        let out_project = graph.get_similar("Rust", 5, Some("other"), None).unwrap();
        assert!(
            in_project.len() > out_project.len(),
            "project filter should narrow results"
        );
    }

    #[test]
    fn resolve_succeeds_with_high_confidence_match() {
        let graph = make_graph_with_entities();
        let candidates = graph
            .get_similar("rust ownership", 5, Some("test"), None)
            .unwrap();
        assert!(!candidates.is_empty(), "should find at least one candidate");
        let top_score = candidates.first().map(|c| c.score).unwrap_or(0.0);
        assert!(top_score > 0.0, "top candidate should have positive score");
        // Resolve with a threshold just below the top score
        let result = graph
            .resolve("rust ownership", top_score * 0.9, Some("test"), None)
            .unwrap();
        assert!(
            result.is_resolved(),
            "should resolve when threshold is below top score"
        );
        assert!(!result.candidates.is_empty());
    }

    #[test]
    fn resolve_fails_with_high_threshold() {
        let graph = make_graph_with_entities();
        let result = graph
            .resolve("completely unrelated xyz", 0.99, None, None)
            .unwrap();
        assert!(
            !result.is_resolved(),
            "should not resolve with very high threshold"
        );
    }
}
