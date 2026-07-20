use std::io::{self, Write};
use std::path::PathBuf;

use atheneum::{
    AtheneumGraph, ClaudeTranscriptImportParams, Config, CrossRouter, EdgeType, GraphEdge,
    GraphEntity, MetaRouter, SearchResult, WikiPage,
};
use serde_json::json;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    if let Err(err) = run() {
        if is_broken_pipe(&err) {
            std::process::exit(0);
        }
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    // nosemgrep: rust.lang.security.args.args
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage()?;
        return Ok(());
    }

    match args[1].as_str() {
        "--version" | "-v" => {
            stdoutln(format_args!("atheneum v{}", VERSION))?;
        }
        "help" | "--help" | "-h" => {
            print_usage()?;
        }
        "init" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum init <db-path>");
                std::process::exit(1);
            }
            let path = PathBuf::from(positional(&args, 2, "db-path")?);
            stdoutln(format_args!(
                "Initializing Atheneum graph at: {}",
                path.display()
            ))?;
            let graph = AtheneumGraph::open(&path)?;
            stdoutln(format_args!("Graph initialized successfully"))?;
            stdoutln(format_args!(
                "   Health: {}",
                if graph.is_healthy() { "OK" } else { "BAD" }
            ))?;
        }
        "sync-wiki" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum sync-wiki <db-path> <wiki-dir> [project-id]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let wiki_dir = PathBuf::from(positional(&args, 3, "wiki-dir")?);
            let project_id = optional_positional(&args, 4, "project-id")?;

            if !wiki_dir.is_dir() {
                eprintln!("Not a directory: {}", wiki_dir.display());
                std::process::exit(1);
            }

            let graph = AtheneumGraph::open(&db_path)?;
            let ids = graph.sync_wiki_directory(&wiki_dir, project_id)?;
            stdoutln(format_args!(
                "Synced {} wiki pages from {}",
                ids.len(),
                wiki_dir.display()
            ))?;
            for id in ids {
                stdoutln(format_args!("  -> graph entity id: {}", id))?;
            }
        }
        "sync-journal" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum sync-journal <db-path> <journal-dir> [project-id]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let journal_dir = PathBuf::from(positional(&args, 3, "journal-dir")?);
            let project_id = optional_positional(&args, 4, "project-id")?;

            if !journal_dir.is_dir() {
                eprintln!("Not a directory: {}", journal_dir.display());
                std::process::exit(1);
            }

            let graph = AtheneumGraph::open(&db_path)?;
            let ids = graph.sync_journal_directory(&journal_dir, project_id)?;
            stdoutln(format_args!(
                "Synced {} journal sections from {}",
                ids.len(),
                journal_dir.display()
            ))?;
            for id in ids {
                stdoutln(format_args!("  -> graph entity id: {}", id))?;
            }
        }
        "sync-logseq" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum sync-logseq <db-path> <wiki-root> [project-id]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let wiki_root = PathBuf::from(positional(&args, 3, "wiki-root")?);
            let project_id = optional_positional(&args, 4, "project-id")?;

            if !wiki_root.is_dir() {
                eprintln!("Not a directory: {}", wiki_root.display());
                std::process::exit(1);
            }

            let graph = AtheneumGraph::open(&db_path)?;
            let (page_ids, journal_ids) = sync_logseq(&graph, &wiki_root, project_id)?;
            stdoutln(format_args!(
                "Synced {} wiki pages and {} journal sections from {}",
                page_ids.len(),
                journal_ids.len(),
                wiki_root.display()
            ))?;
            for id in page_ids {
                stdoutln(format_args!("  -> wiki graph entity id: {}", id))?;
            }
            for id in journal_ids {
                stdoutln(format_args!("  -> journal graph entity id: {}", id))?;
            }
        }
        "sync-claude-transcript" => {
            if args.len() < 4 {
                eprintln!(
                    "Usage: atheneum sync-claude-transcript <db-path> <transcript-path> [project-id] [agent-name]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let transcript_path = PathBuf::from(positional(&args, 3, "transcript-path")?);
            let project = optional_positional(&args, 4, "project")?.map(|s| s.to_string());
            let agent_name = optional_positional(&args, 5, "agent-name")?
                .unwrap_or("claude")
                .to_string();
            let graph = AtheneumGraph::open(&db_path)?;
            let summary = graph.sync_claude_transcript(ClaudeTranscriptImportParams {
                transcript_path,
                session_id: None,
                project,
                agent_name,
                tool: "claude-code".into(),
                trigger: "transcript-import".into(),
            })?;
            print_json(json!({
                "session_id": summary.session_id,
                "project": summary.project,
                "model": summary.model,
                "git_branch": summary.git_branch,
                "total_input_tokens": summary.total_input_tokens,
                "total_output_tokens": summary.total_output_tokens,
                "total_cache_read_tokens": summary.total_cache_read_tokens,
                "total_cache_create_tokens": summary.total_cache_create_tokens,
                "prompt_count": summary.prompt_count,
                "tool_call_count": summary.tool_call_count,
                "file_access_count": summary.file_access_count,
                "file_write_count": summary.file_write_count,
                "compaction_count": summary.compaction_count,
                "imported_prompts": summary.imported_prompts,
                "imported_tool_calls": summary.imported_tool_calls,
                "imported_file_accesses": summary.imported_file_accesses,
                "imported_file_writes": summary.imported_file_writes,
                "imported_offset": summary.imported_offset,
            }))?;
        }
        "query-wiki" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum query-wiki <db-path> <path>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let path = positional(&args, 3, "path")?;

            let graph = AtheneumGraph::open(&db_path)?;
            match graph.get_wiki_page(path)? {
                Some(page) => {
                    stdoutln(format_args!("WikiPage: {}", page.path))?;
                    if let Some(title) = &page.title {
                        stdoutln(format_args!("  Title: {}", title))?;
                    }
                    stdoutln(format_args!("  Content hash: {:?}", page.content_hash))?;
                    stdoutln(format_args!("  Wikilinks: {:?}", page.wikilinks))?;
                    stdoutln(format_args!("  Project: {:?}", page.project_id))?;
                    stdoutln(format_args!("  Created: {}", page.created_at))?;
                    stdoutln(format_args!("  Updated: {:?}", page.updated_at))?;
                    stdoutln(format_args!("  Body (first 500 chars):"))?;
                    let preview: String = page.body.chars().take(500).collect();
                    stdoutln(format_args!("{}", preview))?;
                }
                None => {
                    stdoutln(format_args!("No wiki page found at path: {}", path))?;
                }
            }
        }
        "wiki-search" => {
            if args.len() < 4 {
                eprintln!(
                    "Usage: atheneum wiki-search <db-path> <query> [--project P] [--limit N]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let query = positional(&args, 3, "query")?;
            let opts = parse_options(&args[4..])?;
            let limit = parse_i64_option(opts.limit.as_deref(), "limit")?.unwrap_or(10) as usize;
            let graph = AtheneumGraph::open(&db_path)?;
            let results = graph.search_wiki_pages(query, opts.project.as_deref(), 0, limit)?;
            if opts.json {
                print_json(json!({
                    "query": query,
                    "project": opts.project,
                    "count": results.len(),
                    "results": results.iter().map(|r| json!({
                        "id": r.id,
                        "path": r.path,
                        "title": r.title,
                        "excerpt": r.excerpt,
                        "score": r.score,
                        "project_id": r.project_id,
                    })).collect::<Vec<_>>(),
                }))?;
            } else {
                if results.is_empty() {
                    stdoutln(format_args!("No wiki pages matched: {}", query))?;
                } else {
                    stdoutln(format_args!(
                        "_{} wiki page(s) matched_ `{}`\n",
                        results.len(),
                        query
                    ))?;
                    for r in &results {
                        let title = r.title.as_deref().unwrap_or("(untitled)");
                        stdoutln(format_args!("- **{}** (`{}`)", title, r.path))?;
                        if !r.excerpt.is_empty() {
                            stdoutln(format_args!(
                                "  _{}_",
                                r.excerpt.chars().take(120).collect::<String>()
                            ))?;
                        }
                    }
                }
            }
        }
        "decision-search" => {
            if args.len() < 4 {
                eprintln!(
                    "Usage: atheneum decision-search <db-path> <query> [--project P] [--limit N]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let query = positional(&args, 3, "query")?;
            let opts = parse_options(&args[4..])?;
            let limit = parse_i64_option(opts.limit.as_deref(), "limit")?.unwrap_or(10);
            let graph = AtheneumGraph::open(&db_path)?;
            let decisions = graph.search_decisions(query, opts.project.as_deref(), limit)?;
            if opts.json {
                print_json(json!({
                    "query": query,
                    "project": opts.project,
                    "count": decisions.len(),
                    "decisions": decisions.iter().map(entity_to_json).collect::<Vec<_>>(),
                }))?;
            } else {
                if decisions.is_empty() {
                    stdoutln(format_args!("No decisions matched: {}\n", query))?;
                } else {
                    stdoutln(format_args!(
                        "_{} decision(s) matched_ `{}`\n",
                        decisions.len(),
                        query
                    ))?;
                    for d in &decisions {
                        let target = d.data.get("target").and_then(|v| v.as_str()).unwrap_or("?");
                        let chosen = d.data.get("chosen").and_then(|v| v.as_str()).unwrap_or("");
                        let agent = d.data.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
                        stdoutln(format_args!("## [{}] `{}` — {}", agent, target, chosen))?;
                        if let Some(why) = d.data.get("why").and_then(|v| v.as_str()) {
                            let preview: String = why.chars().take(200).collect();
                            stdoutln(format_args!("  _why_: {}", preview))?;
                        }
                    }
                }
            }
        }
        "query-journal" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum query-journal <db-path> <path>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let path = positional(&args, 3, "path")?;

            let graph = AtheneumGraph::open(&db_path)?;
            let sections = graph.query_journal_sections(path)?;
            if sections.is_empty() {
                stdoutln(format_args!("No journal sections found at path: {}", path))?;
            } else {
                stdoutln(format_args!("Journal sections for {}:", path))?;
                for section in sections {
                    stdoutln(format_args!(
                        "\n  [{}] {}",
                        section.time.as_deref().unwrap_or("?"),
                        section.title
                    ))?;
                    stdoutln(format_args!(
                        "    Body (first 200 chars): {}",
                        &section.body.chars().take(200).collect::<String>()
                    ))?;
                    if !section.kanban_updates.is_empty() {
                        stdoutln(format_args!("    Kanban updates:"))?;
                        for update in &section.kanban_updates {
                            stdoutln(format_args!(
                                "      '{}' -> {:?}",
                                update.task_title, update.new_status
                            ))?;
                        }
                    }
                }
            }
        }
        "entity" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum entity <db-path> <entity-id>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let entity_id = parse_i64_arg(positional(&args, 3, "entity-id")?, "entity-id")?;
            let graph = AtheneumGraph::open(&db_path)?;
            print_json(entity_to_json(&graph.get_entity(entity_id)?))?;
        }
        "edge" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum edge <db-path> <edge-id>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let edge_id = parse_i64_arg(positional(&args, 3, "edge-id")?, "edge-id")?;
            let graph = AtheneumGraph::open(&db_path)?;
            print_json(edge_to_json(&graph.get_edge(edge_id)?))?;
        }
        "neighbors" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum neighbors <db-path> <entity-id> [--depth N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let entity_id = parse_i64_arg(positional(&args, 3, "entity-id")?, "entity-id")?;
            let opts = parse_options(&args[4..])?;
            let depth = parse_u32_option(opts.depth.as_deref(), "depth")?.unwrap_or(0);
            let graph = AtheneumGraph::open(&db_path)?;
            if depth == 0 {
                let (outgoing, incoming) = graph.get_neighbors(entity_id)?;
                print_json(json!({
                    "entity_id": entity_id,
                    "outgoing": outgoing.iter().map(edge_to_json).collect::<Vec<_>>(),
                    "incoming": incoming.iter().map(edge_to_json).collect::<Vec<_>>(),
                }))?;
            } else {
                print_json(subgraph_to_json(&graph.get_subgraph(entity_id, depth)?))?;
            }
        }
        "navigate" => {
            if args.len() < 4 {
                eprintln!(
                    "Usage: atheneum navigate <db-path> <query> [--k N] [--depth N] [--project P] [--kind K] [--max-tokens N] [--concise]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let query = positional(&args, 3, "query")?;
            let opts = parse_options(&args[4..])?;
            let k = parse_usize_option(opts.k.as_deref(), "k")?.unwrap_or(5);
            let depth = parse_u32_option(opts.depth.as_deref(), "depth")?.unwrap_or(2);
            let max_tokens = parse_usize_option(opts.max_tokens.as_deref(), "max-tokens")?;
            let graph = AtheneumGraph::open(&db_path)?;
            let plan = graph.preview_navigate_query(
                query,
                k,
                depth,
                opts.project.as_deref(),
                opts.kind.as_deref(),
            )?;
            if !plan.executable {
                anyhow::bail!(plan.errors.join("; "));
            }
            let (views, trace_id) = graph.navigate_with_trace(
                query,
                k,
                depth,
                opts.project.as_deref(),
                opts.kind.as_deref(),
                max_tokens,
                opts.trace,
            )?;
            if opts.concise {
                print_navigate_concise(query, &views, max_tokens)?;
                if let Some(tid) = trace_id {
                    println!("Trace ID: {}", tid);
                }
            } else {
                print_json(json!({
                    "query": query,
                    "k": k,
                    "depth": depth,
                    "kind": opts.kind,
                    "project": opts.project,
                    "max_tokens": max_tokens,
                    "plan": plan,
                    "trace_id": trace_id,
                    "subgraphs": views.iter().map(subgraph_to_json).collect::<Vec<_>>(),
                }))?;
            }
        }
        "trace-get" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum trace-get <db-path> --id N");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let id = parse_i64_option(opts.id.as_deref(), "id")?
                .ok_or_else(|| anyhow::anyhow!("missing --id"))?;
            let graph = AtheneumGraph::open(&db_path)?;
            
            let trace_entity = graph.with_raw_connection(|conn| {
                let mut stmt = conn.prepare("SELECT data FROM graph_entities WHERE id = ?1 AND kind = 'QueryTrace'")?;
                let mut rows = stmt.query([id])?;
                if let Some(row) = rows.next()? {
                    let data_str: String = row.get(0)?;
                    let data: serde_json::Value = serde_json::from_str(&data_str)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                    Ok(Some(data))
                } else {
                    Ok(None)
                }
            })?;
            
            let Some(trace) = trace_entity else {
                anyhow::bail!("QueryTrace with ID {} not found", id);
            };
            
            print_json(trace)?;
        }
        "thread" => {
            if args.len() < 4 {
                eprintln!(
                    "Usage: atheneum thread <db-path> <query> [--tokens T] [--depth D] [--k N] [--project P] [--json]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let query = positional(&args, 3, "query")?;
            let opts = parse_options(&args[4..])?;
            let k = parse_usize_option(opts.k.as_deref(), "k")?.unwrap_or(3);
            let depth = parse_u32_option(opts.depth.as_deref(), "depth")?.unwrap_or(3);
            let tokens = parse_usize_option(opts.tokens.as_deref(), "tokens")?.unwrap_or(1500);
            let graph = AtheneumGraph::open(&db_path)?;
            let views = graph.thread_query(query, k, depth, opts.project.as_deref(), tokens)?;
            if opts.json {
                print_json(json!({
                    "query": query,
                    "k": k,
                    "depth": depth,
                    "project": opts.project,
                    "tokens": tokens,
                    "subgraphs": views.iter().map(subgraph_to_json).collect::<Vec<_>>(),
                }))?;
            } else {
                print_thread(query, &views, tokens)?;
            }
        }
        "chat" => {
            if args.len() < 4 {
                eprintln!(
                    "Usage: atheneum chat <db-path> --session <id> \
                     [--tokens N] [--direction recent|chrono] \
                     [--kinds ReasoningLog,ToolCall] [--role <role>] \
                     [--search \"query\"] [--only-decisions] \
                     [--offset N --limit L] [--walk] [--json]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let session_id = opts
                .session
                .clone()
                .ok_or_else(|| anyhow::anyhow!("--session <id> is required"))?;
            let direction = match opts.direction.as_deref() {
                Some(d) => atheneum::graph::ChatDirection::parse(d)?,
                None => atheneum::graph::ChatDirection::Recent,
            };
            let kinds = opts
                .kinds
                .as_deref()
                .map(|s| {
                    s.split(',')
                        .map(|p| p.trim().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let query = atheneum::graph::ChatQuery {
                session_id,
                tokens: parse_usize_option(opts.tokens.as_deref(), "tokens")?.unwrap_or(500),
                direction,
                kinds,
                role: opts.role.clone(),
                search: opts.search.clone(),
                only_decisions: opts.only_decisions,
                offset: parse_i64_option(opts.offset.as_deref(), "offset")?.unwrap_or(0),
                limit: parse_i64_option(opts.limit.as_deref(), "limit")?,
                walk: opts.walk,
            };
            let graph = AtheneumGraph::open(&db_path)?;
            let report = graph.query_chat(query)?;
            if opts.json {
                print_json(serde_json::to_value(&report)?)?;
            } else {
                print_chat(&report)?;
            }
        }
        "graph-stats" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum graph-stats <db-path>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let graph = AtheneumGraph::open(&db_path)?;
            let stats = graph.graph_stats()?;
            let runtime = graph.runtime_stats();
            print_json(json!({
                "total_entities": stats.total_entities,
                "total_edges": stats.total_edges,
                "entity_counts": stats.entity_counts,
                "edge_counts": stats.edge_counts,
                "runtime": runtime,
            }))?;
        }
        "reindex" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum reindex <db-path>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let graph = AtheneumGraph::open(&db_path)?;
            #[cfg(feature = "semantic-search")]
            {
                let stats_before = graph.graph_stats()?;
                graph.build_search_index()?;
                graph.checkpoint()?;
                let stats_after = graph.graph_stats()?;
                stdoutln(format_args!(
                    "Reindexed: {} entities ({} total), was {} entities before",
                    stats_after.total_entities,
                    stats_after.total_entities,
                    stats_before.total_entities,
                ))?;
            }
            #[cfg(not(feature = "semantic-search"))]
            {
                graph.build_search_index()?;
                graph.checkpoint()?;
                let stats_after = graph.graph_stats()?;
                stdoutln(format_args!(
                    "Semantic search disabled. Graph has {} entities (no index to rebuild).",
                    stats_after.total_entities,
                ))?;
            }
        }
        "store-discovery" => {
            if args.len() < 6 {
                eprintln!("Usage: atheneum store-discovery <db-path> <agent> <type> <target> [metadata.json] [--session <id>] [--project <id>] [--dedup] [--force]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let agent = positional(&args, 3, "agent")?;
            let discovery_type = positional(&args, 4, "type")?;
            let target = positional(&args, 5, "target")?;
            // Positional metadata file is args[6] only if it isn't a flag.
            let metadata_path = args.get(6).filter(|s| !s.starts_with('-'));
            let mut metadata: serde_json::Value = if let Some(meta_path) = metadata_path {
                let content = std::fs::read_to_string(meta_path)
                    .map_err(|e| anyhow::anyhow!("read metadata file: {}", e))?;
                serde_json::from_str(&content)
                    .map_err(|e| anyhow::anyhow!("parse metadata JSON: {}", e))?
            } else {
                json!({})
            };
            // Optional flags after the positional args.
            let mut dedup = false;
            let mut force = false;
            let mut i = if metadata_path.is_some() { 7 } else { 6 };
            while i < args.len() {
                match args[i].as_str() {
                    "--session" => {
                        let sid = args
                            .get(i + 1)
                            .ok_or_else(|| anyhow::anyhow!("--session requires a value"))?;
                        if let Some(obj) = metadata.as_object_mut() {
                            obj.insert("session_id".to_string(), json!(sid));
                        }
                        i += 2;
                    }
                    "--project" => {
                        let pid = args
                            .get(i + 1)
                            .ok_or_else(|| anyhow::anyhow!("--project requires a value"))?;
                        if let Some(obj) = metadata.as_object_mut() {
                            obj.insert("project_id".to_string(), json!(pid));
                        }
                        i += 2;
                    }
                    "--dedup" => {
                        dedup = true;
                        i += 1;
                    }
                    "--force" => {
                        force = true;
                        i += 1;
                    }
                    other => anyhow::bail!("unknown store-discovery option: {}", other),
                }
            }
            let graph = AtheneumGraph::open(&db_path)?;
            // Phase 5 dedup guard for the cooperative-skill / manual `/decision`
            // capture paths. The live watcher (Phase 4) and the post-hoc
            // extractor (Phase 3) dedup in-process before calling store_discovery;
            // the CLI is the store path for skill/command capture, so it must
            // guard itself. `--dedup` opts in; `--force` bypasses. The key is
            // (session_id, target, source, chosen) — see
            // [`AtheneumGraph::decision_exists_chosen`]. Only Decisions are
            // deduped; other discovery types insert unconditionally.
            if dedup && !force && discovery_type == "Decision" {
                let sid = metadata.get("session_id").and_then(|v| v.as_str());
                let source = metadata.get("source").and_then(|v| v.as_str());
                let chosen = metadata.get("chosen").and_then(|v| v.as_str());
                if let (Some(sid), Some(source), Some(chosen)) = (sid, source, chosen) {
                    if graph.decision_exists_chosen(sid, target, source, chosen)? {
                        print_json(json!({
                            "discovery_id": null,
                            "deduped": true,
                            "agent": agent,
                            "type": discovery_type,
                            "target": target,
                            "session_id": sid,
                            "source": source,
                            "chosen": chosen,
                        }))?;
                        return Ok(());
                    }
                }
            }
            let id = graph.store_discovery(agent, discovery_type, target, metadata)?;
            print_json(
                json!({"discovery_id": id, "agent": agent, "type": discovery_type, "target": target}),
            )?;
        }
        "add-edge" => {
            if args.len() < 6 {
                eprintln!("Usage: atheneum add-edge <db-path> <from-id> <to-id> <edge-type> [metadata.json|--data 'json']");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let from_id = parse_i64_arg(positional(&args, 3, "from-id")?, "from-id")?;
            let to_id = parse_i64_arg(positional(&args, 4, "to-id")?, "to-id")?;
            let edge_type_str = positional(&args, 5, "edge-type")?;
            let edge_type = EdgeType::from_label(edge_type_str)
                .ok_or_else(|| anyhow::anyhow!("unknown edge type '{}'. Valid: performed_by, assigned_to, called, calls, accessed, modified, verified_by, caused_by, created, related_to, mentions, wikilink, implements, depends_on, tested_by, fixed_by, regressed_by, observed_in, belongs_to_project, similar_failure, requires_skill, handled_by_tool, explains, derived_from, superseded_by, consolidated_from", edge_type_str))?;
            let data = if let Some(data_arg) = args.get(6) {
                if data_arg.starts_with('-') && data_arg != "--data" {
                    anyhow::bail!(
                        "expected optional positional <metadata.json>, got flag-looking \
                         argument '{}'; use --data '<json>' to pass JSON inline",
                        data_arg
                    );
                }
                if data_arg == "--data" {
                    let json_str = args
                        .get(7)
                        .ok_or_else(|| anyhow::anyhow!("--data requires a JSON argument"))?;
                    serde_json::from_str(json_str)
                        .map_err(|e| anyhow::anyhow!("parse JSON: {}", e))?
                } else {
                    let content = std::fs::read_to_string(data_arg)
                        .map_err(|e| anyhow::anyhow!("read data file: {}", e))?;
                    serde_json::from_str(&content)
                        .map_err(|e| anyhow::anyhow!("parse data JSON: {}", e))?
                }
            } else {
                json!({})
            };
            let graph = AtheneumGraph::open(&db_path)?;
            let id = graph.insert_edge(from_id, to_id, edge_type, data)?;
            print_json(
                json!({"edge_id": id, "from_id": from_id, "to_id": to_id, "edge_type": edge_type_str}),
            )?;
        }
        "task-create" => {
            if args.len() < 5 {
                eprintln!(
                    "Usage: atheneum task-create <db-path> <title> [description] [--project P]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let title = positional(&args, 3, "title")?;
            let description = args.get(4).and_then(|s| {
                if s.starts_with('-') {
                    None
                } else {
                    Some(s.as_str())
                }
            });
            let opts = parse_options(&args[4..])?;
            let graph = AtheneumGraph::open(&db_path)?;
            let id = graph.create_task(title, description, opts.project.as_deref())?;
            print_json(json!({"task_id": id, "title": title, "status": "TODO"}))?;
        }
        "task-list" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum task-list <db-path> [--project P] [--status S]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;
            let tasks: Vec<serde_json::Value> = if let Some(status_str) = &opts.status {
                let status = atheneum::graph::KanbanStatus::parse(status_str).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown status '{}'. Valid: TODO, IN_PROGRESS, DONE, BLOCKED, ARCHIVED",
                        status_str
                    )
                })?;
                graph.list_tasks_by_status(status, opts.project.as_deref())?
            } else {
                graph.list_tasks(opts.project.as_deref())?
            }
            .iter()
            .map(entity_to_json)
            .collect();
            print_json(json!({"tasks": tasks, "count": tasks.len()}))?;
        }
        "task-update" => {
            if args.len() < 5 {
                eprintln!("Usage: atheneum task-update <db-path> <task-id> <status>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let task_id = parse_i64_arg(positional(&args, 3, "task-id")?, "task-id")?;
            let status_str = positional(&args, 4, "status")?;
            let status = atheneum::graph::KanbanStatus::parse(status_str).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown status '{}'. Valid: TODO, IN_PROGRESS, DONE, BLOCKED, ARCHIVED",
                    status_str
                )
            })?;
            let graph = AtheneumGraph::open(&db_path)?;
            graph.update_task_status(task_id, status)?;
            print_json(json!({"task_id": task_id, "new_status": status.as_str()}))?;
        }
        "task-done" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum task-done <db-path> <task-id>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let task_id = parse_i64_arg(positional(&args, 3, "task-id")?, "task-id")?;
            let graph = AtheneumGraph::open(&db_path)?;
            graph.update_task_status(task_id, atheneum::graph::KanbanStatus::Done)?;
            print_json(json!({"task_id": task_id, "status": "DONE"}))?;
        }
        "task-archive" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum task-archive <db-path> <task-id>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let task_id = parse_i64_arg(positional(&args, 3, "task-id")?, "task-id")?;
            let graph = AtheneumGraph::open(&db_path)?;
            graph.update_task_status(task_id, atheneum::graph::KanbanStatus::Archived)?;
            print_json(json!({"task_id": task_id, "status": "ARCHIVED"}))?;
        }
        "search" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum search <db-path> <query> [--k N] [--project P] [--max-tokens N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let query = positional(&args, 3, "query")?;
            let opts = parse_options(&args[4..])?;
            let k = parse_usize_option(opts.k.as_deref(), "k")?.unwrap_or(10);
            let max_tokens = parse_usize_option(opts.max_tokens.as_deref(), "max-tokens")?;
            let graph = AtheneumGraph::open(&db_path)?;
            let hits = graph.lexical_search(query, k, opts.project.as_deref(), None, max_tokens)?;
            print_json(json!({
                "query": query,
                "k": k,
                "project": opts.project,
                "max_tokens": max_tokens,
                "results": hits.iter().map(search_result_to_json).collect::<Vec<_>>(),
            }))?;
        }
        "search-wiki" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum search-wiki <db-path> <query> [--limit N] [--offset N] [--project P]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let query = positional(&args, 3, "query")?;
            let opts = parse_options(&args[4..])?;
            let limit = parse_usize_option(opts.limit.as_deref(), "limit")?.unwrap_or(10);
            let offset = parse_usize_option(opts.offset.as_deref(), "offset")?.unwrap_or(0);
            let graph = AtheneumGraph::open(&db_path)?;
            let hits = graph.search_wiki_pages(query, opts.project.as_deref(), offset, limit)?;
            print_json(json!({
                "query": query,
                "offset": offset,
                "limit": limit,
                "project": opts.project,
                "count": hits.len(),
                "results": hits,
            }))?;
        }
        "backfill-wiki" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum backfill-wiki <db-path> [--project P]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;
            let fixed = graph.backfill_wiki_pages_to_graph(opts.project.as_deref())?;
            print_json(json!({
                "project": opts.project,
                "fixed": fixed.len(),
                "pages": fixed.iter().map(|(id, path)| json!({"id": id, "path": path})).collect::<Vec<_>>(),
            }))?;
        }
        "query-knowledge" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum query-knowledge <db-path> <target> [--project P] [--max-tokens N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let target = positional(&args, 3, "target")?;
            let opts = parse_options(&args[4..])?;
            let max_tokens = parse_usize_option(opts.max_tokens.as_deref(), "max-tokens")?;
            let graph = AtheneumGraph::open(&db_path)?;
            let result =
                graph.query_knowledge_in_project(target, opts.project.as_deref(), max_tokens)?;
            print_json(result)?;
        }
        "consolidate" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum consolidate <db-path> [target] [--project P]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let target = args
                .get(3)
                .filter(|s| !s.starts_with('-'))
                .map(|s| s.as_str());
            let start = if target.is_some() { 4 } else { 3 };
            let opts = parse_options(&args[start..])?;
            let graph = AtheneumGraph::open(&db_path)?;
            if let Some(t) = target {
                let kid = graph.consolidate_discoveries(t, opts.project.as_deref())?;
                match kid {
                    Some(id) => {
                        print_json(json!({"consolidated": true, "target": t, "knowledge_id": id}))?
                    }
                    None => print_json(
                        json!({"consolidated": false, "target": t, "reason": "no discoveries found"}),
                    )?,
                }
            } else {
                let results = graph.consolidation_pass(opts.project.as_deref())?;
                print_json(json!({
                    "consolidated_targets": results.len(),
                    "results": results.iter().map(|(t, id)| json!({"target": t, "knowledge_id": id})).collect::<Vec<_>>(),
                }))?;
            }
        }
        "list-pages" => {
            if args.len() < 3 {
                eprintln!(
                    "Usage: atheneum list-pages <db-path> [--project P] [--offset N] [--limit N]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let offset = parse_usize_option(opts.offset.as_deref(), "offset")?.unwrap_or(0);
            let limit = parse_usize_option(opts.limit.as_deref(), "limit")?.unwrap_or(1000);
            let graph = AtheneumGraph::open(&db_path)?;
            let pages = graph.list_wiki_pages_page(opts.project.as_deref(), offset, limit)?;
            print_json(json!({
                "count": pages.len(),
                "offset": offset,
                "limit": limit,
                "pages": pages.iter().map(wiki_page_summary_to_json).collect::<Vec<_>>(),
            }))?;
        }
        "query-sessions" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum query-sessions <db-path> [--project P] [--offset N] [--limit N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let offset = parse_usize_option(opts.offset.as_deref(), "offset")?.unwrap_or(0);
            let limit = parse_i64_option(opts.limit.as_deref(), "limit")?.unwrap_or(20);
            let graph = AtheneumGraph::open(&db_path)?;
            let sessions =
                graph.query_sessions_page(opts.project.as_deref(), None, offset, limit)?;
            print_json(json!({
                "count": sessions.len(),
                "offset": offset,
                "limit": limit,
                "sessions": sessions,
            }))?;
        }
        "query-events" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum query-events <db-path> [--session <id>] [--type <event-type>] [--offset N] [--limit N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let offset = parse_usize_option(opts.offset.as_deref(), "offset")?.unwrap_or(0);
            let limit = parse_usize_option(opts.limit.as_deref(), "limit")?.unwrap_or(50);
            let graph = AtheneumGraph::open(&db_path)?;
            let events = graph.query_events_page(
                opts.session.as_deref(),
                opts.event_type.as_deref(),
                offset,
                limit,
            )?;
            print_json(json!({
                "count": events.len(),
                "offset": offset,
                "limit": limit,
                "events": events,
            }))?;
        }
        "session-digest" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum session-digest <db-path> [--project P] [--last N] [--tokens T] [--json]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let last = parse_i64_option(opts.last.as_deref(), "last")?.unwrap_or(3);
            let tokens = parse_usize_option(opts.tokens.as_deref(), "tokens")?.unwrap_or(500);
            let graph = AtheneumGraph::open(&db_path)?;
            if opts.json {
                let value = graph.compose_digest_json(opts.project.as_deref(), last)?;
                print_json(value)?;
            } else {
                let text = graph.compose_digest(opts.project.as_deref(), last, tokens)?;
                stdoutln(format_args!("{}", text))?;
            }
        }
        "session-trace" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum session-trace <db-path> --session <id> [--limit N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let session_id = opts
                .session
                .clone()
                .ok_or_else(|| anyhow::anyhow!("--session is required"))?;
            let limit = parse_usize_option(opts.limit.as_deref(), "limit")?.unwrap_or(50);
            let graph = AtheneumGraph::open(&db_path)?;
            let session = graph.query_session_by_id(&session_id)?;
            let events = graph.query_events_page(Some(&session_id), None, 0, limit)?;
            let tool_calls: Vec<serde_json::Value> = events
                .iter()
                .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("tool_call"))
                .cloned()
                .collect();
            let tool_call_count = tool_calls.len();
            print_json(json!({
                "session": session,
                "event_count": events.len(),
                "tool_call_count": tool_call_count,
                "tool_calls": tool_calls,
                "events": events,
            }))?;
        }
        "tool-usage" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum tool-usage <db-path> --session <id> [--limit N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let session_id = opts
                .session
                .clone()
                .ok_or_else(|| anyhow::anyhow!("--session is required"))?;
            let limit = parse_usize_option(opts.limit.as_deref(), "limit")?.unwrap_or(100);
            let graph = AtheneumGraph::open(&db_path)?;
            let events = graph.query_events_page(Some(&session_id), Some("tool_call"), 0, limit)?;
            let mut counts = std::collections::BTreeMap::<String, usize>::new();
            for event in &events {
                if let Some(tool_name) = event
                    .get("payload")
                    .and_then(|payload| payload.get("tool_name"))
                    .and_then(|value| value.as_str())
                {
                    *counts.entry(tool_name.to_string()).or_insert(0) += 1;
                }
            }
            let usage = counts
                .into_iter()
                .map(|(tool_name, count)| json!({"tool_name": tool_name, "count": count}))
                .collect::<Vec<_>>();
            print_json(json!({
                "session_id": session_id,
                "count": events.len(),
                "usage": usage,
                "events": events,
            }))?;
        }
        "discoveries-recent" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum discoveries-recent <db-path> [--project P] [--agent A] [--session ID] [--type T] [--limit N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let limit = parse_i64_option(opts.limit.as_deref(), "limit")?.unwrap_or(20);
            let graph = AtheneumGraph::open(&db_path)?;
            // `--type` is the shared type-filter slot (`opts.event_type`); for
            // discoveries-recent it selects a `discovery_type` (e.g. `Decision`).
            let discoveries = graph.recent_discoveries(
                opts.project.as_deref(),
                opts.agent.as_deref(),
                opts.session.as_deref(),
                opts.event_type.as_deref(),
                limit,
            )?;
            print_json(json!({
                "count": discoveries.len(),
                "project": opts.project,
                "agent": opts.agent,
                "session": opts.session,
                "type": opts.event_type,
                "discoveries": discoveries.iter().map(entity_to_json).collect::<Vec<_>>(),
            }))?;
        }
        "watch-decisions" => {
            if args.len() < 3 {
                eprintln!(
                    "Usage: atheneum watch-decisions <db-path> \
                     [--once] [--interval SECS] [--project P] [--agent A] \
                     [--config-dir DIR]... [--dry-run] [--json]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let interval = parse_u64_option(opts.interval.as_deref(), "interval")?
                .map(std::time::Duration::from_secs)
                .unwrap_or_else(|| std::time::Duration::from_secs(2));
            // `--config-dir` overrides the auto-discovered config roots. May be
            // supplied multiple times; each must be an existing directory. It is
            // not parsed by `parse_options` (which would reject it as unknown),
            // so scan the raw args here.
            let mut dirs: Vec<PathBuf> = Vec::new();
            let mut i = 3;
            while i < args.len() {
                if args[i] == "--config-dir" {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("--config-dir requires a value"))?;
                    dirs.push(PathBuf::from(v));
                    i += 2;
                } else if args[i] == "--project" || args[i] == "--agent" || args[i] == "--interval"
                {
                    // value-bearing flags already consumed by parse_options;
                    // skip the value here too.
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let default_config = atheneum::graph::WatchConfig::default();
            let config_dirs = if dirs.is_empty() {
                default_config.config_dirs.clone()
            } else {
                dirs
            };
            let config = atheneum::graph::WatchConfig {
                config_dirs,
                interval,
                project: opts.project.clone(),
                agent: opts.agent.clone().unwrap_or_else(|| "claude".to_string()),
                dry_run: opts.dry_run,
                once: opts.once,
            };
            let graph = AtheneumGraph::open(&db_path)?;
            let stats = atheneum::graph::watch_decisions(&graph, &config)?;
            if opts.json {
                print_json(json!({
                    "files_scanned": stats.files_scanned,
                    "decisions_emitted": stats.decisions_emitted,
                    "decisions_skipped": stats.decisions_skipped,
                    "once": config.once,
                    "dry_run": config.dry_run,
                }))?;
            } else {
                stdoutln(format_args!(
                    "watch-decisions{}: {} file(s) scanned, {} decision(s) emitted, {} skipped (dup)",
                    if config.once { " --once" } else { "" },
                    stats.files_scanned,
                    stats.decisions_emitted,
                    stats.decisions_skipped,
                ))?;
            }
        }
        #[cfg(feature = "extract")]
        "extract-decisions" => {
            // Phase 3 native port. Backfill Decision discoveries from Claude Code
            // transcripts via a local Ollama LLM, stored in-process (no CLI shell).
            // `atheneum extract-decisions <db> [--all | <session-id>] [...]`.
            if args.len() < 3 {
                eprintln!(
                    "Usage: atheneum extract-decisions <db-path> [--all | <session-id>] \
                     [--dry-run] [--force] [--verbose] [--project P] [--agent A] \
                     [--model M] [--transcripts-dir D] [--max-chars N] [--ollama-url U] \
                     [--heuristic | --mode llm|heuristic]"
                );
                std::process::exit(1);
            }
            let mut config = atheneum::graph::ExtractConfig {
                db: PathBuf::from(positional(&args, 2, "db-path")?),
                ..atheneum::graph::ExtractConfig::default()
            };
            let mut json_out = false;
            let mut i = 3;
            while i < args.len() {
                match args[i].as_str() {
                    "--all" => config.all = true,
                    "--dry-run" => config.dry_run = true,
                    "--force" => config.force = true,
                    "--verbose" => config.verbose = true,
                    "--heuristic" => config.mode = atheneum::graph::ExtractMode::Heuristic,
                    "--mode" => {
                        let v = args
                            .get(i + 1)
                            .ok_or_else(|| anyhow::anyhow!("--mode requires a value"))?;
                        config.mode = match v.as_str() {
                            "llm" | "ollama" => atheneum::graph::ExtractMode::Llm,
                            "heuristic" | "rules" | "no-llm" => {
                                atheneum::graph::ExtractMode::Heuristic
                            }
                            other => {
                                anyhow::bail!("unknown --mode {:?} (expected llm|heuristic)", other)
                            }
                        };
                        i += 1;
                    }
                    "--json" => json_out = true,
                    "--project" => {
                        config.project = Some(
                            args.get(i + 1)
                                .ok_or_else(|| anyhow::anyhow!("--project requires a value"))?
                                .clone(),
                        );
                        i += 1;
                    }
                    "--agent" => {
                        config.agent = args
                            .get(i + 1)
                            .ok_or_else(|| anyhow::anyhow!("--agent requires a value"))?
                            .clone();
                        i += 1;
                    }
                    "--model" => {
                        config.model = args
                            .get(i + 1)
                            .ok_or_else(|| anyhow::anyhow!("--model requires a value"))?
                            .clone();
                        i += 1;
                    }
                    "--transcripts-dir" => {
                        config.transcripts_dir =
                            PathBuf::from(args.get(i + 1).ok_or_else(|| {
                                anyhow::anyhow!("--transcripts-dir requires a value")
                            })?);
                        i += 1;
                    }
                    "--ollama-url" => {
                        config.ollama_url = args
                            .get(i + 1)
                            .ok_or_else(|| anyhow::anyhow!("--ollama-url requires a value"))?
                            .clone();
                        i += 1;
                    }
                    "--max-chars" => {
                        config.max_chars = args
                            .get(i + 1)
                            .ok_or_else(|| anyhow::anyhow!("--max-chars requires a value"))?
                            .parse::<usize>()
                            .map_err(|e| anyhow::anyhow!("--max-chars: {}", e))?;
                        i += 1;
                    }
                    other if !other.starts_with('-') => {
                        if config.session_id.is_some() {
                            anyhow::bail!("unexpected positional argument: {}", other);
                        }
                        config.session_id = Some(other.to_string());
                    }
                    other => anyhow::bail!("unknown extract-decisions option: {}", other),
                }
                i += 1;
            }
            let stats = atheneum::graph::run_extract(&config)?;
            if json_out {
                print_json(json!({
                    "sessions": stats.sessions,
                    "extracted": stats.extracted,
                    "stored": stats.stored,
                    "skipped": stats.skipped,
                    "dry_run": config.dry_run,
                    "all": config.all,
                }))?;
            }
        }
        "handoffs-recent" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum handoffs-recent <db-path> [--project P] [--agent A] [--limit N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let limit = parse_i64_option(opts.limit.as_deref(), "limit")?.unwrap_or(20);
            let graph = AtheneumGraph::open(&db_path)?;
            let handoffs =
                graph.recent_handoffs(opts.project.as_deref(), opts.agent.as_deref(), limit)?;
            print_json(json!({
                "count": handoffs.len(),
                "project": opts.project,
                "agent": opts.agent,
                "handoffs": handoffs.iter().map(entity_to_json).collect::<Vec<_>>(),
            }))?;
        }
        "events-recent" => {
            if args.len() < 3 {
                eprintln!(
                    "Usage: atheneum events-recent <db-path> [--session ID] [--type T] [--limit N]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let limit = parse_usize_option(opts.limit.as_deref(), "limit")?.unwrap_or(50);
            let graph = AtheneumGraph::open(&db_path)?;
            let events = graph.query_events_page(
                opts.session.as_deref(),
                opts.event_type.as_deref(),
                0,
                limit,
            )?;
            print_json(json!({
                "count": events.len(),
                "session": opts.session,
                "event_type": opts.event_type,
                "events": events,
            }))?;
        }
        "sessions-recent" => {
            if args.len() < 3 {
                eprintln!(
                    "Usage: atheneum sessions-recent <db-path> [--project P] [--agent A] [--limit N] [--exclude-project P ...]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let limit = parse_i64_option(opts.limit.as_deref(), "limit")?.unwrap_or(20);
            let graph = AtheneumGraph::open(&db_path)?;
            let sessions = graph.query_sessions_recent(
                opts.project.as_deref(),
                opts.agent.as_deref(),
                limit,
                &opts.exclude_projects,
            )?;
            print_json(json!({
                "count": sessions.len(),
                "project": opts.project,
                "agent": opts.agent,
                "exclude_projects": opts.exclude_projects,
                "sessions": sessions,
            }))?;
        }
        "memory-store" => {
            if args.len() < 5 {
                eprintln!("Usage: atheneum memory-store <db-path> <key> <content> [--scope S] [--confidence N] [--project P]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let key = positional(&args, 3, "key")?;
            let content = positional(&args, 4, "content")?;
            let opts = parse_options(&args[5..])?;
            let scope = opts.scope.as_deref().unwrap_or("user");
            let confidence: f64 =
                parse_f64_option(opts.confidence.as_deref(), "confidence")?.unwrap_or(1.0);
            let graph = AtheneumGraph::open(&db_path)?;
            let id = graph.store_memory(
                key,
                content,
                scope,
                confidence,
                opts.project.as_deref(),
                None,
            )?;
            print_json(json!({"memory_id": id, "key": key, "scope": scope}))?;
        }
        "memory-update" => {
            // Story A2 (spec FR-1). Patches an existing Memory entity in place.
            // `--content` replaces the body; `--importance N` (1..10) remaps to
            // confidence exactly as `memory-store` does. `--tags a,b` merges by
            // default; pass `--replace-tags` to overwrite.
            if args.len() < 4 {
                eprintln!(
                    "Usage: atheneum memory-update <db-path> --id N [--content \"...\"] \\\
                     [--importance 1..10] [--tags a,b --replace-tags]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let id_str = opts
                .id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("--id <N> is required"))?;
            let id = parse_i64_arg(id_str, "id")?;
            let patch = atheneum::MemoryPatch {
                content: opts.content.clone(),
                importance: opts
                    .importance
                    .as_deref()
                    .map(|s| {
                        s.parse::<i64>().map_err(|e| {
                            anyhow::anyhow!("invalid --importance '{}': {}", s, e)
                        })
                    })
                    .transpose()?,
                tags: opts
                    .tags
                    .as_deref()
                    .map(|s| s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()),
                replace_tags: opts.replace_tags,
            };
            if patch.is_empty() {
                anyhow::bail!(
                    "memory-update requires at least one of --content, --importance, --tags"
                );
            }
            let graph = AtheneumGraph::open(&db_path)?;
            let returned = graph.update_memory(id, &patch)?;
            let entity = graph.get_entity(returned)?;
            print_json(json!({
                "memory_id": returned,
                "key": entity.name,
                "scope": entity.data.get("scope").and_then(|v| v.as_str()),
                "content": entity.data.get("content").and_then(|v| v.as_str()),
                "confidence": entity.data.get("confidence").and_then(|v| v.as_f64()),
                "updated_at": entity.data.get("updated_at").and_then(|v| v.as_str()),
                "content_hash": entity.data.get("content_hash").and_then(|v| v.as_str()),
                "tags": entity.data.get("tags"),
            }))?;
        }
        "memory-get" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum memory-get <db-path> <key> [--scope S] [--project P]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let key = positional(&args, 3, "key")?;
            let opts = parse_options(&args[4..])?;
            let graph = AtheneumGraph::open(&db_path)?;
            let items = graph.query_memory(key, opts.scope.as_deref(), opts.project.as_deref(), opts.include_superseded)?;
            print_json(json!({
                "key": key,
                "count": items.len(),
                "items": items.iter().map(entity_to_json).collect::<Vec<_>>(),
            }))?;
        }
        "memory-list" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum memory-list <db-path> [--scope S] [--project P] [--offset N] [--limit N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let offset = parse_usize_option(opts.offset.as_deref(), "offset")?.unwrap_or(0);
            let limit = parse_usize_option(opts.limit.as_deref(), "limit")?.unwrap_or(1000);
            let graph = AtheneumGraph::open(&db_path)?;
            let items = graph.list_memory_page(
                opts.scope.as_deref(),
                opts.project.as_deref(),
                offset,
                limit,
            )?;
            print_json(json!({
                "count": items.len(),
                "offset": offset,
                "limit": limit,
                "items": items.iter().map(entity_to_json).collect::<Vec<_>>(),
            }))?;
        }
        "memory-bootstrap" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum memory-bootstrap <db-path> [--project P] [--tokens T] [--last N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let tokens = parse_usize_option(opts.tokens.as_deref(), "tokens")?.unwrap_or(800);
            let last = parse_i64_option(opts.last.as_deref(), "last")?.unwrap_or(3);
            let graph = AtheneumGraph::open(&db_path)?;
            let value = graph.compose_memory_bootstrap(opts.project.as_deref(), tokens, last)?;
            print_json(value)?;
        }
        "dream" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum dream <db-path> [--scope S] [--project P] [--dry-run|--auto-merge]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;

            use atheneum::{DreamConfig, DreamMode, DreamReport};
            let mode = if opts.auto_merge {
                DreamMode::AutoMerge
            } else {
                DreamMode::DryRun
            };
            let report: DreamReport = graph.dream_pass(
                mode,
                opts.scope.as_deref(),
                opts.project.as_deref(),
                &DreamConfig::default(),
            )?;
            print_json(serde_json::to_value(&report)?)?;
        }
        "wiki-dream" => {
            if args.len() < 3 {
                eprintln!(
                    "Usage: atheneum wiki-dream <db-path> [--project P] [--dry-run|--auto-merge]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;

            use atheneum::{DreamConfig, DreamMode, DreamReport};
            let mode = if opts.auto_merge {
                DreamMode::AutoMerge
            } else {
                DreamMode::DryRun
            };
            let report: DreamReport =
                graph.wiki_dream_pass(mode, opts.project.as_deref(), &DreamConfig::default())?;
            print_json(serde_json::to_value(&report)?)?;
        }
        "lint" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum lint <db-path> [--stale-days N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;
            let stale_superseded_days = parse_i64_option(opts.stale_days.as_deref(), "stale-days")?.unwrap_or(30);
            let report = graph.lint_graph(&atheneum::LintConfig { stale_superseded_days })?;
            print_json(serde_json::to_value(&report)?)?;
        }
        "maintain" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum maintain <db-path> [--apply] [--stale-days N] [--rewire-threshold F] [--broken-link-mode <stub|sever>]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;
            let stale_superseded_days = parse_i64_option(opts.stale_days.as_deref(), "stale-days")?.unwrap_or(30);
            let rewire_threshold = opts.rewire_threshold.as_deref()
                .map(|s| s.parse::<f64>().map_err(|e| anyhow::anyhow!("invalid rewire-threshold '{}': {}", s, e)))
                .transpose()?
                .unwrap_or(0.3);
            let broken_link_mode = match opts.broken_link_mode.as_deref() {
                Some("sever") => atheneum::BrokenLinkMode::Sever,
                _ => atheneum::BrokenLinkMode::Stub,
            };
            let apply = opts.apply && !opts.dry_run;
            let report = graph.maintain(&atheneum::MaintainConfig {
                rewire_threshold,
                broken_link_mode,
                stale_superseded_days,
            }, apply)?;
            print_json(serde_json::to_value(&report)?)?;
        }
        "seed-memory" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum seed-memory <db-path> [--project P] [--tokens N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(&args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let tokens = parse_usize_option(opts.tokens.as_deref(), "tokens")?.unwrap_or(800);
            let graph = AtheneumGraph::open(&db_path)?;
            let seed = graph.seed_memory(opts.project.as_deref(), tokens)?;
            print_json(serde_json::to_value(&seed)?)?;
        }
        "config" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum config <init|show> [args]");
                std::process::exit(1);
            }
            match args[2].as_str() {
                "init" => {
                    let force = args.iter().any(|a| a == "--force" || a == "-f");
                    let path = atheneum::default_config_path();
                    if path.exists() && !force {
                        stdoutln(format_args!(
                            "Config already exists at {}. Use --force to overwrite.",
                            path.display()
                        ))?;
                        return Ok(());
                    }
                    let cfg = Config::default();
                    atheneum::save_config(&cfg)?;
                    stdoutln(format_args!("Created default config at {}", path.display()))?;
                }
                "show" => {
                    let cfg = atheneum::load_config()?;
                    print_json(serde_json::to_value(&cfg)?)?;
                }
                other => {
                    eprintln!("Unknown config subcommand: {}", other);
                    eprintln!("Usage: atheneum config <init|show>");
                    std::process::exit(1);
                }
            }
        }
        "meta-register" => {
            if args.len() < 5 {
                eprintln!("Usage: atheneum meta-register <name> <root-path> <magellan-db> [--atheneum-db PATH] [--language LANG]");
                std::process::exit(1);
            }
            let name = positional(&args, 2, "name")?;
            let root_path = positional(&args, 3, "root-path")?;
            let magellan_db = positional(&args, 4, "magellan-db")?;
            let opts = parse_options(&args[5..])?;
            let mut router = MetaRouter::open()?;
            router.register_project(
                name,
                root_path,
                magellan_db,
                opts.atheneum_db.as_deref(),
                opts.language.as_deref(),
            )?;
            stdoutln(format_args!("Registered project: {}", name))?;
        }
        "meta-list" => {
            let router = MetaRouter::open()?;
            let projects = if args.len() > 2 && args[2] == "--language" {
                if args.len() < 4 {
                    eprintln!("Usage: atheneum meta-list [--language LANG]");
                    std::process::exit(1);
                }
                router.list_projects_by_language(&args[3])?
            } else {
                router.list_projects()?
            };
            if projects.is_empty() {
                stdoutln(format_args!("No projects registered."))?;
            } else {
                stdoutln(format_args!("Registered projects ({}):", projects.len()))?;
                for p in projects {
                    let lang = p.language.as_deref().unwrap_or("unknown");
                    let ath = p.atheneum_db.as_deref().unwrap_or("none");
                    stdoutln(format_args!(
                        "  {} [{}] root={} magellan={} atheneum={}",
                        p.name, lang, p.root_path, p.magellan_db, ath
                    ))?;
                }
            }
        }
        "cross-search" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum cross-search <query> [--language LANG] [--k N]");
                std::process::exit(1);
            }
            let query = positional(&args, 2, "query")?;
            let opts = parse_options(&args[3..])?;
            let k = parse_usize_option(opts.k.as_deref(), "k")?.unwrap_or(10);
            let mut router = CrossRouter::open()?;
            let hits = router.cross_search(query, opts.language.as_deref(), k)?;
            print_json(json!({
                "query": query,
                "language": opts.language,
                "k": k,
                "count": hits.len(),
                "results": hits.iter().map(cross_result_to_json).collect::<Vec<_>>(),
            }))?;
        }
        "cross-navigate" => {
            if args.len() < 3 {
                eprintln!(
                    "Usage: atheneum cross-navigate <query> [--language LANG] [--k N] [--depth N]"
                );
                std::process::exit(1);
            }
            let query = positional(&args, 2, "query")?;
            let opts = parse_options(&args[3..])?;
            let k = parse_usize_option(opts.k.as_deref(), "k")?.unwrap_or(5);
            let depth = parse_u32_option(opts.depth.as_deref(), "depth")?.unwrap_or(1);
            let mut router = CrossRouter::open()?;
            let views = router.cross_navigate(query, opts.language.as_deref(), k, depth)?;
            print_json(json!({
                "query": query,
                "language": opts.language,
                "k": k,
                "depth": depth,
                "count": views.len(),
                "views": views.iter().map(cross_subgraph_to_json).collect::<Vec<_>>(),
            }))?;
        }
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            print_usage()?;
            std::process::exit(1);
        }
    }

    Ok(())
}

fn print_usage() -> anyhow::Result<()> {
    write_usage(&mut io::stdout().lock())?;
    Ok(())
}

fn write_usage(mut writer: impl Write) -> io::Result<()> {
    writeln!(
        writer,
        "Atheneum v{} - Agent Coordination Graph Database",
        VERSION
    )?;
    writeln!(writer)?;
    writeln!(writer, "USAGE:")?;
    writeln!(writer, "  atheneum <command> [args]")?;
    writeln!(writer)?;
    writeln!(writer, "INGEST:")?;
    writeln!(
        writer,
        "  init <db-path>                          Initialize a new graph database"
    )?;
    writeln!(
        writer,
        "  sync-wiki <db-path> <dir> [project]     Ingest all .md files as wiki pages"
    )?;
    writeln!(
        writer,
        "  sync-journal <db-path> <dir> [project]  Ingest all .md files as journal sections"
    )?;
    writeln!(
        writer,
        "  sync-logseq <db-path> <root> [project]  Recursively ingest Logseq pages/ and journals/"
    )?;
    writeln!(
        writer,
        "  sync-claude-transcript <db> <jsonl> [project] [agent]  Import Claude transcript"
    )?;
    writeln!(
        writer,
        "  watch-decisions <db> [--once] [--interval S] [--config-dir D]...  Tail live transcripts, capture decisions"
    )?;
    writeln!(
        writer,
        "  store-discovery <db> <agent> <type> <target> [meta.json] [--session ID] [--project P] [--dedup] [--force]  Store a discovery (--dedup skips a duplicate Decision on session+target+source+chosen; --force bypasses)"
    )?;
    writeln!(
        writer,
        "    [--session <id>] [--project <id>]  Attribute discovery to a session/project"
    )?;
    writeln!(
        writer,
        "  add-edge <db> <from-id> <to-id> <edge-type> [data.json]  Create a relation"
    )?;
    writeln!(writer)?;
    writeln!(writer, "TASKS:")?;
    writeln!(
        writer,
        "  task-create <db> <title> [desc] [--project P]  Create a new task"
    )?;
    writeln!(
        writer,
        "  task-list <db> [--project P] [--status S]        List tasks (default: non-archived)"
    )?;
    writeln!(
        writer,
        "  task-update <db> <task-id> <status>                Update task status"
    )?;
    writeln!(
        writer,
        "  task-done <db> <task-id>                         Mark task as DONE"
    )?;
    writeln!(
        writer,
        "  task-archive <db> <task-id>                      Archive a task"
    )?;
    writeln!(writer)?;
    writeln!(writer, "MEMORY:")?;
    writeln!(
        writer,
        "  memory-store <db> <key> <content> [--scope S] [--confidence N] [--project P]  Store a memory"
    )?;
    writeln!(
        writer,
        "  memory-get <db> <key> [--scope S] [--project P]      Retrieve memory by key"
    )?;
    writeln!(
        writer,
        "  memory-list <db> [--scope S] [--project P] [--offset N] [--limit N]  List memories (paginated)"
    )?;
    writeln!(
        writer,
        "  memory-bootstrap <db> [--project P] [--tokens T] [--last N]  Memories + session digest packet"
    )?;
    writeln!(writer)?;
    writeln!(writer, "DREAM:")?;
    writeln!(
        writer,
        "  dream <db> [--scope S] [--project P] [--dry-run|--auto-merge]"
    )?;
    writeln!(writer, "    Run reflective memory consolidation pass")?;
    writeln!(writer)?;
    writeln!(
        writer,
        "  search <db-path> <query> [--k N] [--project P] [--max-tokens N]  HNSW/lexical search"
    )?;
    writeln!(
        writer,
        "  navigate <db-path> <query> [--k N] [--depth N] [--project P] [--kind K] [--max-tokens N] [--concise]  Search then walk graph subgraphs"
    )?;
    writeln!(
        writer,
        "  query-wiki <db-path> <path>             Query a wiki page by path (supports partial path suffix)"
    )?;
    writeln!(
        writer,
        "  wiki-search <db-path> <query> [--project P] [--limit N]  Full-text search wiki pages via FTS5"
    )?;
    writeln!(
        writer,
        "  query-journal <db-path> <path>          Query journal sections by path"
    )?;
    writeln!(
        writer,
        "  query-knowledge <db-path> <target> [--project P] [--max-tokens N]  Aggregated knowledge"
    )?;
    writeln!(
        writer,
        "  query-sessions <db-path> [--project P] [--offset N] [--limit N]  Session history"
    )?;
    writeln!(
        writer,
        "  session-digest <db-path> [--project P] [--last N] [--tokens T] [--json]  Bounded bootstrap digest"
    )?;
    writeln!(
        writer,
        "  thread <db-path> <query> [--tokens T] [--depth D] [--k N] [--project P] [--json]  Walk a decision chain (caused_by/led_to)"
    )?;
    writeln!(
        writer,
        "  query-events <db-path> [--session <id>] [--type <t>] [--offset N] [--limit N]  Event log"
    )?;
    writeln!(
        writer,
        "  session-trace <db-path> --session <id> [--limit N]  Session summary plus recent events"
    )?;
    writeln!(
        writer,
        "  tool-usage <db-path> --session <id> [--limit N]  Tool-call breakdown for one session"
    )?;
    writeln!(
        writer,
        "  discoveries-recent <db-path> [--project P] [--agent A] [--session S] [--type T] [--limit N]  Recent discoveries (filter by session and/or discovery type)"
    )?;
    writeln!(
        writer,
        "  decision-search <db-path> <query> [--project P] [--limit N]  Search decisions by content (target/chosen/why)"
    )?;
    writeln!(
        writer,
        "  handoffs-recent <db-path> [--project P] [--agent A] [--limit N]  Recent handoffs"
    )?;
    writeln!(
        writer,
        "  events-recent <db-path> [--session ID] [--type T] [--limit N]  Recent events"
    )?;
    writeln!(
        writer,
        "  sessions-recent <db-path> [--project P] [--agent A] [--limit N]  Recent sessions"
    )?;
    writeln!(
        writer,
        "  list-pages <db-path> [--project P] [--offset N] [--limit N]  List wiki pages"
    )?;
    writeln!(
        writer,
        "  entity <db-path> <id>                   Print a graph entity as JSON"
    )?;
    writeln!(
        writer,
        "  edge <db-path> <id>                     Print a graph edge as JSON"
    )?;
    writeln!(
        writer,
        "  neighbors <db-path> <id> [--depth N]    One-hop edges or BFS subgraph"
    )?;
    writeln!(
        writer,
        "  graph-stats <db-path>                   Graph topology and runtime counters"
    )?;
    writeln!(writer)?;
    writeln!(writer, "CONFIG:")?;
    writeln!(
        writer,
        "  config init [--force]                   Create default ~/.config/atheneum/config.toml"
    )?;
    writeln!(
        writer,
        "  config show                             Print effective configuration as JSON"
    )?;
    writeln!(writer)?;
    writeln!(writer, "META (cross-project registry):")?;
    writeln!(
        writer,
        "  meta-register <name> <root-path> <magellan-db> [--atheneum-db PATH] [--language LANG]"
    )?;
    writeln!(
        writer,
        "    Register a project in the meta.db routing layer"
    )?;
    writeln!(
        writer,
        "  meta-list [--language LANG]               List registered projects"
    )?;
    writeln!(writer)?;
    writeln!(writer, "CROSS-PROJECT (via meta.db + lazy ATTACH):")?;
    writeln!(
        writer,
        "  cross-search <query> [--language LANG] [--k N]  Search symbols across attached magellan DBs"
    )?;
    writeln!(
        writer,
        "  cross-navigate <query> [--language LANG] [--k N] [--depth N]  Search + BFS subgraph per project"
    )?;
    writeln!(writer)?;
    writeln!(writer, "MAINTENANCE:")?;
    writeln!(
        writer,
        "  reindex <db-path>                       Rebuild HNSW search index"
    )?;
    writeln!(
        writer,
        "  consolidate <db-path> [target] [--project P]  Merge discoveries into Knowledge"
    )?;
    writeln!(
        writer,
        "  --version, -v                           Print version"
    )?;
    writeln!(
        writer,
        "  help, --help, -h                        Print this help message"
    )?;
    writeln!(writer)?;
    writeln!(writer, "EXAMPLES:")?;
    writeln!(writer, "  atheneum init ./atheneum.db")?;
    writeln!(writer, "  atheneum sync-wiki ./atheneum.db ./wiki")?;
    writeln!(
        writer,
        "  atheneum sync-journal ./atheneum.db ./journal my-project"
    )?;
    writeln!(writer, "  atheneum sync-logseq ./atheneum.db ~/wiki forge")?;
    writeln!(
        writer,
        "  atheneum sync-claude-transcript ./atheneum.db transcript.jsonl forge claude"
    )?;
    writeln!(
        writer,
        "  atheneum watch-decisions ./atheneum.db --once --config-dir ~/.claude --project atheneum"
    )?;
    writeln!(
        writer,
        "  atheneum store-discovery ./atheneum.db claude Bug http_handler bug.json"
    )?;
    writeln!(
        writer,
        "  atheneum store-discovery ./atheneum.db claude Decision gemv_q4_0 --session c663d1ff --project rocmforge"
    )?;
    writeln!(
        writer,
        "  atheneum store-discovery ./atheneum.db claude Decision storage-engine dec.json --session $CLAUDE_CODE_SESSION_ID --dedup"
    )?;
    writeln!(
        writer,
        "  atheneum session-digest ./atheneum.db --project rocmforge --last 3 --tokens 500"
    )?;
    writeln!(
        writer,
        "  atheneum thread ./atheneum.db \"gemv q4_0 dispatch\" --tokens 1500 --depth 3"
    )?;
    writeln!(
        writer,
        "  atheneum chat ./atheneum.db --session abc123 --tokens 500 --direction recent"
    )?;
    writeln!(
        writer,
        "  atheneum chat ./atheneum.db --session abc123 --search \"HNSW\" --walk"
    )?;
    writeln!(writer, "  atheneum add-edge ./atheneum.db 1 2 explains")?;
    writeln!(
        writer,
        "  atheneum search ./atheneum.db \"router\" --k 5 --project envoy"
    )?;
    writeln!(
        writer,
        "  atheneum navigate ./atheneum.db \"router construction\" --k 3 --depth 2"
    )?;
    writeln!(
        writer,
        "  atheneum query-knowledge ./atheneum.db http_handler --project envoy"
    )?;
    writeln!(
        writer,
        "  atheneum consolidate ./atheneum.db --project forge"
    )?;
    writeln!(
        writer,
        "  atheneum list-pages ./atheneum.db --project forge"
    )?;
    writeln!(
        writer,
        "  atheneum query-sessions ./atheneum.db --offset 0 --limit 5"
    )?;
    writeln!(
        writer,
        "  atheneum query-events ./atheneum.db --session abc123 --offset 0 --limit 20"
    )?;
    writeln!(writer, "  atheneum graph-stats ./atheneum.db")?;
    writeln!(writer, "  atheneum reindex ./atheneum.db")?;
    writeln!(
        writer,
        "  atheneum task-create ./atheneum.db \"Build search index\" --project forge"
    )?;
    writeln!(writer, "  atheneum task-list ./atheneum.db --status TODO")?;
    writeln!(writer, "  atheneum task-done ./atheneum.db 42")?;
    writeln!(writer, "  atheneum task-archive ./atheneum.db 42")?;
    Ok(())
}

#[derive(Default)]
struct CliOptions {
    agent: Option<String>,
    k: Option<String>,
    depth: Option<String>,
    kind: Option<String>,
    project: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    session: Option<String>,
    event_type: Option<String>,
    status: Option<String>,
    scope: Option<String>,
    confidence: Option<String>,
    max_tokens: Option<String>,
    atheneum_db: Option<String>,
    language: Option<String>,
    tokens: Option<String>,
    last: Option<String>,
    direction: Option<String>,
    kinds: Option<String>,
    role: Option<String>,
    search: Option<String>,
    importance: Option<String>,
    tags: Option<String>,
    id: Option<String>,
    content: Option<String>,
    walk: bool,
    only_decisions: bool,
    replace_tags: bool,
    dry_run: bool,
    auto_merge: bool,
    concise: bool,
    json: bool,
    once: bool,
    include_superseded: bool,
    apply: bool,
    trace: bool,
    stale_days: Option<String>,
    rewire_threshold: Option<String>,
    broken_link_mode: Option<String>,
    interval: Option<String>,
    exclude_projects: Vec<String>,
}

/// Read a required positional argument, rejecting flag-looking values.
///
/// Subcommand arms historically did `PathBuf::from(&args[2])` / `&args[3]`
/// directly, so a bare flag in a positional slot (e.g. `atheneum init --help`)
/// was silently accepted as the value — `--help` became the db path and a
/// SQLite file named `--help` got created. This guard fails fast with a clear
/// message instead. A lone `-` is allowed (stdin convention) even though no
/// atheneum positional currently uses it.
fn positional<'a>(args: &'a [String], idx: usize, name: &str) -> anyhow::Result<&'a str> {
    let v = args
        .get(idx)
        .ok_or_else(|| anyhow::anyhow!("missing required positional <{}>", name))?;
    if v.starts_with('-') && v.as_str() != "-" {
        anyhow::bail!(
            "expected positional <{}>, got flag-looking argument '{}'; \
             flags must come after positionals or be passed as --flag value",
            name,
            v
        );
    }
    Ok(v)
}

/// Read an optional positional argument (no `parse_options` slice follows),
/// rejecting flag-looking values when present. Use `positional` for required
/// slots; for optional slots followed by `parse_options(&args[N..])`, keep the
/// existing `args.get(N).filter(|s| !s.starts_with('-'))` pattern so a flag is
/// treated as "not provided" and consumed by the option parser instead.
fn optional_positional<'a>(
    args: &'a [String],
    idx: usize,
    name: &str,
) -> anyhow::Result<Option<&'a str>> {
    match args.get(idx) {
        None => Ok(None),
        Some(v) if v.starts_with('-') && v.as_str() != "-" => anyhow::bail!(
            "expected optional positional <{}>, got flag-looking argument '{}'; \
             flags must come after positionals or be passed as --flag value",
            name,
            v
        ),
        Some(v) => Ok(Some(v.as_str())),
    }
}

fn parse_options(args: &[String]) -> anyhow::Result<CliOptions> {
    let mut opts = CliOptions::default();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--dry-run" {
            opts.dry_run = true;
            i += 1;
        } else if args[i] == "--auto-merge" {
            opts.auto_merge = true;
            i += 1;
        } else if args[i] == "--concise" {
            opts.concise = true;
            i += 1;
        } else if args[i] == "--json" {
            opts.json = true;
            i += 1;
        } else if args[i] == "--walk" {
            opts.walk = true;
            i += 1;
        } else if args[i] == "--only-decisions" {
            opts.only_decisions = true;
            i += 1;
        } else if args[i] == "--replace-tags" {
            opts.replace_tags = true;
            i += 1;
        } else if args[i] == "--once" {
            opts.once = true;
            i += 1;
        } else if args[i] == "--include-superseded" {
            opts.include_superseded = true;
            i += 1;
        } else if args[i] == "--apply" {
            opts.apply = true;
            i += 1;
        } else if args[i] == "--trace" {
            opts.trace = true;
            i += 1;
        } else if args[i] == "--config-dir" {
            // Multi-valued flag consumed by `watch-decisions`; skipped here so
            // it isn't rejected as unknown. The arm re-scans the raw args.
            if args.get(i + 1).is_none() {
                anyhow::bail!("--config-dir requires a value");
            }
            i += 2;
        } else if args[i].starts_with('-') && args[i] != "--data" {
            let key = args[i].as_str();
            let value = args
                .get(i + 1)
                .ok_or_else(|| anyhow::anyhow!("missing value for {}", key))?
                .clone();
            match key {
                "--agent" => opts.agent = Some(value),
                "--k" => opts.k = Some(value),
                "--depth" => opts.depth = Some(value),
                "--kind" => opts.kind = Some(value),
                "--project" => opts.project = Some(value),
                "--limit" => opts.limit = Some(value),
                "--offset" => opts.offset = Some(value),
                "--session" => opts.session = Some(value),
                "--type" => opts.event_type = Some(value),
                "--status" => opts.status = Some(value),
                "--scope" => opts.scope = Some(value),
                "--confidence" => opts.confidence = Some(value),
                "--stale-days" => opts.stale_days = Some(value),
                "--rewire-threshold" => opts.rewire_threshold = Some(value),
                "--broken-link-mode" => opts.broken_link_mode = Some(value),
                "--max-tokens" => opts.max_tokens = Some(value),
                "--atheneum-db" => opts.atheneum_db = Some(value),
                "--language" => opts.language = Some(value),
                "--tokens" => opts.tokens = Some(value),
                "--last" => opts.last = Some(value),
                "--direction" => opts.direction = Some(value),
                "--kinds" => opts.kinds = Some(value),
                "--role" => opts.role = Some(value),
                "--search" => opts.search = Some(value),
                "--importance" => opts.importance = Some(value),
                "--tags" => opts.tags = Some(value),
                "--id" => opts.id = Some(value),
                "--content" => opts.content = Some(value),
                "--interval" => opts.interval = Some(value),
                "--exclude-project" => opts.exclude_projects.push(value),
                other => anyhow::bail!("unknown option: {}", other),
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(opts)
}

fn parse_i64_arg(value: &str, name: &str) -> anyhow::Result<i64> {
    value
        .parse::<i64>()
        .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, value, e))
}

fn parse_u32_option(value: Option<&str>, name: &str) -> anyhow::Result<Option<u32>> {
    value
        .map(|s| {
            s.parse::<u32>()
                .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, s, e))
        })
        .transpose()
}

