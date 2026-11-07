//! Everything the UI needs from the machine: keys, screen frames, input watching
//! and voice. macOS has the real implementation; headless runs use the fake, so
//! every window can be driven on any machine.
use crate::lens::session::{Area, HostTime, Segment};
use image::RgbaImage;
use std::{sync::Arc, time::Duration};

pub mod fake;
#[cfg(target_os = "macos")]
pub mod mac;

/// Receives events on whichever thread the port produces them. Callers keep
/// their work short: stills and sends go through [`PlatformPort::run`].
pub type Sink<T> = Arc<dyn Fn(T) + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chord {
    /// ⌘⌥X, always registered.
    Capture,
    /// ⌘⌥S, only while a capture is open.
    Voice,
    /// ⌘⌥C, only while a capture is open.
    Keyboard,
    /// ⌘⌥↩, only while a capture is open.
    Send,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    DrawDown,
    DrawUp,
    Pressed(Chord),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WatchEvent {
    Click,
    Scroll,
    WindowSwitch,
    /// The fraction of the area that changed since the last frame compared.
    Change(f32),
}

/// Watching stops when this drops.
pub struct Watch(pub Option<Box<dyn FnOnce() + Send>>);

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            stop();
        }
    }
}

/// A running voice source.
pub struct Voice(pub Box<dyn FnOnce() -> Result<(), String> + Send>);

impl Voice {
    /// Stops listening and returns once the sentence in progress has reached the sink.
    pub fn finish(self) -> Result<(), String> {
        (self.0)()
    }
}

pub trait PlatformPort: Send + Sync {
    fn now(&self) -> HostTime;
    /// Starts reporting the draw key and ⌘⌥X. Called once, on the UI thread.
    fn keys(&self, sink: Sink<KeyEvent>);
    /// Registers or releases the keys that exist only during a capture.
    fn capture_keys(&self, open: bool);
    /// The whole display under the pointer.
    fn pointer_display(&self) -> Result<Area, String>;
    /// True when stills can be taken. Otherwise shows the user how to allow it.
    fn screen_recording(&self) -> bool;
    fn still(&self, area: Area) -> Result<RgbaImage, String>;
    fn watch(&self, area: Area, sink: Sink<WatchEvent>) -> Watch;
    fn voice(&self, sink: Sink<Segment>) -> Result<Voice, String>;
    /// The token from pairing with attn.
    fn token(&self) -> Option<String>;
    /// Runs slow work off the UI thread.
    fn run(&self, job: Box<dyn FnOnce() + Send>) {
        std::thread::spawn(job);
    }
    /// Called from each window's `poll` with Rune's clock. A port whose events
    /// follow that clock returns when to call again.
    fn tick(&self, _now: f64) -> Option<Duration> {
        None
    }
}

/// The fake when Rune runs headless, otherwise this machine's port.
pub fn select() -> Result<Arc<dyn PlatformPort>, String> {
    if rune::tooling::is_headless() {
        return Ok(Arc::new(fake::FakePort::from_headless_run()));
    }
    #[cfg(target_os = "macos")]
    return Ok(Arc::new(mac::MacPort::new()));
    #[cfg(not(target_os = "macos"))]
    Err(
        "Attn Lens runs on macOS. Elsewhere it runs only headless (--script, --tree, --snap)."
            .into(),
    )
}

/// How often the fake looks for script marks on the virtual clock: one 60 Hz frame.
pub const FAKE_TICK: Duration = Duration::from_micros(16_667);
