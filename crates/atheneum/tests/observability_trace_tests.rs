use atheneum::graph::{
    AtheneumGraph, EdgeType, FileAccessParams, ProvenanceData, SessionParams, ToolCallParams,
};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn query_session_by_id_returns_recent_summary() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("atheneum.db");
    let graph = AtheneumGraph::open(&db_path).expect("open graph");

    graph
        .record_session(SessionParams {
            session_id: "sess-1".to_string(),
            agent_name: "hermes".to_string(),
            project: "envoy".to_string(),
            tool: "hermes".to_string(),
            trigger: "cli".to_string(),
            model: Some("openai/gpt-test".to_string()),
            git_branch: Some("main".to_string()),
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record session");

    graph
        .record_evidence_tool_call(ToolCallParams {
            session_id: "sess-1".to_string(),
            tool_name: "magellan".to_string(),
            tool_version: None,
            sequence: Some(0),
            source: None,
            input_hash: None,
            input_summary: Some("status".to_string()),
            output_hash: None,
            output_summary: Some("ok".to_string()),
            exit_status: "ok".to_string(),
            latency_ms: 42,
            input_tokens_est: Some(12),
            tool_category: "grounded".to_string(),
            relations: vec![],
        })
        .expect("record tool call");

    let summary = graph
        .query_session_by_id("sess-1")
        .expect("query session")
        .expect("session exists");

    assert_eq!(summary.session_id, "sess-1");
    assert_eq!(summary.project, "envoy");
    assert_eq!(summary.last_tool.as_deref(), Some("magellan"));
    assert_eq!(summary.last_tool_summary.as_deref(), Some("status"));
}

#[test]
fn recent_discoveries_filters_by_project_and_agent() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("atheneum.db");
    let graph = AtheneumGraph::open(&db_path).expect("open graph");

    graph
        .store_discovery_in_project(
            "hermes",
            "Finding",
            "session-observability",
            Some("envoy"),
            json!({"summary": "first"}),
        )
        .expect("store discovery");
    graph
        .store_discovery_in_project(
            "codex",
            "Finding",
            "session-observability",
            Some("envoy"),
            json!({"summary": "second"}),
        )
        .expect("store discovery");
    graph
        .store_discovery_in_project(
            "hermes",
            "Finding",
            "session-observability",
            Some("atheneum"),
            json!({"summary": "third"}),
        )
        .expect("store discovery");

    let envoy_only = graph
        .recent_discoveries(Some("envoy"), None, None, None, 10)
        .expect("recent discoveries");
    assert_eq!(envoy_only.len(), 2);

    let hermes_envoy = graph
        .recent_discoveries(Some("envoy"), Some("hermes"), None, None, 10)
        .expect("recent discoveries");
    assert_eq!(hermes_envoy.len(), 1);
    assert_eq!(hermes_envoy[0].data["agent"].as_str(), Some("hermes"));
    assert_eq!(hermes_envoy[0].data["project_id"].as_str(), Some("envoy"));
}

/// `recent_discoveries` session filter: `json_extract(data,'$.session_id')`.
/// Backs the `discoveries-recent --session` flag used by the Phase-3 decision
/// backfill idempotency pre-scan.
#[test]
fn recent_discoveries_filters_by_session() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("atheneum.db");
    let graph = AtheneumGraph::open(&db_path).expect("open graph");

    // Two discoveries in sess_a, one in sess_b — all project envoy.
    graph
        .store_discovery_in_project(
            "claude",
            "Decision",
            "pick_a1",
            Some("envoy"),
            json!({"session_id": "sess_a", "sequence": 1}),
        )
        .expect("store a1");
    graph
        .store_discovery_in_project(
            "claude",
            "Decision",
            "pick_a2",
            Some("envoy"),
            json!({"session_id": "sess_a", "sequence": 2}),
        )
        .expect("store a2");
    graph
        .store_discovery_in_project(
            "claude",
            "Decision",
            "pick_b1",
            Some("envoy"),
            json!({"session_id": "sess_b", "sequence": 1}),
        )
        .expect("store b1");

    let sess_a = graph
        .recent_discoveries(None, None, Some("sess_a"), None, 100)
        .expect("recent sess_a");
    assert_eq!(sess_a.len(), 2, "only the two sess_a decisions");
    assert!(sess_a.iter().all(|d| d.data["session_id"] == "sess_a"));

    let sess_b = graph
        .recent_discoveries(None, None, Some("sess_b"), None, 100)
        .expect("recent sess_b");
    assert_eq!(sess_b.len(), 1);
    assert_eq!(sess_b[0].data["target"].as_str(), Some("pick_b1"));

    let none = graph
        .recent_discoveries(None, None, Some("sess_missing"), None, 100)
        .expect("recent missing");
    assert!(none.is_empty(), "unknown session -> empty, not error");
}

