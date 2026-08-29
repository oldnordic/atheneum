use serde_json::json;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::{AtheneumGraph, GraphEdge, GraphEntity, SearchResult, WikiPage};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn write_usage(mut writer: impl Write) -> io::Result<()> {
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
    writeln!(writer)?;
    writeln!(writer, "GROUNDED CLAIMS:")?;
    writeln!(
        writer,
        "  claim-pin <db> <entity-id> <project> <file-path> [--symbol <name>] [--id <receipt>]  Pin a falsifiable claim to source code"
    )?;
    writeln!(
        writer,
        "  claim-verify <db> <repo-root> [--project P] [--apply]  Audit and verify claims against live filesystem"
    )?;
    writeln!(
        writer,
        "  audit <db> [--project P]                Compute staleness and claim verification report"
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
        "  store-discovery <db> <agent> <type> <target> [meta.json] [--session ID] [--project P] [--dedup] [--force]  Store a discovery (--dedup skips a duplicate on agent+type+target+content_hash, and a duplicate Decision on session+target+source+chosen; --force bypasses)"
    )?;
    writeln!(
        writer,
        "  export-ledger <db> [--until RFC3339] [--kinds discoveries,memories,tasks]  Export ledger records as NDJSON to stdout (per-kind counts on stderr)"
    )?;
    writeln!(
        writer,
        "  import-ledger <db> <file.ndjson> [--dry-run] [--map PATH]  Import ledger NDJSON through the normal store paths, skipping records whose kind+agent+target+content_hash already exists (writes <file>.import-map.ndjson audit map)"
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
        "  dream-semantic <db> [--model M] [--ollama-url U] [--similarity-threshold F] [--swap-guard M] [--dry-run|--apply]"
    )?;
    writeln!(
        writer,
        "    Merge closely-related/redundant concepts via LLM (provider from config [llm]) with lexical fallback"
    )?;
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
pub struct CliOptions {
    pub agent: Option<String>,
    pub k: Option<String>,
    pub depth: Option<String>,
    pub kind: Option<String>,
    pub project: Option<String>,
    pub limit: Option<String>,
    pub offset: Option<String>,
    pub session: Option<String>,
    pub event_type: Option<String>,
    pub status: Option<String>,
    pub scope: Option<String>,
    pub confidence: Option<String>,
    pub max_tokens: Option<String>,
    pub atheneum_db: Option<String>,
    pub language: Option<String>,
    pub tokens: Option<String>,
    pub last: Option<String>,
    pub direction: Option<String>,
    pub kinds: Option<String>,
    pub role: Option<String>,
    pub search: Option<String>,
    pub importance: Option<String>,
    pub tags: Option<String>,
    pub id: Option<String>,
    pub content: Option<String>,
    pub walk: bool,
    pub only_decisions: bool,
    pub replace_tags: bool,
    pub dry_run: bool,
    pub auto_merge: bool,
    pub concise: bool,
    pub json: bool,
    pub once: bool,
    pub include_superseded: bool,
    pub include_wikilinks: bool,
    pub apply: bool,
    pub trace: bool,
    pub stale_days: Option<String>,
    pub port: Option<String>,
    pub budget: Option<String>,
    pub edge_limit: Option<String>,
    pub rewire_threshold: Option<String>,
    pub broken_link_mode: Option<String>,
    pub interval: Option<String>,
    pub exclude_projects: Vec<String>,
    pub model: Option<String>,
    pub ollama_url: Option<String>,
    pub similarity_threshold: Option<String>,
    pub swap_guard: Option<String>,
}

/// Read a required positional argument, rejecting flag-looking values.
///
/// Subcommand arms historically did `PathBuf::from(&args[2])` / `&args[3]`
/// directly, so a bare flag in a positional slot (e.g. `atheneum init --help`)
/// was silently accepted as the value — `--help` became the db path and a
/// SQLite file named `--help` got created. This guard fails fast with a clear
/// message instead. A lone `-` is allowed (stdin convention) even though no
/// atheneum positional currently uses it.
pub(crate) fn positional<'a>(
    args: &'a [String],
    idx: usize,
    name: &str,
) -> anyhow::Result<&'a str> {
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
pub(crate) fn optional_positional<'a>(
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

