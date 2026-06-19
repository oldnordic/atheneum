//! Cross-project query router with lazy SQLite ATTACH.
//!
//! Wraps `MetaRouter` with an LRU cache of attached magellan databases.
//! Read-only attach keeps magellan data immutable.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::meta::{MetaRouter, ProjectInfo};

const DEFAULT_MAX_ATTACHED: usize = 8; // SQLite default limit is 10; stay safe

/// Symbol hit from a cross-project search.
#[derive(Debug, Clone)]
pub struct CrossSearchResult {
    pub project: String,
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub file_path: Option<String>,
    pub data: Value,
}

/// Edge from a cross-project subgraph walk.
#[derive(Debug, Clone)]
pub struct CrossEdge {
    pub id: i64,
    pub kind: String,
    pub from_id: i64,
    pub to_id: i64,
    pub data: Value,
}

/// Subgraph view returned by cross-project navigate.
#[derive(Debug, Clone)]
pub struct CrossSubgraph {
    pub project: String,
    pub entry_id: i64,
    pub entities: Vec<CrossSearchResult>,
    pub edges: Vec<CrossEdge>,
}

/// Cross-project query router.
pub struct CrossRouter {
    meta: MetaRouter,
    /// project name -> schema name
    attached: HashMap<String, String>,
    /// LRU order: front = most recently used
    lru: VecDeque<String>,
    max_attached: usize,
    schema_counter: usize,
}

impl CrossRouter {
    /// Open the default cross router using the default meta.db.
    pub fn open() -> Result<Self> {
        Self::with_capacity(DEFAULT_MAX_ATTACHED)
    }

    /// Open with a custom attached-database cache size.
    pub fn with_capacity(max_attached: usize) -> Result<Self> {
        Ok(Self {
            meta: MetaRouter::open()?,
            attached: HashMap::new(),
            lru: VecDeque::new(),
            max_attached: max_attached.min(125),
            schema_counter: 0,
        })
    }

    /// Wrap an existing `MetaRouter`.
    pub fn from_meta(meta: MetaRouter, max_attached: usize) -> Self {
        Self {
            meta,
            attached: HashMap::new(),
            lru: VecDeque::new(),
            max_attached: max_attached.min(125),
            schema_counter: 0,
        }
    }

    /// Immutable borrow of the underlying meta router.
    pub fn meta(&self) -> &MetaRouter {
        &self.meta
    }

    /// Mutable borrow of the underlying meta router.
    pub fn meta_mut(&mut self) -> &mut MetaRouter {
        &mut self.meta
    }

    fn ensure_attached(&mut self, project: &ProjectInfo) -> Result<String> {
        if let Some(schema) = self.attached.get(&project.name).cloned() {
            self.touch(&project.name);
            return Ok(schema);
        }

        let schema = format!("cross_{}_{}", self.schema_counter, sanitize(&project.name));
        self.schema_counter += 1;

        while self.attached.len() >= self.max_attached {
            if let Some(old_project) = self.lru.pop_back() {
                if let Some(old_schema) = self.attached.remove(&old_project) {
                    let _ = self.detach_schema(&old_schema);
                }
            }
        }

        let db_path = std::path::Path::new(&project.magellan_db);
        if !db_path.exists() {
            anyhow::bail!(
                "Magellan database for project '{}' not found at {}",
                project.name,
                db_path.display()
            );
        }

        self.meta
            .conn()
            .execute(
                "ATTACH DATABASE ?1 AS ?2",
                rusqlite::params![project.magellan_db, &schema],
            )
            .with_context(|| format!("Failed to attach {} as {}", project.magellan_db, schema))?;

        self.attached.insert(project.name.clone(), schema.clone());
        self.lru.push_front(project.name.clone());
        Ok(schema)
    }

    fn touch(&mut self, project_name: &str) {
        self.lru.retain(|n| n != project_name);
        self.lru.push_front(project_name.to_string());
    }

    fn detach_schema(&mut self, schema: &str) -> Result<()> {
        self.meta
            .conn()
            .execute(&format!("DETACH DATABASE \"{}\"", schema), [])
            .with_context(|| format!("Failed to detach database {}", schema))?;
        Ok(())
    }