#[test]
fn recent_discoveries_filter_by_discovery_type() {
    // The `--type` / `discovery_type` filter narrows to one discovery_type and
    // combines with the session filter. Three Decisions across sess_a/sess_b
    // plus one Bug; `Decision` + no session = 3, `Decision` + sess_a = 2,
    // `Bug` = 1, an absent type = 0.
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("atheneum.db");
    let graph = AtheneumGraph::open(&db_path).expect("open graph");

    graph
        .store_discovery_in_project(
            "claude",
            "Decision",
            "pick_a1",
            Some("envoy"),
            json!({"session_id": "sess_a", "sequence": 1}),
        )
        .expect("store a1");
    graph
        .store_discovery_in_project(
            "claude",
            "Decision",
            "pick_a2",
            Some("envoy"),
            json!({"session_id": "sess_a", "sequence": 2}),
        )
        .expect("store a2");
    graph
        .store_discovery_in_project(
            "claude",
            "Decision",
            "pick_b1",
            Some("envoy"),
            json!({"session_id": "sess_b", "sequence": 1}),
        )
        .expect("store b1");
    graph
        .store_discovery_in_project(
            "claude",
            "Bug",
            "bug_x",
            Some("envoy"),
            json!({"session_id": "sess_b", "sequence": 2}),
        )
        .expect("store bug");

    let decisions = graph
        .recent_discoveries(None, None, None, Some("Decision"), 100)
        .expect("recent decisions");
    assert_eq!(decisions.len(), 3, "all three Decisions, no the Bug");

    let sess_a_decisions = graph
        .recent_discoveries(None, None, Some("sess_a"), Some("Decision"), 100)
        .expect("recent sess_a decisions");
    assert_eq!(sess_a_decisions.len(), 2, "type + session combine");

    let bugs = graph
        .recent_discoveries(None, None, None, Some("Bug"), 100)
        .expect("recent bugs");
    assert_eq!(bugs.len(), 1);

    let absent = graph
        .recent_discoveries(None, None, None, Some("Invariant"), 100)
        .expect("recent absent type");
    assert!(absent.is_empty(), "unknown type -> empty, not error");
}

#[test]
fn recent_handoffs_filter_by_agent_and_project() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("atheneum.db");
    let graph = AtheneumGraph::open(&db_path).expect("open graph");

    graph
        .store_handoff_in_project(
            "claude3",
            "claude4",
            Some("envoy"),
            json!({"task": "inspect sessions"}),
        )
        .expect("store handoff");
    graph
        .store_handoff_in_project(
            "codex",
            "claude4",
            Some("atheneum"),
            json!({"task": "inspect handoffs"}),
        )
        .expect("store handoff");

    let envoy_only = graph
        .recent_handoffs(Some("envoy"), None, 10)
        .expect("recent handoffs");
    assert_eq!(envoy_only.len(), 1);
    assert_eq!(envoy_only[0].data["project_id"].as_str(), Some("envoy"));

    let claude4_only = graph
        .recent_handoffs(None, Some("claude4"), 10)
        .expect("recent handoffs");
    assert_eq!(claude4_only.len(), 2);

    let envoy_claude4 = graph
        .recent_handoffs(Some("envoy"), Some("claude4"), 10)
        .expect("recent handoffs");
    assert_eq!(envoy_claude4.len(), 1);
    assert_eq!(envoy_claude4[0].data["to_agent"].as_str(), Some("claude4"));
}

