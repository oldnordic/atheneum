use anyhow::Result;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::super::{AtheneumGraph, EdgeType, RelationEndpoint, RelationHint};

impl AtheneumGraph {
    pub(super) fn relation_hints_from_payload(&self, payload: &Value) -> Result<Vec<RelationHint>> {
        let Some(relations) = payload.get("relations") else {
            return Ok(Vec::new());
        };
        serde_json::from_value(relations.clone())
            .map_err(|e| anyhow::anyhow!("Invalid payload.relations: {}", e))
    }

    pub(super) fn ensure_relation_endpoint(&self, endpoint: &RelationEndpoint) -> Result<i64> {
        if let Some(id) = self.find_entity_id_by_kind_and_name(&endpoint.kind, &endpoint.name)? {
            return Ok(id);
        }

        let entity = GraphEntity {
            id: 0,
            kind: endpoint.kind.clone(),
            name: endpoint.name.clone(),
            file_path: endpoint.file_path.clone(),
            data: endpoint.data.clone(),
        };
        self.inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert relation endpoint: {}", e))
    }

    fn ensure_project_entity(&self, project: &str) -> Result<i64> {
        let endpoint = RelationEndpoint {
            kind: "Project".to_string(),
            name: project.to_string(),
            file_path: None,
            data: json!({ "project_id": project }),
        };
        self.ensure_relation_endpoint(&endpoint)
    }

    pub(in super::super) fn maybe_session_entity_id(
        &self,
        session_id: &str,
    ) -> Result<Option<i64>> {
        self.find_entity_id_by_data("Session", "session_id", session_id)
    }

    pub(super) fn link_entity_to_project(
        &self,
        entity_id: i64,
        project: Option<&str>,
    ) -> Result<()> {
        let Some(project) = project else {
            return Ok(());
        };
        let project_id = self.ensure_project_entity(project)?;
        self.insert_edge(
            entity_id,
            project_id,
            EdgeType::BelongsToProject,
            json!({"project_id": project}),
        )?;
        Ok(())
    }

    pub(super) fn ingest_relation_hints(&self, hints: &[RelationHint]) -> Result<Vec<i64>> {
        let mut edge_ids = Vec::with_capacity(hints.len());
        for hint in hints {
            let from_id = self.ensure_relation_endpoint(&hint.from)?;
            let to_id = self.ensure_relation_endpoint(&hint.to)?;
            let edge_id = self.insert_edge(from_id, to_id, hint.edge_type, hint.data.clone())?;
            edge_ids.push(edge_id);
        }
        Ok(edge_ids)
    }
}
