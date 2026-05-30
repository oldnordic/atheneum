use serde_json::Value;
use sqlitegraph::GraphEntity;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityType {
    Agent,
    Task,
    Event,
    Decision,
    ToolCall,
    FileChange,
    Verification,
    Knowledge,
    Discovery,
    Handoff,
    Session,
    Commit,
    TestRun,
    Benchmark,
    Release,
    EventLog,
}

impl EntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityType::Agent => "Agent",
            EntityType::Task => "Task",
            EntityType::Event => "Event",
            EntityType::Decision => "Decision",
            EntityType::ToolCall => "ToolCall",
            EntityType::FileChange => "FileChange",
            EntityType::Verification => "Verification",
            EntityType::Knowledge => "Knowledge",
            EntityType::Discovery => "Discovery",
            EntityType::Handoff => "Handoff",
            EntityType::Session => "Session",
            EntityType::Commit => "Commit",
            EntityType::TestRun => "TestRun",
            EntityType::Benchmark => "Benchmark",
            EntityType::Release => "Release",
            EntityType::EventLog => "EventLog",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeType {
    PerformedBy,
    AssignedTo,
    Called,
    Modified,
    VerifiedBy,
    DependsOn,
    CausedBy,
    Supersedes,
    Created,
    RelatedTo,
}

impl EdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeType::PerformedBy => "performed_by",
            EdgeType::AssignedTo => "assigned_to",
            EdgeType::Called => "called",
            EdgeType::Modified => "modified",
            EdgeType::VerifiedBy => "verified_by",
            EdgeType::DependsOn => "depends_on",
            EdgeType::CausedBy => "caused_by",
            EdgeType::Supersedes => "supersedes",
            EdgeType::Created => "created",
            EdgeType::RelatedTo => "related_to",
        }
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

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub score: f32,
    pub data: Value,
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
pub struct PromptParams {
    pub session_id: String,
    pub role: String,
    pub sequence: i64,
    pub input_hash: String,
    pub input_tokens: Option<i64>,
    pub output_hash: Option<String>,
    pub output_tokens: Option<i64>,
    pub latency_ms: Option<i64>,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ToolCallParams {
    pub session_id: String,
    pub tool_name: String,
    pub tool_version: Option<String>,
    pub input_hash: Option<String>,
    pub input_summary: Option<String>,
    pub output_hash: Option<String>,
    pub output_summary: Option<String>,
    pub exit_status: String,
    pub latency_ms: i64,
    pub input_tokens_est: Option<i64>,
    pub tool_category: String,
}

#[derive(Debug, Clone)]
pub struct FileWriteParams {
    pub session_id: String,
    pub file_path: String,
    pub file_id: Option<String>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub lines_added: i64,
    pub lines_deleted: i64,
    pub lines_changed: i64,
    pub write_type: String,
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
}
