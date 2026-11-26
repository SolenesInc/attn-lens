//! Timing lines: where each dictation's time goes, and the speech helper's own
//! times. They hold times, counts and states, never the words.
//!
//! `LENS_DICTATION_LOG=1` prints them to stderr. The `diagnostics.timings` setting,
//! read at launch, appends them to `timings.log` beside `prefs.json`, so a history
//! builds up across launches of the app the Dock opens, whose stderr goes nowhere.
//! Each line has the time since launch and the wall clock, so a check outside Lens
//! can line its own times up; `scripts/lens-dictation.sh` parses that format.
use std::{
    fs::File,
    io::Write,
    path::Path,
    sync::{Mutex, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// The log's name, in the folder that holds `prefs.json`.
pub const FILE: &str = "timings.log";

static LAUNCH: OnceLock<Instant> = OnceLock::new();
static LOG: Mutex<Option<File>> = Mutex::new(None);

/// Starts the launch clock, and appends every line to `dir/timings.log` from now
/// on when `dir` is given. A log that does not open says why on stderr and stays off.
pub fn start(dir: Option<&Path>) {
    LAUNCH.get_or_init(Instant::now);
    let Some(dir) = dir else {
        return;
    };
    let path = dir.join(FILE);
    let opened = std::fs::create_dir_all(dir)
        .and_then(|()| File::options().create(true).append(true).open(&path));
    match opened {
        Ok(file) => {
            *LOG.lock().unwrap() = Some(file);
            line(format_args!(
                "timings log opens, Attn Lens {}",
                env!("CARGO_PKG_VERSION")
            ));
        }
        Err(error) => eprintln!(
            "Attn Lens: timings are off, {} did not open: {error}",
            path.display()
        ),
    }
}

/// One timing line, to stderr under `LENS_DICTATION_LOG=1` and to the log while
/// it is open.
pub fn line(message: std::fmt::Arguments) {
    static STDERR: OnceLock<bool> = OnceLock::new();
    let stderr =
        *STDERR.get_or_init(|| std::env::var_os("LENS_DICTATION_LOG").is_some_and(|v| v == "1"));
    let mut log = LOG.lock().unwrap();
    if !stderr && log.is_none() {
        return;
    }
    let launch = LAUNCH.get_or_init(Instant::now);
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let text = format!(
        "Attn Lens dictation {:>8.1} ms (unix {} ms): {message}\n",
        launch.elapsed().as_secs_f64() * 1e3,
        wall.as_millis()
    );
    if stderr {
        eprint!("{text}");
    }
    if let Some(file) = log.as_mut() {
        // A line lost to a full disk costs a measurement, never a dictation.
        let _ = file.write_all(text.as_bytes());
    }
}
