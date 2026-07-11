//! Manual smoke test for the `SleepDisabled` IOKit FFI.
//!
//! `read_sleep_disabled` needs no privileges; `set_sleep_disabled` requires
//! root. Run with:
//!
//! ```bash
//! cargo build --example smoke_power
//! sudo ./target/debug/examples/smoke_power
//! pmset -g | grep -i SleepDisabled
//! ```
//!
//! Expected output: `false`, `true`, `false`, with the flag restored to 0.

fn main() {
    println!(
        "before: SleepDisabled = {}",
        espresso::power::read_sleep_disabled().unwrap()
    );
    espresso::power::set_sleep_disabled(true).expect("set true (run with sudo)");
    println!(
        "after set true: SleepDisabled = {}",
        espresso::power::read_sleep_disabled().unwrap()
    );
    espresso::power::set_sleep_disabled(false).unwrap();
    println!(
        "after set false: SleepDisabled = {}",
        espresso::power::read_sleep_disabled().unwrap()
    );
}
