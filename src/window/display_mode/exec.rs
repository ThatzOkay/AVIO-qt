//! Runs a `Command` with a hard timeout, killing it if it doesn't finish in time. Both
//! `kscreen-doctor` and `wlr-randr` are external tools this app doesn't control - if one of them
//! ever hangs (e.g. blocked on an unreachable D-Bus session address in this deeply re-exec'd
//! process tree), a "nice to have" display-mode listing must never be able to hang the app with
//! it. `std::process::Command::output()` has no built-in timeout, so this does it by hand:
//! `wait_with_output()` on a background thread, raced against a timeout on the caller's side,
//! with an explicit kill-by-pid if the deadline passes (the `Child` itself is stuck inside the
//! background thread by then, so a pid-based kill is the only way back to it).

use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<Output> {
    // Without this, stdout/stderr default to `Stdio::inherit()` - the child's actual output goes
    // straight to this process's own terminal instead of into `Output`, and `wait_with_output()`
    // hands back empty buffers regardless of what the child actually printed.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().ok()?;
    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) => Some(output),
        _ => {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
            None
        }
    }
}