fn parse_u64_option(value: Option<&str>, name: &str) -> anyhow::Result<Option<u64>> {
    value
        .map(|s| {
            s.parse::<u64>()
                .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, s, e))
        })
        .transpose()
}

fn parse_usize_option(value: Option<&str>, name: &str) -> anyhow::Result<Option<usize>> {
    value
        .map(|s| {
            s.parse::<usize>()
                .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, s, e))
        })
        .transpose()
}

fn parse_i64_option(value: Option<&str>, name: &str) -> anyhow::Result<Option<i64>> {
    value
        .map(|s| {
            s.parse::<i64>()
                .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, s, e))
        })
        .transpose()
}

fn parse_f64_option(value: Option<&str>, name: &str) -> anyhow::Result<Option<f64>> {
    value
        .map(|s| {
            s.parse::<f64>()
                .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, s, e))
        })
        .transpose()
}

fn search_result_to_json(sr: &SearchResult) -> serde_json::Value {
    json!({
        "id": sr.id,
        "name": sr.name,
        "kind": sr.kind,
        "score": sr.score,
        "data": sr.data,
    })
}

fn wiki_page_summary_to_json(page: &WikiPage) -> serde_json::Value {
    json!({
        "id": page.id,
        "path": page.path,
        "title": page.title,
        "project_id": page.project_id,
        "wikilinks": page.wikilinks,
        "created_at": page.created_at,
    })
}

