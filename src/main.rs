use std::path::PathBuf;

use atheneum::AtheneumGraph;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> anyhow::Result<()> {
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
            println!("✅ Graph initialized successfully");
            println!(
                "   Health: {}",
                if graph.is_healthy() { "OK" } else { "BAD" }
            );
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
    println!("  init <db-path>     Initialize a new graph database");
    println!("  --version, -v      Print version information");
    println!("  help, --help, -h   Print this help message");
    println!();
    println!("EXAMPLES:");
    println!("  atheneum init ./atheneum.db");
}
