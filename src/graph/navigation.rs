//! Graph navigation — neighbors, subgraph extraction, stats.
//!
//! These primitives let the LLM walk the graph after finding an entry point
//! via semantic search (search.rs) or direct query.

use std::collections::{HashSet, VecDeque};

use anyhow::Result;
use sqlitegraph::{GraphEdge, GraphEntity};

use super::{AtheneumGraph, EdgeType, EntityType, GraphStats, NavigateQueryPlan, SubgraphView};

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

        Ok(NavigateQueryPlan {
            original_query: query.to_string(),
            normalized_query,
            k,
            depth,
            project_id: project_id.map(str::to_string),
            requested_kind,
            resolved_kind,
            kind_repaired,
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
    ) -> Result<Vec<SubgraphView>> {
        let plan = self.preview_navigate_query(query, k, depth, project_id, entity_kind)?;
        if !plan.executable {
            anyhow::bail!(plan.errors.join("; "));
        }

        let hits = self.lexical_search(
            &plan.normalized_query,
            plan.k,
            project_id,
            plan.resolved_kind.as_deref(),
        )?;
        if hits.is_empty() {
            return Ok(Vec::new());
        }

        let mut views = Vec::with_capacity(hits.len());
        for hit in hits {
            let sg = self.get_subgraph_scoped(hit.id, depth, project_id)?;
            views.push(sg);
        }
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
        let hits = self.lexical_search(query, k, project_id, None)?;
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
}