fn print_json(value: serde_json::Value) -> anyhow::Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

fn stdoutln(args: std::fmt::Arguments<'_>) -> anyhow::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_fmt(args)?;
    stdout.write_all(b"\n")?;
    Ok(())
}

fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .map(|io_err| io_err.kind() == io::ErrorKind::BrokenPipe)
            .unwrap_or(false)
    })
}

fn entity_to_json(entity: &GraphEntity) -> serde_json::Value {
    json!({
        "id": entity.id,
        "kind": entity.kind,
        "name": entity.name,
        "file_path": entity.file_path,
        "data": entity.data,
    })
}

fn edge_to_json(edge: &GraphEdge) -> serde_json::Value {
    json!({
        "id": edge.id,
        "from_id": edge.from_id,
        "to_id": edge.to_id,
        "edge_type": edge.edge_type,
        "data": edge.data,
    })
}

fn subgraph_to_json(sg: &atheneum::graph::SubgraphView) -> serde_json::Value {
    json!({
        "entry": entity_to_json(&sg.entry),
        "depth": sg.depth,
        "entities": sg.entities.iter().map(entity_to_json).collect::<Vec<_>>(),
        "edges": sg.edges.iter().map(edge_to_json).collect::<Vec<_>>(),
    })
}

fn print_navigate_concise(
    query: &str,
    views: &[atheneum::graph::SubgraphView],
    max_tokens: Option<usize>,
) -> anyhow::Result<()> {
    use std::collections::HashMap;
    let mut out = String::new();
    out.push_str(&format!("# navigate: {}\n\n", query));

    let Some(view) = views.first() else {
        out.push_str("_No matches found._\n");
        stdoutln(format_args!("{}", out))?;
        return Ok(());
    };

    let entry = &view.entry;
    out.push_str(&format!(
        "## {} `{}` ({})",
        entry.kind, entry.name, entry.id
    ));
    if let Some(fp) = &entry.file_path {
        out.push_str(&format!(" — `{}`", fp));
    }
    out.push('\n');

    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
    let mut incoming: HashMap<String, Vec<String>> = HashMap::new();
    for edge in &view.edges {
        let label = format!(
            "{} `{}`",
            if edge.from_id == entry.id {
                "→"
            } else {
                "←"
            },
            entity_name_in_view(
                view,
                if edge.from_id == entry.id {
                    edge.to_id
                } else {
                    edge.from_id
                }
            )
        );
        if edge.from_id == entry.id {
            outgoing
                .entry(edge.edge_type.clone())
                .or_default()
                .push(label);
        } else {
            incoming
                .entry(edge.edge_type.clone())
                .or_default()
                .push(label);
        }
    }

    if !outgoing.is_empty() {
        out.push_str("\n**outgoing**\n");
        for (ty, items) in &outgoing {
            out.push_str(&format!("- {}:\n", ty));
            for item in items.iter().take(5) {
                out.push_str(&format!("  {}\n", item));
            }
        }
    }
    if !incoming.is_empty() {
        out.push_str("\n**incoming**\n");
        for (ty, items) in &incoming {
            out.push_str(&format!("- {}:\n", ty));
            for item in items.iter().take(5) {
                out.push_str(&format!("  {}\n", item));
            }
        }
    }

    if views.len() > 1 {
        out.push_str(&format!(
            "\n_{} additional subgraphs omitted._\n",
            views.len() - 1
        ));
    }

    if let Some(budget) = max_tokens {
        let approx_chars = budget.saturating_mul(4);
        if out.len() > approx_chars {
            let trunc = &out[..approx_chars];
            out = format!("{}\n\n_[truncated to ~{} tokens]_\n", trunc, budget);
        }
    }

    stdoutln(format_args!("{}", out))?;
    Ok(())
}

