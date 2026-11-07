//! Attn Lens's live capture: what a capture holds, and the state every window
//! and the automation socket read. Input arrives through the platform port and
//! changes state here; each change wakes the windows, because a Rune window does
//! not redraw from another window's change.
pub mod attn;
pub mod automation;
#[cfg(target_os = "macos")]
pub mod capture;
pub mod session;

use crate::{
    document::Document,
    platform_port::{Chord, KeyEvent, PlatformPort, Sink, Voice, Watch, WatchEvent},
};
use serde_json::{Value, json};
use session::{Capture, Cause, Segment, Still};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

/// Whether the ink layer takes the pointer: `All` while the draw key is down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Input {
    #[default]
    None,
    All,
}

/// The last send, whichever capture it belonged to.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SendStatus {
    #[default]
    Idle,
    Sending,
    Sent(String),
    /// attn's error, verbatim. The capture stays whole.
    Failed(String),
}

#[derive(Default)]
struct State {
    capture: Option<Capture>,
    input: Input,
    send: SendStatus,
    /// A failure with no capture to hold it, such as a refused still.
    error: Option<String>,
    next_capture: u64,
    voice: Option<Voice>,
    watch: Option<Watch>,
}

type WindowVisible = Box<dyn Fn() -> bool + Send>;

pub struct Lens {
    port: Arc<dyn PlatformPort>,
    state: Mutex<State>,
    /// Stills being taken; a send waits for them so none is left out.
    pending_stills: (Mutex<usize>, Condvar),
    generation: AtomicU64,
    wakers: Mutex<Vec<Box<dyn Fn() + Send>>>,
    /// Each window's name and how to ask whether it shows, for automation.
    windows: Mutex<Vec<(&'static str, WindowVisible)>>,
}

impl Lens {
    pub fn new(port: Arc<dyn PlatformPort>) -> Arc<Self> {
        Arc::new(Self {
            port,
            state: Default::default(),
            pending_stills: Default::default(),
            generation: AtomicU64::new(0),
            wakers: Default::default(),
            windows: Default::default(),
        })
    }

    pub fn port(&self) -> &Arc<dyn PlatformPort> {
        &self.port
    }

    /// Calls `wake` after every change.
    pub fn on_change(&self, wake: impl Fn() + Send + 'static) {
        self.wakers.lock().unwrap().push(Box::new(wake));
    }

    /// Reports `window` in the automation state.
    pub fn report_window(&self, window: &'static str, visible: impl Fn() -> bool + Send + 'static) {
        self.windows
            .lock()
            .unwrap()
            .push((window, Box::new(visible)));
    }

