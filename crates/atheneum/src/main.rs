use atheneum::cli;

fn main() {
    // nosemgrep: rust.lang.security.args.args
    let args: Vec<String> = std::env::args().collect();
    if let Err(err) = cli::run(&args) {
        if cli::is_broken_pipe(&err) {
            std::process::exit(0);
        }
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
