use atheneum::graph::{
    AtheneumGraph, CommitParams, EdgeType, FileWriteParams, FixChainParams, PromptParams,
    RecordEventParams, RelationEndpoint, RelationHint, SessionParams, TestRunParams,
    ToolCallParams,
};
use serde_json::{json, Value};

fn entity_id_by_data(graph: &AtheneumGraph, kind: &str, field: &str, expected: &str) -> i64 {
    graph
        .entities_by_kind(kind)
        .expect("entities_by_kind")
        .into_iter()
        .find(|entity| entity.data.get(field).and_then(Value::as_str) == Some(expected))
        .map(|entity| entity.id)
        .unwrap_or_else(|| panic!("missing {kind} entity with {field}={expected}"))
}

fn entity_id_by_name(graph: &AtheneumGraph, kind: &str, name: &str) -> i64 {
    graph
        .entities_by_kind(kind)
        .expect("entities_by_kind")
        .into_iter()
        .find(|entity| entity.name == name)
        .map(|entity| entity.id)
        .unwrap_or_else(|| panic!("missing {kind} entity named {name}"))
}

fn assert_edge(graph: &AtheneumGraph, from_id: i64, to_id: i64, edge_type: EdgeType) {
    let found = graph
        .outgoing_edges(from_id)
        .expect("outgoing_edges")
        .into_iter()
        .any(|edge| edge.to_id == to_id && edge.edge_type == edge_type.as_str());
    assert!(
        found,
        "missing {} edge from {} to {}",
        edge_type.as_str(),
        from_id,
        to_id
    );
}

