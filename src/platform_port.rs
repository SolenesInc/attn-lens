//! Everything the UI needs from the machine: keys, screen frames, input watching
//! and voice. macOS has the real implementation; headless runs use the fake, so
//! every window can be driven on any machine.
use crate::lens::session::{Area, DisplayFrame, HostTime, Segment};
use image::RgbaImage;
use serde::Serialize;
use std::{path::PathBuf, sync::Arc, time::Duration};

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

/// Where voice audio comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceSource {
    Microphone,
    /// A WAV file played at real speed, so driven checks need nobody to speak.
    /// Only the automation socket chooses it.
    Wav(PathBuf),
}

/// How far voice has got since ⌘⌥S.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VoiceStatus {
    /// The first ⌘⌥S fetches the speech model.
    Downloading {
        bytes: u64,
        total: u64,
    },
    /// The model is loading into memory.
    Loading,
    /// The model is ready and the microphone has not delivered audio yet.
    WarmingUp,
    Listening,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VoiceEvent {
    Status(VoiceStatus),
    Segment(Segment),
    /// Voice stopped on its own; the capture goes on without it.
    Failed(String),
}

/// A running voice source.
pub struct Voice(pub Box<dyn FnOnce() -> Result<(), String> + Send>);

impl Voice {
    /// Stops listening and returns once the sentence in progress has reached the
    /// sink. Slow: it waits for recognition, so call it from [`PlatformPort::run`].
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
    /// The displays connected now, which may differ from those at launch.
    fn displays(&self) -> Vec<DisplayFrame>;
    /// Gives the port the displays the window factory saw. Only a headless run,
    /// whose displays come from its script, needs them.
    fn launch_displays(&self, _displays: Vec<DisplayFrame>) {}
    /// True while ⇧ is held: it makes the next stroke a box.
    fn shift_held(&self) -> bool;
    /// True when stills can be taken. Otherwise shows the user how to allow it.
    fn screen_recording(&self) -> bool;
    fn still(&self, area: Area) -> Result<RgbaImage, String>;
    fn watch(&self, area: Area, sink: Sink<WatchEvent>) -> Watch;
    /// Starts voice. It reports its progress, then segments, to `sink` from
    /// other threads, and returns at once.
    fn voice(&self, source: VoiceSource, sink: Sink<VoiceEvent>) -> Result<Voice, String>;
    /// Lets go of the speech model once the capture has ended. A voice started
    /// since keeps it.
    fn end_voice(&self) {}
    /// The token from pairing with attn.
    fn token(&self) -> Option<String>;
    fn store_token(&self, token: &str) -> Result<(), String>;
    fn forget_token(&self);
    /// Puts an image on the system clipboard.
    fn copy_image(&self, image: RgbaImage) -> Result<(), String>;
    /// The image on the system clipboard.
    fn clipboard_image(&self) -> Result<RgbaImage, String>;
    /// Puts text on the system clipboard.
    fn copy_text(&self, text: &str) -> Result<(), String>;
    /// Reads text from `image`, optionally within [x, y, width, height]. Slow: call it from [`Self::run`].
    fn ocr(&self, image: &RgbaImage, rectangle: Option<[u32; 4]>) -> Result<String, String>;
    /// True on the fake port: the editor then exposes its state for scripts to assert on.
    fn exposes_state(&self) -> bool {
        false
    }
    /// Lets the user pick a color anywhere on screen. `None` means they cancelled.
    fn pick_color(&self, sink: Sink<Result<Option<[u8; 3]>, String>>);
    /// Runs slow work off the UI thread.
    fn run(&self, job: Box<dyn FnOnce() + Send>) {
        std::thread::spawn(job);
    }
    /// One long-lived thread that runs its jobs in order, for work that keeps GPU state
    /// per thread, such as animation frames. The fake runs them inline.
    fn lane(&self) -> Lane {
        Lane::thread()
    }
    /// The helper that encodes MP4, which `scripts/bundle.sh` puts beside the app.
    fn video_encoder(&self) -> Result<std::path::PathBuf, String> {
        crate::video::encoder_path()
    }
    /// Called from each window's `poll` with Rune's clock. A port whose events
    /// follow that clock returns when to call again.
    fn tick(&self, _now: f64) -> Option<Duration> {
        None
    }
    /// What pressing `window`'s close button does. On the Mac the window has a real
    /// one, so this does nothing; the fake presses it on a `close:WINDOW` mark.
    fn close_button(&self, _window: &'static str, _press: Sink<()>) {}
}

type Job = Box<dyn FnOnce() + Send>;

/// Where [`PlatformPort::lane`] sends its jobs.
pub struct Lane(Box<dyn Fn(Job) + Send + Sync>);

impl Lane {
    fn thread() -> Self {
        let (send, jobs) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("lane".into())
            .spawn(move || {
                for job in jobs {
                    job();
                }
            })
            .expect("start the lane thread");
        // The thread ends when the last sender drops.
        let send = std::sync::Mutex::new(send);
        Self(Box::new(move |job| {
            let _ = send.lock().unwrap().send(job);
        }))
    }

    pub fn inline() -> Self {
        Self(Box::new(|job| job()))
    }

    pub fn run(&self, job: Job) {
        (self.0)(job)
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
