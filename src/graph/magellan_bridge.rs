use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::{AtheneumGraph, EntityType};

#[derive(Debug, Clone)]
struct MagellanSymbol {
    name: String,
    file_path: Option<String>,
    data: Value,
}

fn read_magellan_symbols(
    db_path: &std::path::Path,
    name_filter: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<MagellanSymbol>> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| {
                anyhow::anyhow!("Failed to open magellan DB {}: {}", db_path.display(), e)
            })?;

    let mut sql =
        String::from("SELECT name, file_path, data FROM graph_entities WHERE kind = 'Symbol'");
    if name_filter.is_some() {
        sql.push_str(" AND name = ?1");
    }
    if let Some(n) = limit {
        sql.push_str(&format!(" LIMIT {}", n));
    }

    let mut stmt = conn.prepare(&sql)?;
    let mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<MagellanSymbol> {
        let data_str: String = row.get(2)?;
        let data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);
        Ok(MagellanSymbol {
            name: row.get::<_, String>(0)?,
            file_path: row.get::<_, Option<String>>(1)?,
            data,
        })
    };

    let rows: Vec<MagellanSymbol> = if let Some(filter) = name_filter {
        stmt.query_map(rusqlite::params![filter], mapper)?
            .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], mapper)?.collect::<Result<Vec<_>, _>>()?
    };
    Ok(rows)
}

fn symbol_to_metadata(sym: &MagellanSymbol) -> Value {
    let start_line = sym.data.get("start_line").cloned().unwrap_or(Value::Null);
    let end_line = sym.data.get("end_line").cloned().unwrap_or(Value::Null);
    let fqn = sym.data.get("fqn").cloned().unwrap_or(Value::Null);
    let kind = sym.data.get("kind").cloned().unwrap_or(Value::Null);
    json!({
        "file": sym.file_path,
        "start_line": start_line,
        "end_line": end_line,
        "fqn": fqn,
        "kind": kind,
        "magellan_raw": sym.data,
    })
}

impl AtheneumGraph {
    pub fn import_symbol_from_magellan(
        &self,
        magellan_db_path: &std::path::Path,
        symbol_name: &str,
        agent_name: &str,
        project_id: Option<&str>,
    ) -> Result<Option<i64>> {
        let symbols = read_magellan_symbols(magellan_db_path, Some(symbol_name), None)?;
        if symbols.is_empty() {
            return Ok(None);
        }
        let sym = &symbols[0];
        let metadata = symbol_to_metadata(sym);
        self.upsert_symbol_discovery(agent_name, symbol_name, project_id, metadata)
            .map(Some)
    }

    pub fn import_all_symbols_from_magellan(
        &self,
        magellan_db_path: &std::path::Path,
        agent_name: &str,
        project_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<usize> {
        let symbols = read_magellan_symbols(magellan_db_path, None, limit)?;
        let mut count = 0;
        for sym in &symbols {
            let metadata = symbol_to_metadata(sym);
            let target = sym.name.clone();
            self.upsert_symbol_discovery(agent_name, &target, project_id, metadata)?;
            count += 1;
        }
        Ok(count)
    }

    fn upsert_symbol_discovery(
        &self,
        agent_name: &str,
        target: &str,
        project_id: Option<&str>,
        mut metadata: Value,
    ) -> Result<i64> {
        let agent_s = agent_name.to_string();
        let target_s = target.to_string();
        let project_s = project_id.map(|s| s.to_string());
        let existing_sql_id: Option<i64> = self.with_raw_connection(|conn| {
            let row = conn
                .query_row(
                    "SELECT id FROM discoveries
                     WHERE agent_name = ?1 AND target = ?2 AND discovery_type = 'Symbol'
                       AND ((project_id IS NULL AND ?3 IS NULL) OR project_id = ?3)
                     LIMIT 1",
                    rusqlite::params![agent_s, target_s, project_s],
                    |r| r.get::<_, i64>(0),
                )
                .ok();
            Ok(row)
        })?;

        if let Some(sql_id) = existing_sql_id {
            let agent_s = agent_name.to_string();
            let target_s = target.to_string();
            let project_s = project_id.map(|s| s.to_string());
            if let Some(obj) = metadata.as_object_mut() {
                obj.insert("agent".to_string(), Value::String(agent_s.clone()));
                obj.insert(
                    "discovery_type".to_string(),
                    Value::String("Symbol".to_string()),
                );
                obj.insert("target".to_string(), Value::String(target_s.clone()));
                obj.insert(
                    "timestamp".to_string(),
                    Value::String(Utc::now().to_rfc3339()),
                );
                obj.insert("sql_id".to_string(), Value::Number(sql_id.into()));
                if let Some(ref pid) = project_s {
                    obj.insert("project_id".to_string(), Value::String(pid.clone()));
                }
            }
            let metadata_str = super::json_to_string(&metadata)?;
            let metadata_for_entity = metadata.clone();
            self.with_raw_connection(|conn| {
                conn.execute(
                    "UPDATE discoveries SET metadata = ?1, project_id = ?2 WHERE id = ?3",
                    rusqlite::params![metadata_str, project_s, sql_id],
                )?;
                Ok(())
            })?;

            let entity_id: Option<i64> = self.with_raw_connection(|conn| {
                let row = conn
                    .query_row(
                        "SELECT id FROM graph_entities
                         WHERE kind = 'Discovery'
                           AND json_extract(data, '$.sql_id') = ?1
                         LIMIT 1",
                        rusqlite::params![sql_id],
                        |r| r.get::<_, i64>(0),
                    )
                    .ok();
                Ok(row)
            })?;

            if let Some(entity_id) = entity_id {
                self.update_entity_data(entity_id, &metadata_for_entity)?;
                Ok(entity_id)
            } else {
                let name = format!("{}: {}", agent_name, target);
                let entity = GraphEntity {
                    id: 0,
                    kind: EntityType::Discovery.as_str().to_string(),
                    name,
                    file_path: None,
                    data: metadata_for_entity,
                };
                self.inner.insert_entity(&entity).map_err(Into::into)
            }
        } else {
            self.store_discovery_in_project(agent_name, "Symbol", target, project_id, metadata)
        }
    }
}
