//! Cross-project meta-database routing layer.
//!
//! `MetaRouter` is the "front door" for cross-project queries. It maintains a
//! small SQLite database (`meta.db`) of atheneum-owned enrichment data
//! (language, atheneum-db path, symbol index, symbol analogies) and — when the
//! magellan indexer is installed — reads the *canonical* project registry from
//! magellan's `~/.magellan/meta.db`, overlaying the enrichment on top.
//!
//! ## One source of truth, zero coupling
//!
//! Magellan owns the project registry. Atheneum never maintains a competing
//! copy. When magellan is present (`MetaRouter::open`), project existence and
//! root/db paths come from `mg.project_registry`; atheneum's `project_overlay`
//! contributes only the fields magellan does not track (`language`,
//! `atheneum_db`) and a disable veto. When magellan is absent
//! (`MetaRouter::open_at`, or `open` with no magellan DB), the overlay table
//! alone acts as the registry — so atheneum works standalone.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::config;

fn default_meta_db_path() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|_| PathBuf::from(".atheneum"))
        .join("atheneum")
        .join("meta.db")
}

/// Path to magellan's canonical project registry.
///
/// Resolved as: `$MAGELLAN_META_DB` > `$HOME/.magellan/meta.db`.
fn magellan_meta_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("MAGELLAN_META_DB") {
        return PathBuf::from(p);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".magellan").join("meta.db");
    }
    PathBuf::from(".magellan/meta.db")
}

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
    /// `true` when magellan's canonical `meta.db` was attached as schema `mg`.
    magellan_attached: bool,
}

impl MetaRouter {
    /// Open the default meta.db and (if present) attach magellan's canonical
    /// project registry.
    ///
    /// Uses the path from `~/.config/atheneum/config.toml` when present and
    /// valid; otherwise falls back to `~/.local/share/atheneum/meta.db`
    /// (or `$XDG_DATA_HOME/atheneum/meta.db` if set).
    pub fn open() -> Result<Self> {
        let path = config::load()
            .map(|cfg| cfg.meta_db_path())
            .unwrap_or_else(|_| default_meta_db_path());
        Self::open_with_magellan(path)
    }

