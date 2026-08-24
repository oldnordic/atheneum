//! Graph navigation — neighbors, subgraph extraction, stats.
//!
//! These primitives let the LLM walk the graph after finding an entry point
//! via semantic search (search.rs) or direct query.

use std::collections::{HashSet, VecDeque};

use anyhow::Result;
use sqlitegraph::{GraphEdge, GraphEntity};

use super::cache::{CacheDomain, QueryCacheKey, QueryCacheValue};
use super::{
    AtheneumGraph, EdgeType, EntityType, GraphStats, NavigateQueryPlan, QueryIntent,
    ResolvedEntity, SearchResult, SubgraphView,
};

const CHARS_PER_TOKEN: usize = 4;

pub fn estimate_entity_tokens(entity: &GraphEntity) -> usize {
    let mut chars = entity.kind.len() + entity.name.len();
    if let Some(ref fp) = entity.file_path {
        chars += fp.len();
    }
    chars += entity.data.to_string().len();
    chars / CHARS_PER_TOKEN
}

fn estimate_edge_tokens(edge: &GraphEdge) -> usize {
    let chars = edge.edge_type.len() + edge.data.to_string().len() + 20;
    chars / CHARS_PER_TOKEN
}

pub fn truncate_subgraph(view: SubgraphView, max_tokens: usize) -> SubgraphView {
    let entry_id = view.entry.id;
    let entry_tokens = estimate_entity_tokens(&view.entry);

    if entry_tokens >= max_tokens {
        return SubgraphView {
            entry: view.entry,
            depth: view.depth,
            entities: vec![],
            edges: vec![],
        };
    }

    let mut budget = max_tokens.saturating_sub(entry_tokens);
    let mut kept_entities = vec![];
    let mut kept_entity_ids = HashSet::new();
    kept_entity_ids.insert(entry_id);

    for entity in &view.entities {
        if entity.id == entry_id {
            continue;
        }
        let cost = estimate_entity_tokens(entity);
        if cost <= budget {
            kept_entities.push(entity.clone());
            kept_entity_ids.insert(entity.id);
            budget = budget.saturating_sub(cost);
        }
    }

    let mut kept_edges = vec![];
    for edge in view.edges {
        if !kept_entity_ids.contains(&edge.from_id) || !kept_entity_ids.contains(&edge.to_id) {
            continue;
        }
        let cost = estimate_edge_tokens(&edge);
        if cost <= budget {
            kept_edges.push(edge);
            budget = budget.saturating_sub(cost);
        }
    }

    SubgraphView {
        entry: view.entry,
        depth: view.depth,
        entities: kept_entities,
        edges: kept_edges,
    }
}

/// Scope predicate shared by all navigation and traversal functions.
///
/// Policy:
/// - Entity has `project_id == scope`  → in scope (allowed)
/// - Entity has no `project_id` field  → shared/global (allowed)
/// - Entity has a different `project_id` → out of scope (denied)
///
/// Callers that want a stricter policy (deny entities with no project_id) must
/// apply an additional filter — this function encodes the "absent = shared" rule.
pub(crate) fn entity_in_project_scope(entity: &sqlitegraph::GraphEntity, scope: &str) -> bool {
    match entity.data.get("project_id").and_then(|v| v.as_str()) {
        Some(pid) => pid == scope,
        None => true, // no project_id = shared/global entity
    }
}

