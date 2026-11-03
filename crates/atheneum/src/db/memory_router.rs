//! Migration v15: `memory-router-domain`.
//!
//! Adds additive schema for canonical memory routing, project states,
//! agent states, artifacts, verification receipts, sources, external
//! memory references, and router query audits.

use anyhow::Result;
use rusqlite::Transaction;

fn table_exists(tx: &Transaction<'_>, table: &str) -> Result<bool> {
    let exists: bool = tx
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);
    Ok(exists)
}

fn column_exists(tx: &Transaction<'_>, table: &str, column: &str) -> Result<bool> {
    let mut stmt = tx.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn add_column_if_missing(
    tx: &Transaction<'_>,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    if !column_exists(tx, table, column)? {
        tx.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition};"
        ))?;
    }
    Ok(())
}

/// Migration v15 — `memory-router-domain`.
pub fn migrate_v15_memory_router(tx: &Transaction<'_>) -> Result<()> {
    // 1. Extend memory_entries if the table exists
    if table_exists(tx, "memory_entries")? {
        add_column_if_missing(
            tx,
            "memory_entries",
            "kind",
            "TEXT NOT NULL DEFAULT 'memory'",
        )?;
        add_column_if_missing(tx, "memory_entries", "title", "TEXT")?;
        add_column_if_missing(tx, "memory_entries", "summary", "TEXT")?;
        add_column_if_missing(
            tx,
            "memory_entries",
            "status",
            "TEXT NOT NULL DEFAULT 'active'",
        )?;
        add_column_if_missing(tx, "memory_entries", "source_uri", "TEXT")?;
        add_column_if_missing(
            tx,
            "memory_entries",
            "evidence_json",
            "TEXT NOT NULL DEFAULT '[]'",
        )?;
        add_column_if_missing(tx, "memory_entries", "content_hash", "TEXT")?;
        add_column_if_missing(tx, "memory_entries", "token_estimate", "INTEGER")?;
        add_column_if_missing(
            tx,
            "memory_entries",
            "sensitivity",
            "TEXT NOT NULL DEFAULT 'normal'",
        )?;
        add_column_if_missing(tx, "memory_entries", "valid_from", "TEXT")?;
        add_column_if_missing(tx, "memory_entries", "valid_to", "TEXT")?;
        add_column_if_missing(tx, "memory_entries", "ttl_seconds", "INTEGER")?;
        add_column_if_missing(tx, "memory_entries", "graph_entity_id", "INTEGER")?;
        add_column_if_missing(
            tx,
            "memory_entries",
            "supersedes_graph_entity_id",
            "INTEGER",
        )?;

        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS memory_kind_idx ON memory_entries(kind);
             CREATE INDEX IF NOT EXISTS memory_status_idx ON memory_entries(status);
             CREATE INDEX IF NOT EXISTS memory_project_kind_idx ON memory_entries(project_id, kind);
             CREATE INDEX IF NOT EXISTS memory_content_hash_idx ON memory_entries(content_hash);
             CREATE INDEX IF NOT EXISTS memory_graph_entity_idx ON memory_entries(graph_entity_id);
             CREATE INDEX IF NOT EXISTS memory_supersedes_graph_idx ON memory_entries(supersedes_graph_entity_id);",
        )?;
    }

    // 2. Create new tables and indexes
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS project_states (
            id INTEGER PRIMARY KEY,
            graph_entity_id INTEGER NOT NULL UNIQUE,
            project_id TEXT NOT NULL,
            version INTEGER NOT NULL,
            is_head INTEGER NOT NULL DEFAULT 0,
            objective TEXT,
            state_json TEXT NOT NULL,
            branch TEXT,
            head_sha TEXT,
            constraints_json TEXT NOT NULL DEFAULT '[]',
            active_task_ids_json TEXT NOT NULL DEFAULT '[]',
            blocker_ids_json TEXT NOT NULL DEFAULT '[]',
            decision_ids_json TEXT NOT NULL DEFAULT '[]',
            receipt_ids_json TEXT NOT NULL DEFAULT '[]',
            changelog TEXT,
            evidence_json TEXT NOT NULL DEFAULT '[]',
            token_estimate INTEGER,
            sensitivity TEXT NOT NULL DEFAULT 'normal',
            created_at TEXT NOT NULL,
            UNIQUE(project_id, version)
        );
        CREATE INDEX IF NOT EXISTS project_states_project_head_idx ON project_states(project_id, is_head);
        CREATE INDEX IF NOT EXISTS project_states_project_created_idx ON project_states(project_id, created_at);

        CREATE TABLE IF NOT EXISTS agent_states (
            id INTEGER PRIMARY KEY,
            graph_entity_id INTEGER NOT NULL UNIQUE,
            agent_name TEXT NOT NULL,
            project_id TEXT,
            session_id TEXT,
            dispatch_id TEXT,
            task_id INTEGER REFERENCES tasks(id),
            version INTEGER NOT NULL,
            status TEXT NOT NULL,
            current_objective TEXT,
            state_json TEXT NOT NULL DEFAULT '{}',
            budget_json TEXT NOT NULL DEFAULT '{}',
            last_receipt_graph_entity_id INTEGER,
            sensitivity TEXT NOT NULL DEFAULT 'normal',
            created_at TEXT NOT NULL,
            UNIQUE(agent_name, project_id, dispatch_id, version)
        );
        CREATE INDEX IF NOT EXISTS agent_states_project_idx ON agent_states(project_id);
        CREATE INDEX IF NOT EXISTS agent_states_agent_idx ON agent_states(agent_name);
        CREATE INDEX IF NOT EXISTS agent_states_dispatch_idx ON agent_states(dispatch_id);

        CREATE TABLE IF NOT EXISTS artifacts (
            id INTEGER PRIMARY KEY,
            graph_entity_id INTEGER NOT NULL UNIQUE,
            project_id TEXT,
            task_id INTEGER REFERENCES tasks(id),
            artifact_kind TEXT NOT NULL,
            path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            bytes INTEGER,
            commit_sha TEXT,
            created_by TEXT,
            sensitivity TEXT NOT NULL DEFAULT 'normal',
            metadata_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            UNIQUE(project_id, path, sha256)
        );

        CREATE TABLE IF NOT EXISTS receipts (
            id INTEGER PRIMARY KEY,
            graph_entity_id INTEGER NOT NULL UNIQUE,
            project_id TEXT,
            task_id INTEGER REFERENCES tasks(id),
            artifact_graph_entity_id INTEGER,
            session_id TEXT REFERENCES sessions(session_id),
            verdict TEXT NOT NULL,
            verifier TEXT NOT NULL,
            command TEXT,
            exit_code INTEGER,
            duration_ms INTEGER,
            output_hash TEXT,
            output_sample_redacted TEXT,
            evidence_json TEXT NOT NULL DEFAULT '[]',
            sensitivity TEXT NOT NULL DEFAULT 'normal',
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS receipts_project_idx ON receipts(project_id);
        CREATE INDEX IF NOT EXISTS receipts_task_idx ON receipts(task_id);
        CREATE INDEX IF NOT EXISTS receipts_artifact_idx ON receipts(artifact_graph_entity_id);

        CREATE TABLE IF NOT EXISTS sources (
            id INTEGER PRIMARY KEY,
            graph_entity_id INTEGER NOT NULL UNIQUE,
            project_id TEXT,
            source_type TEXT NOT NULL,
            uri TEXT NOT NULL,
            content_hash TEXT,
            title TEXT,
            summary TEXT,
            author TEXT,
            published_at TEXT,
            retrieved_at TEXT,
            credibility REAL,
            sensitivity TEXT NOT NULL DEFAULT 'normal',
            metadata_json TEXT NOT NULL DEFAULT '{}',
            UNIQUE(project_id, uri)
        );

        CREATE TABLE IF NOT EXISTS external_memory_refs (
            id INTEGER PRIMARY KEY,
            graph_entity_id INTEGER NOT NULL UNIQUE,
            external_store TEXT NOT NULL,
            external_id TEXT NOT NULL,
            atheneum_graph_entity_id INTEGER NOT NULL,
            atheneum_kind TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            sync_state TEXT NOT NULL,
            last_synced_at TEXT NOT NULL,
            metadata_json TEXT NOT NULL DEFAULT '{}',
            UNIQUE(external_store, external_id)
        );

        CREATE TABLE IF NOT EXISTS router_queries (
            id INTEGER PRIMARY KEY,
            graph_entity_id INTEGER UNIQUE,
            project_id TEXT,
            agent_name TEXT,
            query_hash TEXT NOT NULL,
            kinds_json TEXT NOT NULL DEFAULT '[]',
            budget_tokens INTEGER NOT NULL,
            returned_ids_json TEXT NOT NULL DEFAULT '[]',
            omitted_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );",
    )?;

    // 3. Backfill memory_entries from graph_entities when matching data.sql_id exists
    backfill_memory_entries_from_graph(tx)?;

    Ok(())
}

