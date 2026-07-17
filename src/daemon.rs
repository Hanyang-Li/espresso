//! The `espresso __daemon` runtime: a single-threaded coordinator fed by an
//! mpsc `Event` channel, plus the client-side helpers used by the CLI to talk
//! to it over the Unix-domain socket.
//!
//! Threading model: worker threads (the accept loop, per-connection handler
//! threads, the lid-watch poll thread, and grace-timer threads) only ever
//! *send* `Event`s. The single `coordinator` thread started from `run` is the
//! only place that owns a `RefcountState` and executes `Action`s — this is
//! the invariant that keeps refcount mutation race-free without locks.

use crate::ipc::{
    ClientMsg, SOCKET_PATH, ServerMsg, StatusInfo, decode_client, decode_server, encode_client,
    encode_server,
};
use crate::lid::lid_closed;
use crate::power::set_sleep_disabled;
use crate::refcount::{Action, RefcountState};
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::FromRawFd;
use std::os::raw::c_char;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

const GRACE: Duration = Duration::from_secs(60);
const LID_POLL: Duration = Duration::from_secs(2);
const VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------- client helpers ----------

/// Connects to the daemon and sends `HOLD`. Holding the returned stream open
/// keeps the hold alive; dropping it (closing the connection) releases it.
pub fn hold_connection() -> std::io::Result<UnixStream> {
    let mut stream = UnixStream::connect(SOCKET_PATH)?;
    stream.write_all(encode_client(&ClientMsg::Hold).as_bytes())?;
    stream.flush()?;
    Ok(stream)
}

/// Connects to the daemon, sends `QUERY`, and parses the reply. The daemon
/// closes the Query connection after replying, so we read to EOF and decode
/// the whole (possibly multi-line) block. Returns `Ok(None)` if the socket is
/// absent or the reply is unparseable.
pub fn query_status() -> std::io::Result<Option<StatusInfo>> {
    let mut stream = match UnixStream::connect(SOCKET_PATH) {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };
    stream.write_all(encode_client(&ClientMsg::Query).as_bytes())?;
    stream.flush()?;
    let mut buf = String::new();
    BufReader::new(stream).read_to_string(&mut buf)?;
    match decode_server(&buf) {
        Ok(ServerMsg::Status(info)) => Ok(Some(info)),
        _ => Ok(None),
    }
}

// ---------- daemon side ----------

/// Events fed into the single coordinator thread. Worker threads only ever
/// send these; only `coordinator` receives and acts on them.
enum Event {
    HoldOpened,
    HoldClosed,
    GraceElapsed(u64),
    Query(Sender<StatusInfo>),
}

/// The `espresso __daemon` entry point.
pub fn run() -> Result<()> {
    // Crash recovery: clear any stale flag we might have left set from a
    // previous run that didn't get to execute `Action::Exit`.
    let _ = set_sleep_disabled(false);

    let listener = obtain_listener().context("failed to obtain daemon socket")?;
    let (tx, rx) = mpsc::channel::<Event>();

    // Accept loop: each Hold connection becomes a thread that reports
    // open/close; each Query connection is answered from a fresh status
    // snapshot. These threads never touch RefcountState directly — they only
    // send Events to the coordinator below.
    {
        let tx = tx.clone();
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { continue };
                let tx = tx.clone();
                thread::spawn(move || handle_connection(conn, tx));
            }
        });
    }

    // The coordinator is the only thread that mutates RefcountState or
    // executes Actions; it runs on the current (main) thread for the
    // lifetime of the daemon.
    coordinator(rx, tx);
    Ok(())
}

fn handle_connection(mut conn: UnixStream, tx: Sender<Event>) {
    let mut reader = BufReader::new(match conn.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    });
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    match decode_client(&line) {
        Ok(ClientMsg::Hold) => {
            if tx.send(Event::HoldOpened).is_err() {
                return;
            }
            let _ = conn.write_all(encode_server(&ServerMsg::Ok).as_bytes());
            let _ = conn.flush();
            // Block until the client goes away (EOF or error), including
            // crash. This is how the daemon detects hold release.
            let mut buf = [0u8; 64];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = tx.send(Event::HoldClosed);
        }
        Ok(ClientMsg::Query) => {
            let (rtx, rrx) = mpsc::channel::<StatusInfo>();
            if tx.send(Event::Query(rtx)).is_ok() {
                if let Ok(info) = rrx.recv() {
                    let _ = conn.write_all(encode_server(&ServerMsg::Status(info)).as_bytes());
                    let _ = conn.flush();
                }
            }
        }
        Err(_) => {}
    }
}