impl AtheneumGraph {
    pub fn preview_navigate_query(
        &self,
        query: &str,
        k: usize,
        depth: u32,
        project_id: Option<&str>,
        entity_kind: Option<&str>,
    ) -> Result<NavigateQueryPlan> {
        let normalized_query = query.trim().to_string();
        let mut warnings = Vec::new();
        let mut errors = Vec::new();
        let requested_kind = entity_kind.map(str::to_string);
        let mut resolved_kind = None;
        let mut kind_repaired = false;

        if normalized_query.is_empty() {
            errors.push("query must not be empty after trimming".to_string());
        } else if normalized_query != query {
            warnings.push("query was trimmed before execution".to_string());
        }

        if let Some(kind) = entity_kind {
            match EntityType::from_query_label(kind) {
                Some(resolved) => {
                    let canonical = resolved.as_str().to_string();
                    kind_repaired = kind != canonical;
                    if kind_repaired {
                        warnings.push(format!(
                            "entity kind repaired from '{}' to '{}'",
                            kind, canonical
                        ));
                    }
                    resolved_kind = Some(canonical);
                }
                None => {
                    errors.push(format!(
                        "unknown entity kind '{}'; expected one of: {}",
                        kind,
                        EntityType::query_labels().join(", ")
                    ));
                }
            }
        }

        // Entity resolution: try to resolve query terms to graph entities
        let mut resolved_entities = Vec::new();
        if !normalized_query.is_empty() {
            let terms: Vec<&str> = normalized_query
                .split_whitespace()
                .filter(|w| w.len() > 2) // skip short words
                .collect();

            for term in &terms {
                let disambiguation = self.resolve(term, 0.3, project_id, resolved_kind.as_deref());
                match disambiguation {
                    Ok(result) => {
                        let clean_candidates: Vec<SearchResult> = result
                            .candidates
                            .into_iter()
                            .map(|c| SearchResult {
                                id: c.id,
                                name: c.name,
                                kind: c.kind,
                                score: c.score,
                                data: serde_json::json!({}),
                            })
                            .collect();
                        let (entity_id, entity_name, confidence, alternatives) =
                            if let Some(resolved) = &result.resolved {
                                (
                                    Some(resolved.id),
                                    Some(resolved.name.clone()),
                                    resolved.score,
                                    clean_candidates,
                                )
                            } else if !clean_candidates.is_empty() {
                                let top = &clean_candidates[0];
                                (None, Some(top.name.clone()), top.score, clean_candidates)
                            } else {
                                (None, None, 0.0, vec![])
                            };

                        if entity_name.is_some() || !alternatives.is_empty() {
                            resolved_entities.push(ResolvedEntity {
                                query_term: term.to_string(),
                                entity_id,
                                entity_name,
                                confidence,
                                alternatives,
                            });
                        }
                    }
                    Err(_) => {
                        // Resolution failed silently -- not an error for preview
                    }
                }
            }

            // Add warning if no entities could be resolved
            if resolved_entities.is_empty() && !terms.is_empty() {
                warnings.push("no query terms matched any graph entities".to_string());
            }
        }

        Ok(NavigateQueryPlan {
            original_query: query.to_string(),
            intent: QueryIntent::classify(&normalized_query),
            normalized_query,
            k,
            depth,
            project_id: project_id.map(str::to_string),
            requested_kind,
            resolved_kind,
            kind_repaired,
            resolved_entities,
            executable: errors.is_empty(),
            warnings,
            errors,
        })
    }

    /// Return (outgoing_edges, incoming_edges) for a single entity.
    pub fn get_neighbors(&self, entity_id: i64) -> Result<(Vec<GraphEdge>, Vec<GraphEdge>)> {
        Ok((
            self.outgoing_edges(entity_id)?,
            self.incoming_edges(entity_id)?,
        ))
    }

    /// Extract a connected subgraph around `entry_id` by BFS up to `depth`.
    ///
    /// Returns the entry entity, all reached entities, and all traversed edges.
    pub fn get_subgraph(&self, entry_id: i64, depth: u32) -> Result<SubgraphView> {
        let entry = self.get_entity(entry_id)?;

        let mut visited_entities: HashSet<i64> = HashSet::new();
        let mut visited_edges: HashSet<i64> = HashSet::new();
        let mut entities: Vec<GraphEntity> = Vec::new();
        let mut edges: Vec<GraphEdge> = Vec::new();
        let mut queue: VecDeque<(i64, u32)> = VecDeque::new();

        queue.push_back((entry_id, 0));
        visited_entities.insert(entry_id);
        entities.push(entry.clone());

        while let Some((current_id, current_depth)) = queue.pop_front() {
            if current_depth >= depth {
                continue;
            }

            // Navigate both directions — the graph is semantic, not strictly directed
            let out = self.outgoing_edges(current_id).unwrap_or_default();
            let inc = self.incoming_edges(current_id).unwrap_or_default();

            for edge in out.into_iter().chain(inc) {
                if !visited_edges.insert(edge.id) {
                    continue;
                }
                edges.push(edge.clone());

                let neighbor_id = if edge.from_id == current_id {
                    edge.to_id
                } else {
                    edge.from_id
                };

                if visited_entities.insert(neighbor_id) {
                    if let Ok(neighbor) = self.get_entity(neighbor_id) {
                        entities.push(neighbor.clone());
                        queue.push_back((neighbor_id, current_depth + 1));
                    }
                }
            }
        }

        Ok(SubgraphView {
            entry,
            depth,
            entities,
            edges,
        })
    }