    /// Search for symbols across all enabled projects.
    ///
    /// Returns up to `k` results ranked with exact name matches first.
    pub fn cross_search(
        &mut self,
        query: &str,
        language: Option<&str>,
        k: usize,
    ) -> Result<Vec<CrossSearchResult>> {
        let projects = if let Some(lang) = language {
            self.meta.list_projects_by_language(lang)?
        } else {
            self.meta.list_projects()?
        };

        let pattern = format!("%{}%", query);
        let mut results = Vec::new();

        for project in projects {
            let schema = match self.ensure_attached(&project) {
                Ok(s) => s.to_string(),
                Err(e) => {
                    tracing::warn!("Skipping project {}: {}", project.name, e);
                    continue;
                }
            };

            // Query this project's graph. Skip projects whose DB schema is
            // incompatible (not fully indexed, different table layout) rather
            // than failing the entire cross-search.
            let sql = format!(
                "SELECT id, kind, name, file_path, data FROM \"{}\".graph_entities
                 WHERE name LIKE ?1
                 LIMIT ?2",
                schema
            );
            let mut stmt = match self.meta.conn().prepare(&sql) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        "Skipping project {} (schema incompatible): {}",
                        project.name,
                        e
                    );
                    continue;
                }
            };
            let rows = match stmt.query_map(rusqlite::params![&pattern, k as i64], |row| {
                Ok(CrossSearchResult {
                    project: project.name.clone(),
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    name: row.get(2)?,
                    file_path: row.get(3)?,
                    data: parse_json_column(row.get(4)?),
                })
            }) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Skipping project {} (query failed): {}", project.name, e);
                    continue;
                }
            };
            for row in rows {
                match row {
                    Ok(r) => results.push(r),
                    Err(e) => tracing::warn!("Malformed row in {}: {}", project.name, e),
                }
            }
        }

        results.sort_by(|a, b| {
            let a_exact = a.name.eq_ignore_ascii_case(query);
            let b_exact = b.name.eq_ignore_ascii_case(query);
            b_exact
                .cmp(&a_exact)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        results.truncate(k);
        Ok(results)
    }

    /// Navigate from search hits into per-project subgraphs.
    pub fn cross_navigate(
        &mut self,
        query: &str,
        language: Option<&str>,
        k: usize,
        depth: u32,
    ) -> Result<Vec<CrossSubgraph>> {
        let entries = self.cross_search(query, language, k)?;
        let mut views = Vec::new();

        for entry in entries {
            let schema = match self.ensure_attached_for_name(&entry.project) {
                Ok(s) => s.to_string(),
                Err(e) => {
                    tracing::warn!("Skipping navigate for {}: {}", entry.project, e);
                    continue;
                }
            };

            let (mut entities, edges) = self.bfs(&schema, entry.id, depth)?;
            for e in &mut entities {
                e.project.clone_from(&entry.project);
            }
            views.push(CrossSubgraph {
                project: entry.project,
                entry_id: entry.id,
                entities,
                edges,
            });
        }

        Ok(views)
    }

    fn ensure_attached_for_name(&mut self, project_name: &str) -> Result<String> {
        let project = self
            .meta
            .get_project(project_name)?
            .ok_or_else(|| anyhow::anyhow!("Project {} not found in meta.db", project_name))?;
        self.ensure_attached(&project)
    }

    fn bfs(
        &self,
        schema: &str,
        entry_id: i64,
        depth: u32,
    ) -> Result<(Vec<CrossSearchResult>, Vec<CrossEdge>)> {
        let mut visited_entities = HashSet::new();
        let mut visited_edges = HashSet::new();
        let mut frontier = vec![entry_id];
        let mut entities = Vec::new();
        let mut edges = Vec::new();

        for _ in 0..=depth {
            let current = frontier.clone();
            frontier.clear();
            for id in current {
                if !visited_entities.insert(id) {
                    continue;
                }
                let sql = format!(
                    "SELECT id, kind, name, file_path, data FROM \"{}\".graph_entities WHERE id = ?1",
                    schema
                );
                if let Ok(row) = self.meta.conn().query_row(&sql, [id], |row| {
                    Ok(CrossSearchResult {
                        project: String::new(),
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        name: row.get(2)?,
                        file_path: row.get(3)?,
                        data: parse_json_column(row.get(4)?),
                    })
                }) {
                    entities.push(row);
                }

                // Production magellan DBs use `edge_type` for the column name,
                // while the test fixture uses `kind`.  Alias to `kind` so the
                // row-mapping below works regardless of which schema we hit.
                let edge_sql = format!(
                    "SELECT id, edge_type AS kind, from_id, to_id, data FROM \"{}\".graph_edges\n\
                     WHERE from_id = ?1 OR to_id = ?1",
                    schema
                );
                let mut stmt = self
                    .meta
                    .conn()
                    .prepare(&edge_sql)
                    .with_context(|| format!("prepare edges for schema {}", schema))?;
                let rows = stmt.query_map([id], |row| {
                    Ok(CrossEdge {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        from_id: row.get(2)?,
                        to_id: row.get(3)?,
                        data: parse_json_column(row.get(4)?),
                    })
                })?;
                for row in rows {
                    let edge = row?;
                    if visited_edges.insert(edge.id) {
                        frontier.push(edge.from_id);
                        frontier.push(edge.to_id);
                        edges.push(edge);
                    }
                }
            }
        }

        Ok((entities, edges))
    }
}

