//! Manual smoke test for `SleepAssertion`.
//!
//! Run with `cargo run --example smoke_assertion &`, then check
//! `pmset -g assertions | grep -i PreventUserIdleSystemSleep` to confirm the
//! assertion is held by this process. Press enter (or `kill %1`) to release it.

fn main() {
    let _a = espresso::assertion::SleepAssertion::prevent_idle_sleep("espresso smoke").unwrap();
    println!("assertion held; check `pmset -g assertions` then press enter");
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf).unwrap();
}
