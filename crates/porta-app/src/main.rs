//! openporta CLI. Subcommands (new/script/render/export) arrive in M3;
//! until then this is a placeholder that prints usage.

fn main() {
    println!(
        "openporta {} - 4-track portastudio",
        env!("CARGO_PKG_VERSION")
    );
    println!("usage: porta-app <new|script|render|export> (coming in M3)");
}
