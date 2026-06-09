//! Dreaming: reflective memory consolidation pass.
//!
//! Inspired by Anthropic's AutoDream: scan memories for near-duplicates,
//! stale entries, contradictions, and verbosity. Merge or prune as needed
//! so future sessions orient quickly against a high-signal memory store.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlitegraph::GraphEntity;
use std::collections::HashMap;

use super::{AtheneumGraph, EdgeType};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// How aggressive the dream pass should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DreamMode {
    /// Report findings only; do not mutate the graph.
    DryRun,
    /// Merge near-duplicates and mark superseded entries.
    AutoMerge,
}

/// One issue found during a dream pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamFinding {
    pub phase: DreamPhase,
    pub entity_ids: Vec<i64>,
    pub description: String,
    /// When `mode == AutoMerge` and action was taken.
    pub action_taken: Option<String>,
}

/// Phases of a dream pass, matching AutoDream's pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DreamPhase {
    /// Collected memories for analysis.
    Scan,
    /// Two or more memories are near-duplicates (Jaccard ≥ threshold).
    Deduplicate,
    /// Memory has not been updated in a long time and has low confidence.
    Stale,
    /// Same key, overlapping scopes, but contradictory content.
    Contradiction,
    /// Content is long but has low information density.
    Verbose,
    /// Entries that were merged or superseded.
    Consolidated,
    /// Wiki page has no incoming wikilinks from other pages (isolated stub).
    Orphan,
}

/// Full output of a dream pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamReport {
    pub mode: DreamMode,
    pub scope: Option<String>,
    pub project_id: Option<String>,
    pub memories_scanned: usize,
    /// Populated by wiki_dream_pass; 0 for memory dream passes.
    pub pages_scanned: usize,
    pub findings: Vec<DreamFinding>,
    pub started_at: String,
    pub finished_at: String,
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Tunable knobs for the dream pass.
#[derive(Debug, Clone)]
pub struct DreamConfig {
    /// Minimum trigram-Jaccard similarity to flag as near-duplicate (0..1).
    pub dedup_threshold: f64,
    /// Memories not updated in this many days are considered stale.
    pub stale_days: i64,
    /// Confidence below which a stale memory is flagged for pruning.
    pub stale_confidence_threshold: f64,
    /// Content length (chars) above which verbosity is checked.
    pub verbose_length_threshold: usize,
    /// Unique-word ratio below which content is flagged as verbose.
    pub verbose_density_threshold: f64,
}

impl Default for DreamConfig {
    fn default() -> Self {
        Self {
            dedup_threshold: 0.65,
            stale_days: 30,
            stale_confidence_threshold: 0.5,
            verbose_length_threshold: 500,
            verbose_density_threshold: 0.25,
        }
    }
}

// ---------------------------------------------------------------------------
// Text similarity
// ---------------------------------------------------------------------------

/// Extract character trigrams from text (lowercased, ascii-only).
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

/// Jaccard similarity between two texts via character trigrams.
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

/// Unique-word ratio: |unique words| / max(|total words|, 1).
fn unique_word_ratio(text: &str) -> f64 {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return 0.0;
    }
    let unique: std::collections::HashSet<&str> = words.iter().copied().collect();
    unique.len() as f64 / words.len() as f64
}

// ---------------------------------------------------------------------------
// Helper: extract scalar fields from a GraphEntity's JSON data
// ---------------------------------------------------------------------------

