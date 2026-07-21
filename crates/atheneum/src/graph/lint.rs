use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::{AtheneumGraph, EdgeType, EntityType};

const METADATA_EDGE_TYPES: &[&str] = &[
    "belongs_to_project",
    "observed_in",
    "performed_by",
    "called",
    "accessed",
    "modified",
    "handled_by_tool",
];

#[derive(Debug, Clone)]
pub struct LintConfig {
    pub stale_superseded_days: i64,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            stale_superseded_days: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OrphanFinding {
    pub entity_id: i64,
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrokenLinkFinding {
    pub source_page_id: i64,
    pub source_path: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StaleSupersededFinding {
    pub entity_id: i64,
    pub kind: String,
    pub name: String,
    pub age_days: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExpiredFinding {
    pub entity_id: i64,
    pub name: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LintReport {
    pub orphans: Vec<OrphanFinding>,
    pub broken_links: Vec<BrokenLinkFinding>,
    pub stale_superseded: Vec<StaleSupersededFinding>,
    pub expired: Vec<ExpiredFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokenLinkMode {
    Stub,
    Sever,
}

#[derive(Debug, Clone)]
pub struct MaintainConfig {
    pub rewire_threshold: f64,
    pub broken_link_mode: BrokenLinkMode,
    pub stale_superseded_days: i64,
}

impl Default for MaintainConfig {
    fn default() -> Self {
        Self {
            rewire_threshold: 0.3,
            broken_link_mode: BrokenLinkMode::Stub,
            stale_superseded_days: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct MaintainReport {
    pub orphans_rewired: usize,
    pub broken_links_resolved: usize,
    pub contradictions_superseded: usize,
    pub stale_superseded_pruned: usize,
    pub expired_memories_pruned: usize,
}

fn trigrams(text: &str) -> std::collections::HashSet<String> {
    let lower = text.to_ascii_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    if chars.len() < 3 {
        return chars.iter().map(|c| c.to_string()).collect();
    }
    let mut set = std::collections::HashSet::new();
    for window in chars.windows(3) {
        let trig: String = window.iter().collect();
        set.insert(trig);
    }
    set
}

fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let ta = trigrams(a);
    let tb = trigrams(b);
    if ta.is_empty() && tb.is_empty() {
        return 1.0;
    }
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let intersection = ta.intersection(&tb).count() as f64;
    let union = ta.union(&tb).count() as f64;
    intersection / union
}

impl AtheneumGraph {
    pub fn lint_graph(&self, config: &LintConfig) -> Result<LintReport> {
        let mut orphans = Vec::new();
        let mut broken_links = Vec::new();
        let mut stale_superseded = Vec::new();

        // 1. Orphans scan
        let kinds = &[
            EntityType::Concept,
            EntityType::Memory,
            EntityType::WikiPage,
        ];
        for kind in kinds {
            let entities = self.entities_by_kind(kind.as_str())?;
            for entity in entities {
                // Exclude index files themselves from being reported as orphans
                let is_index_entity = (entity.kind == "WikiPage"
                    && (entity.name == "index.md" || entity.name.ends_with("/index.md")))
                    || entity.data.get("role").and_then(|v| v.as_str()) == Some("auto_index");
                if is_index_entity {
                    continue;
                }

                let incoming = self.incoming_edges(entity.id)?;
                let mut active_incoming = 0;
                for edge in incoming {
                    if METADATA_EDGE_TYPES.contains(&edge.edge_type.as_str()) {
                        continue;
                    }
                    if let Ok(source) = self.get_entity(edge.from_id) {
                        let is_index = (source.kind == "WikiPage"
                            && source.name.ends_with("/index.md"))
                            || source.data.get("role").and_then(|v| v.as_str())
                                == Some("auto_index");
                        if is_index {
                            continue;
                        }
                    }
                    active_incoming += 1;
                }
                if active_incoming == 0 {
                    orphans.push(OrphanFinding {
                        entity_id: entity.id,
                        kind: entity.kind,
                        name: entity.name,
                    });
                }
            }
        }

        // 2. Broken links scan
        let pages = self.list_wiki_pages(None)?;
        for page in pages {
            for target in &page.wikilinks {
                let resolved = self.find_entity_id_by_kind_and_wikilink("WikiPage", target)?;
                let is_broken = match resolved {
                    None => true,
                    Some(tid) => {
                        if let Ok(t_ent) = self.get_entity(tid) {
                            t_ent.data.get("stub").and_then(|v| v.as_bool()) == Some(true)
                        } else {
                            true
                        }
                    }
                };
                if is_broken {
                    broken_links.push(BrokenLinkFinding {
                        source_page_id: page.id,
                        source_path: page.path.clone(),
                        target: target.clone(),
                    });
                }
            }
        }

        // 3. Stale superseded scan
        let self_edges = self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT from_id FROM graph_edges WHERE from_id = to_id AND edge_type = 'superseded_by'"
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
            let mut ids = Vec::new();
            for r in rows {
                ids.push(r?);
            }
            Ok(ids)
        })?;

        for eid in self_edges {
            if let Ok(entity) = self.get_entity(eid) {
                let superseded_at = entity
                    .data
                    .get("superseded_at")
                    .or_else(|| entity.data.get("updated_at"))
                    .and_then(|v| v.as_str());
                if let Some(sat_str) = superseded_at {
                    if let Ok(sat) = chrono::DateTime::parse_from_rfc3339(sat_str) {
                        let age = chrono::Utc::now()
                            .signed_duration_since(sat.with_timezone(&chrono::Utc));
                        if age.num_days() >= config.stale_superseded_days {
                            stale_superseded.push(StaleSupersededFinding {
                                entity_id: entity.id,
                                kind: entity.kind,
                                name: entity.name,
                                age_days: age.num_days(),
                            });
                        }
                    }
                }
            }
        }

        let mut expired = Vec::new();
        let memories = self.entities_by_kind("Memory")?;
        for m in memories {
            if let Some(exp_str) = m.data.get("expires_at").and_then(|v| v.as_str()) {
                if let Ok(exp_dt) = chrono::DateTime::parse_from_rfc3339(exp_str) {
                    if chrono::Utc::now() > exp_dt.with_timezone(&chrono::Utc) {
                        expired.push(ExpiredFinding {
                            entity_id: m.id,
                            name: m.name.clone(),
                            expires_at: exp_str.to_string(),
                        });
                    }
                }
            }
        }

        Ok(LintReport {
            orphans,
            broken_links,
            stale_superseded,
            expired,
        })
    }

    pub fn maintain(&self, config: &MaintainConfig, apply: bool) -> Result<MaintainReport> {
        let mut report = MaintainReport::default();
        let lint = self.lint_graph(&LintConfig {
            stale_superseded_days: config.stale_superseded_days,
        })?;

        // 1. Orphan rewiring
        let concepts = self.entities_by_kind(EntityType::Concept.as_str())?;
        for orphan in &lint.orphans {
            let mut best_concept: Option<&GraphEntity> = None;
            let mut best_score = 0.0;
            for c in &concepts {
                if c.id == orphan.entity_id {
                    continue;
                }
                let score = jaccard_similarity(&orphan.name, &c.name);
                if score > best_score {
                    best_score = score;
                    best_concept = Some(c);
                }
            }
            if best_score >= config.rewire_threshold {
                if let Some(c) = best_concept {
                    if apply {
                        self.insert_edge_pair(
                            orphan.entity_id,
                            c.id,
                            EdgeType::RelatedTo,
                            json!({}),
                            EdgeType::RelatedTo,
                            json!({}),
                        )?;
                    }
                    report.orphans_rewired += 1;
                }
            }
        }

        // 2. Broken links repair
        let mut stubbed_targets = std::collections::HashSet::new();
        let mut wiki_changed = false;
        for bl in &lint.broken_links {
            match config.broken_link_mode {
                BrokenLinkMode::Stub => {
                    if stubbed_targets.contains(&bl.target) {
                        continue;
                    }
                    if apply {
                        let source_entity = self.get_entity(bl.source_page_id)?;
                        let project_id = source_entity
                            .data
                            .get("project_id")
                            .and_then(|v| v.as_str());
                        let pages = self.list_wiki_pages(project_id)?;
                        let project_wiki_dir = pages
                            .iter()
                            .find(|p| p.path.contains('/') || p.path.contains('\\'))
                            .and_then(|p| {
                                std::path::Path::new(&p.path)
                                    .parent()
                                    .map(|d| d.to_path_buf())
                            });

                        let filename =
                            format!("{}.md", bl.target.to_ascii_lowercase().replace(' ', "_"));
                        let target_path = if let Some(dir) = project_wiki_dir {
                            dir.join(filename)
                        } else {
                            std::env::current_dir()?.join(filename)
                        };

                        let body = format!("# {}\n\nThis page is a stub.", bl.target);
                        std::fs::write(&target_path, &body)?;
                        let new_page_id = self.ingest_wiki_page(
                            target_path.to_str().unwrap_or_default(),
                            &body,
                            project_id,
                        )?;

                        // Clean up the stub entity and re-route its incoming edges to the new page id
                        if let Some(stub_id) =
                            self.find_entity_id_by_kind_and_name("WikiPage", &bl.target)?
                        {
                            if let Ok(t_ent) = self.get_entity(stub_id) {
                                if t_ent.data.get("stub").and_then(|v| v.as_bool()) == Some(true) {
                                    self.with_raw_connection(|conn| {
                                        conn.execute(
                                            "DELETE FROM graph_entities WHERE id = ?1",
                                            rusqlite::params![stub_id],
                                        )?;
                                        conn.execute(
                                            "UPDATE graph_edges SET to_id = ?1 WHERE to_id = ?2",
                                            rusqlite::params![new_page_id, stub_id],
                                        )?;
                                        Ok::<(), anyhow::Error>(())
                                    })?;
                                    self.runtime.remove_entity_id("WikiPage", &bl.target);
                                }
                            }
                        }
                        wiki_changed = true;
                    }
                    stubbed_targets.insert(bl.target.clone());
                    report.broken_links_resolved += 1;
                }
                BrokenLinkMode::Sever => {
                    if apply {
                        let source_entity = self.get_entity(bl.source_page_id)?;
                        let path = &source_entity.name;
                        let project_id = source_entity
                            .data
                            .get("project_id")
                            .and_then(|v| v.as_str());
                        if let Some(page) = self.get_wiki_page(path)? {
                            let target_pattern = format!("[[{}]]", bl.target);
                            let new_body = page.body.replace(&target_pattern, &bl.target);
                            std::fs::write(&page.path, &new_body)?;
                            self.ingest_wiki_page(&page.path, &new_body, project_id)?;
                        }
                        // Delete the specific edge from source to stub
                        if let Some(stub_id) =
                            self.find_entity_id_by_kind_and_name("WikiPage", &bl.target)?
                        {
                            self.with_raw_connection(|conn| {
                                conn.execute(
                                    "DELETE FROM graph_edges WHERE from_id = ?1 AND to_id = ?2 AND edge_type = 'wikilink'",
                                    rusqlite::params![bl.source_page_id, stub_id]
                                )?;
                                Ok::<(), anyhow::Error>(())
                            })?;
                        }
                        wiki_changed = true;
                    }
                    report.broken_links_resolved += 1;
                }
            }
        }
        if wiki_changed {
            self.runtime
                .bump_generation(super::cache::CacheDomain::Wiki);
        }

        // 3. Contradiction resolve
        let memories = self.entities_by_kind(EntityType::Memory.as_str())?;
        let mut active_memories = Vec::new();
        for m in &memories {
            if m.data.get("superseded_at").is_none() {
                active_memories.push(m);
            }
        }
        let mut by_key: std::collections::HashMap<&str, Vec<&GraphEntity>> =
            std::collections::HashMap::new();
        for m in &active_memories {
            by_key.entry(&m.name).or_default().push(m);
        }
        let mut memory_changed = false;
        for (_key, group) in by_key {
            if group.len() < 2 {
                continue;
            }
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    let mi = group[i];
                    let mj = group[j];
                    let si = mi.data.get("scope").and_then(|v| v.as_str()).unwrap_or("");
                    let sj = mj.data.get("scope").and_then(|v| v.as_str()).unwrap_or("");
                    if si == sj {
                        continue;
                    }
                    let ci = mi
                        .data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let cj = mj
                        .data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if jaccard_similarity(ci, cj) > 0.5 {
                        continue;
                    }
                    let ti_str = mi
                        .data
                        .get("updated_at")
                        .or_else(|| mi.data.get("created_at"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let tj_str = mj
                        .data
                        .get("updated_at")
                        .or_else(|| mj.data.get("created_at"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let ti = chrono::DateTime::parse_from_rfc3339(ti_str)
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now());
                    let tj = chrono::DateTime::parse_from_rfc3339(tj_str)
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now());

                    let old_mem = if ti < tj { mi } else { mj };
                    if apply {
                        let mut updated_data = old_mem.data.clone();
                        updated_data.as_object_mut().unwrap().insert(
                            "superseded_at".to_string(),
                            Value::String(chrono::Utc::now().to_rfc3339()),
                        );
                        self.update_entity_data(old_mem.id, &updated_data)?;
                        self.insert_edge(
                            old_mem.id,
                            old_mem.id,
                            EdgeType::SupersededBy,
                            json!({"reason": "contradiction"}),
                        )?;
                        memory_changed = true;
                    }
                    report.contradictions_superseded += 1;
                }
            }
        }

        // 4. Stale superseded pruning
        for ss in &lint.stale_superseded {
            if apply {
                if let Ok(entity) = self.get_entity(ss.entity_id) {
                    self.with_raw_connection(|conn| {
                        let tx = conn.unchecked_transaction()?;
                        tx.execute(
                            "DELETE FROM graph_edges WHERE from_id = ?1 OR to_id = ?1",
                            rusqlite::params![ss.entity_id],
                        )?;
                        tx.execute(
                            "DELETE FROM graph_entities WHERE id = ?1",
                            rusqlite::params![ss.entity_id],
                        )?;
                        if entity.kind == EntityType::Memory.as_str() {
                            tx.execute(
                                "DELETE FROM memory_entries WHERE key = ?1",
                                rusqlite::params![entity.name],
                            )?;
                        }
                        tx.commit()?;
                        Ok::<(), anyhow::Error>(())
                    })?;
                    self.runtime.remove_entity_id(&entity.kind, &entity.name);
                    memory_changed = true;
                }
            }
            report.stale_superseded_pruned += 1;
        }

        for ss in &lint.expired {
            if apply {
                self.with_raw_connection(|conn| {
                    let tx = conn.unchecked_transaction()?;
                    tx.execute(
                        "DELETE FROM graph_edges WHERE from_id = ?1 OR to_id = ?1",
                        rusqlite::params![ss.entity_id],
                    )?;
                    tx.execute(
                        "DELETE FROM graph_entities WHERE id = ?1",
                        rusqlite::params![ss.entity_id],
                    )?;
                    tx.execute(
                        "DELETE FROM memory_entries WHERE key = ?1",
                        rusqlite::params![ss.name],
                    )?;
                    tx.commit()?;
                    Ok::<(), anyhow::Error>(())
                })?;
                self.runtime
                    .remove_entity_id(EntityType::Memory.as_str(), &ss.name);
                memory_changed = true;
            }
            report.expired_memories_pruned += 1;
        }

        if memory_changed {
            self.runtime
                .bump_generation(super::cache::CacheDomain::Memory);
        }

        Ok(report)
    }
}