fn entity_name_in_view(view: &atheneum::graph::SubgraphView, id: i64) -> String {
    view.entities
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| format!("entity:{}", id))
}

/// Extract a one-line content snippet for a thread entity.
///
/// ReasoningLog entities carry `content_summary` (transcript-sync) or
/// `content` (the `insert_reasoning_log` audit path); Discovery entities carry
/// `target` and optionally a `summary`/`file` in their metadata. Try each in
/// priority order and return the first non-empty string value.
fn entity_snippet(entity: &GraphEntity) -> Option<String> {
    let obj = entity.data.as_object()?;
    for key in ["content_summary", "content", "summary", "target"] {
        if let Some(serde_json::Value::String(s)) = obj.get(key) {
            if !s.is_empty() {
                return Some(s.clone());
            }
        }
    }
    None
}

fn truncate_snippet(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max_chars {
        return s.replace('\n', " ");
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out.replace('\n', " ")
}

fn print_chat(report: &atheneum::graph::ChatReport) -> anyhow::Result<()> {
    let mut out = String::new();
    out.push_str(&format!(
        "# chat: session `{}` ({})\n",
        report.session_id, report.direction
    ));
    out.push_str(&format!(
        "_tokens {}/{} · offset {} · has_more: {}_\n",
        report.token_total, report.token_budget, report.offset, report.has_more
    ));

    if !report.decisions.is_empty() {
        out.push_str("\n## decisions\n");
        for d in &report.decisions {
            let source = d.metadata.get("source").and_then(|v| v.as_str());
            let seq = d.metadata.get("sequence").and_then(|v| v.as_i64());
            out.push_str(&format!("- [{}] `{}`", d.id, d.target));
            if let Some(src) = source {
                out.push_str(&format!(" _src={}_", src));
            }
            if let Some(s) = seq {
                out.push_str(&format!(" seq={}", s));
            }
            out.push_str(&format!(" — {}", d.created_at));
            if let Some(chosen) = d.metadata.get("chosen").and_then(|v| v.as_str()) {
                out.push_str(&format!(
                    "\n    **chosen**: {}",
                    truncate_snippet(chosen, 200)
                ));
            }
            if let Some(rat) = d.metadata.get("rationale").and_then(|v| v.as_str()) {
                if !rat.trim().is_empty() {
                    out.push_str(&format!(
                        "\n    _rationale_: {}",
                        truncate_snippet(rat, 240)
                    ));
                }
            }
            if let Some(alts) = d.metadata.get("alternatives").and_then(|v| v.as_array()) {
                if !alts.is_empty() {
                    let list = alts
                        .iter()
                        .filter_map(|a| a.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    if !list.is_empty() {
                        out.push_str(&format!("\n    _alternatives_: {}", list));
                    }
                }
            }
            if let Some(why) = d.metadata.get("why").and_then(|v| v.as_str()) {
                if !why.trim().is_empty() {
                    out.push_str(&format!("\n    _why_: {}", truncate_snippet(why, 160)));
                }
            }
            if !d.chain.is_empty() {
                out.push_str(&format!("\n    _chain_ ({}): ", d.chain.len()));
                let labels: Vec<String> = d
                    .chain
                    .iter()
                    .take(8)
                    .map(|n| format!("[{}] {}", n.via, n.name))
                    .collect();
                out.push_str(&labels.join(" → "));
                if d.chain.len() > 8 {
                    out.push_str(" …");
                }
            }
            out.push('\n');
        }
        if report.turns.is_empty() {
            return stdoutln(format_args!("{}", out));
        }
    }

    if report.turns.is_empty() && report.decisions.is_empty() {
        out.push_str("\n_No chat turns or decisions found for this session._\n");
        return stdoutln(format_args!("{}", out));
    }

    out.push_str("\n## turns\n");
    for t in &report.turns {
        let tag = match t.role.as_deref() {
            Some(role) => role.to_string(),
            None => t.kind.clone(),
        };
        out.push_str(&format!(
            "\n**[{}]** seq={} kind=`{}` ({} tok)",
            tag,
            t.sequence
                .map(|s| s.to_string())
                .unwrap_or_else(|| "—".into()),
            t.kind,
            t.tokens
        ));
        let body = if t.content_text.trim().is_empty() {
            t.data
                .get("tool_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default()
        } else {
            t.content_text.trim().replace('\n', " ")
        };
        if !body.is_empty() {
            out.push_str(&format!("\n{}", truncate_snippet(&body, 200)));
        }
        if !t.chain.is_empty() {
            out.push_str("\n  _chain:_");
            for node in &t.chain {
                out.push_str(&format!(" [{}] `{}` ({})", node.id, node.kind, node.via));
            }
        }
        out.push('\n');
    }

    if report.has_more {
        out.push_str(&format!(
            "\n_…more rows beyond this window (offset {}, tokens {}/{})._\n",
            report.offset, report.token_total, report.token_budget
        ));
    }
    stdoutln(format_args!("{}", out))
}

fn print_thread(
    query: &str,
    views: &[atheneum::graph::SubgraphView],
    max_tokens: usize,
) -> anyhow::Result<()> {
    let mut out = String::new();
    out.push_str(&format!("# thread: {}\n\n", query));
    if views.is_empty() {
        out.push_str("_No decision-chain matches found._\n");
        stdoutln(format_args!("{}", out))?;
        return Ok(());
    }
    out.push_str(&format!(
        "_{} entry point(s) · depth up to {} · token budget ~{}_\n",
        views.len(),
        views.iter().map(|v| v.depth).max().unwrap_or(0),
        max_tokens
    ));

    for (vi, view) in views.iter().enumerate() {
        if vi > 0 {
            out.push_str("\n---\n\n");
        }
        let entry = &view.entry;
        out.push_str(&format!(
            "## entry [{}] {} — `{}`\n",
            entry.id, entry.kind, entry.name
        ));

        // Decision metadata block, same visual style as `chat --only-decisions`.
        if let Some(obj) = entry.data.as_object() {
            push_thread_decision_meta(obj, &mut out);
        }

        // Resolve endpoint names, including the entry itself (which is not in
        // `view.entities`). caused_by links to a lower id, led_to to a higher
        // id, so rendering each edge literally as `from --edge_type--> to`
        // reads in chronological order when scanned top-to-bottom.
        let name_for = |id: i64| -> String {
            if id == view.entry.id {
                return view.entry.name.clone();
            }
            view.entities
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.name.clone())
                .unwrap_or_else(|| format!("entity:{}", id))
        };

        if !view.edges.is_empty() {
            out.push_str(&format!("\n_chain_ ({} edge(s)):\n", view.edges.len()));
            for e in &view.edges {
                out.push_str(&format!(
                    "  [{}] {}  ──{}──>  [{}] {}\n",
                    e.from_id,
                    truncate_snippet(&name_for(e.from_id), 80),
                    e.edge_type,
                    e.to_id,
                    truncate_snippet(&name_for(e.to_id), 80)
                ));
            }
        }

        // BFS-expanded neighbors beyond the entry, for context. Skip the
        // snippet when it merely repeats the entity name (Decision `target`
        // often equals the name).
        let related: Vec<&GraphEntity> =
            view.entities.iter().filter(|e| e.id != entry.id).collect();
        if !related.is_empty() {
            out.push_str("\n_related_:\n");
            for e in related {
                out.push_str(&format!("  - [{}] {} `{}`", e.id, e.kind, e.name));
                if let Some(s) = entity_snippet(e) {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() && trimmed != e.name.trim() {
                        out.push_str(&format!(" — {}", truncate_snippet(&s, 140)));
                    }
                }
                out.push('\n');
            }
        }
    }

    let approx_chars = max_tokens.saturating_mul(4);
    if out.len() > approx_chars {
        let trunc = &out[..approx_chars];
        out = format!("{}\n\n_[truncated to ~{} tokens]_\n", trunc, max_tokens);
    }

    stdoutln(format_args!("{}", out))?;
    Ok(())
}

/// Append decision metadata (`source` / `sequence` / `chosen` / `rationale` /
/// `alternatives` / `why`) from a Discovery entity's JSON data, in the same
/// visual style as `chat --only-decisions`. No-op for non-decision entities.
fn push_thread_decision_meta(obj: &serde_json::Map<String, serde_json::Value>, out: &mut String) {
    let is_decision = obj
        .get("discovery_type")
        .and_then(|v| v.as_str())
        .map(|s| s == "Decision")
        .unwrap_or(false);
    if !is_decision {
        return;
    }
    let source = obj.get("source").and_then(|v| v.as_str());
    let seq = obj.get("sequence").and_then(|v| v.as_i64());
    if source.is_some() || seq.is_some() {
        out.push_str("  ");
        if let Some(src) = source {
            out.push_str(&format!("_src={}_ ", src));
        }
        if let Some(s) = seq {
            out.push_str(&format!("seq={}", s));
        }
        out.push('\n');
    }
    if let Some(chosen) = obj.get("chosen").and_then(|v| v.as_str()) {
        if !chosen.trim().is_empty() {
            out.push_str(&format!(
                "  **chosen**: {}\n",
                truncate_snippet(chosen, 200)
            ));
        }
    }
    if let Some(rat) = obj.get("rationale").and_then(|v| v.as_str()) {
        if !rat.trim().is_empty() {
            out.push_str(&format!("  _rationale_: {}\n", truncate_snippet(rat, 240)));
        }
    }
    if let Some(alts) = obj.get("alternatives").and_then(|v| v.as_array()) {
        if !alts.is_empty() {
            let list = alts
                .iter()
                .filter_map(|a| a.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            if !list.is_empty() {
                out.push_str(&format!("  _alternatives_: {}\n", list));
            }
        }
    }
    if let Some(why) = obj.get("why").and_then(|v| v.as_str()) {
        if !why.trim().is_empty() {
            out.push_str(&format!("  _why_: {}\n", truncate_snippet(why, 160)));
        }
    }
}

fn cross_result_to_json(hit: &atheneum::CrossSearchResult) -> serde_json::Value {
    json!({
        "project": hit.project,
        "id": hit.id,
        "kind": hit.kind,
        "name": hit.name,
        "file_path": hit.file_path,
        "data": hit.data,
    })
}

fn cross_edge_to_json(edge: &atheneum::CrossEdge) -> serde_json::Value {
    json!({
        "id": edge.id,
        "kind": edge.kind,
        "from_id": edge.from_id,
        "to_id": edge.to_id,
        "data": edge.data,
    })
}

fn cross_subgraph_to_json(view: &atheneum::CrossSubgraph) -> serde_json::Value {
    json!({
        "project": view.project,
        "entry_id": view.entry_id,
        "entities": view.entities.iter().map(cross_result_to_json).collect::<Vec<_>>(),
        "edges": view.edges.iter().map(cross_edge_to_json).collect::<Vec<_>>(),
    })
}

fn sync_logseq(
    graph: &AtheneumGraph,
    wiki_root: &std::path::Path,
    project_id: Option<&str>,
) -> anyhow::Result<(Vec<i64>, Vec<i64>)> {
    let mut page_ids = Vec::new();
    let mut journal_ids = Vec::new();
    let pages_dir = wiki_root.join("pages");
    let journals_dir = wiki_root.join("journals");

    if pages_dir.is_dir() {
        for path in markdown_files_recursive(&pages_dir)? {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("read {} failed: {}", path.display(), e))?;
            let rel_path = path
                .strip_prefix(wiki_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            page_ids.push(graph.ingest_wiki_page(&rel_path, &content, project_id)?);
        }
    }

    if journals_dir.is_dir() {
        for path in markdown_files_recursive(&journals_dir)? {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("read {} failed: {}", path.display(), e))?;
            let rel_path = path
                .strip_prefix(wiki_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            journal_ids.extend(graph.ingest_journal(&rel_path, &content, project_id)?);
        }
    }

    Ok((page_ids, journal_ids))
}

fn markdown_files_recursive(dir: &std::path::Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_markdown_files(dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_markdown_files(dir: &std::path::Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("read_dir {} failed: {}", dir.display(), e))?
    {
        let entry = entry.map_err(|e| anyhow::anyhow!("dir entry: {}", e))?;
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(&path, files)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BrokenPipeWriter;

    impl Write for BrokenPipeWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "pipe closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_usage_returns_broken_pipe_error() {
        let err = write_usage(BrokenPipeWriter).expect_err("broken pipe expected");
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn write_usage_mentions_observability_commands() {
        let mut buf = Vec::new();
        write_usage(&mut buf).expect("usage should render");
        let text = String::from_utf8(buf).expect("usage should be utf8");
        assert!(text.contains("session-trace <db-path> --session <id>"));
        assert!(text.contains("tool-usage <db-path> --session <id>"));
        assert!(text.contains("discoveries-recent <db-path>"));
        assert!(text.contains("handoffs-recent <db-path>"));
        assert!(text.contains("events-recent <db-path>"));
        assert!(text.contains("sessions-recent <db-path>"));
    }

    fn args(vec: &[&str]) -> Vec<String> {
        vec.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn positional_returns_value_when_present() {
        let a = args(&["atheneum", "init", "/tmp/x.db"]);
        assert_eq!(positional(&a, 2, "db-path").unwrap(), "/tmp/x.db");
    }

    #[test]
    fn positional_errors_when_missing() {
        let a = args(&["atheneum", "init"]);
        let err = positional(&a, 2, "db-path").expect_err("missing positional must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("missing required positional <db-path>"),
            "got: {msg}"
        );
    }

    #[test]
    fn positional_rejects_flag_looking_value() {
        // The original bug: `atheneum init --help` treated "--help" as the db path.
        let a = args(&["atheneum", "init", "--help"]);
        let err = positional(&a, 2, "db-path").expect_err("flag-looking positional must error");
        let msg = format!("{err}");
        assert!(msg.contains("expected positional <db-path>"), "got: {msg}");
        assert!(
            msg.contains("--help"),
            "message must name the offending arg: {msg}"
        );
    }

    #[test]
    fn positional_rejects_short_flag_looking_value() {
        let a = args(&["atheneum", "entity", "x.db", "-v"]);
        let err = positional(&a, 3, "entity-id").expect_err("short-flag positional must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("expected positional <entity-id>"),
            "got: {msg}"
        );
        assert!(msg.contains("-v"), "got: {msg}");
    }

    #[test]
    fn positional_allows_lone_dash() {
        let a = args(&["atheneum", "cmd", "-"]);
        assert_eq!(positional(&a, 2, "value").unwrap(), "-");
    }

    #[test]
    fn optional_positional_none_when_absent() {
        let a = args(&["atheneum", "sync-wiki", "x.db", "/wiki"]);
        assert!(optional_positional(&a, 4, "project-id").unwrap().is_none());
    }

    #[test]
    fn optional_positional_some_when_present() {
        let a = args(&["atheneum", "sync-wiki", "x.db", "/wiki", "atheneum"]);
        assert_eq!(
            optional_positional(&a, 4, "project-id").unwrap(),
            Some("atheneum")
        );
    }

    #[test]
    fn optional_positional_rejects_flag_looking_value() {
        let a = args(&["atheneum", "sync-wiki", "x.db", "/wiki", "--help"]);
        let err = optional_positional(&a, 4, "project-id")
            .expect_err("flag-looking optional positional must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("expected optional positional <project-id>"),
            "got: {msg}"
        );
        assert!(msg.contains("--help"), "got: {msg}");
    }
}
