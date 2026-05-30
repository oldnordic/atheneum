use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde_json::json;
use sqlitegraph::GraphEntity;

use super::{
    AtheneumGraph, OntologyClassInfo, OntologyPropertyInfo, ONTOLOGY_CLASS_KIND,
    ONTOLOGY_PROPERTY_KIND,
};

impl AtheneumGraph {
    pub fn define_class(&self, name: &str, description: Option<&str>) -> Result<i64> {
        let existing = self.find_ontology_entity(ONTOLOGY_CLASS_KIND, name)?;

        let data = json!({
            "name": name,
            "description": description,
            "registered_at": Utc::now().to_rfc3339(),
        });

        if let Some(id) = existing {
            self.update_entity_data(id, &data)?;
            Ok(id)
        } else {
            let entity = GraphEntity {
                id: 0,
                kind: ONTOLOGY_CLASS_KIND.to_string(),
                name: name.to_string(),
                file_path: None,
                data,
            };
            self.inner
                .insert_entity(&entity)
                .map_err(|e| anyhow::anyhow!("Failed to insert OntologyClass: {}", e))
        }
    }

    pub fn define_property(
        &self,
        name: &str,
        domain_class: &str,
        range_class: &str,
        description: Option<&str>,
    ) -> Result<i64> {
        let existing = self.find_ontology_entity(ONTOLOGY_PROPERTY_KIND, name)?;

        let data = json!({
            "name": name,
            "domain_class": domain_class,
            "range_class": range_class,
            "description": description,
            "registered_at": Utc::now().to_rfc3339(),
        });

        if let Some(id) = existing {
            self.update_entity_data(id, &data)?;
            Ok(id)
        } else {
            let entity = GraphEntity {
                id: 0,
                kind: ONTOLOGY_PROPERTY_KIND.to_string(),
                name: name.to_string(),
                file_path: None,
                data,
            };
            self.inner
                .insert_entity(&entity)
                .map_err(|e| anyhow::anyhow!("Failed to insert OntologyProperty: {}", e))
        }
    }

    pub fn list_classes(&self) -> Result<Vec<OntologyClassInfo>> {
        let entities = self.entities_by_kind(ONTOLOGY_CLASS_KIND)?;
        Ok(entities
            .into_iter()
            .map(|e| OntologyClassInfo {
                id: e.id,
                name: e.name.clone(),
                description: e
                    .data
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
            .collect())
    }

    pub fn list_properties(&self) -> Result<Vec<OntologyPropertyInfo>> {
        let entities = self.entities_by_kind(ONTOLOGY_PROPERTY_KIND)?;
        Ok(entities
            .into_iter()
            .map(|e| OntologyPropertyInfo {
                id: e.id,
                name: e.name.clone(),
                domain_class: e
                    .data
                    .get("domain_class")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ANY")
                    .to_string(),
                range_class: e
                    .data
                    .get("range_class")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ANY")
                    .to_string(),
                description: e
                    .data
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
            .collect())
    }

    pub fn validate_edge(&self, from_kind: &str, to_kind: &str, edge_type: &str) -> Result<bool> {
        let props = self.list_properties()?;
        let Some(prop) = props.iter().find(|p| p.name == edge_type) else {
            return Ok(true);
        };
        let domain_ok = prop.domain_class == "ANY" || prop.domain_class == from_kind;
        let range_ok = prop.range_class == "ANY" || prop.range_class == to_kind;
        Ok(domain_ok && range_ok)
    }

    pub fn seed_standard_ontology(&self) -> Result<()> {
        const STANDARD: &[(&str, &str)] = &[
            ("Agent", "An autonomous participant"),
            ("Task", "A unit of work"),
            ("Event", "Something that happened, recorded for provenance"),
            ("Decision", "A choice made by an agent"),
            ("ToolCall", "An action taken by an agent"),
            ("FileChange", "A modification to a source file"),
            ("Verification", "A verification gate result"),
            ("Knowledge", "Persistent contextual information"),
            ("Discovery", "A dynamic insight found by an agent"),
            ("Handoff", "Context transfer between agents"),
            ("Project", "A workspace namespace"),
            ("CodeSymbol", "A source code entity"),
            ("WikiPage", "A static knowledge document"),
            ("JournalSection", "An entry in a Logseq journal"),
            ("ReasoningLog", "A stream of thought from an agent"),
            ("Session", "A single invocation of a development tool"),
            ("Commit", "A git commit recorded as evidence"),
            ("TestRun", "A test execution result"),
            ("Benchmark", "A benchmark execution result"),
            ("Release", "A software release with aggregated metrics"),
            ("EventLog", "An append-only evidence event"),
        ];
        for (name, description) in STANDARD {
            self.define_class(name, Some(description))?;
        }
        Ok(())
    }

    pub(super) fn find_ontology_entity(&self, kind: &str, name: &str) -> Result<Option<i64>> {
        let conn = if let Some(direct) = self.inner.pool.direct_connection() {
            direct
        } else {
            &self
                .inner
                .pool
                .get()
                .map_err(|e| anyhow::anyhow!("Failed to get connection: {}", e))?
        };
        let mut stmt = conn
            .prepare_cached("SELECT id FROM graph_entities WHERE kind = ?1 AND name = ?2 LIMIT 1")
            .map_err(|e| anyhow::anyhow!("Failed to prepare statement: {}", e))?;
        let id_opt: Option<i64> = stmt.query_row(params![kind, name], |row| row.get(0)).ok();
        Ok(id_opt)
    }
}