#[test]
fn test_record_session_links_project_and_parent() {
    let graph = AtheneumGraph::open_in_memory().expect("open");

    graph
        .record_session(SessionParams {
            session_id: "parent-session".into(),
            agent_name: "claude".into(),
            project: "forge".into(),
            tool: "claude-code".into(),
            trigger: "cli".into(),
            model: Some("model-a".into()),
            git_branch: Some("main".into()),
            git_head: Some("abc123".into()),
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record parent session");
    graph
        .record_session(SessionParams {
            session_id: "child-session".into(),
            agent_name: "claude".into(),
            project: "forge".into(),
            tool: "claude-code".into(),
            trigger: "subagent".into(),
            model: Some("model-a".into()),
            git_branch: Some("main".into()),
            git_head: Some("def456".into()),
            parent_session_id: Some("parent-session".into()),
            relations: vec![],
        })
        .expect("record child session");

    let parent_id = entity_id_by_data(&graph, "Session", "session_id", "parent-session");
    let child_id = entity_id_by_data(&graph, "Session", "session_id", "child-session");
    let project_id = entity_id_by_name(&graph, "Project", "forge");

    assert_edge(&graph, child_id, parent_id, EdgeType::DependsOn);
    assert_edge(&graph, parent_id, project_id, EdgeType::BelongsToProject);
    assert_edge(&graph, child_id, project_id, EdgeType::BelongsToProject);
}

#[test]
fn test_prompt_tool_and_file_evidence_create_graph_links() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .record_session(SessionParams {
            session_id: "session-1".into(),
            agent_name: "claude".into(),
            project: "atheneum".into(),
            tool: "claude-code".into(),
            trigger: "cli".into(),
            model: Some("model-b".into()),
            git_branch: None,
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record session");

    graph
        .record_evidence_prompt(PromptParams {
            session_id: "session-1".into(),
            role: "assistant".into(),
            sequence: 3,
            content_summary: Some("symbol found".into()),
            source: None,
            input_hash: "input-hash".into(),
            input_tokens: Some(11),
            output_hash: Some("output-hash".into()),
            output_tokens: Some(17),
            latency_ms: Some(30),
            model: Some("model-b".into()),
            cost_usd: Some(0.02),
            relations: vec![],
        })
        .expect("record prompt");
    graph
        .record_evidence_tool_call(ToolCallParams {
            session_id: "session-1".into(),
            tool_name: "magellan_find".into(),
            sequence: Some(1),
            source: None,
            tool_version: None,
            input_hash: Some("tool-input".into()),
            input_summary: Some("find EdgeType".into()),
            output_hash: Some("tool-output".into()),
            output_summary: Some("symbol found".into()),
            exit_status: "success".into(),
            latency_ms: 42,
            input_tokens_est: Some(5),
            tool_category: "analysis".into(),
            relations: vec![],
        })
        .expect("record tool call");
    graph
        .record_evidence_file_write(FileWriteParams {
            session_id: "session-1".into(),
            file_path: "src/graph/evidence.rs".into(),
            sequence: Some(1),
            file_id: Some("file-1".into()),
            before_hash: Some("before".into()),
            after_hash: Some("after".into()),
            lines_added: 10,
            lines_deleted: 2,
            lines_changed: 12,
            write_type: "patch".into(),
            relations: vec![],
        })
        .expect("record file write");

    let session_id = entity_id_by_data(&graph, "Session", "session_id", "session-1");
    let project_id = entity_id_by_name(&graph, "Project", "atheneum");
    let prompt_id = entity_id_by_data(&graph, "ReasoningLog", "input_hash", "input-hash");
    let tool_id = entity_id_by_data(&graph, "ToolCall", "tool_name", "magellan_find");
    let file_id = entity_id_by_data(&graph, "File", "file_id", "file-1");

    assert_edge(&graph, prompt_id, session_id, EdgeType::ObservedIn);
    assert_edge(&graph, tool_id, session_id, EdgeType::ObservedIn);
    assert_edge(&graph, session_id, tool_id, EdgeType::HandledByTool);
    assert_edge(&graph, file_id, session_id, EdgeType::ObservedIn);
    assert_edge(&graph, prompt_id, project_id, EdgeType::BelongsToProject);
    assert_edge(&graph, tool_id, project_id, EdgeType::BelongsToProject);
    assert_edge(&graph, file_id, project_id, EdgeType::BelongsToProject);
}

#[test]
fn test_commit_test_fix_chain_and_bench_ingest_relations() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .record_session(SessionParams {
            session_id: "session-2".into(),
            agent_name: "claude".into(),
            project: "forge-agent".into(),
            tool: "claude-code".into(),
            trigger: "cli".into(),
            model: Some("model-c".into()),
            git_branch: None,
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record session");

    graph
        .record_evidence_commit(CommitParams {
            session_id: "session-2".into(),
            commit_sha: "aaaaaaaa11111111".into(),
            parent_sha: None,
            message: "feat: add relation ingestion".into(),
            author: "feanor".into(),
            files_changed: 4,
            lines_inserted: 80,
            lines_deleted: 10,
            commit_type: "feature".into(),
            feature_tag: Some("relations".into()),
            relations: vec![],
        })
        .expect("record commit");
    graph
        .record_evidence_test_run(TestRunParams {
            session_id: "session-2".into(),
            test_name: "evidence_relation_tests".into(),
            test_suite: Some("integration".into()),
            test_command: Some("cargo test --test evidence_relation_tests".into()),
            result: "passed".into(),
            duration_ms: 1500,
            logs_summary: Some("all green".into()),
            commit_sha: Some("aaaaaaaa11111111".into()),
            relations: vec![],
        })
        .expect("record test run");
    graph
        .record_evidence_fix_chain(FixChainParams {
            session_id: "session-2".into(),
            bug_commit_sha: "bbbbbbbb22222222".into(),
            fix_commit_sha: "cccccccc33333333".into(),
            fix_type: "bugfix".into(),
            severity: "high".into(),
            cycles_to_fix: 2,
            time_to_fix_ms: 9000,
            relations: vec![],
        })
        .expect("record fix chain");
    graph
        .record_evidence_bench_run(
            "session-2".into(),
            "graph_navigation".into(),
            Some(10_000),
            Some(9_500),
            Some(12_000),
            true,
        )
        .expect("record bench run");

    let session_id = entity_id_by_data(&graph, "Session", "session_id", "session-2");
    let commit_id = entity_id_by_data(&graph, "Commit", "commit_sha", "aaaaaaaa11111111");
    let test_id = entity_id_by_name(&graph, "TestRun", "evidence_relation_tests:passed");
    let bug_commit_id = entity_id_by_data(&graph, "Commit", "commit_sha", "bbbbbbbb22222222");
    let fix_commit_id = entity_id_by_data(&graph, "Commit", "commit_sha", "cccccccc33333333");
    let failure_id = entity_id_by_name(&graph, "Failure", "bench:graph_navigation");

    assert_edge(&graph, commit_id, test_id, EdgeType::TestedBy);
    assert_edge(&graph, test_id, session_id, EdgeType::ObservedIn);
    assert_edge(&graph, bug_commit_id, fix_commit_id, EdgeType::FixedBy);
    assert_edge(&graph, fix_commit_id, session_id, EdgeType::ObservedIn);
    assert_edge(&graph, failure_id, session_id, EdgeType::RegressedBy);
    assert_edge(&graph, failure_id, session_id, EdgeType::ObservedIn);
}

#[test]
fn test_record_event_ingests_relation_hints_from_payload() {
    let graph = AtheneumGraph::open_in_memory().expect("open");
    graph
        .record_session(SessionParams {
            session_id: "session-3".into(),
            agent_name: "claude".into(),
            project: "atheneum".into(),
            tool: "claude-code".into(),
            trigger: "cli".into(),
            model: Some("model-d".into()),
            git_branch: None,
            git_head: None,
            parent_session_id: None,
            relations: vec![],
        })
        .expect("record session");

    graph
        .record_event(RecordEventParams {
            event_type: "hook_capture".into(),
            entity_id: "event-1".into(),
            session_id: "session-3".into(),
            payload: json!({
                "tool_name": "skill-loader",
                "relations": [
                    {
                        "from": {
                            "kind": "Skill",
                            "name": "grounded-coding"
                        },
                        "to": {
                            "kind": "ToolCall",
                            "name": "atheneum-sync"
                        },
                        "edge_type": "requires_skill",
                        "data": {
                            "source": "claude-hook"
                        }
                    },
                    {
                        "from": {
                            "kind": "Failure",
                            "name": "tool-timeout"
                        },
                        "to": {
                            "kind": "Failure",
                            "name": "graph-timeout"
                        },
                        "edge_type": "similar_failure",
                        "data": {
                            "confidence": 0.8
                        }
                    }
                ]
            }),
            relations: vec![RelationHint {
                from: RelationEndpoint {
                    kind: "ToolCall".into(),
                    name: "atheneum-sync".into(),
                    file_path: None,
                    data: json!({"tool_name": "atheneum-sync"}),
                },
                to: RelationEndpoint {
                    kind: "Project".into(),
                    name: "atheneum".into(),
                    file_path: None,
                    data: json!({"project_id": "atheneum"}),
                },
                edge_type: EdgeType::BelongsToProject,
                data: json!({"source": "explicit"}),
            }],
        })
        .expect("record event");

    let skill_id = entity_id_by_name(&graph, "Skill", "grounded-coding");
    let tool_id = entity_id_by_name(&graph, "ToolCall", "atheneum-sync");
    let timeout_id = entity_id_by_name(&graph, "Failure", "tool-timeout");
    let graph_timeout_id = entity_id_by_name(&graph, "Failure", "graph-timeout");
    let project_id = entity_id_by_name(&graph, "Project", "atheneum");

    assert_edge(&graph, skill_id, tool_id, EdgeType::RequiresSkill);
    assert_edge(
        &graph,
        timeout_id,
        graph_timeout_id,
        EdgeType::SimilarFailure,
    );
    assert_edge(&graph, tool_id, project_id, EdgeType::BelongsToProject);
}