/// The single-threaded coordinator. This is the ONLY place `RefcountState`
/// is mutated and the ONLY place `Action`s are executed — all other threads
/// in this module communicate with it exclusively via `Event`s on `rx`.
fn coordinator(rx: Receiver<Event>, tx: Sender<Event>) {
    let mut state = RefcountState::new();
    let mut grace_gen: u64 = 0;
    // The currently-active lid-watch's stop flag, if a watcher thread is
    // running. Each StartLidWatch gets a *fresh* flag rather than reusing
    // a shared one, so a Stop that races a subsequent Start can never be
    // "undone" by the new Start resetting the same flag back to false —
    // that would strand the old watcher thread forever. Only this
    // (single) coordinator thread reads or writes `lid_stop`.
    let mut lid_stop: Option<Arc<AtomicBool>> = None;

    // Arm the idle-exit clock immediately: a daemon that starts at refcount
    // 0 (socket-activated by a `Query`, or relaunched by launchd after a
    // crash) and never receives a `Hold` must still self-exit after the
    // grace period, exactly as if a hold had opened and closed. If a `Hold`
    // does arrive, `on_hold_open`'s `CancelGraceTimer` action bumps
    // `grace_gen` and this timer's eventual `GraceElapsed` is recognized as
    // stale and ignored.
    for action in state.begin_idle() {
        match action {
            Action::StartGraceTimer => grace_gen = start_grace_timer(&tx, grace_gen),
            other => unreachable!("begin_idle() returned unexpected action: {other:?}"),
        }
    }

    while let Ok(event) = rx.recv() {
        let actions = match event {
            Event::HoldOpened => state.on_hold_open(),
            Event::HoldClosed => state.on_hold_close(),
            Event::GraceElapsed(generation) => {
                if generation == grace_gen {
                    state.on_grace_elapsed()
                } else {
                    // Stale timer from a hold that re-opened and cancelled
                    // this generation; ignore it.
                    vec![]
                }
            }
            Event::Query(reply) => {
                let info = StatusInfo {
                    sessions: Vec::new(),
                    pid: std::process::id(),
                    version: VERSION.to_string(),
                };
                let _ = reply.send(info);
                vec![]
            }
        };

        for action in actions {
            match action {
                Action::SetSleepDisabled(v) => {
                    if let Err(e) = set_sleep_disabled(v) {
                        eprintln!("espresso daemon: set_sleep_disabled({v}) failed: {e}");
                    }
                }
                Action::StartLidWatch => {
                    // Signal any previous watcher to stop, but never reuse
                    // its flag: give this watch its own, so a later Start
                    // can't accidentally revive a thread that a prior Stop
                    // already told to exit.
                    if let Some(old) = lid_stop.take() {
                        old.store(true, Ordering::SeqCst);
                    }
                    let flag = Arc::new(AtomicBool::new(false));
                    lid_stop = Some(flag.clone());
                    spawn_lid_watch(flag);
                }
                Action::StopLidWatch => {
                    if let Some(flag) = lid_stop.take() {
                        flag.store(true, Ordering::SeqCst);
                    }
                }
                Action::StartGraceTimer => {
                    grace_gen = start_grace_timer(&tx, grace_gen);
                }
                Action::CancelGraceTimer => {
                    // Bump the generation so any in-flight timer's
                    // GraceElapsed(gen) is recognized as stale and ignored.
                    grace_gen += 1;
                }
                Action::Exit => {
                    let _ = set_sleep_disabled(false);
                    std::process::exit(0);
                }
            }
        }
    }
}

/// Bumps the grace generation and spawns a thread that sleeps for `GRACE`
/// before sending `Event::GraceElapsed` tagged with the new generation.
/// Returns the new generation so callers can update their local `grace_gen`.
/// Shared by the coordinator's startup idle-arm and its `StartGraceTimer`
/// action handler so the timer-spawn logic exists in exactly one place.
fn start_grace_timer(tx: &Sender<Event>, grace_gen: u64) -> u64 {
    let generation = grace_gen + 1;
    let tx = tx.clone();
    thread::spawn(move || {
        thread::sleep(GRACE);
        let _ = tx.send(Event::GraceElapsed(generation));
    });
    generation
}

fn spawn_lid_watch(stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        let mut was_closed = false;
        while !stop.load(Ordering::SeqCst) {
            let closed = lid_closed().unwrap_or(false);
            if closed && !was_closed {
                display_sleep_now();
            }
            was_closed = closed;
            thread::sleep(LID_POLL);
        }
    });
}

fn display_sleep_now() {
    // The single permitted subprocess spawn in this project: turn the
    // display off when the lid closes (sleep itself stays disabled).
    let _ = std::process::Command::new("pmset")
        .arg("displaysleepnow")
        .status();
}

// ---------- launchd socket activation ----------

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    fn launch_activate_socket(name: *const c_char, fds: *mut *mut i32, count: *mut usize) -> i32;
}

fn obtain_listener() -> Result<UnixListener> {
    // Preferred path: launchd handed us the listening socket via the
    // `Sockets` entry named "Listener" in the plist.
    unsafe {
        let mut fds: *mut i32 = std::ptr::null_mut();
        let mut count: usize = 0;
        let rc = launch_activate_socket(c"Listener".as_ptr(), &mut fds, &mut count);
        if rc == 0 && count > 0 && !fds.is_null() {
            let fd = *fds;
            libc::free(fds as *mut libc::c_void);
            return Ok(UnixListener::from_raw_fd(fd));
        }
    }
    // Fallback for manual runs: bind the path ourselves.
    let _ = std::fs::remove_file(SOCKET_PATH);
    let listener =
        UnixListener::bind(SOCKET_PATH).with_context(|| format!("failed to bind {SOCKET_PATH}"))?;
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o666);
    std::fs::set_permissions(SOCKET_PATH, mode).ok();
    Ok(listener)
}