    /// Changes each time the state does; a window redraws when it differs from what it saw.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn changed(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        for wake in self.wakers.lock().unwrap().iter() {
            wake();
        }
    }

    /// Starts listening for keys.
    pub fn listen(self: &Arc<Self>) {
        let lens = self.clone();
        let sink: Sink<KeyEvent> = Arc::new(move |event| lens.key(event));
        self.port.keys(sink);
    }

    pub fn key(self: &Arc<Self>, event: KeyEvent) {
        match event {
            KeyEvent::DrawDown => self.set_input(Input::All),
            KeyEvent::DrawUp => self.set_input(Input::None),
            KeyEvent::Pressed(Chord::Capture) => {
                if self.is_open() {
                    self.take_still(Cause::Hotkey);
                } else {
                    self.start_capture();
                }
            }
            KeyEvent::Pressed(Chord::Voice) => self.toggle_voice(),
            KeyEvent::Pressed(Chord::Keyboard) => {}
            KeyEvent::Pressed(Chord::Send) => {
                if let Err(error) = self.send(None, None) {
                    self.fail(error);
                }
            }
        }
    }

    fn is_open(&self) -> bool {
        self.state.lock().unwrap().capture.is_some()
    }

    fn set_input(&self, input: Input) {
        let mut state = self.state.lock().unwrap();
        // The ink layer exists only during a capture.
        let input = if state.capture.is_some() {
            input
        } else {
            Input::None
        };
        if state.input != input {
            state.input = input;
            drop(state);
            self.changed();
        }
    }

    fn fail(&self, error: String) {
        self.state.lock().unwrap().error = Some(error);
        self.changed();
    }

    /// Opens a capture of the display under the pointer.
    fn start_capture(self: &Arc<Self>) {
        if !self.port.screen_recording() {
            return;
        }
        let area = match self.port.pointer_display() {
            Ok(area) => area,
            Err(error) => return self.fail(error),
        };
        let lens = self.clone();
        let sink: Sink<WatchEvent> = Arc::new(move |event| lens.watched(event));
        let watch = self.port.watch(area, sink);
        {
            let mut state = self.state.lock().unwrap();
            state.next_capture += 1;
            state.capture = Some(Capture {
                id: state.next_capture,
                area,
                started: self.port.now(),
                stills: vec![],
                segments: vec![],
                target: None,
                message: String::new(),
            });
            state.watch = Some(watch);
            state.error = None;
            // A send still running reports its own result, and holds off this capture's.
            if state.send != SendStatus::Sending {
                state.send = SendStatus::Idle;
            }
        }
        self.port.capture_keys(true);
        self.changed();
    }

    /// Closes the open capture and lets go of everything it held.
    pub fn end_capture(&self) {
        let voice = {
            let mut state = self.state.lock().unwrap();
            state.capture = None;
            state.input = Input::None;
            state.watch = None;
            state.voice.take()
        };
        if let Some(voice) = voice
            && let Err(error) = voice.finish()
        {
            self.state.lock().unwrap().error = Some(error);
        }
        self.port.capture_keys(false);
        self.changed();
    }

    fn watched(self: &Arc<Self>, event: WatchEvent) {
        match event {
            WatchEvent::Click => self.take_still(Cause::Click),
            WatchEvent::WindowSwitch => self.take_still(Cause::WindowSwitch),
            WatchEvent::Scroll | WatchEvent::Change(_) => {}
        }
    }

    /// Takes a still of the open capture's area off the UI thread. Its time is now,
    /// whenever the frame arrives, so stills stay in the order they were asked for.
    pub fn take_still(self: &Arc<Self>, cause: Cause) {
        let at = self.port.now();
        let Some((id, area)) = self
            .state
            .lock()
            .unwrap()
            .capture
            .as_ref()
            .map(|c| (c.id, c.area))
        else {
            return;
        };
        *self.pending_stills.0.lock().unwrap() += 1;
        let lens = self.clone();
        self.port.run(Box::new(move || {
            let frame = lens.port.still(area);
            {
                let mut state = lens.state.lock().unwrap();
                match frame {
                    Ok(frame) => {
                        // A capture closed while its frame was on the way drops the frame.
                        if let Some(capture) = state.capture.as_mut().filter(|c| c.id == id) {
                            let still = Still {
                                at,
                                cause,
                                doc: Document::new(frame),
                                times: vec![],
                            };
                            let index = capture.stills.partition_point(|s| s.at <= at);
                            capture.stills.insert(index, still);
                        }
                    }
                    Err(error) => state.error = Some(error),
                }
            }
            *lens.pending_stills.0.lock().unwrap() -= 1;
            lens.pending_stills.1.notify_all();
            lens.changed();
        }));
    }

    fn toggle_voice(self: &Arc<Self>) {
        let running = self.state.lock().unwrap().voice.take();
        if let Some(voice) = running {
            if let Err(error) = voice.finish() {
                self.fail(error);
            }
            return self.changed();
        }
        if !self.is_open() {
            return;
        }
        let lens = self.clone();
        let sink: Sink<Segment> = Arc::new(move |segment| {
            if let Some(capture) = lens.state.lock().unwrap().capture.as_mut() {
                capture.segments.push(segment);
            }
            lens.changed();
        });
        match self.port.voice(sink) {
            Ok(voice) => self.state.lock().unwrap().voice = Some(voice),
            Err(error) => self.state.lock().unwrap().error = Some(error),
        }
        self.changed();
    }

    /// Sends the open capture: every still as a PNG, with the message. `target`
    /// and `message` replace what the capture holds. The result lands in `SendStatus`.
    pub fn send(
        self: &Arc<Self>,
        target: Option<String>,
        message: Option<String>,
    ) -> Result<(), String> {
        let (id, needs_still, voice) = {
            let mut state = self.state.lock().unwrap();
            if state.send == SendStatus::Sending {
                return Err("A send is already running.".into());
            }
            let capture = state
                .capture
                .as_mut()
                .ok_or("No capture is open. Start one with ⌘⌥X.")?;
            if let Some(target) = target {
                capture.target = Some(target);
            }
            if let Some(message) = message {
                capture.message = message;
            }
            if capture.target.is_none() {
                return Err("The capture has no target session.".into());
            }
            let (id, needs_still) = (capture.id, capture.stills.is_empty());
            state.send = SendStatus::Sending;
            (id, needs_still, state.voice.take())
        };
        self.changed();
        if needs_still {
            self.take_still(Cause::Send);
        }
        let lens = self.clone();
        self.port.run(Box::new(move || lens.finish_send(id, voice)));
        Ok(())
    }

    /// Finishes the send of capture `id`. The user may have closed it and opened
    /// another by now, so only that capture is read or closed.
    fn finish_send(self: &Arc<Self>, id: u64, voice: Option<Voice>) {
        let voice = voice.map_or(Ok(()), Voice::finish);
        let mut pending = self.pending_stills.0.lock().unwrap();
        while *pending > 0 {
            pending = self.pending_stills.1.wait(pending).unwrap();
        }
        drop(pending);
        let delivered = voice.and_then(|()| self.deliver(id));
        {
            let mut state = self.state.lock().unwrap();
            state.send = match delivered {
                Ok(target) => {
                    if state.capture.as_ref().is_some_and(|c| c.id == id) {
                        state.capture = None;
                        state.input = Input::None;
                        state.watch = None;
                    }
                    SendStatus::Sent(target)
                }
                Err(error) => SendStatus::Failed(error),
            };
        }
        if !self.is_open() {
            self.port.capture_keys(false);
        }
        self.changed();
    }

    fn deliver(&self, id: u64) -> Result<String, String> {
        let (docs, target, message) = {
            let state = self.state.lock().unwrap();
            let capture = state
                .capture
                .as_ref()
                .filter(|c| c.id == id)
                .ok_or("The capture was closed before it was sent.")?;
            (
                capture
                    .stills
                    .iter()
                    .map(|s| s.doc.clone())
                    .collect::<Vec<_>>(),
                capture
                    .target
                    .clone()
                    .ok_or("The capture has no target session.")?,
                capture.message.clone(),
            )
        };
        if docs.is_empty() {
            return Err("The capture has no still to send.".into());
        }
        let token = self
            .port
            .token()
            .ok_or("Attn Lens is not paired with attn.")?;
        let dir = tempfile::Builder::new()
            .prefix("attn-lens-send-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let mut files: Vec<PathBuf> = vec![];
        for (index, doc) in docs.iter().enumerate() {
            let path = dir.path().join(format!("frame-{:02}.png", index + 1));
            doc.export()
                .save_with_format(&path, image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            files.push(path);
        }
        let bin = attn::locate().map_err(failure_text)?;
        attn::send(&bin, &token, &target, &files, &message).map_err(failure_text)?;
        Ok(target)
    }

    /// Lists the sessions the pairing can send to, for automation.
    pub fn sessions(&self) -> Result<Vec<attn::Session>, String> {
        let token = self
            .port
            .token()
            .ok_or("Attn Lens is not paired with attn.")?;
        let bin = attn::locate().map_err(failure_text)?;
        attn::sessions(&bin, &token).map_err(failure_text)
    }

    /// Runs `view` over what a window shows.
    pub fn read<R>(&self, view: impl FnOnce(&Summary) -> R) -> R {
        view(&self.summary())
    }

    fn summary(&self) -> Summary {
        let state = self.state.lock().unwrap();
        Summary {
            input: state.input,
            capture: state.capture.as_ref().map(|c| CaptureSummary {
                id: c.id,
                area: c.area,
                started: c.started,
                stills: c
                    .stills
                    .iter()
                    .map(|s| StillSummary {
                        at: s.at,
                        cause: s.cause,
                        width: s.doc.base.width(),
                        height: s.doc.base.height(),
                    })
                    .collect(),
                segments: c.segments.clone(),
                target: c.target.clone(),
                message: c.message.clone(),
            }),
            voice: state.voice.is_some(),
            send: state.send.clone(),
            error: state.error.clone(),
        }
    }

    /// What automation reads back.
    pub fn state_json(&self) -> Value {
        let summary = self.summary();
        let send = match &summary.send {
            SendStatus::Idle => json!({"state": "idle"}),
            SendStatus::Sending => json!({"state": "sending"}),
            SendStatus::Sent(target) => json!({"state": "sent", "target": target}),
            SendStatus::Failed(error) => json!({"state": "failed", "error": error}),
        };
        let windows: serde_json::Map<String, Value> = self
            .windows
            .lock()
            .unwrap()
            .iter()
            .map(|(name, visible)| (name.to_string(), json!(visible())))
            .collect();
        json!({
            "windows": windows,
            "input": format!("{:?}", summary.input),
            "voice": summary.voice,
            "send": send,
            "error": summary.error,
            "capture": summary.capture.map(|c| json!({
                "id": c.id,
                "area": c.area,
                "started": c.started,
                "stills": c.stills.iter().map(|s| json!({
                    "at": s.at,
                    "cause": s.cause,
                    "width": s.width,
                    "height": s.height,
                })).collect::<Vec<_>>(),
                "segments": c.segments,
                "target": c.target,
                "message": c.message,
            })),
        })
    }
}

fn failure_text(failure: attn::Failure) -> String {
    match failure {
        attn::Failure::NotPaired(text) | attn::Failure::Error(text) => text,
    }
}

pub struct StillSummary {
    pub at: session::HostTime,
    pub cause: Cause,
    pub width: u32,
    pub height: u32,
}

pub struct CaptureSummary {
    pub id: u64,
    pub area: session::Area,
    pub started: session::HostTime,
    pub stills: Vec<StillSummary>,
    pub segments: Vec<Segment>,
    pub target: Option<String>,
    pub message: String,
}

pub struct Summary {
    pub input: Input,
    pub capture: Option<CaptureSummary>,
    pub voice: bool,
    pub send: SendStatus,
    pub error: Option<String>,
}