fn data_str(entity: &GraphEntity, key: &str) -> String {
    entity
        .data
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn data_f64(entity: &GraphEntity, key: &str) -> f64 {
    entity.data.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Dream pass implementation
// ---------------------------------------------------------------------------

impl AtheneumGraph {
    /// Run a full dreaming pass over Memory entities.
    ///
    /// - `mode` — `DryRun` reports only; `AutoMerge` mutates the graph.
    /// - `scope` / `project_id` — optional filters.
    /// - `config` — tuning knobs; use `DreamConfig::default()` for defaults.
    pub fn dream_pass(
        &self,
        mode: DreamMode,
        scope: Option<&str>,
        project_id: Option<&str>,
        config: &DreamConfig,
    ) -> Result<DreamReport> {
        self.runtime.record_dream_run();
        let started_at = Utc::now().to_rfc3339();
        let mut findings: Vec<DreamFinding> = Vec::new();

        // Phase 1: SCAN — collect memories
        let memories = self.list_memory(scope, project_id)?;
        let n = memories.len();

        // Phase 2: DEDUPLICATE — pairwise Jaccard
        let mut merged_ids: HashMap<i64, i64> = HashMap::new(); // old_id -> keeper_id
        for i in 0..memories.len() {
            let mi = &memories[i];
            if merged_ids.contains_key(&mi.id) {
                continue;
            }
            for mj in memories.iter().skip(i + 1) {
                if merged_ids.contains_key(&mj.id) {
                    continue;
                }
                // Only compare memories with the same key
                if mi.name != mj.name {
                    continue;
                }
                let ci = data_str(mi, "content");
                let cj = data_str(mj, "content");
                let sim = jaccard_similarity(&ci, &cj);
                if sim >= config.dedup_threshold {
                    // Keep the one with higher confidence or more recent update
                    let keep_i = data_f64(mi, "confidence") >= data_f64(mj, "confidence");
                    let (keeper, superseded) = if keep_i { (mi, mj) } else { (mj, mi) };

                    let mut action = None;
                    if mode == DreamMode::AutoMerge {
                        // Create superseded_by edge: superseded -> keeper
                        let _ = self.insert_edge(
                            superseded.id,
                            keeper.id,
                            EdgeType::SupersededBy,
                            json!({
                                "reason": "dream_dedup",
                                "similarity": (sim as f32),
                            }),
                        );
                        merged_ids.insert(superseded.id, keeper.id);
                        action = Some(format!(
                            "superseded {} -> {} (sim={:.2})",
                            superseded.id, keeper.id, sim
                        ));
                    }

                    findings.push(DreamFinding {
                        phase: DreamPhase::Deduplicate,
                        entity_ids: vec![superseded.id, keeper.id],
                        description: format!(
                            "Near-duplicate (Jaccard {:.2}): '{}' keeper={}, superseded={}",
                            sim, mi.name, keeper.id, superseded.id,
                        ),
                        action_taken: action,
                    });
                }
            }
        }

        // Phase 3: STALE — old + low confidence
        let now = Utc::now();
        for m in &memories {
            if merged_ids.contains_key(&m.id) {
                continue;
            }
            let updated = data_str(m, "updated_at");
            let confidence = data_f64(m, "confidence");
            if let Ok(dt) = updated.parse::<DateTime<Utc>>() {
                let age_days = (now - dt).num_days();
                if age_days > config.stale_days && confidence < config.stale_confidence_threshold {
                    let mut action = None;
                    if mode == DreamMode::AutoMerge {
                        // Create consolidated_from edge pointing to a sentinel
                        // (the entry stays but is flagged)
                        let _ = self.insert_edge(
                            m.id,
                            m.id, // self-edge = stale marker
                            EdgeType::SupersededBy,
                            json!({
                                "reason": "dream_stale",
                                "age_days": age_days,
                            }),
                        );
                        action = Some(format!(
                            "marked stale (age={}d, conf={:.2})",
                            age_days, confidence
                        ));
                    }
                    findings.push(DreamFinding {
                        phase: DreamPhase::Stale,
                        entity_ids: vec![m.id],
                        description: format!(
                            "Stale: '{}' not updated in {}d, confidence {:.2}",
                            m.name, age_days, confidence,
                        ),
                        action_taken: action,
                    });
                }
            }
        }

        // Phase 4: CONTRADICTION — same key, different scope, different content
        let mut by_key: HashMap<&str, Vec<&GraphEntity>> = HashMap::new();
        for m in &memories {
            if merged_ids.contains_key(&m.id) {
                continue;
            }
            by_key.entry(&m.name).or_default().push(m);
        }
        for (key, group) in &by_key {
            if group.len() < 2 {
                continue;
            }
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    let mi = group[i];
                    let mj = group[j];
                    let si = data_str(mi, "scope");
                    let sj = data_str(mj, "scope");
                    // Different scope but same key
                    if si == sj {
                        continue;
                    }
                    let ci = data_str(mi, "content");
                    let cj = data_str(mj, "content");
                    // Content must be meaningfully different (low similarity)
                    let sim = jaccard_similarity(&ci, &cj);
                    if sim > 0.5 {
                        continue;
                    }
                    findings.push(DreamFinding {
                        phase: DreamPhase::Contradiction,
                        entity_ids: vec![mi.id, mj.id],
                        description: format!(
                            "Possible contradiction on key '{}': scope '{}' says '{}...' vs scope '{}' says '{}...'",
                            key,
                            si,
                            &ci[..ci.len().min(60)],
                            sj,
                            &cj[..cj.len().min(60)],
                        ),
                        action_taken: None, // Contradictions require human review
                    });
                }
            }
        }

        // Phase 5: VERBOSE — long content, low information density
        for m in &memories {
            if merged_ids.contains_key(&m.id) {
                continue;
            }
            let content = data_str(m, "content");
            if content.len() > config.verbose_length_threshold {
                let density = unique_word_ratio(&content);
                if density < config.verbose_density_threshold {
                    findings.push(DreamFinding {
                        phase: DreamPhase::Verbose,
                        entity_ids: vec![m.id],
                        description: format!(
                            "Verbose: '{}' is {} chars but unique-word ratio only {:.2} (threshold {:.2})",
                            m.name,
                            content.len(),
                            density,
                            config.verbose_density_threshold,
                        ),
                        action_taken: None, // Requires human rewrite
                    });
                }
            }
        }

        let finished_at = Utc::now().to_rfc3339();

        Ok(DreamReport {
            mode,
            scope: scope.map(String::from),
            project_id: project_id.map(String::from),
            memories_scanned: n,
            pages_scanned: 0,
            findings,
            started_at,
            finished_at,
        })
    }

    /// Run a dreaming pass over WikiPage entities.
    ///
    /// Phases:
    /// - **Deduplicate**: pages with near-identical body content (Jaccard ≥ threshold).
    /// - **Stale**: pages not updated in `stale_days` and with short body (likely stubs).
    /// - **Verbose**: long pages with low unique-word ratio.
    /// - **Orphan**: pages with no incoming wikilinks from other pages in the same project.
    pub fn wiki_dream_pass(
        &self,
        mode: DreamMode,
        project_id: Option<&str>,
        config: &DreamConfig,
    ) -> Result<DreamReport> {
        self.runtime.record_wiki_dream_run();
        let started_at = Utc::now().to_rfc3339();
        let mut findings: Vec<DreamFinding> = Vec::new();

        let pages = self.list_wiki_pages(project_id)?;
        let n = pages.len();

        // Build incoming-link map: path -> count of pages that link to it
        let mut incoming: HashMap<String, usize> = HashMap::new();
        for page in &pages {
            for link in &page.wikilinks {
                *incoming.entry(link.clone()).or_default() += 1;
            }
        }

        // Phase 2: DEDUPLICATE — pairwise Jaccard on body
        let mut merged_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
        for i in 0..pages.len() {
            let pi = &pages[i];
            if merged_paths.contains(&pi.path) {
                continue;
            }
            for pj in pages.iter().skip(i + 1) {
                if merged_paths.contains(&pj.path) {
                    continue;
                }
                let sim = jaccard_similarity(&pi.body, &pj.body);
                if sim >= config.dedup_threshold {
                    let mut action = None;
                    if mode == DreamMode::AutoMerge {
                        let _ = self.insert_edge(
                            pj.id,
                            pi.id,
                            EdgeType::SupersededBy,
                            serde_json::json!({
                                "reason": "wiki_dream_dedup",
                                "similarity": sim as f32,
                            }),
                        );
                        merged_paths.insert(pj.path.clone());
                        action = Some(format!(
                            "superseded {} -> {} (sim={:.2})",
                            pj.id, pi.id, sim
                        ));
                    }
                    findings.push(DreamFinding {
                        phase: DreamPhase::Deduplicate,
                        entity_ids: vec![pi.id, pj.id],
                        description: format!(
                            "Near-duplicate pages (Jaccard {:.2}): '{}' and '{}'",
                            sim, pi.path, pj.path
                        ),
                        action_taken: action,
                    });
                }
            }
        }

        // Phase 3: STALE — old page with short body (stub likely abandoned)
        let now = Utc::now();
        let stub_len = 120; // bodies shorter than this are "stub"
        for page in &pages {
            if merged_paths.contains(&page.path) {
                continue;
            }
            let updated = page.updated_at.as_deref().unwrap_or(&page.created_at);
            if let Ok(dt) = updated.parse::<DateTime<Utc>>() {
                let age_days = (now - dt).num_days();
                if age_days > config.stale_days && page.body.len() < stub_len {
                    findings.push(DreamFinding {
                        phase: DreamPhase::Stale,
                        entity_ids: vec![page.id],
                        description: format!(
                            "Stale stub: '{}' not updated in {}d, body only {} chars",
                            page.path,
                            age_days,
                            page.body.len()
                        ),
                        action_taken: None,
                    });
                }
            }
        }

        // Phase 5: VERBOSE — long body, low information density
        for page in &pages {
            if merged_paths.contains(&page.path) {
                continue;
            }
            if page.body.len() > config.verbose_length_threshold {
                let density = unique_word_ratio(&page.body);
                if density < config.verbose_density_threshold {
                    findings.push(DreamFinding {
                        phase: DreamPhase::Verbose,
                        entity_ids: vec![page.id],
                        description: format!(
                            "Verbose: '{}' is {} chars, unique-word ratio {:.2}",
                            page.path,
                            page.body.len(),
                            density
                        ),
                        action_taken: None,
                    });
                }
            }
        }

        // Phase ORPHAN — no incoming links from other pages in same project
        for page in &pages {
            if merged_paths.contains(&page.path) {
                continue;
            }
            // Strip directory prefix to get the link target (e.g. "pages/foo.md" -> "foo")
            let link_key = page
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&page.path)
                .trim_end_matches(".md");
            let count = incoming.get(link_key).copied().unwrap_or(0);
            if count == 0 {
                findings.push(DreamFinding {
                    phase: DreamPhase::Orphan,
                    entity_ids: vec![page.id],
                    description: format!("Orphan: '{}' has no incoming wikilinks", page.path),
                    action_taken: None,
                });
            }
        }

        let finished_at = Utc::now().to_rfc3339();

        Ok(DreamReport {
            mode,
            scope: None,
            project_id: project_id.map(String::from),
            memories_scanned: 0,
            pages_scanned: n,
            findings,
            started_at,
            finished_at,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigram_jaccard_identical() {
        let sim = jaccard_similarity("hello world", "hello world");
        assert!(
            sim > 0.99,
            "identical strings should have similarity ~1.0, got {:.3}",
            sim
        );
    }

    #[test]
    fn trigram_jaccard_similar() {
        let sim = jaccard_similarity(
            "User prefers concise responses",
            "User prefers concise response",
        );
        assert!(
            sim >= 0.65,
            "near-duplicate should score >= 0.65, got {:.3}",
            sim
        );
    }

    #[test]
    fn trigram_jaccard_different() {
        let sim = jaccard_similarity("RX 7900 XT powers desktop", "magellan is a code indexer");
        assert!(
            sim < 0.3,
            "unrelated strings should have low similarity, got {:.3}",
            sim
        );
    }

    #[test]
    fn unique_word_ratio_dense() {
        let ratio = unique_word_ratio("each word here is unique totally");
        assert!(
            ratio > 0.8,
            "all-unique words should have high ratio, got {:.2}",
            ratio
        );
    }

    #[test]
    fn unique_word_ratio_repetitive() {
        let ratio = unique_word_ratio(&"the the the the the the the the the data".repeat(10));
        assert!(
            ratio < 0.25,
            "repetitive text should have low ratio, got {:.2}",
            ratio
        );
    }

    #[test]
    fn trigram_jaccard_empty() {
        assert_eq!(jaccard_similarity("", ""), 1.0);
        assert_eq!(jaccard_similarity("hello", ""), 0.0);
    }

    #[test]
    fn dream_dry_run_no_mutations() {
        let graph = AtheneumGraph::open_in_memory().expect("in-memory graph");
        // Store near-duplicate memories with different scopes so both persist
        // (store_memory upserts by key+scope, so same key+scope would merge)
        let _id1 = graph
            .store_memory(
                "test-key",
                "User prefers concise responses in English",
                "user",
                1.0,
                None,
            )
            .unwrap();
        let _id2 = graph
            .store_memory(
                "test-key",
                "User prefers concise response in English",
                "memory",
                0.9,
                None,
            )
            .unwrap();

        let report = graph
            .dream_pass(DreamMode::DryRun, None, None, &DreamConfig::default())
            .unwrap();

        assert_eq!(report.mode, DreamMode::DryRun);
        assert_eq!(report.memories_scanned, 2);
        // Should find the near-duplicate
        let dedup_findings: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.phase == DreamPhase::Deduplicate)
            .collect();
        assert_eq!(
            dedup_findings.len(),
            1,
            "dry run should detect the near-duplicate"
        );
        // Dry run must not create edges
        assert!(
            dedup_findings[0].action_taken.is_none(),
            "dry run must not take actions"
        );
    }

    #[test]
    fn dream_auto_merge_creates_superseded_edge() {
        let graph = AtheneumGraph::open_in_memory().expect("in-memory graph");
        let id1 = graph
            .store_memory(
                "test-key",
                "User prefers concise responses in English",
                "user",
                1.0,
                None,
            )
            .unwrap();
        let id2 = graph
            .store_memory(
                "test-key",
                "User prefers concise response in English",
                "memory",
                0.8,
                None,
            )
            .unwrap();

        let report = graph
            .dream_pass(DreamMode::AutoMerge, None, None, &DreamConfig::default())
            .unwrap();

        let dedup: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.phase == DreamPhase::Deduplicate)
            .collect();
        assert_eq!(dedup.len(), 1);
        assert!(dedup[0].action_taken.is_some());

        // Verify the edge exists from superseded -> keeper
        // id1 has higher confidence (1.0 > 0.8) so id2 is superseded, pointing to id1
        let edges = graph.outgoing_edges(id2).unwrap();
        let has_superseded = edges
            .iter()
            .any(|e| e.edge_type == "superseded_by" && e.to_id == id1);
        assert!(
            has_superseded,
            "id2 should have a superseded_by edge pointing to id1"
        );
    }

    #[test]
    fn wiki_dream_dedup_similar_pages() {
        let graph = AtheneumGraph::open_in_memory().expect("in-memory graph");
        graph
            .ingest_wiki_page(
                "pages/rust-async.md",
                "# Rust Async Guide\nTokio enables async IO in Rust. Use async/await syntax. Spawn tasks with tokio::spawn.",
                Some("grounded"),
            )
            .unwrap();
        graph
            .ingest_wiki_page(
                "pages/async-rust.md",
                "# Async Rust Guide\nTokio enables async IO in Rust. Use async/await syntax. Spawn tasks with tokio::spawn.",
                Some("grounded"),
            )
            .unwrap();

        let report = graph
            .wiki_dream_pass(DreamMode::DryRun, Some("grounded"), &DreamConfig::default())
            .unwrap();

        assert_eq!(report.pages_scanned, 2);
        let dedup: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.phase == DreamPhase::Deduplicate)
            .collect();
        assert_eq!(
            dedup.len(),
            1,
            "identical bodies should flag as near-duplicate"
        );
        assert!(dedup[0].action_taken.is_none(), "dry run takes no action");
    }

    #[test]
    fn wiki_dream_orphan_no_incoming_links() {
        let graph = AtheneumGraph::open_in_memory().expect("in-memory graph");
        // Page A links to B; C is isolated
        graph
            .ingest_wiki_page(
                "pages/a.md",
                "# Page A\nSee also [[b]] for more details.",
                Some("grounded"),
            )
            .unwrap();
        graph
            .ingest_wiki_page(
                "pages/b.md",
                "# Page B\nThis is page B with real content about something useful.",
                Some("grounded"),
            )
            .unwrap();
        graph
            .ingest_wiki_page(
                "pages/orphan.md",
                "# Orphan Page\nNobody links to me, I am lost and forgotten in the wiki.",
                Some("grounded"),
            )
            .unwrap();

        let report = graph
            .wiki_dream_pass(DreamMode::DryRun, Some("grounded"), &DreamConfig::default())
            .unwrap();

        assert_eq!(report.pages_scanned, 3);
        let orphans: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.phase == DreamPhase::Orphan)
            .collect();
        // page A and orphan.md have no incoming links; b.md is linked from A
        assert!(
            orphans.iter().any(|f| f.description.contains("orphan")),
            "orphan.md should be flagged"
        );
    }

    #[test]
    fn wiki_dream_verbose_page() {
        let graph = AtheneumGraph::open_in_memory().expect("in-memory graph");
        let repetitive = "the the the the the the the the the data ".repeat(60);
        graph
            .ingest_wiki_page("pages/verbose.md", &repetitive, Some("grounded"))
            .unwrap();

        let cfg = DreamConfig {
            verbose_length_threshold: 50,
            verbose_density_threshold: 0.25,
            ..DreamConfig::default()
        };
        let report = graph
            .wiki_dream_pass(DreamMode::DryRun, Some("grounded"), &cfg)
            .unwrap();

        let verbose: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.phase == DreamPhase::Verbose)
            .collect();
        assert_eq!(
            verbose.len(),
            1,
            "repetitive page should be flagged as verbose"
        );
    }

    #[test]
    fn wiki_dream_stale_page() {
        use rusqlite::params;
        let graph = AtheneumGraph::open_in_memory().expect("in-memory graph");
        let id = graph
            .ingest_wiki_page("pages/old.md", "# Old Page\nShort stub.", Some("grounded"))
            .unwrap();

        // Back-date updated_at to 60 days ago using RFC3339 so chrono can parse it
        let old_date = (Utc::now() - chrono::Duration::days(60)).to_rfc3339();
        graph
            .with_raw_connection(|conn| {
                conn.execute(
                    "UPDATE wiki_pages SET updated_at = ?1 WHERE id = ?2",
                    params![old_date, id],
                )?;
                Ok(())
            })
            .unwrap();

        let cfg = DreamConfig {
            stale_days: 30,
            stale_confidence_threshold: 1.0, // everything below 1.0 is stale — but wiki uses body length
            verbose_length_threshold: 500,
            ..DreamConfig::default()
        };
        let report = graph
            .wiki_dream_pass(DreamMode::DryRun, Some("grounded"), &cfg)
            .unwrap();

        let stale: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.phase == DreamPhase::Stale)
            .collect();
        assert_eq!(stale.len(), 1, "60-day-old short page should be stale");
        assert!(stale[0].entity_ids.contains(&id));
    }

    #[test]
    fn wiki_dream_project_filter() {
        let graph = AtheneumGraph::open_in_memory().expect("in-memory graph");
        graph
            .ingest_wiki_page(
                "pages/a.md",
                "content about Rust async programming patterns",
                Some("proj1"),
            )
            .unwrap();
        graph
            .ingest_wiki_page(
                "pages/b.md",
                "content about Rust async programming patterns",
                Some("proj2"),
            )
            .unwrap();

        let report = graph
            .wiki_dream_pass(DreamMode::DryRun, Some("proj1"), &DreamConfig::default())
            .unwrap();

        assert_eq!(
            report.pages_scanned, 1,
            "project filter should only scan proj1"
        );
        assert_eq!(report.project_id.as_deref(), Some("proj1"));
    }

    #[test]
    fn dream_contradiction_detection() {
        let graph = AtheneumGraph::open_in_memory().expect("in-memory graph");
        let _id1 = graph
            .store_memory(
                "gpu-safe-mode",
                "GPU safe mode is enabled for all kernels",
                "memory",
                1.0,
                None,
            )
            .unwrap();
        let _id2 = graph
            .store_memory(
                "gpu-safe-mode",
                "Unsafe kernels bypass safety checks entirely",
                "project",
                1.0,
                None,
            )
            .unwrap();

        let report = graph
            .dream_pass(DreamMode::DryRun, None, None, &DreamConfig::default())
            .unwrap();

        let contradictions: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.phase == DreamPhase::Contradiction)
            .collect();
        assert_eq!(
            contradictions.len(),
            1,
            "should detect contradiction between scopes"
        );
    }
}
