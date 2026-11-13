//! Writes the executable stand-ins that tests run in place of `attn`.
//!
//! A test that writes a script and runs it can fail on Linux with `Text file busy`
//! (ETXTBSY). While the script's write handle is open, a fork on another test's
//! thread gives the child a copy of it, and the child keeps it until its own exec.
//! An exec of the script in that window is refused, however early the writer closed.
//! Closing first, or renaming into place, does not help: the child holds the inode.
//!
//! So this process never holds the script's write handle. A child shell writes the
//! file and is waited for, and no other thread's fork can copy a handle this process
//! does not have.
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

/// Writes `body` to `path` with mode 0755, replacing any file there.
pub fn write_executable(path: &Path, body: &str) {
    let mut child = Command::new("sh")
        .args(["-c", r#"cat > "$0" && chmod 755 "$0""#])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .expect("sh starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(body.as_bytes())
        .expect("sh reads the script");
    let status = child.wait().expect("sh finishes");
    assert!(
        status.success(),
        "writing {} failed: {status}",
        path.display()
    );
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Threads that fork all the time, as the other tests in a binary do, cannot make
    /// a freshly written stand-in refuse to run.
    #[test]
    fn a_stand_in_runs_while_other_threads_fork() {
        let stop = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    while !stop.load(Ordering::Relaxed) {
                        Command::new("true").status().unwrap();
                    }
                });
            }
            let writers: Vec<_> = (0..4)
                .map(|n| {
                    let dir = dir.path();
                    scope.spawn(move || {
                        for i in 0..100 {
                            let bin = dir.join(format!("attn-{n}-{i}"));
                            write_executable(&bin, "#!/bin/sh\nexit 0\n");
                            let ran = Command::new(&bin).status().unwrap();
                            assert!(ran.success(), "{ran}");
                        }
                    })
                })
                .collect();
            // Stop the forking threads even when a writer failed, or the scope never ends.
            let results: Vec<_> = writers.into_iter().map(|writer| writer.join()).collect();
            stop.store(true, Ordering::Relaxed);
            for result in results {
                result.unwrap();
            }
        });
    }
}
