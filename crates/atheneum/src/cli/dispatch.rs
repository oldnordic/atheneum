use serde_json::json;
use std::io;
use std::path::PathBuf;

use super::util::*;
use crate::{
    AtheneumGraph, ClaudeTranscriptImportParams, Config, CrossRouter, EdgeType, MetaRouter,
};

pub fn run(args: &[String]) -> anyhow::Result<()> {
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
            let path = PathBuf::from(positional(args, 2, "db-path")?);
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
        "claim-pin" => {
            if args.len() < 6 {
                eprintln!("Usage: atheneum claim-pin <db-path> <entity-id> <project> <file-path> [--search <symbol>] [--id <receipt-hash>]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let entity_id = parse_i64_arg(positional(args, 3, "entity-id")?, "entity-id")?;
            let project = positional(args, 4, "project")?;
            let file_path = positional(args, 5, "file-path")?;
            let opts = parse_options(&args[6..])?;

            let graph = AtheneumGraph::open(&db_path)?;
            let file_full = PathBuf::from(file_path);
            let hash = if file_full.exists() {
                crate::compute_file_sha256(&file_full)?
            } else {
                "".to_string()
            };

            let now = chrono::Utc::now().to_rfc3339();
            let claim_id = format!(
                "{}_{}_{}",
                project,
                entity_id,
                &crate::graph::hashing::compute_bytes_sha256(file_path.as_bytes())[..8]
            );
            let claim = crate::GroundedClaim {
                id: claim_id,
                entity_id,
                project: project.to_string(),
                file_path: file_path.to_string(),
                symbol_name: opts.search,
                ast_hash: hash,
                receipt_hash: opts.id,
                status: "verified".to_string(),
                created_at: now.clone(),
                last_verified_at: now,
            };

            graph.pin_grounded_claim(&claim)?;
            print_json(serde_json::to_value(&claim)?)?;
        }
        "claim-verify" => {
            if args.len() < 4 {
                eprintln!(
                    "Usage: atheneum claim-verify <db-path> <repo-root> [--project <id>] [--apply]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let repo_root = PathBuf::from(positional(args, 3, "repo-root")?);
            let opts = parse_options(&args[4..])?;
            let fix = opts.apply || !opts.dry_run;

            let graph = AtheneumGraph::open(&db_path)?;
            let project = opts.project.as_deref().unwrap_or("");
            let report = graph.verify_project_claims(&repo_root, project, fix)?;
            print_json(serde_json::to_value(&report)?)?;
        }
        "audit" | "audit-claims" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum audit <db-path> [--project <id>]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;

            let graph = AtheneumGraph::open(&db_path)?;
            let project = opts.project.as_deref().unwrap_or("");
            let report = graph.audit_claims(project)?;
            print_json(serde_json::to_value(&report)?)?;
        }
        "sync-wiki" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum sync-wiki <db-path> <wiki-dir> [project-id]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let wiki_dir = PathBuf::from(positional(args, 3, "wiki-dir")?);
            let project_id = optional_positional(args, 4, "project-id")?;

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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let journal_dir = PathBuf::from(positional(args, 3, "journal-dir")?);
            let project_id = optional_positional(args, 4, "project-id")?;

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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let wiki_root = PathBuf::from(positional(args, 3, "wiki-root")?);
            let project_id = optional_positional(args, 4, "project-id")?;

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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let transcript_path = PathBuf::from(positional(args, 3, "transcript-path")?);
            let project = optional_positional(args, 4, "project")?.map(|s| s.to_string());
            let agent_name = optional_positional(args, 5, "agent-name")?
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
                eprintln!(
                    "Usage: atheneum query-wiki <db-path> <path> [--offset N] [--limit N] [--json]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let path = positional(args, 3, "path")?;
            let opts = parse_options(&args[4..])?;
            let offset = parse_usize_option(opts.offset.as_deref(), "offset")?.unwrap_or(0);
            let limit = parse_usize_option(opts.limit.as_deref(), "limit")?.unwrap_or(8192);

            let graph = AtheneumGraph::open(&db_path)?;
            match graph.get_wiki_page(path)? {
                Some(page) => {
                    let (body_slice, truncated, total_bytes) =
                        crate::graph::paginate_body(&page.body, offset, limit);
                    if opts.json {
                        print_json(json!({
                            "id": page.id,
                            "path": page.path,
                            "title": page.title,
                            "content_hash": page.content_hash,
                            "wikilinks": page.wikilinks,
                            "project_id": page.project_id,
                            "metadata": page.metadata,
                            "created_at": page.created_at,
                            "updated_at": page.updated_at,
                            "offset": offset,
                            "limit": limit,
                            "total_bytes": total_bytes,
                            "truncated": truncated,
                            "has_more": truncated,
                            "body": body_slice,
                        }))?;
                    } else {
                        stdoutln(format_args!("WikiPage: {}", page.path))?;
                        if let Some(title) = &page.title {
                            stdoutln(format_args!("  Title: {}", title))?;
                        }
                        stdoutln(format_args!("  Content hash: {:?}", page.content_hash))?;
                        stdoutln(format_args!("  Wikilinks: {:?}", page.wikilinks))?;
                        stdoutln(format_args!("  Project: {:?}", page.project_id))?;
                        stdoutln(format_args!("  Created: {}", page.created_at))?;
                        stdoutln(format_args!("  Updated: {:?}", page.updated_at))?;
                        stdoutln(format_args!("  Total bytes: {}", total_bytes))?;
                        stdoutln(format_args!("  Offset: {}, Limit: {}", offset, limit))?;
                        stdoutln(format_args!("  Body:"))?;
                        stdoutln(format_args!("{}", body_slice))?;
                        if truncated {
                            let next_offset = offset.saturating_add(body_slice.len());
                            stdoutln(format_args!("\n[truncated: showing bytes {}..{} of {}. Pass --offset {} to page through]", offset, next_offset, total_bytes, next_offset))?;
                        }
                    }
                }
                None => {
                    if opts.json {
                        print_json(json!({ "found": false, "path": path }))?;
                    } else {
                        stdoutln(format_args!("No wiki page found at path: {}", path))?;
                    }
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let query = positional(args, 3, "query")?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let query = positional(args, 3, "query")?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let path = positional(args, 3, "path")?;

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
                        section.body.chars().take(200).collect::<String>()
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let entity_id = parse_i64_arg(positional(args, 3, "entity-id")?, "entity-id")?;
            let graph = AtheneumGraph::open(&db_path)?;
            print_json(entity_to_json(&graph.get_entity(entity_id)?))?;
        }
        "edge" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum edge <db-path> <edge-id>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let edge_id = parse_i64_arg(positional(args, 3, "edge-id")?, "edge-id")?;
            let graph = AtheneumGraph::open(&db_path)?;
            print_json(edge_to_json(&graph.get_edge(edge_id)?))?;
        }
        "neighbors" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum neighbors <db-path> <entity-id> [--depth N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let entity_id = parse_i64_arg(positional(args, 3, "entity-id")?, "entity-id")?;
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
                    "Usage: atheneum navigate <db-path> <query> [--k N] [--depth N] [--project P] [--kind K] [--max-tokens N] [--budget N] [--edge-limit N] [--include-wikilinks] [--concise]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let query = positional(args, 3, "query")?;
            let opts = parse_options(&args[4..])?;
            let k = parse_usize_option(opts.k.as_deref(), "k")?.unwrap_or(5);
            let depth = parse_u32_option(opts.depth.as_deref(), "depth")?.unwrap_or(2);
            let max_tokens = parse_usize_option(opts.max_tokens.as_deref(), "max-tokens")?;
            let budget = parse_usize_option(opts.budget.as_deref(), "budget")?.unwrap_or(8192);
            let edge_limit =
                parse_usize_option(opts.edge_limit.as_deref(), "edge-limit")?.unwrap_or(50);
            let include_wikilinks = opts.include_wikilinks;
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
                let serialized_subgraphs: Vec<serde_json::Value> = views
                    .iter()
                    .map(|sg| subgraph_to_json_bounded(sg, include_wikilinks, edge_limit, 50))
                    .collect();

                let mut out_subgraphs = Vec::new();
                let mut truncated = false;

                for sg in serialized_subgraphs {
                    let mut candidate_list = out_subgraphs.clone();
                    candidate_list.push(sg.clone());
                    let candidate = json!({
                        "query": query,
                        "k": k,
                        "depth": depth,
                        "kind": opts.kind,
                        "project": opts.project,
                        "max_tokens": max_tokens,
                        "plan": plan,
                        "trace_id": trace_id,
                        "subgraphs": candidate_list,
                        "truncated": truncated || (out_subgraphs.len() + 1 < views.len()),
                    });
                    let size = serde_json::to_string(&candidate)
                        .map(|s| s.len())
                        .unwrap_or(usize::MAX);
                    if size <= budget {
                        out_subgraphs.push(sg);
                    } else {
                        truncated = true;
                        if out_subgraphs.is_empty() {
                            let mut trimmed_sg = sg.clone();
                            if let Some(edges) = trimmed_sg
                                .get_mut("edges")
                                .and_then(serde_json::Value::as_array_mut)
                            {
                                edges.clear();
                            }
                            if let Some(entities) = trimmed_sg
                                .get_mut("entities")
                                .and_then(serde_json::Value::as_array_mut)
                            {
                                entities.truncate(5);
                            }
                            let fallback = json!({
                                "query": query,
                                "k": k,
                                "depth": depth,
                                "kind": opts.kind,
                                "project": opts.project,
                                "max_tokens": max_tokens,
                                "plan": plan,
                                "trace_id": trace_id,
                                "subgraphs": [trimmed_sg.clone()],
                                "truncated": true,
                            });
                            let fallback_size = serde_json::to_string(&fallback)
                                .map(|s| s.len())
                                .unwrap_or(usize::MAX);
                            if fallback_size <= budget {
                                out_subgraphs.push(trimmed_sg);
                            }
                        }
                        break;
                    }
                }

                if out_subgraphs.len() < views.len() {
                    truncated = true;
                }

                print_json(json!({
                    "query": query,
                    "k": k,
                    "depth": depth,
                    "kind": opts.kind,
                    "project": opts.project,
                    "max_tokens": max_tokens,
                    "plan": plan,
                    "trace_id": trace_id,
                    "subgraphs": out_subgraphs,
                    "truncated": truncated,
                }))?;
            }
        }
        "trace-get" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum trace-get <db-path> --id N");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let id = parse_i64_option(opts.id.as_deref(), "id")?
                .ok_or_else(|| anyhow::anyhow!("missing --id"))?;
            let graph = AtheneumGraph::open(&db_path)?;

            let trace_entity = graph.with_raw_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT data FROM graph_entities WHERE id = ?1 AND kind = 'QueryTrace'",
                )?;
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
        "pin" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum pin <db-path> --id N");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let id = parse_i64_option(opts.id.as_deref(), "id")?
                .ok_or_else(|| anyhow::anyhow!("missing --id"))?;
            let graph = AtheneumGraph::open(&db_path)?;
            graph.pin_entity(id)?;
            println!("Pinned entity {}", id);
        }
        "unpin" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum unpin <db-path> --id N");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let id = parse_i64_option(opts.id.as_deref(), "id")?
                .ok_or_else(|| anyhow::anyhow!("missing --id"))?;
            let graph = AtheneumGraph::open(&db_path)?;
            graph.unpin_entity(id)?;
            println!("Unpinned entity {}", id);
        }
        "thread" => {
            if args.len() < 4 {
                eprintln!(
                    "Usage: atheneum thread <db-path> <query> [--tokens T] [--depth D] [--k N] [--project P] [--json]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let query = positional(args, 3, "query")?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let session_id = opts
                .session
                .clone()
                .ok_or_else(|| anyhow::anyhow!("--session <id> is required"))?;
            let direction = match opts.direction.as_deref() {
                Some(d) => crate::graph::ChatDirection::parse(d)?,
                None => crate::graph::ChatDirection::Recent,
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
            let query = crate::graph::ChatQuery {
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let agent = positional(args, 3, "agent")?;
            let discovery_type = positional(args, 4, "type")?;
            let target = positional(args, 5, "target")?;
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
            // General content-hash dedup guard (all discovery types). Skips
            // when an identical (agent, type, target, content_hash) Discovery
            // already exists and reports the existing id. This is the guard
            // bulk import paths rely on; probe-verified broken before the
            // ledger reconciliation fix (identical payload stored 3x produced
            // 3 discoveries).
            if dedup && !force {
                if let Some(existing_id) = graph.find_discovery_by_content_hash(
                    agent,
                    discovery_type,
                    target,
                    &metadata,
                )? {
                    print_json(json!({
                        "discovery_id": existing_id,
                        "deduped": true,
                        "agent": agent,
                        "type": discovery_type,
                        "target": target,
                    }))?;
                    return Ok(());
                }
            }
            let id = graph.store_discovery(agent, discovery_type, target, metadata)?;
            print_json(
                json!({"discovery_id": id, "agent": agent, "type": discovery_type, "target": target}),
            )?;
        }
        "export-ledger" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum export-ledger <db-path> [--until <rfc3339>] [--kinds discoveries,memories,tasks]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let mut until: Option<String> = None;
            let mut kinds = crate::graph::LedgerKind::all();
            let mut i = 3;
            while i < args.len() {
                match args[i].as_str() {
                    "--until" => {
                        let value = args
                            .get(i + 1)
                            .ok_or_else(|| anyhow::anyhow!("--until requires a value"))?;
                        until = Some(value.clone());
                        i += 2;
                    }
                    "--kinds" => {
                        let value = args
                            .get(i + 1)
                            .ok_or_else(|| anyhow::anyhow!("--kinds requires a value"))?;
                        kinds = crate::graph::LedgerKind::parse_list(value)?;
                        i += 2;
                    }
                    other => anyhow::bail!("unknown export-ledger option: {}", other),
                }
            }
            let graph = AtheneumGraph::open(&db_path)?;
            let stdout = io::stdout();
            let counts =
                crate::graph::export_ledger(&graph, &kinds, until.as_deref(), stdout.lock())?;
            let summary = counts
                .iter()
                .map(|(kind, n)| format!("{}={}", kind, n))
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("export-ledger: {}", summary);
        }
        "import-ledger" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum import-ledger <db-path> <file.ndjson> [--dry-run] [--map <path>]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let file = PathBuf::from(positional(args, 3, "file.ndjson")?);
            let mut dry_run = false;
            let mut map_path: Option<PathBuf> = None;
            let mut i = 4;
            while i < args.len() {
                match args[i].as_str() {
                    "--dry-run" => {
                        dry_run = true;
                        i += 1;
                    }
                    "--map" => {
                        let value = args
                            .get(i + 1)
                            .ok_or_else(|| anyhow::anyhow!("--map requires a value"))?;
                        map_path = Some(PathBuf::from(value));
                        i += 2;
                    }
                    other => anyhow::bail!("unknown import-ledger option: {}", other),
                }
            }
            let map_path = map_path.unwrap_or_else(|| {
                let mut name = file.clone().into_os_string();
                name.push(".import-map.ndjson");
                PathBuf::from(name)
            });
            let graph = AtheneumGraph::open(&db_path)?;
            let map_file = std::fs::File::create(&map_path)
                .map_err(|e| anyhow::anyhow!("create map file {}: {}", map_path.display(), e))?;
            let counts =
                crate::graph::import_ledger(&graph, &file, dry_run, io::BufWriter::new(map_file))?;
            print_json(json!({
                "merged": counts.merged,
                "skipped": counts.skipped,
                "failed": counts.failed,
                "dry_run": dry_run,
                "map_file": map_path.display().to_string(),
            }))?;
        }
        "add-edge" => {
            if args.len() < 6 {
                eprintln!("Usage: atheneum add-edge <db-path> <from-id> <to-id> <edge-type> [metadata.json|--data 'json']");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let from_id = parse_i64_arg(positional(args, 3, "from-id")?, "from-id")?;
            let to_id = parse_i64_arg(positional(args, 4, "to-id")?, "to-id")?;
            let edge_type_str = positional(args, 5, "edge-type")?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let title = positional(args, 3, "title")?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;
            let tasks: Vec<serde_json::Value> = if let Some(status_str) = &opts.status {
                let status = crate::graph::KanbanStatus::parse(status_str).ok_or_else(|| {
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
            let mut response = json!({"tasks": tasks, "count": tasks.len()});
            if tasks.is_empty() {
                if let Some(ref p) = opts.project {
                    let known_projects = graph.list_distinct_projects()?;
                    if !known_projects.iter().any(|k| k == p) {
                        response["unknown_project"] = json!(true);
                        response["known_projects"] = json!(known_projects);
                        response["hint"] = json!(if known_projects.is_empty() {
                            format!("project '{}' matches no recorded project (no projects recorded in database)", p)
                        } else {
                            format!(
                                "project '{}' matches no recorded project. Known projects: {}",
                                p,
                                known_projects.join(", ")
                            )
                        });
                    }
                }
            }
            print_json(response)?;
        }
        "task-update" => {
            if args.len() < 5 {
                eprintln!("Usage: atheneum task-update <db-path> <task-id> <status>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let task_id = parse_i64_arg(positional(args, 3, "task-id")?, "task-id")?;
            let status_str = positional(args, 4, "status")?;
            let status = crate::graph::KanbanStatus::parse(status_str).ok_or_else(|| {
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let task_id = parse_i64_arg(positional(args, 3, "task-id")?, "task-id")?;
            let graph = AtheneumGraph::open(&db_path)?;
            graph.update_task_status(task_id, crate::graph::KanbanStatus::Done)?;
            print_json(json!({"task_id": task_id, "status": "DONE"}))?;
        }
        "task-archive" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum task-archive <db-path> <task-id>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let task_id = parse_i64_arg(positional(args, 3, "task-id")?, "task-id")?;
            let graph = AtheneumGraph::open(&db_path)?;
            graph.update_task_status(task_id, crate::graph::KanbanStatus::Archived)?;
            print_json(json!({"task_id": task_id, "status": "ARCHIVED"}))?;
        }
        "search" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum search <db-path> <query> [--k N] [--project P] [--max-tokens N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let query = positional(args, 3, "query")?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let query = positional(args, 3, "query")?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let target = positional(args, 3, "target")?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let mut response = json!({
                "count": discoveries.len(),
                "project": opts.project,
                "agent": opts.agent,
                "session": opts.session,
                "type": opts.event_type,
                "discoveries": discoveries.iter().map(entity_to_json).collect::<Vec<_>>(),
            });
            if discoveries.is_empty() {
                if let Some(ref p) = opts.project {
                    let known_projects = graph.list_distinct_projects()?;
                    if !known_projects.iter().any(|k| k == p) {
                        response["unknown_project"] = json!(true);
                        response["known_projects"] = json!(known_projects);
                        response["hint"] = json!(if known_projects.is_empty() {
                            format!("project '{}' matches no recorded project (no projects recorded in database)", p)
                        } else {
                            format!(
                                "project '{}' matches no recorded project. Known projects: {}",
                                p,
                                known_projects.join(", ")
                            )
                        });
                    }
                }
            }
            print_json(response)?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let default_config = crate::graph::WatchConfig::default();
            let config_dirs = if dirs.is_empty() {
                default_config.config_dirs.clone()
            } else {
                dirs
            };
            let config = crate::graph::WatchConfig {
                config_dirs,
                interval,
                project: opts.project.clone(),
                agent: opts.agent.clone().unwrap_or_else(|| "claude".to_string()),
                dry_run: opts.dry_run,
                once: opts.once,
            };
            let graph = AtheneumGraph::open(&db_path)?;
            let stats = crate::graph::watch_decisions(&graph, &config)?;
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
            let mut config = crate::graph::ExtractConfig {
                db: PathBuf::from(positional(args, 2, "db-path")?),
                ..crate::graph::ExtractConfig::default()
            };
            let mut json_out = false;
            let mut i = 3;
            while i < args.len() {
                match args[i].as_str() {
                    "--all" => config.all = true,
                    "--dry-run" => config.dry_run = true,
                    "--force" => config.force = true,
                    "--verbose" => config.verbose = true,
                    "--heuristic" => config.mode = crate::graph::ExtractMode::Heuristic,
                    "--mode" => {
                        let v = args
                            .get(i + 1)
                            .ok_or_else(|| anyhow::anyhow!("--mode requires a value"))?;
                        config.mode = match v.as_str() {
                            "llm" | "ollama" => crate::graph::ExtractMode::Llm,
                            "heuristic" | "rules" | "no-llm" => {
                                crate::graph::ExtractMode::Heuristic
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
            let stats = crate::graph::run_extract(&config)?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let key = positional(args, 3, "key")?;
            let content = positional(args, 4, "content")?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let id_str = opts
                .id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("--id <N> is required"))?;
            let id = parse_i64_arg(id_str, "id")?;
            let patch = crate::MemoryPatch {
                content: opts.content.clone(),
                importance: opts
                    .importance
                    .as_deref()
                    .map(|s| {
                        s.parse::<i64>()
                            .map_err(|e| anyhow::anyhow!("invalid --importance '{}': {}", s, e))
                    })
                    .transpose()?,
                tags: opts.tags.as_deref().map(|s| {
                    s.split(',')
                        .map(|p| p.trim().to_string())
                        .filter(|p| !p.is_empty())
                        .collect()
                }),
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let key = positional(args, 3, "key")?;
            let opts = parse_options(&args[4..])?;
            let graph = AtheneumGraph::open(&db_path)?;
            let items = graph.query_memory(
                key,
                opts.scope.as_deref(),
                opts.project.as_deref(),
                opts.include_superseded,
            )?;
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;

            use crate::{DreamConfig, DreamMode, DreamReport};
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
        "dream-semantic" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum dream-semantic <db-path> [--model M] [--ollama-url U] [--similarity-threshold F] [--swap-guard <strict|adapt|fallback>] [--dry-run|--apply]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;

            // Base LLM settings come from ~/.config/atheneum/config.toml
            // [llm] (provider, base_url, model, api_key); flag overrides below.
            let mut config = match crate::load_config() {
                Ok(cfg) => crate::ConsolidationConfig::from_llm_config(&cfg.llm),
                Err(_) => crate::ConsolidationConfig::default(),
            };
            if let Some(model) = opts.model {
                config.model = model;
            }
            if let Some(ollama_url) = opts.ollama_url {
                // Explicit --ollama-url selects the legacy ollama path.
                config.provider = crate::LlmProvider::Ollama;
                config.ollama_url = ollama_url;
            }
            if let Some(t) = opts.similarity_threshold.as_deref() {
                config.similarity_threshold = t
                    .parse::<f64>()
                    .map_err(|e| anyhow::anyhow!("invalid similarity-threshold '{}': {}", t, e))?;
            }
            if let Some(sg) = opts.swap_guard.as_deref() {
                config.swap_guard = match sg {
                    "strict" => crate::SwapGuardMode::Strict,
                    "adapt" => crate::SwapGuardMode::Adapt,
                    "fallback" => crate::SwapGuardMode::Fallback,
                    other => anyhow::bail!(
                        "unknown swap-guard '{}' (expected strict|adapt|fallback)",
                        other
                    ),
                };
            }
            // Default to dry-run unless --apply is passed explicitly.
            config.dry_run = !opts.apply;

            let report = graph.semantic_consolidation(&config)?;
            print_json(serde_json::to_value(&report)?)?;
        }
        "wiki-dream" => {
            if args.len() < 3 {
                eprintln!(
                    "Usage: atheneum wiki-dream <db-path> [--project P] [--dry-run|--auto-merge]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;

            use crate::{DreamConfig, DreamMode, DreamReport};
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
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;
            let stale_superseded_days =
                parse_i64_option(opts.stale_days.as_deref(), "stale-days")?.unwrap_or(30);
            let report = graph.lint_graph(&crate::LintConfig {
                stale_superseded_days,
            })?;
            print_json(serde_json::to_value(&report)?)?;
        }
        "maintain" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum maintain <db-path> [--apply] [--stale-days N] [--rewire-threshold F] [--broken-link-mode <stub|sever>]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let graph = AtheneumGraph::open(&db_path)?;
            let stale_superseded_days =
                parse_i64_option(opts.stale_days.as_deref(), "stale-days")?.unwrap_or(30);
            let rewire_threshold = opts
                .rewire_threshold
                .as_deref()
                .map(|s| {
                    s.parse::<f64>()
                        .map_err(|e| anyhow::anyhow!("invalid rewire-threshold '{}': {}", s, e))
                })
                .transpose()?
                .unwrap_or(0.3);
            let broken_link_mode = match opts.broken_link_mode.as_deref() {
                Some("sever") => crate::BrokenLinkMode::Sever,
                _ => crate::BrokenLinkMode::Stub,
            };
            let apply = opts.apply && !opts.dry_run;
            let report = graph.maintain(
                &crate::MaintainConfig {
                    rewire_threshold,
                    broken_link_mode,
                    stale_superseded_days,
                },
                apply,
            )?;
            print_json(serde_json::to_value(&report)?)?;
        }
        "seed-memory" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum seed-memory <db-path> [--project P] [--tokens N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let tokens = parse_usize_option(opts.tokens.as_deref(), "tokens")?.unwrap_or(800);
            let graph = AtheneumGraph::open(&db_path)?;
            let seed = graph.seed_memory(opts.project.as_deref(), tokens)?;
            print_json(serde_json::to_value(&seed)?)?;
        }
        "models-list" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum models-list <db-path>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let graph = AtheneumGraph::open(&db_path)?;
            let models = graph.discover_available_models()?;
            print_json(serde_json::json!({
                "models": models,
                "count": models.len()
            }))?;
        }
        "dashboard" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum dashboard <db-path> [--port N]");
                std::process::exit(1);
            }
            let _db_path = PathBuf::from(positional(args, 2, "db-path")?);
            let opts = parse_options(&args[3..])?;
            let _port = parse_u32_option(opts.port.as_deref(), "port")?.unwrap_or(8080) as u16;

            #[cfg(feature = "web-ui")]
            {
                let graph = AtheneumGraph::open(&_db_path)?;
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(crate::web_ui::start_web_server(graph, _port))?;
            }
            #[cfg(not(feature = "web-ui"))]
            {
                anyhow::bail!(
                    "Atheneum must be compiled with --features web-ui to run the dashboard server."
                );
            }
        }
        "config" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum config <init|show> [args]");
                std::process::exit(1);
            }
            match args[2].as_str() {
                "init" => {
                    let force = args.iter().any(|a| a == "--force" || a == "-f");
                    let path = crate::default_config_path();
                    if path.exists() && !force {
                        stdoutln(format_args!(
                            "Config already exists at {}. Use --force to overwrite.",
                            path.display()
                        ))?;
                        return Ok(());
                    }
                    let cfg = Config::default();
                    crate::save_config(&cfg)?;
                    stdoutln(format_args!("Created default config at {}", path.display()))?;
                }
                "show" => {
                    let cfg = crate::load_config()?;
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
            let name = positional(args, 2, "name")?;
            let root_path = positional(args, 3, "root-path")?;
            let magellan_db = positional(args, 4, "magellan-db")?;
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
            let query = positional(args, 2, "query")?;
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
            let query = positional(args, 2, "query")?;
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
