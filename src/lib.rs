//! Atheneum: Agent Coordination Graph Database
//!
//! A graph-based memory system for tracking multi-agent workflows,
//! decisions, and their provenance.

pub mod graph;

pub use graph::{AtheneumGraph, EdgeType, EntityType};

// Re-export the sqlitegraph types so consumers can name them as
// `atheneum::GraphEntity` and pick up exactly the sqlitegraph version
// atheneum itself depends on. This avoids the dual-major-version trap
// when downstream crates also pull in an older sqlitegraph directly.
pub use sqlitegraph::{GraphEdge, GraphEntity};
