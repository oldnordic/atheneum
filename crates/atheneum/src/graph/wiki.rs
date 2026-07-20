use anyhow::Result;
use chrono::Utc;
use regex::Regex;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;
use std::sync::OnceLock;

use super::cache::{CacheDomain, QueryCacheKey, QueryCacheValue};
use super::hashing::sha256_hex;
use super::planning::{KanbanStatus, KanbanUpdate};
use super::types::{EdgeType, WikiSearchResult};
use super::AtheneumGraph;

const WIKILINK_AUTOLINK_MIN_SCORE: f32 = 0.95;
const WIKILINK_AUTOLINK_MARGIN: f32 = 0.20;

fn wikilink_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[\[([^\[\]]+?)\]\]").expect("static regex"))
}

fn journal_header_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^##\s+(?:(\d{2}:\d{2})\s*\|\s*)?(.+?)\s*$").expect("static regex")
    })
}

fn kanban_update_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"["']([^"']+?)["']\s*(?:->|→)\s*(TODO|IN[_ -]?PROGRESS|DONE|BLOCKED)"#)
            .expect("static regex")
    })
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WikiPage {
    pub id: i64,
    pub path: String,
    pub title: Option<String>,
    pub content_hash: Option<String>,
    pub body: String,
    pub wikilinks: Vec<String>,
    pub project_id: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalSection {
    pub time: Option<String>,
    pub title: String,
    pub body: String,
    pub kanban_updates: Vec<KanbanUpdate>,
}

pub fn extract_wikilinks(content: &str) -> Vec<String> {
    wikilink_re()
        .captures_iter(content)
        .map(|c| c[1].to_string())
        .collect()
}

pub fn content_hash(content: &str) -> String {
    sha256_hex(content)
}

pub fn parse_journal_sections(content: &str) -> Vec<JournalSection> {
    let re = journal_header_re();
    let mut headers: Vec<(usize, Option<String>, String)> = Vec::new();
    for m in re.captures_iter(content) {
        let full = m.get(0).expect("full match");
        let body_start = full.end();
        let time = m.get(1).map(|t| t.as_str().to_string());
        let title = m.get(2).map(|t| t.as_str().to_string()).unwrap_or_default();
        headers.push((body_start, time, title));
    }

    let mut sections = Vec::with_capacity(headers.len());
    for (i, (body_start, time, title)) in headers.iter().enumerate() {
        let raw_end = headers
            .get(i + 1)
            .map(|h| content[..h.0].rfind("\n##").map(|p| p + 1).unwrap_or(h.0))
            .unwrap_or(content.len());

        let body = content[*body_start..raw_end].trim().to_string();
        let kanban_updates = extract_kanban_updates(&body);
        sections.push(JournalSection {
            time: time.clone(),
            title: title.clone(),
            body,
            kanban_updates,
        });
    }
    sections
}

pub fn extract_kanban_updates(content: &str) -> Vec<KanbanUpdate> {
    kanban_update_re()
        .captures_iter(content)
        .filter_map(|c| {
            let task = c.get(1)?.as_str().to_string();
            let status = KanbanStatus::parse(c.get(2)?.as_str())?;
            Some(KanbanUpdate {
                task_title: task,
                new_status: status,
            })
        })
        .collect()
}

fn parse_frontmatter_lenient(content: &str) -> (Value, &str) {
    super::parse_frontmatter(content).unwrap_or((Value::Object(serde_json::Map::new()), content))
}

impl AtheneumGraph {
    pub fn ingest_wiki_page(
        &self,
        path: &str,
        content: &str,
        project_id: Option<&str>,
    ) -> Result<i64> {
        let (frontmatter, body) = parse_frontmatter_lenient(content);
        let mut data = json!({
            "path": path,
            "body": body,
            "content_hash": content_hash(body),
            "wikilinks": extract_wikilinks(body),
        });
        if let Some(obj) = data.as_object_mut() {
            if let Some(fm_obj) = frontmatter.as_object() {
                for (k, v) in fm_obj {
                    obj.insert(k.clone(), v.clone());
                }
            }
            if let Some(pid) = project_id {
                obj.insert("project_id".to_string(), Value::String(pid.to_string()));
            }
        }

        let path_s = path.to_string();
        let title = data.get("title").and_then(|v| v.as_str()).map(String::from);
        let body_s = body.to_string();
        let content_hash_s = data
            .get("content_hash")
            .and_then(|v| v.as_str())
            .map(String::from);
        let wikilinks_s = data
            .get("wikilinks")
            .map(super::json_to_string)
            .transpose()?;
        let project_s = project_id.map(|s| s.to_string());
        let metadata_str = super::json_to_string(&data)?;
        let now = Utc::now().to_rfc3339();
        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO wiki_pages
                    (path, title, content_hash, body, wikilinks, project_id, metadata, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                 ON CONFLICT(path) DO UPDATE SET
                    title = excluded.title,
                    content_hash = excluded.content_hash,
                    body = excluded.body,
                    wikilinks = excluded.wikilinks,
                    project_id = excluded.project_id,
                    metadata = excluded.metadata,
                    updated_at = excluded.updated_at",
                rusqlite::params![
                    path_s,
                    title,
                    content_hash_s,
                    body_s,
                    wikilinks_s,
                    project_s,
                    metadata_str,
                    now,
                ],
            )?;
            let id: i64 = conn.query_row(
                "SELECT id FROM wiki_pages WHERE path = ?1",
                rusqlite::params![path_s],
                |r| r.get(0),
            )?;
            Ok(id)
        })?;

        if let Some(obj) = data.as_object_mut() {
            obj.insert("sql_id".to_string(), Value::Number(sql_id.into()));
        }

        let existing = self.find_ontology_entity("WikiPage", path)?;
        let page_id = if let Some(id) = existing {
            self.update_entity_data(id, &data)?;
            id
        } else {
            let entity = GraphEntity {
                id: 0,
                kind: "WikiPage".to_string(),
                name: path.to_string(),
                file_path: Some(path.to_string()),
                data: data.clone(),
            };
            self.inner
                .insert_entity(&entity)
                .map_err(|e| anyhow::anyhow!("Failed to insert WikiPage: {}", e))?
        };

        let indexed_page = GraphEntity {
            id: page_id,
            kind: "WikiPage".to_string(),
            name: path.to_string(),
            file_path: Some(path.to_string()),
            data: data.clone(),
        };
        if let Err(e) = self.add_entity_to_search_index(&indexed_page) {
            eprintln!("[atheneum] wiki auto-index warning: {}", e);
        }

        // Create wikilink graph edges — create stub targets if they don't exist
        let wikilinks = extract_wikilinks(body);
        for target in &wikilinks {
            let target_id = match self.resolve_wikilink_target_id(target, page_id, project_id)? {
                Some(id) => id,
                None => {
                    // Create a stub WikiPage entity for the missing target
                    let stub_entity = GraphEntity {
                        id: 0,
                        kind: "WikiPage".to_string(),
                        name: target.clone(),
                        file_path: None,
                        data: serde_json::json!({"stub": true, "name": target}),
                    };
                    self.inner
                        .insert_entity(&stub_entity)
                        .map_err(|e| anyhow::anyhow!("Failed to insert stub WikiPage: {}", e))?
                }
            };
            let _ = self.insert_edge(
                page_id,
                target_id,
                EdgeType::Wikilink,
                serde_json::json!({"link_type": "wikilink", "target": target}),
            );
        }

        self.runtime.record_wiki_write();
        self.runtime.bump_generation(CacheDomain::Wiki);
        Ok(page_id)
    }

    pub fn ingest_journal(
        &self,
        path: &str,
        content: &str,
        project_id: Option<&str>,
    ) -> Result<Vec<i64>> {
        let sections = parse_journal_sections(content);
        let mut ids = Vec::with_capacity(sections.len());
        for (idx, section) in sections.iter().enumerate() {
            let wikilinks_json =
                serde_json::to_value(extract_wikilinks(&section.body)).unwrap_or(Value::Null);
            let kanban_json = serde_json::to_value(
                section
                    .kanban_updates
                    .iter()
                    .map(|u| {
                        json!({
                            "task_title": u.task_title,
                            "new_status": u.new_status.as_str(),
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap_or(Value::Null);

            let mut data = json!({
                "path": path,
                "section_index": idx,
                "time": section.time,
                "title": section.title,
                "body": section.body,
                "wikilinks": wikilinks_json,
                "kanban_updates": kanban_json,
            });
            if let (Some(pid), Some(obj)) = (project_id, data.as_object_mut()) {
                obj.insert("project_id".to_string(), Value::String(pid.to_string()));
            }

            let path_s = path.to_string();
            let idx_i64 = idx as i64;
            let time_s = section.time.clone();
            let title_s = section.title.clone();
            let body_s = section.body.clone();
            let wikilinks_s = super::json_to_string(&wikilinks_json)?;
            let kanban_s = super::json_to_string(&kanban_json)?;
            let project_s = project_id.map(|s| s.to_string());
            let metadata_str = super::json_to_string(&data)?;
            let now = Utc::now().to_rfc3339();
            let sql_id = self.with_raw_connection(|conn| {
                conn.execute(
                    "INSERT INTO journal_sections
                        (path, section_index, time, title, body, kanban_updates,
                         wikilinks, project_id, metadata, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(path, section_index) DO UPDATE SET
                        time = excluded.time,
                        title = excluded.title,
                        body = excluded.body,
                        kanban_updates = excluded.kanban_updates,
                        wikilinks = excluded.wikilinks,
                        project_id = excluded.project_id,
                        metadata = excluded.metadata",
                    rusqlite::params![
                        path_s,
                        idx_i64,
                        time_s,
                        title_s,
                        body_s,
                        kanban_s,
                        wikilinks_s,
                        project_s,
                        metadata_str,
                        now,
                    ],
                )?;
                let id: i64 = conn.query_row(
                    "SELECT id FROM journal_sections WHERE path = ?1 AND section_index = ?2",
                    rusqlite::params![path_s, idx_i64],
                    |r| r.get(0),
                )?;
                Ok(id)
            })?;

            if let Some(obj) = data.as_object_mut() {
                obj.insert("sql_id".to_string(), Value::Number(sql_id.into()));
            }

            let name = format!("{}#{}", path, idx);
            let entity = GraphEntity {
                id: 0,
                kind: "JournalSection".to_string(),
                name,
                file_path: Some(path.to_string()),
                data,
            };
            let id = self
                .inner
                .insert_entity(&entity)
                .map_err(|e| anyhow::anyhow!("Failed to insert JournalSection: {}", e))?;
            let indexed_section = GraphEntity { id, ..entity };
            if let Err(e) = self.add_entity_to_search_index(&indexed_section) {
                eprintln!("[atheneum] journal auto-index warning: {}", e);
            }
            ids.push(id);
        }
        Ok(ids)
    }

    pub fn sync_wiki_directory(
        &self,
        dir: &std::path::Path,
        project_id: Option<&str>,
    ) -> Result<Vec<i64>> {
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(dir)
            .map_err(|e| anyhow::anyhow!("read_dir {} failed: {}", dir.display(), e))?
        {
            let entry = entry.map_err(|e| anyhow::anyhow!("dir entry: {}", e))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("read {} failed: {}", path.display(), e))?;
            let id =
                self.ingest_wiki_page(path.to_str().unwrap_or_default(), &content, project_id)?;
            ids.push(id);
        }
        Ok(ids)
    }

    pub fn query_journal_sections(&self, path: &str) -> Result<Vec<JournalSection>> {
        self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT time, title, body, kanban_updates FROM journal_sections WHERE path = ?1 ORDER BY section_index"
            )?;
            let rows = stmt.query_map(rusqlite::params![path], |r| {
                let kanban_str: Option<String> = r.get(3)?;
                let kanban_updates = kanban_str
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                Ok(JournalSection {
                    time: r.get(0)?,
                    title: r.get(1)?,
                    body: r.get(2)?,
                    kanban_updates,
                })
            })?;
            let mut sections = Vec::new();
            for row in rows {
                sections.push(row?);
            }
            Ok(sections)
        })
    }

    pub fn sync_journal_directory(
        &self,
        dir: &std::path::Path,
        project_id: Option<&str>,
    ) -> Result<Vec<i64>> {
        let mut all_ids = Vec::new();
        for entry in std::fs::read_dir(dir)
            .map_err(|e| anyhow::anyhow!("read_dir {} failed: {}", dir.display(), e))?
        {
            let entry = entry.map_err(|e| anyhow::anyhow!("dir entry: {}", e))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("read {} failed: {}", path.display(), e))?;
            let ids =
                self.ingest_journal(path.to_str().unwrap_or_default(), &content, project_id)?;
            all_ids.extend(ids);
        }
        Ok(all_ids)
    }

    pub fn get_wiki_page(&self, path: &str) -> Result<Option<WikiPage>> {
        self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, path, title, content_hash, body, wikilinks, project_id, metadata, created_at, updated_at
                 FROM wiki_pages WHERE path = ?1"
            )?;
            let row_mapper = |r: &rusqlite::Row| -> rusqlite::Result<WikiPage> {
                let wikilinks_str: Option<String> = r.get(5)?;
                let wikilinks = wikilinks_str
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                let metadata_str: Option<String> = r.get(7)?;
                let metadata = metadata_str
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or(serde_json::Value::Null);
                Ok(WikiPage {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    title: r.get(2)?,
                    content_hash: r.get(3)?,
                    body: r.get(4)?,
                    wikilinks,
                    project_id: r.get(6)?,
                    metadata,
                    created_at: r.get(8)?,
                    updated_at: r.get(9)?,
                })
            };
            // 1. Try exact path match.
            match stmt.query_row(rusqlite::params![path], row_mapper) {
                Ok(page) => return Ok(Some(page)),
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(e) => return Err(e.into()),
            }
            // 2. Fallback: partial path match (contains). Handles user typing
            //    "magellan.md" or "post-mortem-watch-mode" when the stored path
            //    is the full filesystem path. ORDER BY shortest path to prefer
            //    the most specific match.
            let pattern = format!("%{}%", path);
            let mut stmt2 = conn.prepare_cached(
                "SELECT id, path, title, content_hash, body, wikilinks, project_id, metadata, created_at, updated_at
                 FROM wiki_pages WHERE path LIKE ?1 ORDER BY length(path) ASC LIMIT 1"
            )?;
            match stmt2.query_row(rusqlite::params![pattern], row_mapper) {
                Ok(page) => Ok(Some(page)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
    }

    pub fn list_wiki_pages_page(
        &self,
        project_id: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<WikiPage>> {
        let lim = limit as i64;
        let off = offset as i64;
        self.with_raw_connection(move |conn| {
            let sql = if project_id.is_some() {
                "SELECT id, path, title, content_hash, body, wikilinks, project_id, metadata, created_at, updated_at
                 FROM wiki_pages WHERE project_id = ?1 ORDER BY path LIMIT ?2 OFFSET ?3"
            } else {
                "SELECT id, path, title, content_hash, body, wikilinks, project_id, metadata, created_at, updated_at
                 FROM wiki_pages ORDER BY path LIMIT ?1 OFFSET ?2"
            };
            let mut stmt = conn.prepare_cached(sql)?;
            let rows = if let Some(pid) = project_id {
                stmt.query_map(rusqlite::params![pid, lim, off], wiki_page_from_row)?
            } else {
                stmt.query_map(rusqlite::params![lim, off], wiki_page_from_row)?
            };
            let mut pages = Vec::new();
            for row in rows {
                pages.push(row?);
            }
            Ok(pages)
        })
    }

    pub fn list_wiki_pages(&self, project_id: Option<&str>) -> Result<Vec<WikiPage>> {
        self.runtime.record_wiki_query();
        let cache_key = QueryCacheKey::ListWikiPages {
            project_id: project_id.map(str::to_string),
        };
        if let Some(QueryCacheValue::WikiPages(pages)) =
            self.runtime.cache_get(&cache_key, CacheDomain::Wiki)
        {
            return Ok(pages);
        }

        let pages = self.list_wiki_pages_page(project_id, 0, usize::MAX)?;
        self.runtime.cache_store(
            cache_key,
            CacheDomain::Wiki,
            QueryCacheValue::WikiPages(pages.clone()),
        );
        Ok(pages)
    }

    pub fn find_pages_by_wikilink(
        &self,
        target: &str,
        project_id: Option<&str>,
    ) -> Result<Vec<WikiPage>> {
        self.with_raw_connection(|conn| {
            let sql = if project_id.is_some() {
                "SELECT id, path, title, content_hash, body, wikilinks, project_id, metadata, created_at, updated_at
                 FROM wiki_pages WHERE project_id = ?1
                 AND EXISTS (SELECT 1 FROM json_each(wikilinks) WHERE json_each.value = ?2)
                 ORDER BY path"
            } else {
                "SELECT id, path, title, content_hash, body, wikilinks, project_id, metadata, created_at, updated_at
                 FROM wiki_pages
                 WHERE EXISTS (SELECT 1 FROM json_each(wikilinks) WHERE json_each.value = ?1)
                 ORDER BY path"
            };
            let mut stmt = conn.prepare_cached(sql)?;
            let rows = if let Some(pid) = project_id {
                stmt.query_map(rusqlite::params![pid, target], wiki_page_from_row)?
            } else {
                stmt.query_map(rusqlite::params![target], wiki_page_from_row)?
            };
            let mut pages = Vec::new();
            for row in rows {
                pages.push(row?);
            }
            Ok(pages)
        })
    }

    pub fn find_wiki_page_entity_id(&self, path: &str) -> Result<Option<i64>> {
        self.find_entity_id_by_kind_and_name("WikiPage", path)
    }

    pub fn find_entity_id_by_kind_and_wikilink(
        &self,
        kind: &str,
        wikilink: &str,
    ) -> Result<Option<i64>> {
        self.with_raw_connection(|conn| {
            // Try exact name match first
            let exact: Option<i64> = conn
                .query_row(
                    "SELECT id FROM graph_entities WHERE kind = ?1 AND name = ?2 LIMIT 1",
                    rusqlite::params![kind, wikilink],
                    |r| r.get(0),
                )
                .ok();
            if exact.is_some() {
                return Ok(exact);
            }
            // Try path suffix match (e.g. "Dest" matches "wiki/dest.md")
            let suffix_space = format!("%{}.md", wikilink.to_lowercase());
            let suffix_under = format!("%{}.md", wikilink.to_lowercase().replace(' ', "_"));
            let suffix2 = wikilink.to_lowercase();
            let result: Option<i64> = conn
                .query_row(
                    "SELECT id FROM graph_entities WHERE kind = ?1 AND (
                        LOWER(name) LIKE ?2 OR
                        LOWER(name) LIKE ?3 OR
                        LOWER(json_extract(data, '$.path')) LIKE ?4 OR
                        LOWER(json_extract(data, '$.title')) = ?5
                    ) LIMIT 1",
                    rusqlite::params![kind, suffix_space, suffix_under, suffix2, suffix2],
                    |r| r.get(0),
                )
                .ok();
            Ok(result)
        })
    }

    fn resolve_wikilink_target_id(
        &self,
        target: &str,
        source_page_id: i64,
        project_id: Option<&str>,
    ) -> Result<Option<i64>> {
        if let Some(id) = self.find_entity_id_by_kind_and_wikilink("WikiPage", target)? {
            if id != source_page_id {
                return Ok(Some(id));
            }
        }

        let candidates = self.preview_entity_candidates(
            target,
            2,
            project_id,
            Some("WikiPage"),
            WIKILINK_AUTOLINK_MIN_SCORE,
        )?;

        let mut candidates = candidates
            .into_iter()
            .filter(|candidate| candidate.id != source_page_id)
            .filter(|candidate| candidate.data.get("stub").and_then(|v| v.as_bool()) != Some(true))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Ok(None);
        }

        let best = candidates.remove(0);
        if let Some(next) = candidates.first() {
            if best.score - next.score < WIKILINK_AUTOLINK_MARGIN {
                return Ok(None);
            }
        }

        Ok(Some(best.id))
    }

    pub fn outgoing_wikilinks(&self, page_id: i64) -> Result<Vec<GraphEntity>> {
        let mut targets = Vec::new();
        for edge in self.outgoing_edges(page_id)? {
            if is_wikilink_edge(&edge.edge_type, &edge.data) {
                if let Ok(entity) = self.get_entity(edge.to_id) {
                    targets.push(entity);
                }
            }
        }
        Ok(targets)
    }

    pub fn incoming_wikilinks(&self, page_id: i64) -> Result<Vec<GraphEntity>> {
        let mut sources = Vec::new();
        for edge in self.incoming_edges(page_id)? {
            if is_wikilink_edge(&edge.edge_type, &edge.data) {
                if let Ok(entity) = self.get_entity(edge.from_id) {
                    sources.push(entity);
                }
            }
        }
        Ok(sources)
    }
}