    /// Extract a connected subgraph scoped to `project_id`.
    ///
    /// Neighbors whose `data.project_id` does not match are excluded, along
    /// with any edges that would point to them. Entities with no `project_id`
    /// in their data are treated as shared/global and always included.
    ///
    /// When `project_id` is None the call delegates to `get_subgraph` (no filter).
    pub fn get_subgraph_scoped(
        &self,
        entry_id: i64,
        depth: u32,
        project_id: Option<&str>,
    ) -> Result<SubgraphView> {
        let Some(scope) = project_id else {
            return self.get_subgraph(entry_id, depth);
        };

        let entry = self.get_entity(entry_id)?;

        if !entity_in_project_scope(&entry, scope) {
            anyhow::bail!(
                "entry entity {} is not in project scope '{}'",
                entry_id,
                scope
            );
        }

        let mut visited_entities: HashSet<i64> = HashSet::new();
        let mut in_scope_entities: HashSet<i64> = HashSet::new();
        let mut visited_edges: HashSet<i64> = HashSet::new();
        let mut entities: Vec<GraphEntity> = Vec::new();
        let mut edges: Vec<GraphEdge> = Vec::new();
        let mut queue: VecDeque<(i64, u32)> = VecDeque::new();

        queue.push_back((entry_id, 0));
        visited_entities.insert(entry_id);
        in_scope_entities.insert(entry_id);
        entities.push(entry.clone());

        while let Some((current_id, current_depth)) = queue.pop_front() {
            if current_depth >= depth {
                continue;
            }

            let out = self.outgoing_edges(current_id).unwrap_or_default();
            let inc = self.incoming_edges(current_id).unwrap_or_default();

            for edge in out.into_iter().chain(inc) {
                if visited_edges.contains(&edge.id) {
                    continue;
                }

                let neighbor_id = if edge.from_id == current_id {
                    edge.to_id
                } else {
                    edge.from_id
                };

                if visited_entities.insert(neighbor_id) {
                    if let Ok(neighbor) = self.get_entity(neighbor_id) {
                        if entity_in_project_scope(&neighbor, scope) {
                            in_scope_entities.insert(neighbor_id);
                            visited_edges.insert(edge.id);
                            edges.push(edge.clone());
                            entities.push(neighbor);
                            queue.push_back((neighbor_id, current_depth + 1));
                        }
                    }
                } else if in_scope_entities.contains(&neighbor_id) && visited_edges.insert(edge.id)
                {
                    edges.push(edge.clone());
                }
            }
        }

        Ok(SubgraphView {
            entry,
            depth,
            entities,
            edges,
        })
    }

    pub fn get_subgraph_filtered(
        &self,
        entry_id: i64,
        depth: u32,
        allowed_types: &[EdgeType],
    ) -> Result<SubgraphView> {
        let entry = self.get_entity(entry_id)?;

        if allowed_types.is_empty() {
            return self.get_subgraph(entry_id, depth);
        }

        let allowed_labels: HashSet<&str> = allowed_types.iter().map(|t| t.as_str()).collect();

        let mut visited_entities: HashSet<i64> = HashSet::new();
        let mut visited_edges: HashSet<i64> = HashSet::new();
        let mut entities: Vec<GraphEntity> = Vec::new();
        let mut edges: Vec<GraphEdge> = Vec::new();
        let mut queue: VecDeque<(i64, u32)> = VecDeque::new();

        queue.push_back((entry_id, 0));
        visited_entities.insert(entry_id);
        entities.push(entry.clone());

        while let Some((current_id, current_depth)) = queue.pop_front() {
            if current_depth >= depth {
                continue;
            }

            let out = self.outgoing_edges(current_id).unwrap_or_default();
            let inc = self.incoming_edges(current_id).unwrap_or_default();

            for edge in out.into_iter().chain(inc) {
                if !allowed_labels.contains(edge.edge_type.as_str()) {
                    continue;
                }
                if !visited_edges.insert(edge.id) {
                    continue;
                }
                edges.push(edge.clone());

                let neighbor_id = if edge.from_id == current_id {
                    edge.to_id
                } else {
                    edge.from_id
                };

                if visited_entities.insert(neighbor_id) {
                    if let Ok(neighbor) = self.get_entity(neighbor_id) {
                        entities.push(neighbor.clone());
                        queue.push_back((neighbor_id, current_depth + 1));
                    }
                }
            }
        }

        Ok(SubgraphView {
            entry,
            depth,
            entities,
            edges,
        })
    }

