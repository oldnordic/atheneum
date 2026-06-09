use serde_json::Value;
use sqlitegraph::{GraphEdge, GraphEntity};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityType {
    Agent,
    Task,
    Event,
    ToolCall,
    Knowledge,
    Discovery,
    Handoff,
    Session,
    Commit,
    TestRun,
    EventLog,
    Memory,
    WikiPage,
    Concept,
}

impl EntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityType::Agent => "Agent",
            EntityType::Task => "Task",
            EntityType::Event => "Event",
            EntityType::ToolCall => "ToolCall",
            EntityType::Knowledge => "Knowledge",
            EntityType::Discovery => "Discovery",
            EntityType::Handoff => "Handoff",
            EntityType::Session => "Session",
            EntityType::Commit => "Commit",
            EntityType::TestRun => "TestRun",
            EntityType::EventLog => "EventLog",
            EntityType::Memory => "Memory",
            EntityType::WikiPage => "WikiPage",
            EntityType::Concept => "Concept",
        }
    }

    pub fn from_query_label(label: &str) -> Option<Self> {
        let normalized: String = label
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .flat_map(|ch| ch.to_lowercase())
            .collect();

        Some(match normalized.as_str() {
            "agent" | "agents" => EntityType::Agent,
            "task" | "tasks" => EntityType::Task,
            "event" | "events" => EntityType::Event,
            "toolcall" | "toolcalls" | "tool" | "tools" => EntityType::ToolCall,
            "knowledge" | "knowledges" => EntityType::Knowledge,
            "discovery" | "discoveries" => EntityType::Discovery,
            "handoff" | "handoffs" => EntityType::Handoff,
            "session" | "sessions" => EntityType::Session,
            "commit" | "commits" => EntityType::Commit,
            "testrun" | "testruns" | "test" | "tests" => EntityType::TestRun,
            "eventlog" | "eventlogs" | "log" | "logs" => EntityType::EventLog,
            "memory" | "memories" => EntityType::Memory,
            "wikipage" | "wikipages" | "wiki" | "page" | "pages" => EntityType::WikiPage,
            "concept" | "concepts" => EntityType::Concept,
            _ => return None,
        })
    }

    pub fn query_labels() -> &'static [&'static str] {
        &[
            "Agent",
            "Task",
            "Event",
            "ToolCall",
            "Knowledge",
            "Discovery",
            "Handoff",
            "Session",
            "Commit",
            "TestRun",
            "EventLog",
            "Memory",
            "WikiPage",
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    PerformedBy,
    AssignedTo,
    Called,
    Calls,
    Accessed,
    Modified,
    VerifiedBy,
    CausedBy,
    Created,
    RelatedTo,
    Mentions,
    Wikilink,
    Implements,
    DependsOn,
    TestedBy,
    FixedBy,
    RegressedBy,
    ObservedIn,
    BelongsToProject,
    SimilarFailure,
    RequiresSkill,
    HandledByTool,
    Explains,
    DerivedFrom,
    /// Dream: superseded entry points to the entry that replaced it.
    SupersededBy,
    /// Dream: merged entry points to the consolidation that absorbed it.
    ConsolidatedFrom,
}

impl EdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeType::PerformedBy => "performed_by",
            EdgeType::AssignedTo => "assigned_to",
            EdgeType::Called => "called",
            EdgeType::Calls => "calls",
            EdgeType::Accessed => "accessed",
            EdgeType::Modified => "modified",
            EdgeType::VerifiedBy => "verified_by",
            EdgeType::CausedBy => "caused_by",
            EdgeType::Created => "created",
            EdgeType::RelatedTo => "related_to",
            EdgeType::Mentions => "mentions",
            EdgeType::Wikilink => "wikilink",
            EdgeType::Implements => "implements",
            EdgeType::DependsOn => "depends_on",
            EdgeType::TestedBy => "tested_by",
            EdgeType::FixedBy => "fixed_by",
            EdgeType::RegressedBy => "regressed_by",
            EdgeType::ObservedIn => "observed_in",
            EdgeType::BelongsToProject => "belongs_to_project",
            EdgeType::SimilarFailure => "similar_failure",
            EdgeType::RequiresSkill => "requires_skill",
            EdgeType::HandledByTool => "handled_by_tool",
            EdgeType::Explains => "explains",
            EdgeType::DerivedFrom => "derived_from",
            EdgeType::SupersededBy => "superseded_by",
            EdgeType::ConsolidatedFrom => "consolidated_from",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Some(match label {
            "performed_by" => EdgeType::PerformedBy,
            "assigned_to" => EdgeType::AssignedTo,
            "called" => EdgeType::Called,
            "calls" => EdgeType::Calls,
            "accessed" => EdgeType::Accessed,
            "modified" => EdgeType::Modified,
            "verified_by" => EdgeType::VerifiedBy,
            "caused_by" => EdgeType::CausedBy,
            "created" => EdgeType::Created,
            "related_to" => EdgeType::RelatedTo,
            "mentions" => EdgeType::Mentions,
            "wikilink" => EdgeType::Wikilink,
            "implements" => EdgeType::Implements,
            "depends_on" => EdgeType::DependsOn,
            "tested_by" => EdgeType::TestedBy,
            "fixed_by" => EdgeType::FixedBy,
            "regressed_by" => EdgeType::RegressedBy,
            "observed_in" => EdgeType::ObservedIn,
            "belongs_to_project" => EdgeType::BelongsToProject,
            "similar_failure" => EdgeType::SimilarFailure,
            "requires_skill" => EdgeType::RequiresSkill,
            "handled_by_tool" => EdgeType::HandledByTool,
            "explains" => EdgeType::Explains,
            "derived_from" => EdgeType::DerivedFrom,
            "superseded_by" => EdgeType::SupersededBy,
            "consolidated_from" => EdgeType::ConsolidatedFrom,
            _ => return None,
        })
    }

    pub fn all() -> &'static [EdgeType] {
        &[
            EdgeType::PerformedBy,
            EdgeType::AssignedTo,
            EdgeType::Called,
            EdgeType::Calls,
            EdgeType::Accessed,
            EdgeType::Modified,
            EdgeType::VerifiedBy,
            EdgeType::CausedBy,
            EdgeType::Created,
            EdgeType::RelatedTo,
            EdgeType::Mentions,
            EdgeType::Wikilink,
            EdgeType::Implements,
            EdgeType::DependsOn,
            EdgeType::TestedBy,
            EdgeType::FixedBy,
            EdgeType::RegressedBy,
            EdgeType::ObservedIn,
            EdgeType::BelongsToProject,
            EdgeType::SimilarFailure,
            EdgeType::RequiresSkill,
            EdgeType::HandledByTool,
            EdgeType::Explains,
            EdgeType::DerivedFrom,
            EdgeType::SupersededBy,
            EdgeType::ConsolidatedFrom,
        ]
    }
}