fn is_wikilink_edge(edge_type: &str, data: &Value) -> bool {
    edge_type == EdgeType::Wikilink.as_str()
        || (edge_type == EdgeType::RelatedTo.as_str()
            && data.get("link_type").and_then(|v| v.as_str()) == Some("wikilink"))
}

fn fts5_query(input: &str) -> String {
    let tokens: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect();
    if tokens.is_empty() {
        // Match nothing rather than returning arbitrary rows.
        return "\"\"".to_string();
    }
    tokens.join(" ")
}

fn tokens(input: &str) -> Vec<String> {
    input
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

impl AtheneumGraph {
    /// Full-text search over wiki pages using the FTS5 index.
    ///
    /// Returns paginated results with excerpts only. The full body is never
    /// returned here; callers can fetch it via `get_wiki_page(path)`.
    pub fn search_wiki_pages(
        &self,
        query: &str,
        project_id: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<WikiSearchResult>> {
        self.runtime.record_wiki_query();
        let mut hits = self.search_wiki_pages_fts(query, project_id, offset, limit)?;
        if hits.is_empty() {
            hits = self.search_wiki_pages_by_name(query, project_id, offset, limit)?;
        }
        Ok(hits)
    }

    fn search_wiki_pages_fts(
        &self,
        query: &str,
        project_id: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<WikiSearchResult>> {
        let lim = limit as i64;
        let off = offset as i64;
        let fts_query = fts5_query(query);
        self.with_raw_connection(|conn| {
            let sql = if project_id.is_some() {
                "SELECT wp.id, wp.path, wp.title, wp.body, wp.project_id, wp.created_at, wp.updated_at,
                        rank
                 FROM wiki_pages_fts
                 INNER JOIN wiki_pages wp ON wiki_pages_fts.rowid = wp.id
                 WHERE wiki_pages_fts MATCH ?1
                   AND (wp.project_id = ?2 OR wp.project_id IS NULL OR wp.project_id = '')
                 ORDER BY rank
                 LIMIT ?3 OFFSET ?4"
            } else {
                "SELECT wp.id, wp.path, wp.title, wp.body, wp.project_id, wp.created_at, wp.updated_at,
                        rank
                 FROM wiki_pages_fts
                 INNER JOIN wiki_pages wp ON wiki_pages_fts.rowid = wp.id
                 WHERE wiki_pages_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2 OFFSET ?3"
            };
            let project_id_owned = project_id.map(|s| s.to_string());
            let params: Vec<&dyn rusqlite::ToSql> = if let Some(ref pid) = project_id_owned {
                vec![
                    &fts_query as &dyn rusqlite::ToSql,
                    pid as &dyn rusqlite::ToSql,
                    &lim,
                    &off,
                ]
            } else {
                vec![&fts_query as &dyn rusqlite::ToSql, &lim, &off]
            };
            let mut stmt = conn.prepare_cached(sql)?;
            let mut rows = stmt.query(params.as_slice())?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(wiki_search_row_to_result(row, query)?);
            }
            Ok(out)
        })
    }

    /// Fallback search over graph entity names/paths and titles when FTS5
    /// returns no hits. This catches partial path fragments (e.g. "kanban"
    /// matching `/wiki/pages/kanban.md`) and concept words that don't appear
    /// in the indexed body.
    fn search_wiki_pages_by_name(
        &self,
        query: &str,
        project_id: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<WikiSearchResult>> {
        let ts = tokens(query);
        if ts.is_empty() {
            return Ok(Vec::new());
        }

        self.with_raw_connection(|conn| {
            let sql = if project_id.is_some() {
                "SELECT wp.id, wp.path, wp.title, wp.body, wp.project_id, wp.created_at, wp.updated_at
                 FROM graph_entities ge
                 INNER JOIN wiki_pages wp ON ge.name = wp.path
                 WHERE ge.kind = 'WikiPage'
                   AND (wp.project_id = ?1 OR wp.project_id IS NULL OR wp.project_id = '')
                   AND (wp.title LIKE ?2 OR wp.path LIKE ?2 OR wp.body LIKE ?2)
                 ORDER BY wp.path"
            } else {
                "SELECT wp.id, wp.path, wp.title, wp.body, wp.project_id, wp.created_at, wp.updated_at
                 FROM graph_entities ge
                 INNER JOIN wiki_pages wp ON ge.name = wp.path
                 WHERE ge.kind = 'WikiPage'
                   AND (wp.title LIKE ?1 OR wp.path LIKE ?1 OR wp.body LIKE ?1)
                 ORDER BY wp.path"
            };

            let mut candidate_ids: Option<std::collections::HashSet<i64>> = None;
            let project_id_owned = project_id.map(|s| s.to_string());

            for token in &ts {
                let pattern = format!("%{}%", token);
                let params: Vec<&dyn rusqlite::ToSql> = if let Some(ref pid) = project_id_owned {
                    vec![pid as &dyn rusqlite::ToSql, &pattern as &dyn rusqlite::ToSql]
                } else {
                    vec![&pattern as &dyn rusqlite::ToSql]
                };
                let mut stmt = conn.prepare_cached(sql)?;
                let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, i64>(0))?;
                let ids: std::collections::HashSet<i64> = rows.collect::<Result<_, _>>()?;
                candidate_ids = Some(match candidate_ids {
                    Some(prev) => prev.intersection(&ids).copied().collect(),
                    None => ids,
                });
                if candidate_ids.as_ref().map(|s| s.is_empty()).unwrap_or(false) {
                    return Ok(Vec::new());
                }
            }

            let ids: Vec<i64> = candidate_ids.map(|s| s.into_iter().collect()).unwrap_or_default();
            if ids.is_empty() {
                return Ok(Vec::new());
            }

            // Fetch full rows for the surviving ids, ordered by path, then paginate in Rust.
            // Wiki page counts are modest; in-memory pagination is acceptable.
            let placeholders = ids
                .iter()
                .map(|_| "?".to_string())
                .collect::<Vec<_>>()
                .join(",");
            let fetch_sql = format!(
                "SELECT id, path, title, content_hash, body, wikilinks, project_id, metadata, created_at, updated_at
                 FROM wiki_pages
                 WHERE id IN ({placeholders})
                 ORDER BY path"
            );
            let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
            let mut stmt = conn.prepare(&fetch_sql)?;
            let rows = stmt.query_map(params.as_slice(), wiki_page_from_row)?;
            let mut pages: Vec<WikiPage> = rows.collect::<Result<_, _>>()?;
            let total = pages.len();
            let start = offset.min(total);
            let end = (offset + limit).min(total);
            pages = pages.split_off(start);
            pages.truncate(end - start);

            Ok(pages
                .into_iter()
                .map(|p| WikiSearchResult {
                    id: p.id,
                    path: p.path.clone(),
                    title: p.title,
                    excerpt: excerpt_from_body(&p.body, &ts.join(" "), 240),
                    score: 0.0,
                    created_at: p.created_at,
                    updated_at: p.updated_at,
                    project_id: p.project_id,
                })
                .collect())
        })
    }

    /// Backfill any wiki_pages rows that are missing or stubbed in graph_entities.
    ///
    /// This repairs the "stub insanity": pages saved directly to the SQL table
    /// without going through `ingest_wiki_page` become real graph nodes with
    /// wikilink edges.
    pub fn backfill_wiki_pages_to_graph(
        &self,
        project_id: Option<&str>,
    ) -> Result<Vec<(i64, String)>> {
        let pages = self.list_wiki_pages_page(project_id, 0, usize::MAX)?;
        let mut fixed = Vec::new();
        for page in pages {
            let existing = self.find_wiki_page_entity_id(&page.path)?;
            let needs_backfill = match existing {
                Some(id) => {
                    let entity = self.get_entity(id)?;
                    entity.data.get("stub").and_then(|v| v.as_bool()) == Some(true)
                        || entity.data.get("body").is_none()
                }
                None => true,
            };
            if !needs_backfill {
                continue;
            }
            // Reconstruct the content in the format ingest_wiki_page expects,
            // so frontmatter and wikilinks are parsed consistently.
            let mut content = String::new();
            if let Some(title) = &page.title {
                content.push_str("---\n");
                content.push_str(&format!("title: {}\n", title));
                content.push_str("---\n");
            }
            content.push_str(&page.body);
            let id = self.ingest_wiki_page(&page.path, &content, page.project_id.as_deref())?;
            fixed.push((id, page.path));
        }
        Ok(fixed)
    }
}

fn wiki_search_row_to_result(
    r: &rusqlite::Row,
    query: &str,
) -> Result<WikiSearchResult, rusqlite::Error> {
    let id: i64 = r.get(0)?;
    let path: String = r.get(1)?;
    let title: Option<String> = r.get(2)?;
    let body: Option<String> = r.get(3)?;
    let body_str = body.as_deref().unwrap_or("");
    let score: f64 = r.get(7)?;
    let excerpt = excerpt_from_body(body_str, query, 240);
    Ok(WikiSearchResult {
        id,
        path,
        title,
        excerpt,
        score,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        project_id: r.get(4)?,
    })
}

fn excerpt_from_body(body: &str, query: &str, max_len: usize) -> String {
    let query_lower = query.to_lowercase();
    let first_token = query_lower
        .split(|c: char| !c.is_alphanumeric())
        .find(|t| !t.is_empty())
        .map(String::from);

    let (start, end) = if let Some(token) = first_token {
        let body_lower = body.to_lowercase();
        if let Some(pos) = body_lower.find(&token) {
            let half = max_len / 2;
            let s = snap_to_char_boundary(body, pos.saturating_sub(half));
            let raw_end = (pos + token.len() + half).min(body.len());
            let e = snap_to_char_boundary(body, raw_end);
            (s, e)
        } else {
            (0, body.len().min(max_len))
        }
    } else {
        (0, body.len().min(max_len))
    };

    let mut excerpt = String::new();
    if start > 0 {
        excerpt.push_str("...");
    }
    excerpt.push_str(&body[start..end].replace('\n', " "));
    if end < body.len() {
        excerpt.push_str("...");
    }
    excerpt
}

fn snap_to_char_boundary(s: &str, mut byte_idx: usize) -> usize {
    byte_idx = byte_idx.min(s.len());
    while byte_idx > 0 && !s.is_char_boundary(byte_idx) {
        byte_idx -= 1;
    }
    byte_idx
}

fn wiki_page_from_row(r: &rusqlite::Row) -> Result<WikiPage, rusqlite::Error> {
    let wikilinks_str: Option<String> = r.get(5)?;
    let wikilinks = wikilinks_str
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let metadata_str: Option<String> = r.get(7)?;
    let metadata = metadata_str
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null);
    Ok(WikiPage {
        id: r.get(0)?,
        path: r.get(1)?,
        title: r.get(2)?,
        content_hash: r.get(3)?,
        body: r.get(4)?,
        wikilinks,
        project_id: r.get(6)?,
        metadata,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}
