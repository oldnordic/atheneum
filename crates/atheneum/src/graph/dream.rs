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
use std::collections::{HashMap, HashSet};

use super::{cache::CacheDomain, AtheneumGraph, EdgeType, WikiPage};

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

        // Phase 2: DEDUPLICATE — two-pass approach
        // Pass 2a: Exact dedup via hash (eliminate 7% upfront, O(n))
        let mut seen_hashes: HashSet<u64> = HashSet::new();
        let mut merged_paths: HashSet<String> = HashSet::new();
        let mut deduped_pages: Vec<&WikiPage> = Vec::new();

        eprintln!("Pass 2a: Exact dedup ({} pages)...", pages.len());
        for page in &pages {
            let hash = seahash::hash(page.body.as_bytes());
            if seen_hashes.contains(&hash) {
                // Exact duplicate found
                findings.push(DreamFinding {
                    phase: DreamPhase::Deduplicate,
                    entity_ids: vec![page.id],
                    description: format!("Exact duplicate: '{}'", page.path),
                    action_taken: None,
                });
                merged_paths.insert(page.path.clone());
            } else {
                seen_hashes.insert(hash);
                deduped_pages.push(page);
            }
        }
        eprintln!("Pass 2a complete: {} unique pages", deduped_pages.len());

        // Pass 2b: Batched Jaccard similarity on deduped set
        const BATCH_SIZE: usize = 50;
        let total_batches = deduped_pages.len().div_ceil(BATCH_SIZE);

        for batch_num in 0..total_batches {
            let batch_start = batch_num * BATCH_SIZE;
            let batch_end = (batch_start + BATCH_SIZE).min(deduped_pages.len());

            eprintln!(
                "Pass 2b: Batch {}/{}: pages {}-{}",
                batch_num + 1,
                total_batches,
                batch_start,
                batch_end - 1
            );

            // Only Jaccard compare within this batch
            for i in batch_start..batch_end {
                let pi = &deduped_pages[i];
                if merged_paths.contains(&pi.path) {
                    continue;
                }

                // Compare only against pages in THIS batch (skip i+1 to batch_end)
                for pj in deduped_pages.iter().skip(i + 1).take(batch_end - i - 1) {
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
        }
        eprintln!("Pass 2b complete: {} batches processed", total_batches);

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

    pub fn dream_if_idle(&self, threshold_secs: u64) -> Result<Option<DreamReport>> {
        let last_write: Option<String> = self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare("SELECT MAX(updated_at) FROM memory_entries")?;
            let mut rows = stmt.query([])?;
            if let Some(row) = rows.next()? {
                let s: Option<String> = row.get(0)?;
                Ok(s)
            } else {
                Ok(None)
            }
        })?;

        if let Some(lw) = last_write {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&lw) {
                let elapsed =
                    chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
                if elapsed.num_seconds() < threshold_secs as i64 {
                    return Ok(None);
                }
            }
        }

        let report = self.dream_pass(DreamMode::AutoMerge, None, None, &DreamConfig::default())?;
        Ok(Some(report))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConsolidationConfig {
    pub similarity_threshold: f64,
    pub model: String,
    pub ollama_url: String,
    pub swap_guard: crate::config::SwapGuardMode,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.4,
            model: "gemma4:e2b".to_string(),
            ollama_url: "http://127.0.0.1:11434".to_string(),
            swap_guard: crate::config::SwapGuardMode::Fallback,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConsolidationReport {
    pub merges_completed: usize,
    pub details: Vec<String>,
}

#[derive(serde::Deserialize)]
struct LlmMergeResponse {
    should_merge: bool,
    keeper_name: String,
    keeper_description: String,
}

impl AtheneumGraph {
    pub fn semantic_consolidation(
        &self,
        config: &ConsolidationConfig,
    ) -> Result<ConsolidationReport> {
        let concepts = self.entities_by_kind("Concept")?;
        let active_concepts: Vec<_> = concepts
            .into_iter()
            .filter(|c| c.data.get("superseded_at").is_none())
            .collect();

        let mut merges_completed = 0;
        let mut details = Vec::new();
        let mut superseded_ids = std::collections::HashSet::new();

        let model_run_result = self.apply_swap_guard(&config.model, config.swap_guard);
        let use_llm = match model_run_result {
            Ok(_) => true,
            Err(e) => {
                if config.swap_guard == crate::config::SwapGuardMode::Strict {
                    return Err(anyhow::anyhow!(e));
                }
                false
            }
        };

        for i in 0..active_concepts.len() {
            let ca = &active_concepts[i];
            if superseded_ids.contains(&ca.id) {
                continue;
            }

            for cb in active_concepts.iter().skip(i + 1) {
                if superseded_ids.contains(&cb.id) {
                    continue;
                }

                let sim = jaccard_similarity(&ca.name, &cb.name);
                if sim >= config.similarity_threshold {
                    let mut should_merge = false;
                    let mut keeper_name = ca.name.clone();
                    let mut keeper_description = ca
                        .data
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    if use_llm {
                        if let Ok(decision) = self.query_llm_for_merge(ca, cb, config) {
                            should_merge = decision.should_merge;
                            if should_merge {
                                keeper_name = decision.keeper_name;
                                keeper_description = decision.keeper_description;
                            }
                        }
                    } else {
                        if sim >= 0.8 {
                            should_merge = true;
                            let desc_b = cb
                                .data
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if !desc_b.is_empty() {
                                if keeper_description.is_empty() {
                                    keeper_description = desc_b.to_string();
                                } else if !keeper_description.contains(desc_b) {
                                    keeper_description =
                                        format!("{}; {}", keeper_description, desc_b);
                                }
                            }
                        }
                    }

                    if should_merge {
                        self.execute_semantic_merge(
                            ca.id,
                            cb.id,
                            &keeper_name,
                            &keeper_description,
                        )?;
                        superseded_ids.insert(cb.id);
                        merges_completed += 1;
                        details.push(format!(
                            "Merged [{}] '{}' and [{}] '{}' -> '{}'",
                            ca.id, ca.name, cb.id, cb.name, keeper_name
                        ));
                    }
                }
            }
        }

        Ok(ConsolidationReport {
            merges_completed,
            details,
        })
    }

    fn query_llm_for_merge(
        &self,
        a: &sqlitegraph::GraphEntity,
        b: &sqlitegraph::GraphEntity,
        config: &ConsolidationConfig,
    ) -> Result<LlmMergeResponse> {
        let prompt = format!(
            "You are the Atheneum Librarian. Determine if these two concepts refer to the same entity or topic.\n\
             Concept A: Name: '{}', Description: '{}'\n\
             Concept B: Name: '{}', Description: '{}'\n\n\
             If they refer to the same thing, set 'should_merge': true, suggest a unified 'keeper_name', and a unified 'keeper_description'.\n\
             Otherwise set 'should_merge': false.\n\
             Return raw JSON strictly matching this schema: {{ \"should_merge\": bool, \"keeper_name\": \"string\", \"keeper_description\": \"string\" }}",
            a.name, a.data.get("description").and_then(|v| v.as_str()).unwrap_or(""),
            b.name, b.data.get("description").and_then(|v| v.as_str()).unwrap_or("")
        );

        let resp: serde_json::Value = ureq::post(&format!("{}/api/generate", config.ollama_url))
            .send_json(serde_json::json!({
                "model": config.model,
                "prompt": prompt,
                "stream": false,
                "format": "json",
                "options": { "temperature": 0.1 }
            }))?
            .into_json()?;

        let raw_res = resp
            .get("response")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Empty response"))?;
        let parsed: LlmMergeResponse = serde_json::from_str(raw_res)?;
        Ok(parsed)
    }

    pub fn execute_semantic_merge(
        &self,
        keeper_id: i64,
        loser_id: i64,
        _new_name: &str,
        new_desc: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();

        // 1. Update winner concept data
        let mut winner = self.get_entity(keeper_id)?;
        if let Some(obj) = winner.data.as_object_mut() {
            obj.insert(
                "description".to_string(),
                serde_json::Value::String(new_desc.to_string()),
            );
            obj.insert(
                "updated_at".to_string(),
                serde_json::Value::String(now.clone()),
            );
        }
        self.update_entity_data(keeper_id, &winner.data)?;

        // 2. Mark loser as superseded
        let mut loser = self.get_entity(loser_id)?;
        if let Some(obj) = loser.data.as_object_mut() {
            obj.insert(
                "superseded_at".to_string(),
                serde_json::Value::String(now.clone()),
            );
            obj.insert("superseded_by".to_string(), serde_json::json!(keeper_id));
        }
        self.update_entity_data(loser_id, &loser.data)?;

        // 3. Create superseded_by relation from loser to winner
        self.insert_edge(
            loser_id,
            keeper_id,
            EdgeType::SupersededBy,
            serde_json::json!({ "reason": "semantic_dream" }),
        )?;

        // 4. Rewire all incoming and outgoing edges of the loser to the winner
        self.with_raw_connection(|conn| {
            conn.execute(
                "UPDATE graph_edges SET from_id = ?1 WHERE from_id = ?2",
                rusqlite::params![keeper_id, loser_id],
            )?;
            conn.execute(
                "UPDATE graph_edges SET to_id = ?1 WHERE to_id = ?2",
                rusqlite::params![keeper_id, loser_id],
            )?;
            Ok::<(), anyhow::Error>(())
        })?;

        // 5. Invalidate caches
        self.runtime.bump_generation(CacheDomain::Memory);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LintConfig, MaintainConfig};

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

    #[test]
    fn test_dream_if_idle() {
        let graph = AtheneumGraph::open_in_memory().unwrap();

        // Seed a memory with current timestamp
        graph
            .store_memory("key", "val1", "user", 1.0, None, None)
            .unwrap();

        // dream_if_idle with threshold=10 should return None (just written)
        let res = graph.dream_if_idle(10).unwrap();
        assert!(res.is_none());

        // dream_if_idle with threshold=0 should run
        let res2 = graph.dream_if_idle(0).unwrap();
        assert!(res2.is_some());
    }

    #[test]
    fn test_execute_semantic_merge_rewires_edges() {
        let graph = AtheneumGraph::open_in_memory().unwrap();

        // 1. Create winner and loser concept
        let keeper_id = graph
            .store_memory(
                "Winner Concept",
                "Some content",
                "Winner desc",
                1.0,
                None,
                None,
            )
            .unwrap();
        let loser_id = graph
            .store_memory(
                "Loser Concept",
                "Some content",
                "Loser desc",
                1.0,
                None,
                None,
            )
            .unwrap();

        // 2. Add some third entity
        let related_id = graph
            .store_memory(
                "Related Concept",
                "Some content",
                "Related desc",
                1.0,
                None,
                None,
            )
            .unwrap();

        // 3. Connect related entity to loser
        graph
            .insert_edge(
                related_id,
                loser_id,
                EdgeType::RelatedTo,
                serde_json::json!({}),
            )
            .unwrap();

        // 4. Run semantic merge
        graph
            .execute_semantic_merge(keeper_id, loser_id, "Winner Concept", "New description")
            .unwrap();

        // 5. Verify loser is superseded
        let loser_entity = graph.get_entity(loser_id).unwrap();
        assert!(loser_entity.data.get("superseded_at").is_some());
        assert_eq!(
            loser_entity.data.get("superseded_by").unwrap(),
            &serde_json::json!(keeper_id)
        );

        // 6. Verify edges pointing to loser are rewired to winner
        let incoming_edges = graph.incoming_edges(keeper_id).unwrap();
        let has_rewired = incoming_edges
            .iter()
            .any(|e| e.from_id == related_id && e.edge_type == "related_to");
        assert!(has_rewired, "Edge should be rewired to winner");

        // 7. Verify new description is set on winner
        let winner_entity = graph.get_entity(keeper_id).unwrap();
        assert_eq!(
            winner_entity
                .data
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap(),
            "New description"
        );
    }

    #[test]
    fn test_semantic_consolidation_lexical_fallback() {
        let graph = AtheneumGraph::open_in_memory().unwrap();

        let entity_a = sqlitegraph::GraphEntity {
            id: 0,
            kind: "Concept".to_string(),
            name: "Lexical Concept Test One".to_string(),
            file_path: None,
            data: serde_json::json!({
                "description": "Desc A"
            }),
        };
        let keeper_id = graph.inner.insert_entity(&entity_a).unwrap();

        let entity_b = sqlitegraph::GraphEntity {
            id: 0,
            kind: "Concept".to_string(),
            name: "Lexical Concept Test On".to_string(),
            file_path: None,
            data: serde_json::json!({
                "description": "Desc B"
            }),
        };
        let loser_id = graph.inner.insert_entity(&entity_b).unwrap();

        let config = ConsolidationConfig {
            similarity_threshold: 0.7,
            model: "gemma4:e2b".to_string(),
            ollama_url: "http://invalid-url-to-trigger-offline-fallback.local".to_string(),
            swap_guard: crate::config::SwapGuardMode::Fallback,
        };

        let report = graph.semantic_consolidation(&config).unwrap();
        assert_eq!(report.merges_completed, 1);

        let loser_entity = graph.get_entity(loser_id).unwrap();
        assert!(loser_entity.data.get("superseded_at").is_some());
        assert_eq!(
            loser_entity.data.get("superseded_by").unwrap(),
            &serde_json::json!(keeper_id)
        );
    }

    #[test]
    fn test_pinning_and_unpinning() {
        let graph = AtheneumGraph::open_in_memory().unwrap();

        let id = graph
            .store_memory("Concept A", "Content A", "Desc A", 1.0, None, None)
            .unwrap();

        // Check initial pinned state (should be false/absent)
        let ent = graph.get_entity(id).unwrap();
        assert!(!ent
            .data
            .get("pinned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));

        // Pin entity
        graph.pin_entity(id).unwrap();
        let ent2 = graph.get_entity(id).unwrap();
        assert!(ent2
            .data
            .get("pinned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));

        // Unpin entity
        graph.unpin_entity(id).unwrap();
        let ent3 = graph.get_entity(id).unwrap();
        assert!(!ent3
            .data
            .get("pinned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
    }

    #[test]
    fn test_seed_memory_prioritizes_pinned_items() {
        let graph = AtheneumGraph::open_in_memory().unwrap();

        // Insert concept A, concept B, and concept C
        let _id_a = graph
            .store_memory("Concept A", "Content A", "Desc A", 1.0, None, None)
            .unwrap();
        let id_b = graph
            .store_memory("Concept B", "Content B", "Desc B", 1.0, None, None)
            .unwrap();
        let _id_c = graph
            .store_memory("Concept C", "Content C", "Desc C", 1.0, None, None)
            .unwrap();

        // Pin concept B
        graph.pin_entity(id_b).unwrap();

        // Generate seed memory with a very tight budget (e.g. 100 tokens) to ensure sorting puts B first
        let seed = graph.seed_memory(None, 100).unwrap();
        assert!(seed
            .instructions
            .contains("### Pinned Memories\n- [PINNED MEMORY] Concept B:"));
    }

    #[test]
    fn test_ttl_expiry_and_maintenance() {
        let graph = AtheneumGraph::open_in_memory().unwrap();

        // 1. Create a memory entry that has already expired
        let now = chrono::Utc::now();
        let past = now - chrono::Duration::hours(2);

        let data = serde_json::json!({
            "content": "Expired content",
            "expires_at": past.to_rfc3339(),
            "ttl_hours": 1
        });

        // Store memory using sqlitegraph directly or insert_entity
        let entity = sqlitegraph::GraphEntity {
            id: 0,
            kind: "Memory".to_string(),
            name: "ExpiredMemory".to_string(),
            file_path: None,
            data,
        };
        let id = graph.inner.insert_entity(&entity).unwrap();

        // Connect some edge to show edge pruning
        let other_id = graph
            .store_memory("Other Concept", "content", "desc", 1.0, None, None)
            .unwrap();
        graph
            .insert_edge(other_id, id, EdgeType::RelatedTo, serde_json::json!({}))
            .unwrap();

        // 2. Run lint - should find 1 expired entry
        let lint = graph.lint_graph(&LintConfig::default()).unwrap();
        assert_eq!(lint.expired.len(), 1);
        assert_eq!(lint.expired[0].entity_id, id);

        // 3. Run maintain with apply: true - should prune the expired entry and its edges
        let maint = graph.maintain(&MaintainConfig::default(), true).unwrap();
        assert_eq!(maint.expired_memories_pruned, 1);

        // Check entity is gone
        let res = graph.get_entity(id);
        assert!(res.is_err());

        // Check edges are gone
        let edges = graph.incoming_edges(id).unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn test_swap_guard() {
        let graph = AtheneumGraph::open_in_memory().unwrap();

        // Since there are no loaded models under testing, discover_available_models should return empty list.
        // preferred model = "gemma4:e2b"

        // 1. Strict mode should return ModelSwapBlocked error
        let res_strict = graph.apply_swap_guard("gemma4:e2b", crate::config::SwapGuardMode::Strict);
        assert!(matches!(
            res_strict,
            Err(crate::graph::types::AtheneumError::ModelSwapBlocked { .. })
        ));

        // 2. Fallback mode should return ModelSwapBlocked error
        let res_fallback =
            graph.apply_swap_guard("gemma4:e2b", crate::config::SwapGuardMode::Fallback);
        assert!(matches!(
            res_fallback,
            Err(crate::graph::types::AtheneumError::ModelSwapBlocked { .. })
        ));

        // 3. Adapt mode should return the preferred model itself if no models are loaded
        let res_adapt = graph
            .apply_swap_guard("gemma4:e2b", crate::config::SwapGuardMode::Adapt)
            .unwrap();
        assert_eq!(res_adapt, "gemma4:e2b");
    }
}
