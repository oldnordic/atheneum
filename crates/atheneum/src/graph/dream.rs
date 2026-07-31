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
    /// LLM provider used for merge decisions. `Ollama` keeps the legacy
    /// `/api/generate` path; `Anthropic`/`OpenAi`/`Custom` use
    /// `base_url`/`api_key` with their respective wire protocols.
    #[serde(default)]
    pub provider: crate::config::LlmProvider,
    /// Base URL for non-Ollama providers (e.g. `https://api.kimi.com/coding`).
    #[serde(default)]
    pub base_url: String,
    /// API key for non-Ollama providers. Never logged.
    #[serde(default)]
    pub api_key: String,
    /// When true, candidate merges are reported but never executed.
    #[serde(default)]
    pub dry_run: bool,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.4,
            model: "gemma4:e2b".to_string(),
            ollama_url: "http://127.0.0.1:11434".to_string(),
            swap_guard: crate::config::SwapGuardMode::Fallback,
            provider: crate::config::LlmProvider::Ollama,
            base_url: String::new(),
            api_key: String::new(),
            dry_run: false,
        }
    }
}

impl ConsolidationConfig {
    /// Build a consolidation config from the persisted `[llm]` config section.
    ///
    /// For `Ollama` the legacy fields (`ollama_url`, swap-guarded local model)
    /// are populated; for remote providers `base_url`/`model`/`api_key` are
    /// taken verbatim from the config.
    pub fn from_llm_config(llm: &crate::config::LlmConfig) -> Self {
        let mut cfg = Self::default();
        cfg.provider = llm.provider.clone();
        cfg.swap_guard = llm.swap_guard;
        match llm.provider {
            crate::config::LlmProvider::Ollama => {
                if !llm.base_url.is_empty() {
                    cfg.ollama_url = llm.base_url.clone();
                }
                if !llm.model.is_empty() {
                    cfg.model = llm.model.clone();
                }
            }
            _ => {
                cfg.base_url = llm.base_url.clone();
                if !llm.model.is_empty() {
                    cfg.model = llm.model.clone();
                }
                cfg.api_key = llm.api_key.clone();
            }
        }
        cfg
    }