    /// Open a meta.db at an explicit path and attach magellan's canonical
    /// registry when it exists on disk.
    pub fn open_with_magellan<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut router = Self::open_at(path)?;
        let mg_path = magellan_meta_db_path();
        if mg_path.exists() {
            let res = router.conn.execute(
                "ATTACH DATABASE ?1 AS mg",
                rusqlite::params![mg_path.to_string_lossy()],
            );
            match res {
                Ok(_) => router.magellan_attached = true,
                Err(e) => {
                    // Non-fatal: degrade to overlay-only. Magellan stays the
                    // sole owner of its registry; atheneum simply can't see it.
                    eprintln!(
                        "atheneum: could not attach magellan registry at {}: {e} \
                         (continuing with local overlay only)",
                        mg_path.display()
                    );
                    router.magellan_attached = false;
                }
            }
        }
        Ok(router)
    }

    /// Open a meta.db at an explicit path WITHOUT attaching magellan.
    ///
    /// Intended for tests and explicit-path tooling where the overlay table
    /// alone is the registry.
    pub fn open_at<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create parent dir for {}", path.display()))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("Failed to open meta.db at {}", path.display()))?;
        let mut router = Self {
            conn,
            path,
            magellan_attached: false,
        };
        router.ensure_schema()?;
        Ok(router)
    }

    fn ensure_schema(&mut self) -> Result<()> {
        // Atheneum-owned enrichment + standalone registry overlay. Magellan
        // supplies canonical root/db_path in production; the overlay carries
        // those fields too so it can stand alone (tests, no magellan).
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS project_overlay (
                name         TEXT PRIMARY KEY,
                root_path    TEXT,
                magellan_db  TEXT,
                atheneum_db  TEXT,
                language     TEXT,
                enabled      INTEGER NOT NULL DEFAULT 1,
                last_indexed TIMESTAMP,
                file_count   INTEGER DEFAULT 0,
                symbol_count INTEGER DEFAULT 0,
                created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_project_overlay_enabled
             ON project_overlay (enabled)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_project_overlay_lang
             ON project_overlay (language) WHERE enabled = 1",
            [],
        )?;
        // No FK to the registry: project names can originate from magellan's
        // canonical table, which is not always attached.
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS symbol_index (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                project     TEXT NOT NULL,
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
                from_project    TEXT NOT NULL,
                from_symbol     TEXT NOT NULL,
                to_project      TEXT NOT NULL,
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
        // One-time migration from the legacy `project_registry` table
        // (pre-bridge duplicate of magellan's registry) into `project_overlay`.
        // Copies only rows whose enrichment isn't already represented in the
        // overlay, then drops the legacy table so it can never rot again.
        self.migrate_legacy_registry()?;
        Ok(())
    }

    fn migrate_legacy_registry(&self) -> Result<()> {
        // Only act if the legacy table still exists.
        let has_legacy: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='project_registry'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if !has_legacy {
            return Ok(());
        }
        // Port enrichment (language, atheneum_db, root, magellan_db) for any
        // project not yet in the overlay. Existing overlay rows win.
        self.conn.execute(
            "INSERT OR IGNORE INTO project_overlay
                (name, root_path, magellan_db, atheneum_db, language, enabled, last_indexed)
             SELECT name, root_path, magellan_db, atheneum_db, language, enabled, last_indexed
             FROM project_registry",
            [],
        )?;
        // Drop the legacy table — the bridge reads magellan's canonical
        // registry, and the overlay is now the sole atheneum-owned copy.
        let _ = self.conn.execute("DROP TABLE project_registry", []);
        tracing::info!("migrated legacy project_registry into project_overlay");
        Ok(())
    }

    /// Build the project SELECT (column projection + FROM clause) for the
    /// active topology: magellan-canonical UNION overlay, or overlay-only.
    ///
    /// Columns are ordered to match [`ProjectInfo`] field order:
    /// `name, root_path, magellan_db, atheneum_db, language, enabled,
    ///  last_indexed, file_count, symbol_count, created_at`.
    fn projects_source(&self) -> String {
        if self.magellan_attached {
            // Magellan canonical (root, db_path, counts) enriched with the
            // overlay's language/atheneum_db and honouring an overlay disable
            // veto (`COALESCE(ov.enabled,1)=1`); plus overlay-only projects.
            "SELECT name, root_path, magellan_db, atheneum_db, language, enabled,
                    last_indexed, file_count, symbol_count, created_at FROM (
                SELECT mg.name AS name,
                       mg.root AS root_path,
                       mg.db_path AS magellan_db,
                       ov.atheneum_db AS atheneum_db,
                       ov.language AS language,
                       mg.enabled AS enabled,
                       CAST(mg.last_reindexed AS TEXT) AS last_indexed,
                       COALESCE(mg.file_count, 0) AS file_count,
                       COALESCE(mg.symbol_count, 0) AS symbol_count,
                       COALESCE(ov.created_at, CURRENT_TIMESTAMP) AS created_at
                FROM mg.project_registry AS mg
                LEFT JOIN project_overlay AS ov ON mg.name = ov.name
                WHERE mg.enabled = 1 AND COALESCE(ov.enabled, 1) = 1
                UNION ALL
                SELECT name, root_path, magellan_db, atheneum_db, language, enabled,
                       last_indexed, file_count, symbol_count, created_at
                FROM project_overlay
                WHERE enabled = 1
                  AND name NOT IN (SELECT name FROM mg.project_registry)
            ) AS merged"
                .to_string()
        } else {
            "SELECT name, root_path, magellan_db, atheneum_db, language, enabled,
                    last_indexed, file_count, symbol_count, created_at
             FROM project_overlay"
                .to_string()
        }
    }

    /// Register (or update) a project's atheneum-side data in the overlay.
    ///
    /// In production this records enrichment (language, atheneum_db) for a
    /// project whose existence/root paths are owned by magellan. When magellan
    /// is absent it also seeds root_path/magellan_db so the overlay can stand
    /// alone as the registry.
    pub fn register_project(
        &mut self,
        name: &str,
        root_path: &str,
        magellan_db: &str,
        atheneum_db: Option<&str>,
        language: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO project_overlay
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

    /// List all enabled projects (magellan-canonical ∪ overlay).
    pub fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        let sql = format!("{} WHERE enabled = 1 ORDER BY name", self.projects_source());
        self.query_projects(&sql, [])
    }

    /// List enabled projects filtered by language.
    pub fn list_projects_by_language(&self, language: &str) -> Result<Vec<ProjectInfo>> {
        let sql = format!(
            "{} WHERE enabled = 1 AND language = ?1 ORDER BY name",
            self.projects_source()
        );
        self.query_projects(&sql, [language])
    }

    /// Get a single project by name (any enabled state).
    pub fn get_project(&self, name: &str) -> Result<Option<ProjectInfo>> {
        let sql = format!("{} WHERE name = ?1", self.projects_source());
        let mut out = self.query_projects(&sql, [name])?;
        Ok(out.pop())
    }

    /// Disable a project (soft delete / veto).
    ///
    /// Writes an `enabled = 0` overlay row. For overlay-only projects this
    /// removes them from listings; for magellan-canonical projects it acts as a
    /// veto (the canonical row is suppressed via `COALESCE(ov.enabled,1)=1`).
    pub fn disable_project(&mut self, name: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO project_overlay (name, enabled) VALUES (?1, 0)
             ON CONFLICT(name) DO UPDATE SET enabled = 0",
            [name],
        )?;
        Ok(())
    }

    /// Whether magellan's canonical registry is attached.
    pub fn magellan_attached(&self) -> bool {
        self.magellan_attached
    }

    /// Path to the atheneum meta.db file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Raw SQLite connection for cross-router attach/detach operations.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Run a project query and map rows into [`ProjectInfo`].
    fn query_projects<P>(&self, sql: &str, params: P) -> Result<Vec<ProjectInfo>>
    where
        P: rusqlite::Params,
    {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params, |row| {
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