pub(crate) fn parse_options(args: &[String]) -> anyhow::Result<CliOptions> {
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
        } else if args[i] == "--include-wikilinks" {
            opts.include_wikilinks = true;
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
                "--budget" => opts.budget = Some(value),
                "--edge-limit" => opts.edge_limit = Some(value),
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
                "--symbol" | "--search" => opts.search = Some(value),
                "--importance" => opts.importance = Some(value),
                "--tags" => opts.tags = Some(value),
                "--id" => opts.id = Some(value),
                "--content" => opts.content = Some(value),
                "--port" => opts.port = Some(value),
                "--interval" => opts.interval = Some(value),
                "--exclude-project" => opts.exclude_projects.push(value),
                "--model" => opts.model = Some(value),
                "--ollama-url" => opts.ollama_url = Some(value),
                "--similarity-threshold" => opts.similarity_threshold = Some(value),
                "--swap-guard" => opts.swap_guard = Some(value),
                other => anyhow::bail!("unknown option: {}", other),
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(opts)
}

pub(crate) fn parse_i64_arg(value: &str, name: &str) -> anyhow::Result<i64> {
    value
        .parse::<i64>()
        .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, value, e))
}

pub(crate) fn parse_u32_option(value: Option<&str>, name: &str) -> anyhow::Result<Option<u32>> {
    value
        .map(|s| {
            s.parse::<u32>()
                .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, s, e))
        })
        .transpose()
}

pub(crate) fn parse_u64_option(value: Option<&str>, name: &str) -> anyhow::Result<Option<u64>> {
    value
        .map(|s| {
            s.parse::<u64>()
                .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, s, e))
        })
        .transpose()
}

pub(crate) fn parse_usize_option(value: Option<&str>, name: &str) -> anyhow::Result<Option<usize>> {
    value
        .map(|s| {
            s.parse::<usize>()
                .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, s, e))
        })
        .transpose()
}

pub(crate) fn parse_i64_option(value: Option<&str>, name: &str) -> anyhow::Result<Option<i64>> {
    value
        .map(|s| {
            s.parse::<i64>()
                .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, s, e))
        })
        .transpose()
}

pub(crate) fn parse_f64_option(value: Option<&str>, name: &str) -> anyhow::Result<Option<f64>> {
    value
        .map(|s| {
            s.parse::<f64>()
                .map_err(|e| anyhow::anyhow!("invalid {} '{}': {}", name, s, e))
        })
        .transpose()
}

pub(crate) fn search_result_to_json(sr: &SearchResult) -> serde_json::Value {
    json!({
        "id": sr.id,
        "name": sr.name,
        "kind": sr.kind,
        "score": sr.score,
        "data": sr.data,
    })
}

pub(crate) fn wiki_page_summary_to_json(page: &WikiPage) -> serde_json::Value {
    json!({
        "id": page.id,
        "path": page.path,
        "title": page.title,
        "project_id": page.project_id,
        "wikilinks": page.wikilinks,
        "created_at": page.created_at,
    })
}

pub(crate) fn print_json(value: serde_json::Value) -> anyhow::Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

pub(crate) fn stdoutln(args: std::fmt::Arguments<'_>) -> anyhow::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_fmt(args)?;
    stdout.write_all(b"\n")?;
    Ok(())
}

pub fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .map(|io_err| io_err.kind() == io::ErrorKind::BrokenPipe)
            .unwrap_or(false)
    })
}

pub(crate) fn entity_to_json(entity: &GraphEntity) -> serde_json::Value {
    json!({
        "id": entity.id,
        "kind": entity.kind,
        "name": entity.name,
        "file_path": entity.file_path,
        "data": entity.data,
    })
}

pub(crate) fn edge_to_json(edge: &GraphEdge) -> serde_json::Value {
    json!({
        "id": edge.id,
        "from_id": edge.from_id,
        "to_id": edge.to_id,
        "edge_type": edge.edge_type,
        "data": edge.data,
    })
}