    /// Semantic search entry point → walk the graph → return subgraph views.
    ///
    /// Applies the same `project_id` scope to graph traversal as to the
    /// initial semantic search — cross-project entities are not reachable
    /// via edges from in-scope hits.
    pub fn navigate(
        &self,
        query: &str,
        k: usize,
        depth: u32,
        project_id: Option<&str>,
        entity_kind: Option<&str>,
        max_tokens: Option<usize>,
    ) -> Result<Vec<SubgraphView>> {
        self.runtime.record_navigation_query();
        let cache_key = QueryCacheKey::Navigate {
            query: query.to_string(),
            k,
            depth,
            project_id: project_id.map(str::to_string),
            entity_kind: entity_kind.map(str::to_string),
            max_tokens,
        };
        if let Some(QueryCacheValue::SubgraphViews(views)) =
            self.runtime.cache_get(&cache_key, CacheDomain::Navigation)
        {
            return Ok(views);
        }

        let plan = self.preview_navigate_query(query, k, depth, project_id, entity_kind)?;
        if !plan.executable {
            anyhow::bail!(plan.errors.join("; "));
        }

        let hits = self.lexical_search(
            &plan.normalized_query,
            plan.k,
            project_id,
            plan.resolved_kind.as_deref(),
            None,
        )?;
        if hits.is_empty() {
            return Ok(Vec::new());
        }

        let mut views = Vec::with_capacity(hits.len());
        for hit in hits {
            let sg = self.get_subgraph_scoped(hit.id, depth, project_id)?;
            let sg = if let Some(max_tokens) = max_tokens {
                truncate_subgraph(sg, max_tokens)
            } else {
                sg
            };
            views.push(sg);
        }
        self.runtime.cache_store(
            cache_key,
            CacheDomain::Navigation,
            QueryCacheValue::SubgraphViews(views.clone()),
        );
        Ok(views)
    }

    pub fn hopgraph_query(
        &self,
        query: &str,
        k: usize,
        depth: u32,
        allowed_types: &[EdgeType],
        max_tokens: usize,
        project_id: Option<&str>,
    ) -> Result<Vec<SubgraphView>> {
        self.runtime.record_navigation_query();
        let allowed_types_key = allowed_types
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let cache_key = QueryCacheKey::Hopgraph {
            query: query.to_string(),
            k,
            depth,
            allowed_types_key,
            max_tokens,
            project_id: project_id.map(str::to_string),
        };
        if let Some(QueryCacheValue::SubgraphViews(views)) =
            self.runtime.cache_get(&cache_key, CacheDomain::Navigation)
        {
            return Ok(views);
        }

        let hits = self.lexical_search(query, k, project_id, None, None)?;
        if hits.is_empty() {
            return Ok(Vec::new());
        }

        let mut budget = max_tokens;
        let mut views = Vec::new();

        for hit in hits {
            let full_sg = if allowed_types.is_empty() {
                self.get_subgraph_scoped(hit.id, depth, project_id)?
            } else {
                self.get_subgraph_filtered(hit.id, depth, allowed_types)?
            };

            let sg = truncate_subgraph(full_sg, budget);
            let used = estimate_entity_tokens(&sg.entry)
                + sg.entities
                    .iter()
                    .map(estimate_entity_tokens)
                    .sum::<usize>();

            if used > 0 {
                budget = budget.saturating_sub(used);
                views.push(sg);
            }

            if budget == 0 {
                break;
            }
        }
        self.runtime.cache_store(
            cache_key,
            CacheDomain::Navigation,
            QueryCacheValue::SubgraphViews(views.clone()),
        );
        Ok(views)
    }

