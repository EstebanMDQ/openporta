//! openporta CLI. Full subcommand set arrives in M3; the script runner is
//! already usable for headless renders.

mod script;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("script") => match args.get(1) {
            Some(path) => {
                let base = std::path::Path::new(path)
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .to_path_buf();
                match script::Runner::new(base).run_file(path) {
                    Ok(()) => std::process::ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("script failed: {e}");
                        std::process::ExitCode::FAILURE
                    }
                }
            }
            None => {
                eprintln!("usage: porta-app script <file.json>");
                std::process::ExitCode::FAILURE
            }
        },
        _ => {
            println!(
                "openporta {} - 4-track portastudio",
                env!("CARGO_PKG_VERSION")
            );
            println!("usage: porta-app script <file.json>");
            println!("       (new/render/export land in M3)");
            std::process::ExitCode::SUCCESS
        }
    }
}