pub(crate) fn navigate_entity_to_json(entity: &GraphEntity) -> serde_json::Value {
    let mut data = entity.data.clone();
    if let Some(obj) = data.as_object_mut() {
        if let Some(body) = obj.get("body").and_then(serde_json::Value::as_str) {
            if body.len() > 200 {
                let preview: String = body.chars().take(200).collect();
                obj.insert(
                    "body".to_string(),
                    json!(format!("{}… [truncated]", preview)),
                );
            }
        }
        if let Some(wikilinks) = obj.get("wikilinks").and_then(serde_json::Value::as_array) {
            if wikilinks.len() > 10 {
                let capped: Vec<serde_json::Value> = wikilinks.iter().take(10).cloned().collect();
                obj.insert("wikilinks".to_string(), json!(capped));
            }
        }
    }
    json!({
        "id": entity.id,
        "kind": entity.kind,
        "name": entity.name,
        "file_path": entity.file_path,
        "data": data,
    })
}

pub(crate) fn subgraph_to_json_bounded(
    sg: &crate::graph::SubgraphView,
    include_wikilinks: bool,
    edge_cap: usize,
    entity_cap: usize,
) -> serde_json::Value {
    let mut non_wikilinks = Vec::new();
    let mut wikilinks = Vec::new();
    for edge in &sg.edges {
        if edge.edge_type.eq_ignore_ascii_case("wikilink") {
            if include_wikilinks {
                wikilinks.push(edge);
            }
        } else {
            non_wikilinks.push(edge);
        }
    }

    // edge_cap is an upper bound, not a size: callers pass usize::MAX for
    // "unbounded", so capacity must be clamped to the edges that exist.
    let mut capped_edges = Vec::with_capacity(sg.edges.len().min(edge_cap));
    for e in non_wikilinks {
        if capped_edges.len() < edge_cap {
            capped_edges.push(edge_to_json(e));
        }
    }
    if include_wikilinks {
        for e in wikilinks {
            if capped_edges.len() < edge_cap {
                capped_edges.push(edge_to_json(e));
            }
        }
    }

    let capped_entities: Vec<serde_json::Value> = sg
        .entities
        .iter()
        .take(entity_cap)
        .map(navigate_entity_to_json)
        .collect();

    json!({
        "entry": navigate_entity_to_json(&sg.entry),
        "depth": sg.depth,
        "entities": capped_entities,
        "edges": capped_edges,
    })
}

pub(crate) fn subgraph_to_json(sg: &crate::graph::SubgraphView) -> serde_json::Value {
    subgraph_to_json_bounded(sg, true, usize::MAX, usize::MAX)
}

pub(crate) fn print_navigate_concise(
    query: &str,
    views: &[crate::graph::SubgraphView],
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

pub(crate) fn entity_name_in_view(view: &crate::graph::SubgraphView, id: i64) -> String {
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
pub(crate) fn entity_snippet(entity: &GraphEntity) -> Option<String> {
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

pub(crate) fn truncate_snippet(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max_chars {
        return s.replace('\n', " ");
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out.replace('\n', " ")
}

pub(crate) fn print_chat(report: &crate::graph::ChatReport) -> anyhow::Result<()> {
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

pub(crate) fn print_thread(
    query: &str,
    views: &[crate::graph::SubgraphView],
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
pub(crate) fn push_thread_decision_meta(
    obj: &serde_json::Map<String, serde_json::Value>,
    out: &mut String,
) {
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

pub(crate) fn cross_result_to_json(hit: &crate::CrossSearchResult) -> serde_json::Value {
    json!({
        "project": hit.project,
        "id": hit.id,
        "kind": hit.kind,
        "name": hit.name,
        "file_path": hit.file_path,
        "data": hit.data,
    })
}

pub(crate) fn cross_edge_to_json(edge: &crate::CrossEdge) -> serde_json::Value {
    json!({
        "id": edge.id,
        "kind": edge.kind,
        "from_id": edge.from_id,
        "to_id": edge.to_id,
        "data": edge.data,
    })
}

pub(crate) fn cross_subgraph_to_json(view: &crate::CrossSubgraph) -> serde_json::Value {
    json!({
        "project": view.project,
        "entry_id": view.entry_id,
        "entities": view.entities.iter().map(cross_result_to_json).collect::<Vec<_>>(),
        "edges": view.edges.iter().map(cross_edge_to_json).collect::<Vec<_>>(),
    })
}

pub(crate) fn sync_logseq(
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

pub(crate) fn markdown_files_recursive(dir: &std::path::Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_markdown_files(dir, &mut files)?;
    files.sort();
    Ok(files)
}

pub(crate) fn collect_markdown_files(
    dir: &std::path::Path,
    files: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
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