fn parse_json_column(raw: String) -> Value {
    serde_json::from_str(&raw).unwrap_or(Value::Null)
}

fn sanitize(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    // SQLite schema names must not start with a digit.
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out = format!("p{}", out);
    }
    if out.is_empty() {
        out = "project".to_string();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_magellan_like_db(path: &std::path::Path) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute(
            "CREATE TABLE graph_entities (
                id INTEGER PRIMARY KEY,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                file_path TEXT,
                data TEXT NOT NULL DEFAULT '{}'
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE graph_edges (
                id INTEGER PRIMARY KEY,
                edge_type TEXT NOT NULL,
                from_id INTEGER NOT NULL,
                to_id INTEGER NOT NULL,
                data TEXT NOT NULL DEFAULT '{}'
            )",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_sanitize_leading_digit() {
        assert_eq!(sanitize("123abc"), "p123abc");
    }

    #[test]
    fn test_sanitize_special_chars() {
        assert_eq!(sanitize("foo-bar"), "foo_bar");
        assert_eq!(sanitize("a/b"), "a_b");
    }

    #[test]
    fn test_sanitize_empty() {
        assert_eq!(sanitize(""), "project");
    }

    #[test]
    fn test_cross_search_attaches_and_finds_symbols() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let meta_path = tmp_dir.path().join("meta.db");
        let magellan_a = tmp_dir.path().join("a.db");
        let magellan_b = tmp_dir.path().join("b.db");

        // Seed magellan-like dbs
        {
            let ca = make_magellan_like_db(&magellan_a);
            ca.execute(
                "INSERT INTO graph_entities (id, kind, name, file_path, data) VALUES
                 (1, 'Symbol', 'build_router', 'src/lib.rs', '{\"lang\":\"rust\"}')",
                [],
            )
            .unwrap();
        }
        {
            let cb = make_magellan_like_db(&magellan_b);
            cb.execute(
                "INSERT INTO graph_entities (id, kind, name, file_path, data) VALUES
                 (1, 'Symbol', 'build_router', 'handler.go', '{\"lang\":\"go\"}')",
                [],
            )
            .unwrap();
        }

        // Seed meta.db
        let mut meta = MetaRouter::open_at(&meta_path).unwrap();
        meta.register_project(
            "alpha",
            "/alpha",
            magellan_a.to_str().unwrap(),
            None,
            Some("rust"),
        )
        .unwrap();
        meta.register_project(
            "beta",
            "/beta",
            magellan_b.to_str().unwrap(),
            None,
            Some("go"),
        )
        .unwrap();

        let mut cross = CrossRouter::from_meta(meta, 4);
        let hits = cross
            .cross_search("build_router", Some("rust"), 10)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].project, "alpha");
        assert_eq!(hits[0].name, "build_router");

        // No language filter returns both
        let all = cross.cross_search("build_router", None, 10).unwrap();
        assert_eq!(all.len(), 2);

        // LRU should remember alpha is attached; re-querying should still work
        let hits2 = cross
            .cross_search("build_router", Some("rust"), 10)
            .unwrap();
        assert_eq!(hits2.len(), 1);
    }

    #[test]
    fn test_cross_navigate_walks_subgraph() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let meta_path = tmp_dir.path().join("meta.db");
        let magellan_a = tmp_dir.path().join("a.db");

        {
            let ca = make_magellan_like_db(&magellan_a);
            ca.execute(
                "INSERT INTO graph_entities (id, kind, name, file_path, data) VALUES
                 (1, 'Symbol', 'build_router', 'src/lib.rs', '{}'),
                 (2, 'Symbol', 'handler', 'src/lib.rs', '{}')",
                [],
            )
            .unwrap();
            ca.execute(
                "INSERT INTO graph_edges (id, edge_type, from_id, to_id, data) VALUES
                 (1, 'Calls', 1, 2, '{}')",
                [],
            )
            .unwrap();
        }

        let mut meta = MetaRouter::open_at(&meta_path).unwrap();
        meta.register_project(
            "alpha",
            "/alpha",
            magellan_a.to_str().unwrap(),
            None,
            Some("rust"),
        )
        .unwrap();

        let mut cross = CrossRouter::from_meta(meta, 4);
        let views = cross
            .cross_navigate("build_router", Some("rust"), 5, 1)
            .unwrap();
        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert_eq!(view.project, "alpha");
        assert_eq!(view.entry_id, 1);
        assert_eq!(view.entities.len(), 2);
        assert_eq!(view.edges.len(), 1);
        assert_eq!(view.edges[0].kind, "Calls");
    }
}
