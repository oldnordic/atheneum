use std::path::PathBuf;

use atheneum::AtheneumGraph;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> anyhow::Result<()> {
    // nosemgrep: rust.lang.security.args.args
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    match args[1].as_str() {
        "--version" | "-v" => {
            println!("atheneum v{}", VERSION);
        }
        "help" | "--help" | "-h" => {
            print_usage();
        }
        "init" => {
            if args.len() < 3 {
                eprintln!("Usage: atheneum init <db-path>");
                std::process::exit(1);
            }
            let path = PathBuf::from(&args[2]);
            println!("Initializing Atheneum graph at: {}", path.display());
            let graph = AtheneumGraph::open(&path)?;
            println!("Graph initialized successfully");
            println!(
                "   Health: {}",
                if graph.is_healthy() { "OK" } else { "BAD" }
            );
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
            println!(
                "Synced {} wiki pages from {}",
                ids.len(),
                wiki_dir.display()
            );
            for id in ids {
                println!("  -> graph entity id: {}", id);
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
            println!(
                "Synced {} journal sections from {}",
                ids.len(),
                journal_dir.display()
            );
            for id in ids {
                println!("  -> graph entity id: {}", id);
            }
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
                    println!("WikiPage: {}", page.path);
                    if let Some(title) = &page.title {
                        println!("  Title: {}", title);
                    }
                    println!("  Content hash: {:?}", page.content_hash);
                    println!("  Wikilinks: {:?}", page.wikilinks);
                    println!("  Project: {:?}", page.project_id);
                    println!("  Created: {}", page.created_at);
                    println!("  Updated: {:?}", page.updated_at);
                    println!("  Body (first 500 chars):");
                    let preview: String = page.body.chars().take(500).collect();
                    println!("{}", preview);
                }
                None => {
                    println!("No wiki page found at path: {}", path);
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
                println!("No journal sections found at path: {}", path);
            } else {
                println!("Journal sections for {}:", path);
                for section in sections {
                    println!(
                        "\n  [{}] {}",
                        section.time.as_deref().unwrap_or("?"),
                        section.title
                    );
                    println!(
                        "    Body (first 200 chars): {}",
                        &section.body.chars().take(200).collect::<String>()
                    );
                    if !section.kanban_updates.is_empty() {
                        println!("    Kanban updates:");
                        for update in &section.kanban_updates {
                            println!("      '{}' -> {:?}", update.task_title, update.new_status);
                        }
                    }
                }
            }
        }
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            print_usage();
            std::process::exit(1);
        }
    }

    Ok(())
}

fn print_usage() {
    println!("Atheneum v{} - Agent Coordination Graph Database", VERSION);
    println!();
    println!("USAGE:");
    println!("  atheneum <command> [args]");
    println!();
    println!("COMMANDS:");
    println!("  init <db-path>                          Initialize a new graph database");
    println!(
        "  sync-wiki <db-path> <dir> [project]     Ingest all .md files in directory as wiki pages"
    );
    println!("  sync-journal <db-path> <dir> [project]  Ingest all .md files in directory as journal sections");
    println!("  query-wiki <db-path> <path>             Query a wiki page by path");
    println!("  query-journal <db-path> <path>          Query journal sections by path");
    println!("  --version, -v                           Print version information");
    println!("  help, --help, -h                        Print this help message");
    println!();
    println!("EXAMPLES:");
    println!("  atheneum init ./atheneum.db");
    println!("  atheneum sync-wiki ./atheneum.db ./wiki");
    println!("  atheneum sync-journal ./atheneum.db ./journal my-project");
    println!("  atheneum query-wiki ./atheneum.db wiki/getting-started.md");
    println!("  atheneum query-journal ./atheneum.db journal/2024-01-15.md");
}
