//! Manual smoke test for `session::run_command`.
//!
//! Exercises the degrade path (daemon not installed -> warning on stderr,
//! session still proceeds) and exit-code propagation from the child process,
//! without needing root or a TTY.
//!
//! Run with `cargo run --example smoke_session -- bash -c 'exit 7'` and
//! confirm it prints the degrade warning on stderr and exits with code 7.
//! Redirect stdin from `/dev/null` (or pipe it) so the "install the daemon?"
//! prompt doesn't block waiting for interactive input.

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let argv = if argv.is_empty() {
        vec!["bash".to_string(), "-c".to_string(), "exit 7".to_string()]
    } else {
        argv
    };
    match espresso::session::run_command(argv) {
        Ok(code) => {
            println!("child exit code: {code}");
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}
