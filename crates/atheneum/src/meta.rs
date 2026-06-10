//! Cross-project meta-database routing layer.
//!
//! `MetaRouter` maintains a separate SQLite database (`meta.db`) that tracks
//! all registered projects, their magellan/atheneum database paths, languages,
//! and indexing status. This is the "front door" for cross-project queries.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Information about a registered project.
#[derive(Debug, Clone)]
pub struct ProjectInfo {
    pub name: String,
    pub root_path: String,
    pub magellan_db: String,
    pub atheneum_db: Option<String>,
    pub language: Option<String>,
    pub enabled: bool,
    pub last_indexed: Option<String>,
    pub file_count: i64,
    pub symbol_count: i64,
    pub created_at: String,
}

/// Daemon-level meta-index of all registered projects.
pub struct MetaRouter {
    conn: Connection,
    path: PathBuf,
}

impl MetaRouter {
    /// Open the default meta.db at `~/.local/share/atheneum/meta.db`
    /// (or `$XDG_DATA_HOME/atheneum/meta.db` if set).
    pub fn open() -> Result<Self> {
        let dir = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|_| PathBuf::from(".atheneum"))
            .join("atheneum");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create meta.db dir {}", dir.display()))?;
        Self::open_at(dir.join("meta.db"))
    }

    /// Open a meta.db at an explicit path.
    pub fn open_at<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create parent dir for {}", path.display()))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("Failed to open meta.db at {}", path.display()))?;
        let mut router = Self { conn, path };
        router.ensure_schema()?;
        Ok(router)
    }

    fn ensure_schema(&mut self) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS project_registry (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL UNIQUE,
                root_path   TEXT NOT NULL,
                magellan_db TEXT NOT NULL,
                atheneum_db TEXT,
                language    TEXT,
                enabled     INTEGER NOT NULL DEFAULT 1,
                last_indexed TIMESTAMP,
                file_count   INTEGER DEFAULT 0,
                symbol_count INTEGER DEFAULT 0,
                created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_project_registry_enabled
             ON project_registry (enabled)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_project_registry_lang
             ON project_registry (language) WHERE enabled = 1",
            [],
        )?;
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS symbol_index (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                project     TEXT NOT NULL REFERENCES project_registry(name),
                symbol_name TEXT NOT NULL,
                kind        TEXT,
                file_path   TEXT,
                line        INTEGER,
                UNIQUE(project, symbol_name, file_path)
            )",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_symbol_name ON symbol_index(symbol_name)",
            [],
        )?;
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS symbol_analogies (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                from_project    TEXT NOT NULL REFERENCES project_registry(name),
                from_symbol     TEXT NOT NULL,
                to_project      TEXT NOT NULL REFERENCES project_registry(name),
                to_symbol       TEXT NOT NULL,
                similarity_score REAL NOT NULL,
                created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_analogies_from
             ON symbol_analogies (from_project, from_symbol)",
            [],
        )?;
        Ok(())
    }

    /// Register (or update) a project in the meta.db.
    pub fn register_project(
        &mut self,
        name: &str,
        root_path: &str,
        magellan_db: &str,
        atheneum_db: Option<&str>,
        language: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO project_registry
             (name, root_path, magellan_db, atheneum_db, language, enabled, last_indexed)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, CURRENT_TIMESTAMP)
             ON CONFLICT(name) DO UPDATE SET
                 root_path = excluded.root_path,
                 magellan_db = excluded.magellan_db,
                 atheneum_db = excluded.atheneum_db,
                 language = excluded.language,
                 enabled = 1,
                 last_indexed = CURRENT_TIMESTAMP",
            rusqlite::params![name, root_path, magellan_db, atheneum_db, language],
        )?;
        Ok(())
    }

    /// List all enabled projects.
    pub fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, root_path, magellan_db, atheneum_db, language,
                    enabled, last_indexed, file_count, symbol_count, created_at
             FROM project_registry
             WHERE enabled = 1
             ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ProjectInfo {
                name: row.get(0)?,
                root_path: row.get(1)?,
                magellan_db: row.get(2)?,
                atheneum_db: row.get(3)?,
                language: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                last_indexed: row.get(6)?,
                file_count: row.get(7)?,
                symbol_count: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// List projects filtered by language.
    pub fn list_projects_by_language(&self, language: &str) -> Result<Vec<ProjectInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, root_path, magellan_db, atheneum_db, language,
                    enabled, last_indexed, file_count, symbol_count, created_at
             FROM project_registry
             WHERE enabled = 1 AND language = ?1
             ORDER BY name",
        )?;
        let rows = stmt.query_map([language], |row| {
            Ok(ProjectInfo {
                name: row.get(0)?,
                root_path: row.get(1)?,
                magellan_db: row.get(2)?,
                atheneum_db: row.get(3)?,
                language: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                last_indexed: row.get(6)?,
                file_count: row.get(7)?,
                symbol_count: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Get a single project by name.
    pub fn get_project(&self, name: &str) -> Result<Option<ProjectInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, root_path, magellan_db, atheneum_db, language,
                    enabled, last_indexed, file_count, symbol_count, created_at
             FROM project_registry
             WHERE name = ?1",
        )?;
        let row = stmt.query_row([name], |row| {
            Ok(ProjectInfo {
                name: row.get(0)?,
                root_path: row.get(1)?,
                magellan_db: row.get(2)?,
                atheneum_db: row.get(3)?,
                language: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                last_indexed: row.get(6)?,
                file_count: row.get(7)?,
                symbol_count: row.get(8)?,
                created_at: row.get(9)?,
            })
        });
        match row {
            Ok(p) => Ok(Some(p)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Disable a project (soft delete).
    pub fn disable_project(&mut self, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE project_registry SET enabled = 0 WHERE name = ?1",
            [name],
        )?;
        Ok(())
    }

    /// Path to the meta.db file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_router_schema_and_roundtrip() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut router = MetaRouter::open_at(tmp.path()).unwrap();

        router
            .register_project(
                "envoy",
                "/home/feanor/Projects/envoy",
                "/home/feanor/Projects/envoy/.magellan/magellan.db",
                Some("/home/feanor/Projects/envoy/atheneum.db"),
                Some("rust"),
            )
            .unwrap();
        router
            .register_project(
                "magellan",
                "/home/feanor/Projects/magellan",
                "/home/feanor/Projects/magellan/.magellan/magellan.db",
                None,
                Some("rust"),
            )
            .unwrap();

        let projects = router.list_projects().unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].name, "envoy");
        assert_eq!(projects[0].language.as_deref(), Some("rust"));
        assert!(projects[0].enabled);

        let rust_projects = router.list_projects_by_language("rust").unwrap();
        assert_eq!(rust_projects.len(), 2);

        let envoy = router.get_project("envoy").unwrap();
        assert!(envoy.is_some());
        assert_eq!(envoy.unwrap().root_path, "/home/feanor/Projects/envoy");

        router.disable_project("envoy").unwrap();
        let after = router.list_projects().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name, "magellan");
    }

    #[test]
    fn test_meta_router_upsert() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut router = MetaRouter::open_at(tmp.path()).unwrap();

        router
            .register_project("test", "/old", "/old/magellan.db", None, Some("python"))
            .unwrap();
        router
            .register_project(
                "test",
                "/new",
                "/new/magellan.db",
                Some("/new/atheneum.db"),
                Some("rust"),
            )
            .unwrap();

        let p = router.get_project("test").unwrap().unwrap();
        assert_eq!(p.root_path, "/new");
        assert_eq!(p.language.as_deref(), Some("rust"));
        assert_eq!(p.atheneum_db.as_deref(), Some("/new/atheneum.db"));
    }
}
