//! Atheneum: Agent Coordination Graph Database
//!
//! A graph-based memory system for tracking multi-agent workflows,
//! decisions, and their provenance.

pub mod graph;

pub use graph::{AtheneumGraph, EdgeType, EntityType};