    /// Thread navigation — semantic match on `ReasoningLog` + `Discovery`
    /// entities, then BFS outward along `caused_by`/`led_to` chain edges only,
    /// bounded to `max_tokens`.
    ///
    /// This is the Phase 2 wrapper over the existing scoped BFS
    /// (`get_subgraph_filtered`): it restricts entry points to decision
    /// entities and restricts the walk to thread edges, so each returned
    /// subgraph is a chronological decision chain rather than the full
    /// neighborhood `navigate` would return.
    ///
    /// `k` controls how many entry points are expanded; `depth` how far each
    /// chain is followed. ReasoningLogs are included as entry points (search
    /// targets) even though `store_discovery` never auto-links them — a thread
    /// may be anchored on a reasoning turn and then walk into linked
    /// discoveries. Per Open Decision #2 (ReasoningLog has no decision-tag
    /// field), only discoveries ever emit chain edges; reasoning turns surface
    /// only when they themselves are the query match.
    pub fn thread_query(
        &self,
        query: &str,
        k: usize,
        depth: u32,
        project_id: Option<&str>,
        max_tokens: usize,
    ) -> Result<Vec<SubgraphView>> {
        self.runtime.record_navigation_query();
        let cache_key = QueryCacheKey::Hopgraph {
            query: query.to_string(),
            k,
            depth,
            allowed_types_key: "thread:caused_by,led_to".to_string(),
            max_tokens,
            project_id: project_id.map(str::to_string),
        };
        if let Some(QueryCacheValue::SubgraphViews(views)) =
            self.runtime.cache_get(&cache_key, CacheDomain::Navigation)
        {
            return Ok(views);
        }

        // Entry points: ReasoningLog + Discovery matches, merged by score,
        // deduped by id, capped to k. Two searches because lexical_search
        // takes a single entity_kind filter.
        let mut hits = self.lexical_search(query, k, project_id, Some("ReasoningLog"), None)?;
        hits.extend(self.lexical_search(query, k, project_id, Some("Discovery"), None)?);
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.dedup_by(|a, b| a.id == b.id);
        hits.truncate(k);
        if hits.is_empty() {
            return Ok(Vec::new());
        }

        let allowed = [EdgeType::CausedBy, EdgeType::LedTo];
        let mut budget = max_tokens;
        let mut views = Vec::new();
        for hit in hits {
            let full_sg = self.get_subgraph_filtered(hit.id, depth, &allowed)?;
            let sg = truncate_subgraph(full_sg, budget);
            let used = estimate_entity_tokens(&sg.entry)
                + sg.entities
                    .iter()
                    .map(estimate_entity_tokens)
                    .sum::<usize>();
            if used > 0 {
                budget = budget.saturating_sub(used);
                views.push(sg);
            }
            if budget == 0 {
                break;
            }
        }
        self.runtime.cache_store(
            cache_key,
            CacheDomain::Navigation,
            QueryCacheValue::SubgraphViews(views.clone()),
        );
        Ok(views)
    }

    /// Fast topological stats (entity + edge counts by kind / type).
    pub fn graph_stats(&self) -> Result<GraphStats> {
        let entity_counts = self.count_entities_by_kind()?;
        let edge_counts = self.count_edges_by_type()?;
        let total_entities: i64 = entity_counts.iter().map(|(_, c)| c).sum();
        let total_edges: i64 = edge_counts.iter().map(|(_, c)| c).sum();

        Ok(GraphStats {
            total_entities,
            total_edges,
            entity_counts,
            edge_counts,
        })
    }

    pub fn trace_query(&self, plan: &NavigateQueryPlan, result_ids: &[i64]) -> Result<i64> {
        let started_at = chrono::Utc::now().to_rfc3339();
        let finished_at = chrono::Utc::now().to_rfc3339();

        let data = serde_json::json!({
            "plan": plan,
            "result_ids": result_ids,
            "started_at": started_at,
            "finished_at": finished_at,
        });

        let name = format!("Trace: {} @ {}", plan.normalized_query, started_at);
        let trace_id = self.insert_entity_and_index(sqlitegraph::GraphEntity {
            id: 0,
            kind: EntityType::QueryTrace.as_str().to_string(),
            name,
            file_path: None,
            data,
        })?;

        for &to_id in result_ids {
            self.insert_edge(
                trace_id,
                to_id,
                EdgeType::ProducedBy,
                serde_json::Value::Null,
            )?;
        }

        Ok(trace_id)
    }

