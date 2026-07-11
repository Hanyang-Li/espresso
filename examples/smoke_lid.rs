//! Manual smoke test for the `AppleClamshellState` IORegistry read.
//!
//! Run with:
//!
//! ```bash
//! cargo run --example smoke_lid
//! ```
//!
//! Prints the lid state once a second, ten times. Close and reopen the lid
//! during the run (with an external display/keyboard attached, or over SSH)
//! to observe the value flip from `false` (open) to `true` (closed) and back.

fn main() {
    for _ in 0..10 {
        println!("lid_closed = {}", espresso::lid::lid_closed().unwrap());
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
