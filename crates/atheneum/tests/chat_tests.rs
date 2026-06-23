//! Phase 2 — `atheneum chat` navigation surface tests.
//!
//! Builds a synthetic session with known user/assistant/thinking/tool turns,
//! plus decision rows, then exercises [`AtheneumGraph::query_chat`] across
//! direction, token cap, `--role` filter, `--search` (FTS), pagination,
//! `--only-decisions`, and `--walk` (caused_by / led_to chain).

use atheneum::graph::{
    AtheneumGraph, ChatDirection, ChatQuery, PromptParams, SessionParams, ToolCallParams,
};
use serde_json::json;

fn session(id: &str) -> SessionParams {
    SessionParams {
        session_id: id.to_string(),
        agent_name: "agent_chat".to_string(),
        project: "atheneum".to_string(),
        tool: "claude-code".to_string(),
        trigger: "test".to_string(),
        model: Some("test-model".to_string()),
        git_branch: Some("main".to_string()),
        git_head: Some("deadbeef".to_string()),
        parent_session_id: None,
        relations: vec![],
    }
}

fn prompt(session: &str, sequence: i64, role: &str, content: &str) -> PromptParams {
    PromptParams {
        session_id: session.to_string(),
        role: role.to_string(),
        sequence,
        content_summary: Some(content.to_string()),
        source: Some("test".to_string()),
        input_hash: format!("in-{session}-{sequence}"),
        input_tokens: Some(10),
        output_hash: Some(format!("out-{session}-{sequence}")),
        output_tokens: Some(20),
        latency_ms: Some(5),
        model: Some("test-model".to_string()),
        cost_usd: Some(0.001),
        relations: vec![],
    }
}

fn tool(session: &str, sequence: i64, name: &str, category: &str) -> ToolCallParams {
    ToolCallParams {
        session_id: session.to_string(),
        tool_name: name.to_string(),
        sequence: Some(sequence),
        source: Some("test".to_string()),
        tool_version: Some("0.1".to_string()),
        input_hash: Some("tin".to_string()),
        input_summary: Some("tin".to_string()),
        output_hash: Some("tout".to_string()),
        output_summary: Some("tout".to_string()),
        exit_status: "success".to_string(),
        latency_ms: 7,
        input_tokens_est: Some(30),
        tool_category: category.to_string(),
        relations: vec![],
    }
}

fn chat_query(session: &str) -> ChatQuery {
    ChatQuery {
        session_id: session.to_string(),
        tokens: 500,
        direction: ChatDirection::Recent,
        kinds: vec![],
        role: None,
        search: None,
        only_decisions: false,
        offset: 0,
        limit: None,
        walk: false,
    }
}

/// Seed the canonical session with four turns (user / assistant / thinking /
/// tool) across sequences 1..=4. Returns the session id.
fn seed_canonical(graph: &AtheneumGraph) -> &'static str {
    let sid = "sess_chat";
    graph.record_session(session(sid)).expect("session");
    graph
        .record_evidence_prompt(prompt(sid, 1, "user", "user asks about vector search"))
        .expect("p1");
    graph
        .record_evidence_prompt(prompt(
            sid,
            2,
            "assistant",
            "assistant chose HNSW for indexing",
        ))
        .expect("p2");
    graph
        .record_evidence_prompt(prompt(
            sid,
            3,
            "thinking",
            "thinking compared brute force vs HNSW",
        ))
        .expect("p3");
    graph
        .record_evidence_tool_call(tool(sid, 4, "Write", "edit"))
        .expect("tc");
    sid
}