fn backfill_memory_entries_from_graph(tx: &Transaction<'_>) -> Result<()> {
    if table_exists(tx, "memory_entries")? && table_exists(tx, "graph_entities")? {
        let mut stmt = tx.prepare(
            "SELECT id, json_extract(data, '$.sql_id'), json_extract(data, '$.content_hash')
             FROM graph_entities
             WHERE kind = 'Memory'
               AND json_extract(data, '$.sql_id') IS NOT NULL",
        )?;

        let rows = stmt.query_map([], |row| {
            let ge_id: i64 = row.get(0)?;
            let sql_id: i64 = row.get(1)?;
            let hash: Option<String> = row.get(2)?;
            Ok((ge_id, sql_id, hash))
        })?;

        let pairs: Vec<(i64, i64, Option<String>)> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        let mut update_stmt = tx.prepare(
            "UPDATE memory_entries
             SET graph_entity_id = ?1,
                 content_hash = COALESCE(content_hash, ?2)
             WHERE id = ?3 AND graph_entity_id IS NULL",
        )?;

        for (ge_id, sql_id, hash) in pairs {
            update_stmt.execute(rusqlite::params![ge_id, hash, sql_id])?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::MIGRATIONS;
    use crate::graph::types::EntityType;
    use crate::graph::AtheneumGraph;
    use rusqlite::params;

    #[test]
    fn test_v15_is_registered_after_v14() {
        let v14_pos = MIGRATIONS
            .iter()
            .position(|(v, name, _)| *v == 14 && *name == "grounded-claims");
        let v15_pos = MIGRATIONS
            .iter()
            .position(|(v, name, _)| *v == 15 && *name == "memory-router-domain");
        assert!(v14_pos.is_some(), "v14 must be registered in MIGRATIONS");
        assert!(v15_pos.is_some(), "v15 must be registered in MIGRATIONS");
        assert_eq!(
            v15_pos.unwrap(),
            v14_pos.unwrap() + 1,
            "v15 must be registered immediately after v14"
        );
    }

    #[test]
    fn test_all_new_columns_tables_indexes_exist() {
        let graph = AtheneumGraph::open_in_memory().expect("open graph");
        graph
            .with_raw_connection(|conn| {
                // Verify new tables exist
                let required_tables = [
                    "project_states",
                    "agent_states",
                    "artifacts",
                    "receipts",
                    "sources",
                    "external_memory_refs",
                    "router_queries",
                ];
                for table in required_tables {
                    let exists: bool = conn
                        .query_row(
                            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                            params![table],
                            |r| r.get::<_, i64>(0),
                        )
                        .map(|c| c > 0)
                        .unwrap_or(false);
                    assert!(exists, "Table '{table}' must exist after migration v15");
                }

                // Verify new columns on memory_entries
                let required_columns = [
                    "kind",
                    "title",
                    "summary",
                    "status",
                    "source_uri",
                    "evidence_json",
                    "content_hash",
                    "token_estimate",
                    "sensitivity",
                    "valid_from",
                    "valid_to",
                    "ttl_seconds",
                    "graph_entity_id",
                    "supersedes_graph_entity_id",
                ];
                let mut stmt = conn.prepare("PRAGMA table_info(memory_entries)")?;
                let existing_cols: Vec<String> = stmt
                    .query_map([], |r| r.get::<_, String>(1))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for col in required_columns {
                    assert!(
                        existing_cols.contains(&col.to_string()),
                        "Column '{col}' must exist on memory_entries"
                    );
                }

                // Verify new indexes exist
                let required_indexes = [
                    "memory_kind_idx",
                    "memory_status_idx",
                    "memory_project_kind_idx",
                    "memory_content_hash_idx",
                    "memory_graph_entity_idx",
                    "memory_supersedes_graph_idx",
                    "project_states_project_head_idx",
                    "project_states_project_created_idx",
                    "agent_states_project_idx",
                    "agent_states_agent_idx",
                    "agent_states_dispatch_idx",
                    "receipts_project_idx",
                    "receipts_task_idx",
                    "receipts_artifact_idx",
                ];
                for idx in required_indexes {
                    let exists: bool = conn
                        .query_row(
                            "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
                            params![idx],
                            |r| r.get::<_, i64>(0),
                        )
                        .map(|c| c > 0)
                        .unwrap_or(false);
                    assert!(exists, "Index '{idx}' must exist after migration v15");
                }

                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn test_migration_can_run_twice_safely() {
        let graph = AtheneumGraph::open_in_memory().expect("open graph");
        graph
            .with_raw_connection(|conn| {
                // Running v15 explicitly twice must succeed idempotently without error
                let tx1 = conn.unchecked_transaction()?;
                migrate_v15_memory_router(&tx1)?;
                tx1.commit()?;

                let tx2 = conn.unchecked_transaction()?;
                migrate_v15_memory_router(&tx2)?;
                tx2.commit()?;
                Ok(())
            })
            .expect("two runs of v15 must succeed idempotently");
    }

    struct PreservedMemoryRow {
        key: String,
        scope: String,
        content: String,
        confidence: f64,
        project_id: Option<String>,
        created_at: String,
        updated_at: Option<String>,
        kind: String,
        status: String,
        evidence_json: String,
        sensitivity: String,
        summary: Option<String>,
    }

    #[test]
    fn test_existing_memory_entries_preserve_original_content() {
        let graph = AtheneumGraph::open_in_memory().expect("open graph");
        graph
            .with_raw_connection(|conn| {
                // Insert legacy memory row prior to v15
                conn.execute(
                    "INSERT INTO memory_entries (key, scope, content, confidence, project_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        "pref.editor",
                        "global",
                        "always use neovim with line numbers",
                        0.95,
                        "core-project",
                        "2026-01-15T10:00:00Z",
                        "2026-01-16T12:00:00Z"
                    ],
                )?;
                let row_id = conn.last_insert_rowid();

                // Apply v15
                let tx = conn.unchecked_transaction()?;
                migrate_v15_memory_router(&tx)?;
                tx.commit()?;

                // Verify all original columns are preserved and new columns have defaults
                let row: PreservedMemoryRow = conn.query_row(
                    "SELECT key, scope, content, confidence, project_id, created_at, updated_at, kind, status, evidence_json, sensitivity, summary
                     FROM memory_entries WHERE id = ?1",
                    params![row_id],
                    |r| Ok(PreservedMemoryRow {
                        key: r.get(0)?,
                        scope: r.get(1)?,
                        content: r.get(2)?,
                        confidence: r.get(3)?,
                        project_id: r.get(4)?,
                        created_at: r.get(5)?,
                        updated_at: r.get(6)?,
                        kind: r.get(7)?,
                        status: r.get(8)?,
                        evidence_json: r.get(9)?,
                        sensitivity: r.get(10)?,
                        summary: r.get(11)?,
                    })
                )?;

                assert_eq!(row.key, "pref.editor");
                assert_eq!(row.scope, "global");
                assert_eq!(row.content, "always use neovim with line numbers");
                assert!((row.confidence - 0.95).abs() < 1e-6);
                assert_eq!(row.project_id.as_deref(), Some("core-project"));
                assert_eq!(row.created_at, "2026-01-15T10:00:00Z");
                assert_eq!(row.updated_at.as_deref(), Some("2026-01-16T12:00:00Z"));
                assert_eq!(row.kind, "memory");
                assert_eq!(row.status, "active");
                assert_eq!(row.evidence_json, "[]");
                assert_eq!(row.sensitivity, "normal");
                assert!(row.summary.is_none(), "summary must not be populated by silent truncation");

                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn test_existing_memory_graph_ids_backfilled() {
        let graph = AtheneumGraph::open_in_memory().expect("open graph");
        graph
            .with_raw_connection(|conn| {
                // Insert legacy SQL row
                conn.execute(
                    "INSERT INTO memory_entries (id, key, scope, content, confidence, project_id, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        55,
                        "architecture-decision",
                        "project",
                        "Use SQLite graph hybrid store",
                        1.0,
                        "atheneum",
                        "2026-02-01T00:00:00Z"
                    ],
                )?;

                // Insert corresponding Memory graph entity with data.sql_id = 55 and content_hash
                conn.execute(
                    "INSERT INTO graph_entities (id, kind, name, data)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        101,
                        "Memory",
                        "architecture-decision",
                        r#"{"sql_id": 55, "content_hash": "abc123hash", "key": "architecture-decision"}"#
                    ],
                )?;

                // Apply v15
                let tx = conn.unchecked_transaction()?;
                migrate_v15_memory_router(&tx)?;
                tx.commit()?;

                // Verify backfill on row 55
                let (graph_entity_id, content_hash): (Option<i64>, Option<String>) = conn
                    .query_row(
                        "SELECT graph_entity_id, content_hash FROM memory_entries WHERE id = 55",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?;

                assert_eq!(graph_entity_id, Some(101));
                assert_eq!(content_hash.as_deref(), Some("abc123hash"));

                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn test_new_user_visible_entity_type_variants_exist() {
        let variants = [
            (EntityType::ProjectState, "ProjectState", "projectstate"),
            (EntityType::AgentState, "AgentState", "agentstate"),
            (EntityType::Artifact, "Artifact", "artifact"),
            (EntityType::Receipt, "Receipt", "receipt"),
            (EntityType::Source, "Source", "source"),
            (
                EntityType::ExternalMemoryRef,
                "ExternalMemoryRef",
                "externalmemoryref",
            ),
            (EntityType::RouterQuery, "RouterQuery", "routerquery"),
        ];

        for (variant, expected_name, label) in variants {
            assert_eq!(variant.as_str(), expected_name);
            let parsed = EntityType::from_query_label(label);
            assert_eq!(parsed, Some(variant));

            // Verify serialization and deserialization
            let json = serde_json::to_string(&variant).expect("serialize entity type");
            let deserialized: EntityType =
                serde_json::from_str(&json).expect("deserialize entity type");
            assert_eq!(deserialized, variant);

            // Verify query_labels includes expected_name
            assert!(
                EntityType::query_labels().contains(&expected_name),
                "query_labels must contain {expected_name}"
            );
        }
    }
}