#[test]
fn recent_sessions_filter_by_project_and_agent() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("atheneum.db");
    let graph = AtheneumGraph::open(&db_path).expect("open graph");

    graph
        .record_session(SessionParams {
            session_id: "sess-envoy-hermes".to_string(),
            agent_name: "hermes".to_string(),
            project: "envoy".to_string(),
            tool: "hermes".to_string(),
            trigger: "cli".to_string(),
            model: Some("local/hermes".to_string()),
            git_branch: Some("main".to_string()),
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record session");
    graph
        .record_session(SessionParams {
            session_id: "sess-atheneum-codex".to_string(),
            agent_name: "codex".to_string(),
            project: "atheneum".to_string(),
            tool: "codex".to_string(),
            trigger: "cli".to_string(),
            model: Some("local/codex".to_string()),
            git_branch: Some("main".to_string()),
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record session");

    let envoy_sessions = graph
        .query_sessions_recent(Some("envoy"), None, 10)
        .expect("recent sessions");
    assert_eq!(envoy_sessions.len(), 1);
    assert_eq!(envoy_sessions[0].session_id, "sess-envoy-hermes");

    let hermes_sessions = graph
        .query_sessions_recent(None, Some("hermes"), 10)
        .expect("recent sessions");
    assert_eq!(hermes_sessions.len(), 1);
    assert_eq!(hermes_sessions[0].project, "envoy");

    let atheneum_codex = graph
        .query_sessions_recent(Some("atheneum"), Some("codex"), 10)
        .expect("recent sessions");
    assert_eq!(atheneum_codex.len(), 1);
    assert_eq!(atheneum_codex[0].session_id, "sess-atheneum-codex");
}

/// `session-digest` composes a bounded bootstrap packet with activity
/// computed from `event_log` (not the zero `sessions` ledger columns),
/// ReasoningLog decisions linked via `observed_in`, project memory, and
/// discoveries attributed via the new `discoveries.session_id` column.
#[test]
fn session_digest_computes_activity_and_surfaces_decisions() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("atheneum.db");
    let graph = AtheneumGraph::open(&db_path).expect("open graph");

    // Session with no backfilled ledger counts — the digest must compute
    // activity from event_log instead of trusting the zero columns.
    let session_id = "digest-sess-1".to_string();
    graph
        .record_session(SessionParams {
            session_id: session_id.clone(),
            agent_name: "claude".to_string(),
            project: "atheneum".to_string(),
            tool: "claude-code".to_string(),
            trigger: "cli".to_string(),
            model: Some("claude-fable-5".to_string()),
            git_branch: Some("main".to_string()),
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record session");

    // Three tool calls recorded as events.
    for (i, tool) in ["Bash", "Edit", "Write"].iter().enumerate() {
        graph
            .record_evidence_tool_call(ToolCallParams {
                session_id: session_id.clone(),
                tool_name: tool.to_string(),
                tool_version: None,
                sequence: Some(i as i64),
                source: None,
                input_hash: None,
                input_summary: Some(format!("{} input", tool)),
                output_hash: None,
                output_summary: Some("ok".to_string()),
                exit_status: "ok".to_string(),
                latency_ms: 10,
                input_tokens_est: Some(5),
                tool_category: "grounded".to_string(),
                relations: vec![],
            })
            .expect("record tool call");
    }

    // Two file writes (transcript-sync style: file_access + access_type=write)
    // and two reads of other files.
    for (i, path) in [
        ("src/digest.rs", "write"),
        ("src/digest.rs", "write"),
        ("README.md", "read"),
        ("Cargo.toml", "read"),
    ]
    .iter()
    .enumerate()
    {
        graph
            .record_evidence_file_access(FileAccessParams {
                session_id: session_id.clone(),
                file_path: path.0.to_string(),
                sequence: i as i64,
                access_type: path.1.to_string(),
                tool_name: Some("claude-code".to_string()),
                source: Some("test".to_string()),
                relations: vec![],
            })
            .expect("record file access");
    }

    // A ReasoningLog decision linked to the session via observed_in — the
    // edge direction the digest joins on (reasoning -[observed_in]-> session).
    let reasoning_id = graph
        .insert_reasoning_log(
            "claude",
            "Use file_access.access_type=write, not file_write",
            None,
        )
        .expect("insert reasoning log");
    let session_entity_id = graph
        .entities_by_kind("Session")
        .expect("list session entities")
        .into_iter()
        .find(|e| e.data.get("session_id").and_then(|v| v.as_str()) == Some(&session_id))
        .expect("session entity exists")
        .id;
    graph
        .insert_edge(
            reasoning_id,
            session_entity_id,
            EdgeType::ObservedIn,
            json!({"provenance": ProvenanceData::new("test").to_value()}),
        )
        .expect("link reasoning to session");

    // Project memory + a discovery attributed to the session.
    graph
        .store_memory(
            "digest-format",
            "Digest is extractive plain-text, bounded to a token budget.",
            "project",
            0.9,
            Some("atheneum"),
            None,
        )
        .expect("store memory");
    graph
        .store_discovery(
            "claude",
            "Decision",
            "session-digest",
            json!({
                "session_id": session_id,
                "project_id": "atheneum",
                "summary": "compute activity from event_log",
            }),
        )
        .expect("store discovery");

    let text = graph
        .compose_digest(Some("atheneum"), 3, 500)
        .expect("compose digest");

    // Header + session tool (not the last tool call) + computed activity.
    assert!(
        text.contains("PRIOR SESSIONS (project: atheneum"),
        "header: {}",
        text
    );
    assert!(text.contains("tool=claude-code"), "session tool: {}", text);
    assert!(
        text.contains("3 tool calls"),
        "computed tool calls: {}",
        text
    );
    assert!(
        text.contains("2 file writes"),
        "computed file writes: {}",
        text
    );
    // Top files include the written file basename.
    assert!(text.contains("digest.rs"), "top files: {}", text);
    // ReasoningLog decision surfaced via the observed_in join.
    assert!(
        text.contains("file_access.access_type=write"),
        "reasoning decision: {}",
        text
    );
    // Project memory + session-attributed discovery.
    assert!(text.contains("PROJECT MEMORY"), "memory header: {}", text);
    assert!(
        text.contains("extractive plain-text"),
        "memory content: {}",
        text
    );
    assert!(
        text.contains("discovery: session-digest"),
        "discovery: {}",
        text
    );
    // Thread anchors present for navigate follow-up.
    assert!(text.contains("THREAD ANCHORS"), "anchors: {}", text);

    // Bounded: the body must fit within the 500-token budget (4 chars/token)
    // plus the truncation marker allowance used by the renderer.
    let budget = 500 * 4;
    assert!(
        text.len() <= budget + "[truncated]\n".len(),
        "digest not bounded ({} > {}): {}",
        text.len(),
        budget,
        text
    );

    // --json path produces a serializable value with the same computed counts.
    let value = graph
        .compose_digest_json(Some("atheneum"), 3)
        .expect("compose digest json");
    let sessions = value
        .get("sessions")
        .and_then(|v| v.as_array())
        .expect("sessions array");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["tool_calls"], json!(3));
    assert_eq!(sessions[0]["file_writes"], json!(2));
    assert_eq!(sessions[0]["tool"], json!("claude-code"));
    assert_eq!(sessions[0]["tool"].as_str().unwrap(), "claude-code");
}

/// The digest's `decisions` block surfaces `Decision` discoveries from every
/// capture source — filtered to `discovery_type = 'Decision'` and labeled with
/// the capture `source` — without dropping the existing `discoveries` block.
/// A non-Decision discovery (Bug) must NOT appear in the decisions block.
#[test]
fn session_digest_surfaces_structured_decisions_with_source() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("atheneum.db");
    let graph = AtheneumGraph::open(&db_path).expect("open graph");

    let session_id = "digest-dec-sess".to_string();
    graph
        .record_session(SessionParams {
            session_id: session_id.clone(),
            agent_name: "claude".to_string(),
            project: "atheneum".to_string(),
            tool: "claude-code".to_string(),
            trigger: "cli".to_string(),
            model: None,
            git_branch: Some("main".to_string()),
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record session");

    // A live-watcher AskUserQuestion decision with structured fields.
    graph
        .store_discovery(
            "claude",
            "Decision",
            "Index",
            json!({
                "session_id": session_id,
                "project_id": "atheneum",
                "source": "askuser",
                "chosen": "HNSW",
                "alternatives": ["HNSW", "brute"],
                "rationale": "fast ANN",
                "sequence": 1,
            }),
        )
        .expect("store askuser decision");
    // A Bug in the same session — must surface in `discoveries` but NOT in
    // `decisions` (the decisions block is Decision-only).
    graph
        .store_discovery(
            "claude",
            "Bug",
            "query_sessions",
            json!({
                "session_id": session_id,
                "project_id": "atheneum",
                "summary": "anonymous ? params",
            }),
        )
        .expect("store bug");

    let text = graph
        .compose_digest(Some("atheneum"), 3, 800)
        .expect("compose digest");

    // Decision block: chosen label + source tag + rationale.
    assert!(
        text.contains("decision: HNSW [askuser] — fast ANN"),
        "structured decision with source: {}",
        text
    );
    // The Bug appears in the discoveries block, not the decisions block.
    assert!(
        text.contains("discovery: query_sessions [Bug]"),
        "bug still in discoveries: {}",
        text
    );
    // No fabricated "decision:" line for the Bug.
    assert!(
        !text.contains("decision: query_sessions"),
        "bug must not leak into decisions block: {}",
        text
    );

    // --json: the decisions array carries source/chosen/rationale.
    let value = graph
        .compose_digest_json(Some("atheneum"), 3)
        .expect("compose digest json");
    let decisions = value["sessions"][0]["decisions"]
        .as_array()
        .expect("decisions array");
    assert_eq!(decisions.len(), 1, "only the Decision, not the Bug");
    assert_eq!(decisions[0]["target"], json!("Index"));
    assert_eq!(decisions[0]["source"], json!("askuser"));
    assert_eq!(decisions[0]["chosen"], json!("HNSW"));
    assert_eq!(decisions[0]["rationale"], json!("fast ANN"));
}

#[test]
fn thread_query_walks_discovery_chain_in_order() {
    let graph = AtheneumGraph::open_in_memory().expect("open graph");

    let session_id = "thread-sess-1".to_string();
    graph
        .record_session(SessionParams {
            session_id: session_id.clone(),
            agent_name: "claude".to_string(),
            project: "atheneum".to_string(),
            tool: "claude-code".to_string(),
            trigger: "cli".to_string(),
            model: None,
            git_branch: Some("main".to_string()),
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record session");

    let session_entity_id = graph
        .entities_by_kind("Session")
        .expect("list session entities")
        .into_iter()
        .find(|e| e.data.get("session_id").and_then(|v| v.as_str()) == Some(&session_id))
        .expect("session entity exists")
        .id;

    // Three chained discoveries in the same session. Each shares the token
    // "chainstep" so lexical_search surfaces all three as thread entry points.
    let mut discovery_ids: Vec<i64> = Vec::new();
    for (tag, summary) in [
        ("alpha", "first decision: adopt graph chain"),
        ("beta", "second decision: add led_to inverse"),
        ("gamma", "third decision: bound thread to tokens"),
    ] {
        let id = graph
            .store_discovery(
                "claude",
                "Decision",
                &format!("chainstep {}", tag),
                json!({
                    "session_id": session_id,
                    "project_id": "atheneum",
                    "summary": summary,
                }),
            )
            .expect("store discovery");
        discovery_ids.push(id);
    }
    let (d1, d2, d3) = (discovery_ids[0], discovery_ids[1], discovery_ids[2]);

    // Every discovery got an observed_in -> Session edge from store_discovery.
    for &d in &[d1, d2, d3] {
        let has_obs = graph
            .outgoing_edges(d)
            .expect("out edges")
            .iter()
            .any(|e| e.edge_type == "observed_in" && e.to_id == session_entity_id);
        assert!(has_obs, "discovery {} missing observed_in -> Session", d);
    }

    // caused_by chain: d2 -> d1, d3 -> d2 (each links to the prior same-session
    // decision by entity id = insert/chronological order).
    let d2_causes: Vec<i64> = graph
        .outgoing_edges(d2)
        .expect("out d2")
        .iter()
        .filter(|e| e.edge_type == "caused_by")
        .map(|e| e.to_id)
        .collect();
    assert!(
        d2_causes.contains(&d1),
        "d2 should caused_by d1, got {:?}",
        d2_causes
    );
    let d3_causes: Vec<i64> = graph
        .outgoing_edges(d3)
        .expect("out d3")
        .iter()
        .filter(|e| e.edge_type == "caused_by")
        .map(|e| e.to_id)
        .collect();
    assert!(
        d3_causes.contains(&d2),
        "d3 should caused_by d2, got {:?}",
        d3_causes
    );

    // led_to inverse: d1 -> d2, d2 -> d3.
    let d1_led: Vec<i64> = graph
        .outgoing_edges(d1)
        .expect("out d1")
        .iter()
        .filter(|e| e.edge_type == "led_to")
        .map(|e| e.to_id)
        .collect();
    assert!(
        d1_led.contains(&d2),
        "d1 should led_to d2, got {:?}",
        d1_led
    );
    let d2_led: Vec<i64> = graph
        .outgoing_edges(d2)
        .expect("out d2")
        .iter()
        .filter(|e| e.edge_type == "led_to")
        .map(|e| e.to_id)
        .collect();
    assert!(
        d2_led.contains(&d3),
        "d2 should led_to d3, got {:?}",
        d2_led
    );

    // d1 is a thread root: no caused_by edge.
    let d1_causes: Vec<i64> = graph
        .outgoing_edges(d1)
        .expect("out d1")
        .iter()
        .filter(|e| e.edge_type == "caused_by")
        .map(|e| e.to_id)
        .collect();
    assert!(
        d1_causes.is_empty(),
        "d1 should be a thread root, got caused_by {:?}",
        d1_causes
    );

    // thread_query returns the chain as subgraph views. truncate_subgraph
    // stores the entry separately from `entities`, so the full chain for a
    // view is {entry} ∪ {entities}.
    let views = graph
        .thread_query("chainstep", 3, 3, Some("atheneum"), 1500)
        .expect("thread query");
    assert!(!views.is_empty(), "thread query returned no views");

    let view_entity_sets: Vec<Vec<i64>> = views
        .iter()
        .map(|v| {
            let mut ids: Vec<i64> = std::iter::once(v.entry.id)
                .chain(v.entities.iter().map(|e| e.id))
                .collect();
            ids.sort();
            ids
        })
        .collect();
    let any_full_chain = view_entity_sets.iter().any(|ids| ids == &[d1, d2, d3]);
    assert!(
        any_full_chain,
        "no single view contained the full d1/d2/d3 chain; views: {:?}",
        view_entity_sets
    );

    // The view anchored on d3 walks caused_by backward to d1 and d2; ordering
    // the full entity set (entry + entities) by id yields [d1, d2, d3].
    let d3_view = views
        .iter()
        .find(|v| v.entry.id == d3)
        .expect("a view anchored on d3");
    let mut ordered: Vec<i64> = std::iter::once(d3_view.entry.id)
        .chain(d3_view.entities.iter().map(|e| e.id))
        .collect();
    ordered.sort();
    assert_eq!(ordered, vec![d1, d2, d3], "d3-view chain order by id");
}

#[test]
fn decision_exists_chosen_keys_on_session_target_source_chosen() {
    // Phase 5 dedup guard for the skill / manual `/decision` layer. Unlike
    // `decision_exists` (sequence-keyed, for the transcript watcher), this
    // keys on `(session_id, target, source, chosen)` because a re-fired skill
    // decision has no stable sequence. A repeat of the exact same quadruple
    // is a duplicate; any field differing is a distinct decision. Cross-layer
    // doubles (different `source`) are intentionally NOT collapsed here.
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("atheneum.db");
    let graph = AtheneumGraph::open(&db_path).expect("open graph");

    graph
        .store_discovery(
            "claude",
            "Decision",
            "storage-engine",
            json!({
                "session_id": "sess-skill",
                "source": "skill",
                "chosen": "csr-adjacency",
                "alternatives": ["btree", "skip-list"],
                "rationale": "scan-heavy read path favors CSR",
            }),
        )
        .expect("store skill decision");

    // Exact repeat → duplicate.
    assert!(
        graph
            .decision_exists_chosen("sess-skill", "storage-engine", "skill", "csr-adjacency")
            .expect("exists check"),
        "exact (session,target,source,chosen) must be a duplicate"
    );

    // Same decision captured by a different layer (source=askuser) is NOT
    // collapsed — the plan's documented cross-layer tradeoff.
    assert!(
        !graph
            .decision_exists_chosen("sess-skill", "storage-engine", "askuser", "csr-adjacency")
            .expect("exists check askuser"),
        "different source is a different layer, not a duplicate"
    );

    // Different chosen option for the same target → distinct decision.
    assert!(
        !graph
            .decision_exists_chosen("sess-skill", "storage-engine", "skill", "btree")
            .expect("exists check btree"),
        "a different chosen is a distinct decision, not a duplicate"
    );

    // Different target → distinct decision.
    assert!(
        !graph
            .decision_exists_chosen("sess-skill", "auth-strategy", "skill", "csr-adjacency")
            .expect("exists check other target"),
        "a different target is a distinct decision"
    );

    // Different session → distinct decision (session-scoped).
    assert!(
        !graph
            .decision_exists_chosen("sess-other", "storage-engine", "skill", "csr-adjacency")
            .expect("exists check other session"),
        "a different session is a distinct decision"
    );
}