#[test]
fn direction_recent_vs_chrono_same_set_reversed_order() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    seed_canonical(&graph);

    let recent = graph
        .query_chat({
            let mut q = chat_query("sess_chat");
            q.direction = ChatDirection::Recent;
            q
        })
        .expect("recent");
    let chrono = graph
        .query_chat({
            let mut q = chat_query("sess_chat");
            q.direction = ChatDirection::Chronological;
            q
        })
        .expect("chrono");

    let recent_seqs: Vec<i64> = recent.turns.iter().filter_map(|t| t.sequence).collect();
    let chrono_seqs: Vec<i64> = chrono.turns.iter().filter_map(|t| t.sequence).collect();
    assert_eq!(recent_seqs, vec![4, 3, 2, 1], "recent = DESC");
    assert_eq!(chrono_seqs, vec![1, 2, 3, 4], "chrono = ASC");
    // Same row set (as a sorted set), just reversed.
    let mut a = recent_seqs.clone();
    let mut b = chrono_seqs.clone();
    a.sort();
    b.sort();
    assert_eq!(a, b);
}

#[test]
fn token_cap_emits_first_row_and_flags_more() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    seed_canonical(&graph);

    let report = graph
        .query_chat({
            let mut q = chat_query("sess_chat");
            q.direction = ChatDirection::Chronological; // first row = seq 1
            q.tokens = 1; // tiny budget: only the first row is emitted
            q
        })
        .expect("cap");

    assert_eq!(report.turns.len(), 1, "always emit at least the first row");
    assert_eq!(report.turns[0].sequence, Some(1));
    assert!(report.has_more, "budget cutoff must set has_more");
    assert!(report.token_total <= report.token_budget || report.turns.len() == 1);
}

#[test]
fn role_filter_returns_only_matching_role() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    seed_canonical(&graph);

    let report = graph
        .query_chat({
            let mut q = chat_query("sess_chat");
            q.role = Some("assistant".to_string());
            q
        })
        .expect("role");

    assert_eq!(report.turns.len(), 1, "only the assistant turn matches");
    assert_eq!(report.turns[0].role.as_deref(), Some("assistant"));
    assert_eq!(report.turns[0].sequence, Some(2));
    // ToolCall has NULL role and is excluded.
    assert!(report.turns.iter().all(|t| t.kind == "ReasoningLog"));
}

#[test]
fn search_fts_matches_only_hits_and_unknown_term_is_empty_not_error() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    seed_canonical(&graph);

    let hit = graph
        .query_chat({
            let mut q = chat_query("sess_chat");
            q.search = Some("HNSW".to_string());
            q
        })
        .expect("hit");
    // Both seq 2 ("chose HNSW") and seq 3 ("vs HNSW") mention HNSW; FTS returns
    // both, recent-first (DESC) => [3, 2].
    let matched_seqs: Vec<i64> = hit.turns.iter().filter_map(|t| t.sequence).collect();
    assert_eq!(
        matched_seqs,
        vec![3, 2],
        "FTS returns both HNSW turns, recent-first"
    );
    assert!(hit.turns.iter().all(|t| t.kind == "ReasoningLog"));
    assert!(!hit.has_more);

    let miss = graph
        .query_chat({
            let mut q = chat_query("sess_chat");
            q.search = Some("zzqqxx_no_such_token".to_string());
            q
        })
        .expect("miss");
    assert!(miss.turns.is_empty(), "no FTS match -> empty, not an error");
}

#[test]
fn pagination_slice_and_has_more() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    seed_canonical(&graph);

    let page = graph
        .query_chat({
            let mut q = chat_query("sess_chat");
            q.direction = ChatDirection::Chronological; // [1,2,3,4]
            q.offset = 1;
            q.limit = Some(1); // fetch 2, drop 1 -> [seq 2], has_more true
            q
        })
        .expect("page");
    assert_eq!(page.turns.len(), 1);
    assert_eq!(page.turns[0].sequence, Some(2));
    assert!(page.has_more);
    assert_eq!(page.offset, 1);
}

