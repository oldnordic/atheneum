//! Graph navigation — neighbors, subgraph extraction, stats.
//!
//! These primitives let the LLM walk the graph after finding an entry point
//! via semantic search (search.rs) or direct query.

use std::collections::{HashSet, VecDeque};

use anyhow::Result;
use sqlitegraph::{GraphEdge, GraphEntity};

use super::{AtheneumGraph, GraphStats, SubgraphView};

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

        // Validate the entry itself is in scope before starting traversal.
        if !entity_in_project_scope(&entry, scope) {
            anyhow::bail!("entry entity {} is not in project scope '{}'", entry_id, scope);
        }

        let mut visited_entities: HashSet<i64> = HashSet::new();
        let mut in_scope_entities: HashSet<i64> = HashSet::new(); // entities confirmed in scope
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
                    // First time we see this entity — fetch and scope-check it.
                    if let Ok(neighbor) = self.get_entity(neighbor_id) {
                        if entity_in_project_scope(&neighbor, scope) {
                            in_scope_entities.insert(neighbor_id);
                            visited_edges.insert(edge.id);
                            edges.push(edge.clone());
                            entities.push(neighbor);
                            queue.push_back((neighbor_id, current_depth + 1));
                        }
                        // Out-of-scope: entity and edge both dropped.
                    }
                } else if in_scope_entities.contains(&neighbor_id) {
                    // Already-visited entity confirmed in scope — emit the edge.
                    if visited_edges.insert(edge.id) {
                        edges.push(edge.clone());
                    }
                }
                // If neighbor is visited but NOT in in_scope_entities, it was denied — drop edge.
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
    ) -> Result<Vec<SubgraphView>> {
        let hits = self.semantic_search(query, k, project_id)?;
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