    // reason: mirrors `navigate`'s existing 6-argument signature plus `trace`;
    // splitting into a params struct would ripple through CLI and MCP call
    // sites for one extra bool.
    #[allow(clippy::too_many_arguments)]
    pub fn navigate_with_trace(
        &self,
        query: &str,
        k: usize,
        depth: u32,
        project_id: Option<&str>,
        entity_kind: Option<&str>,
        max_tokens: Option<usize>,
        trace: bool,
    ) -> Result<(Vec<SubgraphView>, Option<i64>)> {
        if !trace {
            let views = self.navigate(query, k, depth, project_id, entity_kind, max_tokens)?;
            return Ok((views, None));
        }

        let plan = self.preview_navigate_query(query, k, depth, project_id, entity_kind)?;
        if !plan.executable {
            anyhow::bail!(plan.errors.join("; "));
        }

        let hits = self.lexical_search(
            &plan.normalized_query,
            plan.k,
            project_id,
            plan.resolved_kind.as_deref(),
            None,
        )?;

        let mut views = Vec::with_capacity(hits.len());
        let mut result_ids = Vec::new();
        for hit in &hits {
            result_ids.push(hit.id);
            let sg = self.get_subgraph_scoped(hit.id, depth, project_id)?;
            let sg = if let Some(max_tokens) = max_tokens {
                truncate_subgraph(sg, max_tokens)
            } else {
                sg
            };
            views.push(sg);
        }

        let trace_id = self.trace_query(&plan, &result_ids)?;

        Ok((views, Some(trace_id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_graph() -> AtheneumGraph {
        AtheneumGraph::open_in_memory().unwrap()
    }

    #[test]
    fn intent_classify_search() {
        assert_eq!(
            QueryIntent::classify("find entities related to rust"),
            QueryIntent::Search
        );
        assert_eq!(
            QueryIntent::classify("search for memory"),
            QueryIntent::Search
        );
        assert_eq!(
            QueryIntent::classify("what is ownership"),
            QueryIntent::Search
        );
    }

    #[test]
    fn intent_classify_navigate() {
        assert_eq!(
            QueryIntent::classify("neighbors of rust-ownership"),
            QueryIntent::Navigate
        );
        assert_eq!(
            QueryIntent::classify("explore connections around lending"),
            QueryIntent::Navigate
        );
        assert_eq!(
            QueryIntent::classify("edges from concept"),
            QueryIntent::Navigate
        );
    }

    #[test]
    fn intent_classify_path() {
        assert_eq!(
            QueryIntent::classify("path from ownership to borrowing"),
            QueryIntent::Path
        );
        assert_eq!(
            QueryIntent::classify("how to get between concepts"),
            QueryIntent::Path
        );
    }

    #[test]
    fn intent_classify_unknown() {
        assert_eq!(QueryIntent::classify("rust"), QueryIntent::Unknown);
        assert_eq!(
            QueryIntent::classify("the quick brown fox"),
            QueryIntent::Unknown
        );
    }

    #[test]
    fn preview_plan_classifies_intent() {
        let graph = make_graph();
        let plan = graph
            .preview_navigate_query("find rust concepts", 5, 2, None, None)
            .unwrap();
        assert_eq!(plan.intent, QueryIntent::Search);
        assert!(plan.executable);
    }

    #[test]
    fn preview_plan_resolves_entities() {
        let graph = make_graph();
        // Seed an entity to resolve against
        graph
            .upsert_concept(
                "rust-ownership",
                &serde_json::json!({"topic": "memory safety"}),
            )
            .unwrap();

        let plan = graph
            .preview_navigate_query("rust-ownership", 5, 2, None, None)
            .unwrap();
        // Should have resolved at least one entity
        assert!(
            !plan.resolved_entities.is_empty(),
            "expected resolved entities for 'rust-ownership'"
        );
        let resolved = &plan.resolved_entities[0];
        assert!(
            resolved.confidence > 0.0,
            "expected positive confidence, got {}",
            resolved.confidence
        );
    }

    #[test]
    fn preview_plan_warns_no_match() {
        let graph = make_graph();
        // Empty graph -- nothing to resolve
        let plan = graph
            .preview_navigate_query("nonexistent_xyzzy_entity", 5, 2, None, None)
            .unwrap();
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("no query terms matched")),
            "expected no-match warning, got {:?}",
            plan.warnings
        );
    }

    #[test]
    fn preview_plan_repairs_kind() {
        let graph = make_graph();
        let plan = graph
            .preview_navigate_query("test query", 5, 2, None, Some("memory"))
            .unwrap();
        assert_eq!(plan.resolved_kind.as_deref(), Some("Memory"));
        assert!(
            plan.kind_repaired,
            "expected kind to be repaired from 'memory' to 'Memory'"
        );
        assert!(plan.warnings.iter().any(|w| w.contains("repaired")));
    }
}