#[test]
fn only_decisions_returns_session_decisions_only() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph.record_session(session("sess_chat")).expect("session");
    graph
        .record_evidence_prompt(prompt("sess_chat", 1, "assistant", "decided"))
        .expect("p");

    // A Decision for sess_chat (the one we want).
    graph
        .store_discovery_in_project(
            "agent_chat",
            "Decision",
            "use_hnsw",
            Some("atheneum"),
            json!({"session_id": "sess_chat", "why": "HNSW is faster"}),
        )
        .expect("store decision");
    // A Bug for sess_chat (wrong type — must be excluded).
    graph
        .store_discovery_in_project(
            "agent_chat",
            "Bug",
            "off_by_one",
            Some("atheneum"),
            json!({"session_id": "sess_chat"}),
        )
        .expect("store bug");
    // A Decision for a different session (must be excluded).
    graph
        .record_session(session("sess_other"))
        .expect("other session");
    graph
        .store_discovery_in_project(
            "agent_chat",
            "Decision",
            "use_brute_force",
            Some("atheneum"),
            json!({"session_id": "sess_other"}),
        )
        .expect("store other decision");

    let report = graph
        .query_chat({
            let mut q = chat_query("sess_chat");
            q.only_decisions = true;
            q
        })
        .expect("decisions");

    assert_eq!(report.decisions.len(), 1, "only the sess_chat Decision");
    assert_eq!(report.decisions[0].target, "use_hnsw");
    assert!(report.turns.is_empty());
}

#[test]
fn walk_attaches_caused_by_led_to_chain_to_turns() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let sid = "sess_walk";
    graph.record_session(session(sid)).expect("session");
    graph
        .record_evidence_prompt(prompt(sid, 1, "assistant", "first turn"))
        .expect("p1");
    graph
        .record_evidence_prompt(prompt(sid, 2, "assistant", "second turn"))
        .expect("p2");
    graph
        .record_evidence_prompt(prompt(sid, 3, "assistant", "third turn chosen HNSW"))
        .expect("p3");
    // Store a Decision for the same session AFTER the reasoning logs so the
    // decision's caused_by edge points at the most-recent prior reasoning log,
    // and led_to runs the other way (reasoning log -> decision).
    graph
        .store_discovery_in_project(
            "agent_chat",
            "Decision",
            "pick_hnsw",
            Some("atheneum"),
            json!({"session_id": sid, "why": "faster"}),
        )
        .expect("store decision");

    let report = graph
        .query_chat({
            let mut q = chat_query(sid);
            q.walk = true;
            q
        })
        .expect("walk");

    // At least one emitted turn must have followed a led_to edge to the
    // Discovery entity (the only caused_by/led_to link in this graph).
    let any_chain_has_discovery = report
        .turns
        .iter()
        .any(|t| t.chain.iter().any(|n| n.kind == "Discovery"));
    assert!(
        any_chain_has_discovery,
        "expected at least one turn with a Discovery chain node; got {:?}",
        report
            .turns
            .iter()
            .map(|t| (&t.role, &t.chain))
            .collect::<Vec<_>>()
    );
    // All chain nodes reached via caused_by/led_to only.
    assert!(report
        .turns
        .iter()
        .flat_map(|t| t.chain.iter())
        .all(|n| n.via == "caused_by" || n.via == "led_to"));
}

#[test]
fn walk_with_no_chain_edges_is_empty_not_error() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    let sid = "sess_walk_empty";
    graph.record_session(session(sid)).expect("session");
    graph
        .record_evidence_prompt(prompt(sid, 1, "assistant", "lonely turn"))
        .expect("p1");
    graph
        .record_evidence_prompt(prompt(sid, 2, "assistant", "another turn"))
        .expect("p2");

    let report = graph
        .query_chat({
            let mut q = chat_query(sid);
            q.walk = true;
            q
        })
        .expect("walk empty");

    assert!(
        report.turns.iter().all(|t| t.chain.is_empty()),
        "no decision edges -> empty chains"
    );
    assert!(!report.has_more);
}