#[derive(Error, Debug)]
pub enum AtheneumError {
    #[error("SQLite graph error: {0}")]
    GraphError(#[from] sqlitegraph::SqliteGraphError),

    #[error("Entity not found: {0}")]
    EntityNotFound(i64),

    #[error("Edge not found: {0}")]
    EdgeNotFound(i64),

    #[error("Invalid entity data: {0}")]
    InvalidData(String),

    #[error("Edge validation failed: {edge_type} from {from_kind} to {to_kind} violates ontology (domain={domain}, range={range})")]
    EdgeValidation {
        edge_type: String,
        from_kind: String,
        to_kind: String,
        domain: String,
        range: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OntologyClassInfo {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OntologyPropertyInfo {
    pub id: i64,
    pub name: String,
    pub domain_class: String,
    pub range_class: String,
    pub description: Option<String>,
}

pub const ONTOLOGY_CLASS_KIND: &str = "OntologyClass";

pub const ONTOLOGY_PROPERTY_KIND: &str = "OntologyProperty";

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub score: f32,
    pub data: Value,
}

/// Result of entity disambiguation via vector similarity.
///
/// When resolving a name to an existing graph entity, this struct captures
/// the best match (if any), its confidence score, and alternative candidates
/// that fell below the confidence threshold but might still be relevant.
#[derive(Debug, Clone)]
pub struct DisambiguationResult {
    /// The single best-matching entity above the confidence threshold, if any.
    pub resolved: Option<SearchResult>,
    /// All candidates ranked by score, including those below the threshold.
    pub candidates: Vec<SearchResult>,
    /// The minimum confidence score that was required for resolution.
    pub min_confidence: f32,
}

impl DisambiguationResult {
    /// Returns true if a single entity was resolved above the confidence threshold.
    pub fn is_resolved(&self) -> bool {
        self.resolved.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveryPreview {
    pub proposed_name: String,
    pub proposed_data: Value,
    pub content_hash: String,
    pub exact_matches: Vec<GraphEntity>,
    pub candidate_matches: Vec<SearchResult>,
    /// Vector-based disambiguation analysis using ATH-15 resolve.
    /// Present when the preview can compute similarity against existing entities.
    pub disambiguation: Option<DisambiguationResult>,
}

#[derive(Debug, Clone)]
pub struct MemoryPreview {
    pub proposed_key: String,
    pub proposed_data: Value,
    pub content_hash: String,
    pub exact_matches: Vec<GraphEntity>,
    pub candidate_matches: Vec<SearchResult>,
    /// Vector-based disambiguation analysis using ATH-15 resolve.
    pub disambiguation: Option<DisambiguationResult>,
}

#[derive(Debug, Clone)]
pub struct HandoffPreview {
    pub proposed_name: String,
    pub proposed_data: Value,
    pub content_hash: String,
    pub exact_matches: Vec<GraphEntity>,
    pub candidate_matches: Vec<SearchResult>,
    /// Vector-based disambiguation analysis using ATH-15 resolve.
    pub disambiguation: Option<DisambiguationResult>,
}

/// Classified intent of a navigation query.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum QueryIntent {
    /// Find entities matching a name/description.
    Search,
    /// Explore the neighborhood around specific entities.
    Navigate,
    /// Find paths between entities.
    Path,
    /// Ambiguous or unclear intent.
    Unknown,
}

impl QueryIntent {
    /// Classify a query string into a likely intent.
    pub fn classify(query: &str) -> Self {
        let lower = query.to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();

        // Path-finding patterns
        if words.contains(&"path")
            || words.contains(&"between")
            || words.contains(&"from") && words.contains(&"to")
        {
            return QueryIntent::Path;
        }

        // Navigation patterns
        if words.contains(&"neighbors")
            || words.contains(&"neighbours")
            || words.contains(&"connections")
            || words.contains(&"edges")
            || words.contains(&"links")
            || words.contains(&"around")
            || words.contains(&"explore")
        {
            return QueryIntent::Navigate;
        }

        // Search patterns (default for most queries)
        if words.contains(&"find")
            || words.contains(&"search")
            || words.contains(&"where")
            || words.contains(&"what")
            || words.contains(&"who")
            || words.contains(&"list")
            || words.contains(&"show")
        {
            return QueryIntent::Search;
        }

        QueryIntent::Unknown
    }
}

/// A resolved entity reference from a query term.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResolvedEntity {
    pub query_term: String,
    pub entity_id: Option<i64>,
    pub entity_name: Option<String>,
    pub confidence: f32,
    pub alternatives: Vec<SearchResult>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NavigateQueryPlan {
    pub original_query: String,
    pub normalized_query: String,
    pub intent: QueryIntent,
    pub k: usize,
    pub depth: u32,
    pub project_id: Option<String>,
    pub requested_kind: Option<String>,
    pub resolved_kind: Option<String>,
    pub kind_repaired: bool,
    pub resolved_entities: Vec<ResolvedEntity>,
    pub executable: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub args: Value,
    pub modified_targets: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct ActionTrace {
    pub agent_id: i64,
    pub reasoning_log_id: i64,
    pub tool_call_ids: Vec<i64>,
    pub modified_edge_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct ActionRecord {
    pub reasoning_log: GraphEntity,
    pub tool_calls: Vec<ToolCallTrace>,
}

#[derive(Debug, Clone)]
pub struct ToolCallTrace {
    pub tool_call: GraphEntity,
    pub modified: Vec<GraphEntity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementStatus {
    Unmet,
    Met,
}

impl RequirementStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RequirementStatus::Unmet => "UNMET",
            RequirementStatus::Met => "MET",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockerType {
    Dependency,
    Bug,
    InfoGap,
}

impl BlockerType {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlockerType::Dependency => "DEPENDENCY",
            BlockerType::Bug => "BUG",
            BlockerType::InfoGap => "INFO_GAP",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskDetail {
    pub task: GraphEntity,
    pub requirements: Vec<GraphEntity>,
    pub blockers: Vec<GraphEntity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedKanbanUpdate {
    pub task_id: i64,
    pub task_title: String,
    pub previous_status: super::planning::KanbanStatus,
    pub new_status: super::planning::KanbanStatus,
}

#[derive(Debug, Clone)]
pub struct SessionParams {
    pub session_id: String,
    pub agent_name: String,
    pub project: String,
    pub tool: String,
    pub trigger: String,
    pub model: Option<String>,
    pub git_branch: Option<String>,
    pub git_head: Option<String>,
    pub parent_session_id: Option<String>,
    pub relations: Vec<RelationHint>,
}

/// Compact session summary for history display and handover queries.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub project: String,
    pub git_branch: Option<String>,
    pub trigger: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub exit_status: Option<String>,
    pub tool_call_count: i64,
    pub file_write_count: i64,
    pub commit_count: i64,
    pub parent_session_id: Option<String>,
    pub last_tool: Option<String>,
    pub last_tool_summary: Option<String>,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct EndSessionParams {
    pub session_id: String,
    pub exit_status: String,
    pub prompt_count: i64,
    pub tool_call_count: i64,
    pub file_write_count: i64,
    pub commit_count: i64,
    pub test_run_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct SessionProgressParams {
    pub session_id: String,
    pub model: Option<String>,
    pub git_branch: Option<String>,
    pub prompt_count: i64,
    pub tool_call_count: i64,
    pub file_write_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct PromptParams {
    pub session_id: String,
    pub role: String,
    pub sequence: i64,
    pub content_summary: Option<String>,
    pub source: Option<String>,
    pub input_hash: String,
    pub input_tokens: Option<i64>,
    pub output_hash: Option<String>,
    pub output_tokens: Option<i64>,
    pub latency_ms: Option<i64>,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
    pub relations: Vec<RelationHint>,
}

#[derive(Debug, Clone)]
pub struct ToolCallParams {
    pub session_id: String,
    pub tool_name: String,
    pub sequence: Option<i64>,
    pub source: Option<String>,
    pub tool_version: Option<String>,
    pub input_hash: Option<String>,
    pub input_summary: Option<String>,
    pub output_hash: Option<String>,
    pub output_summary: Option<String>,
    pub exit_status: String,
    pub latency_ms: i64,
    pub input_tokens_est: Option<i64>,
    pub tool_category: String,
    pub relations: Vec<RelationHint>,
}

#[derive(Debug, Clone)]
pub struct FileWriteParams {
    pub session_id: String,
    pub file_path: String,
    pub sequence: Option<i64>,
    pub file_id: Option<String>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub lines_added: i64,
    pub lines_deleted: i64,
    pub lines_changed: i64,
    pub write_type: String,
    pub relations: Vec<RelationHint>,
}

#[derive(Debug, Clone)]
pub struct FileAccessParams {
    pub session_id: String,
    pub file_path: String,
    pub sequence: i64,
    pub access_type: String,
    pub tool_name: Option<String>,
    pub source: Option<String>,
    pub relations: Vec<RelationHint>,
}

#[derive(Debug, Clone)]
pub struct CommitParams {
    pub session_id: String,
    pub commit_sha: String,
    pub parent_sha: Option<String>,
    pub message: String,
    pub author: String,
    pub files_changed: i64,
    pub lines_inserted: i64,
    pub lines_deleted: i64,
    pub commit_type: String,
    pub feature_tag: Option<String>,
    pub relations: Vec<RelationHint>,
}

#[derive(Debug, Clone)]
pub struct TestRunParams {
    pub session_id: String,
    pub test_name: String,
    pub test_suite: Option<String>,
    pub test_command: Option<String>,
    pub result: String,
    pub duration_ms: i64,
    pub logs_summary: Option<String>,
    pub commit_sha: Option<String>,
    pub relations: Vec<RelationHint>,
}

#[derive(Debug, Clone)]
pub struct RecordEventParams {
    pub event_type: String,
    pub entity_id: String,
    pub session_id: String,
    pub payload: serde_json::Value,
    pub relations: Vec<RelationHint>,
}

#[derive(Debug, Clone)]
pub struct ClaudeTranscriptImportParams {
    pub transcript_path: std::path::PathBuf,
    pub session_id: Option<String>,
    pub project: Option<String>,
    pub agent_name: String,
    pub tool: String,
    pub trigger: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ClaudeTranscriptImportSummary {
    pub session_id: String,
    pub project: String,
    pub model: Option<String>,
    pub git_branch: Option<String>,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_create_tokens: i64,
    pub prompt_count: i64,
    pub tool_call_count: i64,
    pub file_access_count: i64,
    pub file_write_count: i64,
    pub compaction_count: i64,
    pub imported_prompts: i64,
    pub imported_tool_calls: i64,
    pub imported_file_accesses: i64,
    pub imported_file_writes: i64,
    pub imported_offset: u64,
}

/// One-hop neighborhood: outgoing edges + incoming edges for an entity.
#[derive(Debug, Clone)]
pub struct Neighbors {
    pub entity_id: i64,
    pub outgoing: Vec<GraphEdge>,
    pub incoming: Vec<GraphEdge>,
}

/// Subgraph extracted around an entry point by BFS traversal.
#[derive(Debug, Clone)]
pub struct SubgraphView {
    pub entry: GraphEntity,
    pub depth: u32,
    pub entities: Vec<GraphEntity>,
    pub edges: Vec<GraphEdge>,
}

/// Topological summary of the graph.
#[derive(Debug, Clone)]
pub struct GraphStats {
    pub total_entities: i64,
    pub total_edges: i64,
    pub entity_counts: Vec<(String, i64)>,
    pub edge_counts: Vec<(String, i64)>,
}

#[derive(Debug, Clone)]
pub struct FixChainParams {
    pub session_id: String,
    pub bug_commit_sha: String,
    pub fix_commit_sha: String,
    pub fix_type: String,
    pub severity: String,
    pub cycles_to_fix: i64,
    pub time_to_fix_ms: i64,
    pub relations: Vec<RelationHint>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RelationEndpoint {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub data: Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RelationHint {
    pub from: RelationEndpoint,
    pub to: RelationEndpoint,
    pub edge_type: EdgeType,
    #[serde(default)]
    pub data: Value,
}

/// Structured provenance metadata for graph edges.
///
/// Replaces the ad-hoc `{"provenance": {"method": "..."}}` pattern with a typed
/// struct that carries `method`, `actor`, `created_at`, `extraction_mode`, and
/// `source_text`. Serialized as the `"provenance"` key inside edge data JSON.
///
/// Backward-compatible: deserializes old format (`{"method": "..."}` without
/// the new fields) by providing defaults.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProvenanceData {
    pub method: String,
    #[serde(default = "default_actor")]
    pub actor: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub extraction_mode: Option<String>,
    #[serde(default)]
    pub source_text: Option<String>,
}

fn default_actor() -> String {
    "atheneum".to_string()
}

impl ProvenanceData {
    pub fn new(method: &str) -> Self {
        Self {
            method: method.to_string(),
            actor: default_actor(),
            created_at: Some(chrono::Utc::now().to_rfc3339()),
            extraction_mode: None,
            source_text: None,
        }
    }

    pub fn with_extraction_mode(mut self, mode: &str) -> Self {
        self.extraction_mode = Some(mode.to_string());
        self
    }

    pub fn with_source_text(mut self, text: &str) -> Self {
        self.source_text = Some(text.to_string());
        self
    }

    pub fn with_actor(mut self, actor: &str) -> Self {
        self.actor = actor.to_string();
        self
    }

    /// Convert to a serde_json Value for embedding in edge data.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| {
            serde_json::json!({
                "method": &self.method,
                "actor": &self.actor,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_data_serializes_method_and_actor() {
        let p = ProvenanceData::new("record_evidence_prompt");
        let val = p.to_value();
        assert_eq!(val["method"], "record_evidence_prompt");
        assert_eq!(val["actor"], "atheneum");
        assert!(val["created_at"].is_string());
    }

    #[test]
    fn provenance_data_roundtrips_through_json() {
        let p = ProvenanceData::new("store_discovery")
            .with_extraction_mode("wiki_ingest")
            .with_source_text("some prose text");
        let json = serde_json::to_value(&p).unwrap();
        let back: ProvenanceData = serde_json::from_value(json).unwrap();
        assert_eq!(back.method, "store_discovery");
        assert_eq!(back.extraction_mode.as_deref(), Some("wiki_ingest"));
        assert_eq!(back.source_text.as_deref(), Some("some prose text"));
    }

    #[test]
    fn provenance_data_deserializes_old_format() {
        // Old format: just method, no actor/created_at
        let old = serde_json::json!({"method": "record_evidence_prompt"});
        let p: ProvenanceData = serde_json::from_value(old).unwrap();
        assert_eq!(p.method, "record_evidence_prompt");
        assert_eq!(p.actor, "atheneum"); // default
        assert!(p.created_at.is_none()); // no default for backward compat
        assert!(p.extraction_mode.is_none());
        assert!(p.source_text.is_none());
    }

    #[test]
    fn provenance_data_old_format_with_actor() {
        // Old format with actor
        let old = serde_json::json!({"actor": "atheneum", "method": "insert_reasoning_log"});
        let p: ProvenanceData = serde_json::from_value(old).unwrap();
        assert_eq!(p.method, "insert_reasoning_log");
        assert_eq!(p.actor, "atheneum");
    }
}