    /// Short lowercase label for the active provider (for log/detail lines).
    fn provider_label(&self) -> &'static str {
        match self.provider {
            crate::config::LlmProvider::Ollama => "ollama",
            crate::config::LlmProvider::OpenAi => "openai",
            crate::config::LlmProvider::Anthropic => "anthropic",
            crate::config::LlmProvider::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConsolidationReport {
    pub merges_completed: usize,
    pub details: Vec<String>,
    /// Number of merge pairs actually evaluated by the LLM (0 when the
    /// lexical fallback path was used throughout).
    #[serde(default)]
    pub llm_evaluations: usize,
    #[serde(default)]
    pub dry_run: bool,
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
        let mut llm_evaluations = 0;
        let mut details = Vec::new();
        let mut superseded_ids = std::collections::HashSet::new();

        // Decide whether an LLM is available for merge evaluation.
        // - Ollama: honor the swap guard against locally loaded models.
        // - Remote providers: require an API key; the swap guard semantics
        //   are reused for the missing-credentials case (strict -> hard
        //   error, fallback/adapt -> lexical path).
        let llm_unavailable: Option<String> = match config.provider {
            crate::config::LlmProvider::Ollama => self
                .apply_swap_guard(&config.model, config.swap_guard)
                .err()
                .map(|e| e.to_string()),
            _ => {
                if config.api_key.is_empty() {
                    Some(format!(
                        "no api_key configured for {} provider",
                        config.provider_label()
                    ))
                } else if config.base_url.is_empty() {
                    Some(format!(
                        "no base_url configured for {} provider",
                        config.provider_label()
                    ))
                } else {
                    None
                }
            }
        };
        let use_llm = match llm_unavailable {
            None => true,
            Some(reason) => {
                if config.swap_guard == crate::config::SwapGuardMode::Strict {
                    return Err(anyhow::anyhow!(reason));
                }
                details.push(format!(
                    "LLM unavailable ({}); using lexical fallback",
                    reason
                ));
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
                        match self.query_llm_for_merge(ca, cb, config) {
                            Ok(decision) => {
                                llm_evaluations += 1;
                                let evidence = format!(
                                    "LLM evaluation via {} (model={}): [{}] '{}' vs [{}] '{}' -> should_merge={}",
                                    config.provider_label(),
                                    config.model,
                                    ca.id,
                                    ca.name,
                                    cb.id,
                                    cb.name,
                                    decision.should_merge
                                );
                                // Evidence line: proves a real (non-fallback) LLM
                                // call happened and which provider served it.
                                eprintln!("dream-semantic: {}", evidence);
                                tracing::info!("{}", evidence);
                                details.push(evidence);
                                should_merge = decision.should_merge;
                                if should_merge {
                                    keeper_name = decision.keeper_name;
                                    keeper_description = decision.keeper_description;
                                }
                            }
                            Err(e) => {
                                let msg = format!(
                                    "LLM evaluation via {} (model={}) FAILED for [{}] '{}' vs [{}] '{}': {:#}",
                                    config.provider_label(),
                                    config.model,
                                    ca.id,
                                    ca.name,
                                    cb.id,
                                    cb.name,
                                    e
                                );
                                eprintln!("dream-semantic: {}", msg);
                                tracing::warn!("{}", msg);
                                details.push(msg);
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
                        if config.dry_run {
                            details.push(format!(
                                "[dry-run] Would merge [{}] '{}' and [{}] '{}' -> '{}'",
                                ca.id, ca.name, cb.id, cb.name, keeper_name
                            ));
                        } else {
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
        }

        Ok(ConsolidationReport {
            merges_completed,
            details,
            llm_evaluations,
            dry_run: config.dry_run,
        })
    }

    fn query_llm_for_merge(
        &self,
        a: &sqlitegraph::GraphEntity,
        b: &sqlitegraph::GraphEntity,
        config: &ConsolidationConfig,
    ) -> Result<LlmMergeResponse> {
        // NOTE: this prompt and its JSON schema are identical across all
        // providers — only the wire protocol differs.
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

        let raw_res = match config.provider {
            crate::config::LlmProvider::Ollama => call_ollama_generate(config, &prompt)?,
            crate::config::LlmProvider::Anthropic => call_anthropic_messages(config, &prompt)?,
            crate::config::LlmProvider::OpenAi | crate::config::LlmProvider::Custom => {
                call_openai_chat_completions(config, &prompt)?
            }
        };

        parse_merge_response(&raw_res)
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
// LLM provider wire protocols (semantic consolidation)
// ---------------------------------------------------------------------------

/// Legacy Ollama path: POST {ollama_url}/api/generate.
fn call_ollama_generate(config: &ConsolidationConfig, prompt: &str) -> Result<String> {
    let resp: serde_json::Value = ureq::post(&format!("{}/api/generate", config.ollama_url))
        .send_json(serde_json::json!({
            "model": config.model,
            "prompt": prompt,
            "stream": false,
            "format": "json",
            "options": { "temperature": 0.1 }
        }))?
        .into_json()?;

    resp.get("response")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("Empty response"))
}

/// Anthropic Messages protocol: POST {base_url}/v1/messages with
/// `x-api-key` + `anthropic-version` headers. Response text at
/// `.content[0].text`.
fn call_anthropic_messages(config: &ConsolidationConfig, prompt: &str) -> Result<String> {
    let url = format!("{}/v1/messages", config.base_url.trim_end_matches('/'));
    let resp: serde_json::Value = ureq::post(&url)
        .set("x-api-key", &config.api_key)
        .set("anthropic-version", "2023-06-01")
        .set("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(120))
        .send_json(serde_json::json!({
            "model": config.model,
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": prompt }]
        }))?
        .into_json()?;

    resp.get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            // Some Anthropic-compatible endpoints (e.g. Kimi) prepend
            // `thinking` blocks — take the first block of type "text",
            // falling back to any block carrying a text field.
            arr.iter()
                .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                .or_else(|| arr.iter().find(|b| b.get("text").is_some()))
        })
        .and_then(|block| block.get("text"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("Anthropic response missing a text content block"))
}

/// OpenAI-compatible chat completions: POST {base_url}/chat/completions
/// with `Authorization: Bearer <key>`. Response text at
/// `.choices[0].message.content`. Retries once without `response_format`
/// for servers that reject it.
fn call_openai_chat_completions(config: &ConsolidationConfig, prompt: &str) -> Result<String> {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let send = |with_response_format: bool| -> std::result::Result<String, ureq::Error> {
        let mut body = serde_json::json!({
            "model": config.model,
            "messages": [{ "role": "user", "content": prompt }],
            "temperature": 0.1
        });
        if with_response_format {
            body["response_format"] = serde_json::json!({ "type": "json_object" });
        }
        let mut req = ureq::post(&url)
            .set("content-type", "application/json")
            .timeout(std::time::Duration::from_secs(120));
        if !config.api_key.is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", config.api_key));
        }
        let resp: serde_json::Value = req.send_json(body)?.into_json()?;
        resp.get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|ch| ch.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                ureq::Error::Status(
                    502,
                    ureq::Response::new(502, "Bad Gateway", "missing choices[0].message.content")
                        .unwrap(),
                )
            })
    };

    match send(true) {
        Ok(text) => Ok(text),
        Err(first_err) => {
            // Some compatible servers (older vLLM, llama.cpp) reject
            // response_format — retry without it before giving up.
            send(false).map_err(|_| anyhow::anyhow!(first_err))
        }
    }
}

/// Parse the LLM merge decision out of a raw response string, tolerating
/// Markdown code fences and surrounding prose.
fn parse_merge_response(raw: &str) -> Result<LlmMergeResponse> {
    let mut text = raw.trim();

    // Strip a single wrapping code fence (```json ... ``` or ``` ... ```).
    if text.starts_with("```") {
        if let Some(first_nl) = text.find('\n') {
            let inner = &text[first_nl + 1..];
            text = inner.strip_suffix("```").unwrap_or(inner).trim();
        }
    }

    if let Ok(parsed) = serde_json::from_str::<LlmMergeResponse>(text) {
        return Ok(parsed);
    }

    // Fall back to the outermost JSON object in the string.
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
        if start < end {
            if let Ok(parsed) = serde_json::from_str::<LlmMergeResponse>(&text[start..=end]) {
                return Ok(parsed);
            }
        }
    }

    Err(anyhow::anyhow!(
        "Failed to parse LLM merge decision JSON from response"
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LintConfig, MaintainConfig};

    #[test]
    fn parse_merge_response_plain_json() {
        let raw = r#"{"should_merge": true, "keeper_name": "Foo", "keeper_description": "Bar"}"#;
        let parsed = parse_merge_response(raw).unwrap();
        assert!(parsed.should_merge);
        assert_eq!(parsed.keeper_name, "Foo");
        assert_eq!(parsed.keeper_description, "Bar");
    }

    #[test]
    fn parse_merge_response_code_fence() {
        let raw = "```json\n{\"should_merge\": false, \"keeper_name\": \"\", \"keeper_description\": \"\"}\n```";
        let parsed = parse_merge_response(raw).unwrap();
        assert!(!parsed.should_merge);
    }

    #[test]
    fn parse_merge_response_surrounding_prose() {
        let raw = "Here is my decision:\n{\"should_merge\": true, \"keeper_name\": \"K\", \"keeper_description\": \"D\"}\nHope that helps.";
        let parsed = parse_merge_response(raw).unwrap();
        assert!(parsed.should_merge);
        assert_eq!(parsed.keeper_name, "K");
    }

    #[test]
    fn parse_merge_response_garbage_errors() {
        assert!(parse_merge_response("no json here").is_err());
    }

    #[test]
    fn consolidation_config_from_llm_config_anthropic() {
        let llm = crate::config::LlmConfig {
            provider: crate::config::LlmProvider::Anthropic,
            base_url: "https://api.kimi.com/coding".to_string(),
            model: "kimi-k3".to_string(),
            api_key: "sk-test".to_string(),
            swap_guard: crate::config::SwapGuardMode::Fallback,
        };
        let cfg = ConsolidationConfig::from_llm_config(&llm);
        assert_eq!(cfg.provider, crate::config::LlmProvider::Anthropic);
        assert_eq!(cfg.base_url, "https://api.kimi.com/coding");
        assert_eq!(cfg.model, "kimi-k3");
        assert_eq!(cfg.api_key, "sk-test");
        // Legacy ollama default retained for the ollama path.
        assert_eq!(cfg.ollama_url, "http://127.0.0.1:11434");
    }

    #[test]
    fn consolidation_config_from_llm_config_ollama() {
        let llm = crate::config::LlmConfig {
            provider: crate::config::LlmProvider::Ollama,
            base_url: "http://localhost:11434".to_string(),
            model: "codellama".to_string(),
            api_key: String::new(),
            swap_guard: crate::config::SwapGuardMode::Adapt,
        };
        let cfg = ConsolidationConfig::from_llm_config(&llm);
        assert_eq!(cfg.provider, crate::config::LlmProvider::Ollama);
        assert_eq!(cfg.ollama_url, "http://localhost:11434");
        assert_eq!(cfg.model, "codellama");
        assert_eq!(cfg.swap_guard, crate::config::SwapGuardMode::Adapt);
    }

    #[test]
    fn semantic_consolidation_dry_run_does_not_merge() {
        let graph = AtheneumGraph::open_in_memory().unwrap();

        for (name, desc) in [
            ("Lexical Concept Test One", "Desc A"),
            ("Lexical Concept Test On", "Desc B"),
        ] {
            let entity = sqlitegraph::GraphEntity {
                id: 0,
                kind: "Concept".to_string(),
                name: name.to_string(),
                file_path: None,
                data: serde_json::json!({ "description": desc }),
            };
            graph.inner.insert_entity(&entity).unwrap();
        }

        let config = ConsolidationConfig {
            similarity_threshold: 0.7,
            ollama_url: "http://invalid-url-to-trigger-offline-fallback.local".to_string(),
            dry_run: true,
            ..Default::default()
        };

        let report = graph.semantic_consolidation(&config).unwrap();
        assert!(report.dry_run);
        assert_eq!(report.merges_completed, 0);
        assert!(report.details.iter().any(|d| d.contains("[dry-run]")));
        // Nothing was superseded.
        let concepts = graph.entities_by_kind("Concept").unwrap();
        assert!(concepts
            .iter()
            .all(|c| c.data.get("superseded_at").is_none()));
    }

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
            ..Default::default()
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
        // discover_available_models() makes real network calls to
        // OLLAMA_HOST/LLAMACPP_HOST with no injection seam, so this test
        // previously only passed by accident (whenever the dev machine
        // happened to have nothing bound on the default Ollama/llama.cpp
        // ports). Point both at addresses guaranteed to refuse connection
        // so "no models loaded" is actually true, not just usually true.
        // SAFETY: no other test in this crate reads/writes these two vars
        // (verified via crate-wide grep), so no cross-test race.
        unsafe {
            std::env::set_var("OLLAMA_HOST", "http://127.0.0.1:1");
            std::env::set_var("LLAMACPP_HOST", "http://127.0.0.1:1");
        }

        let graph = AtheneumGraph::open_in_memory().unwrap();

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

        unsafe {
            std::env::remove_var("OLLAMA_HOST");
            std::env::remove_var("LLAMACPP_HOST");
        }
    }
}
