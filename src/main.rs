use std::io::{self, Write};
use std::path::PathBuf;

use atheneum::{AtheneumGraph, ClaudeTranscriptImportParams, GraphEdge, GraphEntity};
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
            let path = PathBuf::from(&args[2]);
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
            let db_path = PathBuf::from(&args[2]);
            let wiki_dir = PathBuf::from(&args[3]);
            let project_id = args.get(4).map(|s| s.as_str());

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
            let db_path = PathBuf::from(&args[2]);
            let journal_dir = PathBuf::from(&args[3]);
            let project_id = args.get(4).map(|s| s.as_str());

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
            let db_path = PathBuf::from(&args[2]);
            let wiki_root = PathBuf::from(&args[3]);
            let project_id = args.get(4).map(|s| s.as_str());

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
            let db_path = PathBuf::from(&args[2]);
            let transcript_path = PathBuf::from(&args[3]);
            let project = args.get(4).cloned();
            let agent_name = args.get(5).cloned().unwrap_or_else(|| "claude".into());
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
            let db_path = PathBuf::from(&args[2]);
            let path = &args[3];

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
        "query-journal" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum query-journal <db-path> <path>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(&args[2]);
            let path = &args[3];

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
            let db_path = PathBuf::from(&args[2]);
            let entity_id = parse_i64_arg(&args[3], "entity-id")?;
            let graph = AtheneumGraph::open(&db_path)?;
            print_json(entity_to_json(&graph.get_entity(entity_id)?))?;
        }
        "edge" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum edge <db-path> <edge-id>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(&args[2]);
            let edge_id = parse_i64_arg(&args[3], "edge-id")?;
            let graph = AtheneumGraph::open(&db_path)?;
            print_json(edge_to_json(&graph.get_edge(edge_id)?))?;
        }
        "neighbors" => {
            if args.len() < 4 {
                eprintln!("Usage: atheneum neighbors <db-path> <entity-id> [--depth N]");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(&args[2]);
            let entity_id = parse_i64_arg(&args[3], "entity-id")?;
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
                    "Usage: atheneum navigate <db-path> <query> [--k N] [--depth N] [--project P]"
                );
                std::process::exit(1);
            }
            let db_path = PathBuf::from(&args[2]);
            let query = &args[3];
            let opts = parse_options(&args[4..])?;
            let k = parse_usize_option(opts.k.as_deref(), "k")?.unwrap_or(5);
            let depth = parse_u32_option(opts.depth.as_deref(), "depth")?.unwrap_or(2);
            let graph = AtheneumGraph::open(&db_path)?;
            let views = graph.navigate(query, k, depth, opts.project.as_deref())?;
            print_json(json!({
                "query": query,
                "k": k,
                "depth": depth,
                "project": opts.project,
                "subgraphs": views.iter().map(subgraph_to_json).collect::<Vec<_>>(),
            }))?;
        }
        "graph-stats" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum graph-stats <db-path>");
                std::process::exit(1);
            }
            let db_path = PathBuf::from(&args[2]);
            let graph = AtheneumGraph::open(&db_path)?;
            let stats = graph.graph_stats()?;
            print_json(json!({
                "total_entities": stats.total_entities,
                "total_edges": stats.total_edges,
                "entity_counts": stats.entity_counts,
                "edge_counts": stats.edge_counts,
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
    writeln!(writer, "COMMANDS:")?;
    writeln!(
        writer,
        "  init <db-path>                          Initialize a new graph database"
    )?;
    writeln!(
        writer,
        "  sync-wiki <db-path> <dir> [project]     Ingest all .md files in directory as wiki pages"
    )?;
    writeln!(
        writer,
        "  sync-journal <db-path> <dir> [project]  Ingest all .md files in directory as journal sections"
    )?;
    writeln!(
        writer,
        "  sync-logseq <db-path> <root> [project]  Recursively ingest Logseq pages/ and journals/"
    )?;
    writeln!(
        writer,
        "  sync-claude-transcript <db> <jsonl> [project] [agent]  Import Claude Code transcript evidence"
    )?;
    writeln!(
        writer,
        "  query-wiki <db-path> <path>             Query a wiki page by path"
    )?;
    writeln!(
        writer,
        "  query-journal <db-path> <path>          Query journal sections by path"
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
        "  neighbors <db-path> <id> [--depth N]    Print one-hop edges or a BFS subgraph"
    )?;
    writeln!(
        writer,
        "  navigate <db-path> <query> [opts]       Search then walk graph subgraphs"
    )?;
    writeln!(
        writer,
        "  graph-stats <db-path>                   Print graph topology counts as JSON"
    )?;
    writeln!(
        writer,
        "  --version, -v                           Print version information"
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
        "  atheneum sync-claude-transcript ./atheneum.db ~/.claude/projects/-home-feanor-Projects-forge/<session>.jsonl forge claude"
    )?;
    writeln!(
        writer,
        "  atheneum query-wiki ./atheneum.db wiki/getting-started.md"
    )?;
    writeln!(
        writer,
        "  atheneum query-journal ./atheneum.db journal/2024-01-15.md"
    )?;
    writeln!(writer, "  atheneum graph-stats ./atheneum.db")?;
    writeln!(
        writer,
        "  atheneum navigate ./atheneum.db \"router construction\" --k 3 --depth 2 --project envoy"
    )?;
    Ok(())
}

#[derive(Default)]
struct CliOptions {
    k: Option<String>,
    depth: Option<String>,
    project: Option<String>,
}

fn parse_options(args: &[String]) -> anyhow::Result<CliOptions> {
    let mut opts = CliOptions::default();
    let mut i = 0;
    while i < args.len() {
        let key = args[i].as_str();
        let value = args
            .get(i + 1)
            .ok_or_else(|| anyhow::anyhow!("missing value for {}", key))?
            .clone();
        match key {
            "--k" => opts.k = Some(value),
            "--depth" => opts.depth = Some(value),
            "--project" => opts.project = Some(value),
            other => anyhow::bail!("unknown option: {}", other),
        }
        i += 2;
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

fn parse_usize_option(value: Option<&str>, name: &str) -> anyhow::Result<Option<usize>> {
    value
        .map(|s| {
            s.parse::<usize>()
                .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, s, e))
        })
        .transpose()
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
}
