//! Attn Lens's live capture: what a capture holds, and the state every window
//! and the automation socket read. Input arrives through the platform port and
//! changes state here; each change wakes the windows, because a Rune window does
//! not redraw from another window's change.
pub mod attn;
pub mod automation;
pub mod bar;
#[cfg(target_os = "macos")]
pub mod capture;
pub mod ink;
pub mod select;
pub mod session;
#[cfg(target_os = "macos")]
pub mod watch;

use crate::{
    document::Document,
    platform_port::{Chord, KeyEvent, PlatformPort, Sink, Voice, Watch, WatchEvent},
};
use ink::InkTool;
use serde_json::{Value, json};
use session::{Area, Capture, Cause, DisplayFrame, HostTime, InkMark, Segment, Still};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

/// The fraction of the area that counts as a new screen (D8 on the plan), past the
/// largest change that is not one. Measured on Safari pages (C5 on s-dqjzpt): 250 ms
/// apart, a big scroll changes up to 0.50, a tab between similar articles 0.35,
/// playing video 0.39, a blinking cursor 0.0004. Video cuts reach 0.96 and do count.
const CHANGE_FRACTION: f32 = 0.6;

/// How long ink takes to fade: a feel choice, judged in the user's session (C15).
const FADE: std::time::Duration = std::time::Duration::from_millis(250);

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

/// Who gets the user's typing during a capture (D6 on the plan).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyOwner {
    #[default]
    Bar,
    Page,
}

/// What Lens knows about reaching attn.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum AttnStatus {
    #[default]
    Loading,
    Ready,
    /// Lens holds no token, or attn refused it. ↩ pairs.
    Unpaired(String),
    /// Waiting for the user to allow Lens in attn; the code shows once attn gives it.
    Pairing(Option<String>),
    /// attn could not list the sessions. ↩ asks again.
    Unlisted(String),
}

/// A stroke under way: what its press decided.
#[derive(Clone, Copy, Debug)]
pub struct StrokeStart {
    pub still: u64,
    pub at: HostTime,
    pub boxed: bool,
    pub tool: InkTool,
}

/// A label placed on the ink layer, waiting for its text in the bar.
#[derive(Clone, Copy, Debug)]
struct LabelDraft {
    start: StrokeStart,
    at: (f32, f32),
    end: HostTime,
}

#[derive(Default)]
struct State {
    /// The displays the area picker covers while it is open.
    picking: Option<Vec<DisplayFrame>>,
    last_area: Option<Area>,
    capture: Option<Capture>,
    keys: KeyOwner,
    /// Counts requests to hand the keyboard to the bar.
    bar_key_requests: u64,
    tool: InkTool,
    label: Option<LabelDraft>,
    attn: AttnStatus,
    sessions: Vec<attn::Session>,
    /// The Tab palette's selected row, while it is open.
    palette: Option<usize>,
    input: Input,
    /// A corner handle is being dragged. The handles hold still until it ends, so
    /// the window that owns the drag never moves under the pointer.
    resizing: bool,
    send: SendStatus,
    /// The capture a running send is delivering. It takes no more stills: the
    /// send has its copies and closes the capture when it succeeds.
    sending: Option<u64>,
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
    /// The capture of each still being taken. A send waits for its capture's
    /// stills so none is left out; another capture's stills are not its concern.
    pending_stills: (Mutex<Vec<u64>>, Condvar),
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
                    // The asked-for still begins a new view, so the next stroke takes its own.
                    if let Some(capture) = self.state.lock().unwrap().capture.as_mut() {
                        capture.view_still = None;
                    }
                    self.take_still(Cause::Hotkey);
                    self.give_keys_to_bar();
                } else {
                    self.open_picker();
                }
            }
            KeyEvent::Pressed(Chord::Voice) => self.toggle_voice(),
            KeyEvent::Pressed(Chord::Keyboard) => self.give_keys_to_bar(),
            KeyEvent::Pressed(Chord::Send) => self.send_or_recover(),
        }
    }

    fn is_open(&self) -> bool {
        self.state.lock().unwrap().capture.is_some()
    }

    fn give_keys_to_bar(&self) {
        let mut state = self.state.lock().unwrap();
        if state.capture.is_some() {
            state.keys = KeyOwner::Bar;
            state.bar_key_requests += 1;
            drop(state);
            self.changed();
        }
    }

    /// Records who has the keyboard, as the bar's window reports it.
    pub fn set_key_owner(&self, keys: KeyOwner) {
        let mut state = self.state.lock().unwrap();
        if state.keys != keys {
            state.keys = keys;
            drop(state);
            self.changed();
        }
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

    /// Opens the area picker on every display connected now. ⌘⌥X during a pick does nothing.
    fn open_picker(&self) {
        if !self.port.screen_recording() {
            return;
        }
        let displays = self.port.displays();
        {
            let mut state = self.state.lock().unwrap();
            if state.picking.is_some() {
                return;
            }
            state.picking = Some(displays);
            state.error = None;
        }
        self.changed();
    }

    /// Closes the area picker without starting a capture.
    pub fn cancel_pick(&self) {
        self.state.lock().unwrap().picking = None;
        self.changed();
    }

    /// Starts a capture from the picker: `rect` on `display`, or the whole display.
    /// The rect is in points from the display's top-left.
    pub fn pick(self: &Arc<Self>, display: u32, rect: Option<[f64; 4]>) -> Result<(), String> {
        let frame = self
            .port
            .displays()
            .into_iter()
            .find(|d| d.id == display)
            .ok_or_else(|| format!("Display {display} is not connected."))?;
        let area = rect.map_or(frame.whole(), |rect| Area {
            display,
            rect: select::clamp(rect, frame),
        });
        self.start_capture(area, frame);
        Ok(())
    }

    /// Starts a capture of the whole display under the pointer.
    pub fn pick_pointer_display(self: &Arc<Self>) -> Result<(), String> {
        let area = self.port.pointer_display()?;
        self.pick(area.display, None)
    }

    /// ↩ in the picker. Returns false when no last area is left to use: there is
    /// none yet, or its display is gone.
    pub fn pick_last(self: &Arc<Self>) -> bool {
        let Some(area) = self.state.lock().unwrap().last_area else {
            return false;
        };
        self.pick(area.display, Some(area.rect)).is_ok()
    }

    /// A corner handle's drag begins or ends.
    pub fn set_resizing(self: &Arc<Self>, resizing: bool) {
        let mut state = self.state.lock().unwrap();
        let resizing = resizing && state.capture.is_some();
        if state.resizing != resizing {
            state.resizing = resizing;
            let area = state.capture.as_ref().filter(|_| !resizing).map(|c| c.area);
            drop(state);
            if let Some(area) = area {
                self.rewatch(area);
            }
            self.changed();
        }
    }

    /// Watches `area` from now on, in place of what was watched before.
    fn rewatch(self: &Arc<Self>, area: Area) {
        self.state.lock().unwrap().watch = None;
        let lens = self.clone();
        let sink: Sink<WatchEvent> = Arc::new(move |event| lens.watched(event));
        let watch = self.port.watch(area, sink);
        let mut state = self.state.lock().unwrap();
        if state.capture.is_some() {
            state.watch = Some(watch);
        }
    }

    /// Moves the open capture's area, which the next still will cover. The screen stays live.
    pub fn set_area(&self, rect: [f64; 4]) {
        let mut state = self.state.lock().unwrap();
        let Some(capture) = state.capture.as_mut() else {
            return;
        };
        capture.area.rect = select::clamp(rect, capture.display);
        // The next stroke needs a still of the new area.
        capture.view_still = None;
        state.last_area = state.capture.as_ref().map(|c| c.area);
        drop(state);
        self.changed();
    }

    fn start_capture(self: &Arc<Self>, area: Area, display: DisplayFrame) {
        let lens = self.clone();
        let sink: Sink<WatchEvent> = Arc::new(move |event| lens.watched(event));
        let watch = self.port.watch(area, sink);
        {
            let mut state = self.state.lock().unwrap();
            state.next_capture += 1;
            let target = attn::default_session(&state.sessions)
                .map(|index| state.sessions[index].id.clone());
            state.capture = Some(Capture {
                id: state.next_capture,
                area,
                display,
                started: self.port.now(),
                stills: vec![],
                ink: vec![],
                view_still: None,
                next_still: 0,
                unseen_change: false,
                send_still_error: None,
                segments: vec![],
                target,
                message: String::new(),
            });
            state.picking = None;
            state.last_area = Some(area);
            state.watch = Some(watch);
            state.error = None;
            state.keys = KeyOwner::Bar;
            state.bar_key_requests += 1;
            state.tool = InkTool::default();
            state.label = None;
            state.palette = None;
            // A send still running reports its own result, and holds off this capture's.
            if state.send != SendStatus::Sending {
                state.send = SendStatus::Idle;
            }
        }
        self.port.capture_keys(true);
        self.changed();
        self.load_sessions();
    }

    /// Asks attn for the sessions off the UI thread, and picks a target if the
    /// capture has none or its target has gone.
    pub fn load_sessions(self: &Arc<Self>) {
        let Some(token) = self.port.token() else {
            self.state.lock().unwrap().attn =
                AttnStatus::Unpaired("Attn Lens is not paired with attn yet.".into());
            return self.changed();
        };
        self.state.lock().unwrap().attn = AttnStatus::Loading;
        self.changed();
        let lens = self.clone();
        self.port.run(Box::new(move || {
            let listed = attn::locate().and_then(|bin| attn::sessions(&bin, &token));
            {
                let mut guard = lens.state.lock().unwrap();
                let state = &mut *guard;
                match listed {
                    Ok(sessions) => {
                        if let Some(capture) = state.capture.as_mut() {
                            let known = capture
                                .target
                                .as_ref()
                                .is_some_and(|id| sessions.iter().any(|s| &s.id == id));
                            if !known {
                                capture.target = attn::default_session(&sessions)
                                    .map(|index| sessions[index].id.clone());
                            }
                        }
                        state.sessions = sessions;
                        state.attn = AttnStatus::Ready;
                    }
                    Err(attn::Failure::NotPaired(text)) => state.attn = AttnStatus::Unpaired(text),
                    Err(attn::Failure::Error(text)) => state.attn = AttnStatus::Unlisted(text),
                }
            }
            lens.changed();
        }));
    }

    /// Asks the user to allow Lens in attn, off the UI thread.
    fn pair(self: &Arc<Self>) {
        {
            let mut state = self.state.lock().unwrap();
            if matches!(state.attn, AttnStatus::Pairing(_)) {
                return;
            }
            state.attn = AttnStatus::Pairing(None);
        }
        self.changed();
        let lens = self.clone();
        self.port.run(Box::new(move || {
            let shown = lens.clone();
            let paired = attn::locate()
                .and_then(|bin| {
                    attn::pair(&bin, |request| {
                        shown.state.lock().unwrap().attn = AttnStatus::Pairing(Some(request.code));
                        shown.changed();
                    })
                })
                .map_err(failure_text)
                .and_then(|token| lens.port.store_token(&token));
            match paired {
                Ok(()) => lens.load_sessions(),
                Err(text) => {
                    lens.state.lock().unwrap().attn = AttnStatus::Unpaired(text);
                    lens.changed();
                }
            }
        }));
    }

    /// ↩ and ⌘⌥↩: sends, or does what must happen before a send can.
    pub fn send_or_recover(self: &Arc<Self>) {
        let attn = self.state.lock().unwrap().attn.clone();
        match attn {
            AttnStatus::Unpaired(_) => self.pair(),
            AttnStatus::Unlisted(_) => self.load_sessions(),
            AttnStatus::Pairing(_) | AttnStatus::Loading => {}
            AttnStatus::Ready => {
                if let Err(error) = self.send(None, None) {
                    self.fail(error);
                }
            }
        }
    }

    /// Sets the message the next send carries.
    pub fn set_message(&self, message: &str) {
        if let Some(capture) = self.state.lock().unwrap().capture.as_mut() {
            capture.message = message.to_string();
        }
    }

    /// Opens or closes the Tab palette. Opening after a failed listing asks again.
    pub fn toggle_palette(self: &Arc<Self>) {
        let reload = {
            let mut state = self.state.lock().unwrap();
            state.palette = if state.palette.is_some() {
                None
            } else {
                let chosen = state.capture.as_ref().and_then(|c| c.target.as_ref());
                Some(
                    chosen
                        .and_then(|id| state.sessions.iter().position(|s| &s.id == id))
                        .unwrap_or(0),
                )
            };
            state.palette.is_some() && matches!(state.attn, AttnStatus::Unlisted(_))
        };
        self.changed();
        if reload {
            self.load_sessions();
        }
    }

    /// Moves the palette's selection, wrapping at either end.
    pub fn move_palette(&self, by: isize) {
        let mut state = self.state.lock().unwrap();
        let rows = state.sessions.len() as isize;
        if let (Some(selected), true) = (state.palette, rows > 0) {
            state.palette = Some((selected as isize + by).rem_euclid(rows) as usize);
            drop(state);
            self.changed();
        }
    }

    /// Makes the palette's selected row, or row `index`, the target and closes it.
    pub fn choose_session(&self, index: Option<usize>) {
        let mut guard = self.state.lock().unwrap();
        let state = &mut *guard;
        let chosen = index.or(state.palette).and_then(|i| state.sessions.get(i));
        if let (Some(session), Some(capture)) = (chosen, state.capture.as_mut()) {
            capture.target = Some(session.id.clone());
        }
        state.palette = None;
        drop(guard);
        self.changed();
    }

    /// Arms a tool for the next stroke; arming it again goes back to plain drawing.
    pub fn arm_tool(&self, tool: InkTool) {
        let mut state = self.state.lock().unwrap();
        state.tool = if state.tool == tool {
            InkTool::default()
        } else {
            tool
        };
        drop(state);
        self.changed();
    }

    /// A press on the ink layer while the draw key is held. Takes the still on the
    /// first press of a view, before any ink (D7 on the plan). None if nothing may
    /// be drawn now.
    pub fn begin_stroke(self: &Arc<Self>) -> Option<StrokeStart> {
        let (open, tool) = {
            let state = self.state.lock().unwrap();
            let capture = state.capture.as_ref()?;
            if state.input != Input::All || state.sending == Some(capture.id) {
                return None;
            }
            (capture.view_still, state.tool)
        };
        let still = match open {
            Some(still) => still,
            None => {
                let still = self.take_still(Cause::Draw)?;
                if let Some(capture) = self.state.lock().unwrap().capture.as_mut() {
                    capture.view_still = Some(still);
                }
                still
            }
        };
        // Read after the still's own time, so a frame is never later than its marks.
        let at = self.port.now();
        Some(StrokeStart {
            still,
            at,
            boxed: self.port.shift_held(),
            tool,
        })
    }

    /// Ends a stroke with its points on the display, turning it into a mark.
    pub fn finish_stroke(&self, start: StrokeStart, points: Vec<(f32, f32)>) {
        let end = self.port.now();
        let mut guard = self.state.lock().unwrap();
        let state = &mut *guard;
        let Some(capture) = state.capture.as_mut() else {
            return;
        };
        if start.tool == InkTool::Label {
            if let Some(at) = points.first().copied() {
                state.label = Some(LabelDraft { start, at, end });
                state.tool = InkTool::default();
                state.keys = KeyOwner::Bar;
                state.bar_key_requests += 1;
            }
        } else {
            let counter = 1 + capture
                .ink
                .iter()
                .filter(|m| m.still == start.still && m.mark.tool == crate::document::Tool::Counter)
                .count();
            if let Some(mark) = ink::mark_from_stroke(&start, &points, counter) {
                let stale = capture.view_still != Some(start.still);
                capture.ink.push(InkMark {
                    still: start.still,
                    mark,
                    times: (start.at, end),
                    committed: false,
                    stale,
                    fade_from: None,
                });
                state.tool = InkTool::default();
                join_finished_ink(capture);
            }
        }
        drop(guard);
        self.changed();
    }

    /// Finishes the label being placed. Empty text drops it.
    pub fn finish_label(&self, text: &str) {
        let mut guard = self.state.lock().unwrap();
        let state = &mut *guard;
        let draft = state.label.take();
        if let (Some(draft), Some(capture)) = (draft, state.capture.as_mut())
            && !text.trim().is_empty()
        {
            let stale = capture.view_still != Some(draft.start.still);
            capture.ink.push(InkMark {
                still: draft.start.still,
                mark: ink::label_mark(draft.at, text.trim()),
                times: (draft.start.at, draft.end),
                committed: false,
                stale,
                fade_from: None,
            });
            join_finished_ink(capture);
        }
        drop(guard);
        self.changed();
    }

    /// Drops a still and the ink drawn on it: a secret can repeat across stills.
    pub fn drop_still(&self, id: u64) {
        let mut state = self.state.lock().unwrap();
        if let Some(capture) = state.capture.as_mut() {
            capture.stills.retain(|s| s.id != id);
            capture.ink.retain(|m| m.still != id);
            if capture.view_still == Some(id) {
                capture.view_still = None;
            }
        }
        drop(state);
        self.changed();
    }

    /// The marks on the ink layer, in points on the capture's display, each as
    /// transparent as its fade has made it.
    pub fn ink_marks(&self) -> Vec<crate::document::Mark> {
        let now = self.port.now();
        let state = self.state.lock().unwrap();
        let ink = state.capture.as_ref().map(|c| c.ink.as_slice());
        ink.unwrap_or_default()
            .iter()
            .map(|m| {
                let mut mark = m.mark.clone();
                if let Some(from) = m.fade_from {
                    let left = 1. - now.since(from).as_secs_f32() / FADE.as_secs_f32();
                    mark.color[3] = (f32::from(mark.color[3]) * left.max(0.)) as u8;
                }
                mark
            })
            .collect()
    }

    /// Starts the fades that are due and removes the ink that has finished. Each
    /// mark begins only once its still exists. Returns true while any is fading.
    pub fn advance_fades(&self) -> bool {
        let now = self.port.now();
        let mut state = self.state.lock().unwrap();
        let Some(capture) = state.capture.as_mut() else {
            return false;
        };
        let due = capture
            .ink
            .iter_mut()
            .filter(|m| m.stale && m.committed && m.fade_from.is_none());
        for mark in due {
            mark.fade_from = Some(now);
        }
        let before = capture.ink.len();
        capture
            .ink
            .retain(|m| m.fade_from.is_none_or(|from| now.since(from) < FADE));
        let removed = capture.ink.len() != before;
        let fading = capture.ink.iter().any(|m| m.fade_from.is_some());
        drop(state);
        if fading || removed {
            self.changed();
        }
        fading
    }

    /// Closes the open capture and lets go of everything it held.
    pub fn end_capture(&self) {
        let voice = {
            let mut state = self.state.lock().unwrap();
            state.capture = None;
            state.input = Input::None;
            state.resizing = false;
            state.watch = None;
            state.label = None;
            state.palette = None;
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

    /// The screen moved under the capture: ends the view, stales its ink, and
    /// takes the still D7 asks for.
    fn watched(self: &Arc<Self>, event: WatchEvent) {
        let cause = match event {
            WatchEvent::Click => Some(Cause::Click),
            WatchEvent::WindowSwitch => Some(Cause::WindowSwitch),
            WatchEvent::Scroll => None,
            WatchEvent::Change(fraction) if fraction >= CHANGE_FRACTION => None,
            WatchEvent::Change(_) => return,
        };
        let cause = {
            let mut guard = self.state.lock().unwrap();
            let state = &mut *guard;
            let Some(capture) = state.capture.as_mut() else {
                return;
            };
            // A running send has its stills; the page may still be clicked.
            let cause = cause.filter(|_| state.sending != Some(capture.id));
            let news = cause.is_some()
                || capture.view_still.is_some()
                || !capture.unseen_change
                || capture.ink.iter().any(|m| !m.stale);
            if !news {
                return;
            }
            capture.view_still = None;
            capture.unseen_change = cause.is_none();
            for mark in &mut capture.ink {
                mark.stale = true;
            }
            cause
        };
        if let Some(cause) = cause {
            self.take_still(cause);
        }
        self.changed();
    }

    /// Takes a still of the open capture's area off the UI thread. Its time is now,
    /// whenever the frame arrives, so stills stay in the order they were asked for.
    /// Returns the id the still will have.
    pub fn take_still(self: &Arc<Self>, cause: Cause) -> Option<u64> {
        let at = self.port.now();
        let (id, area, still_id, covers_change) = {
            let mut state = self.state.lock().unwrap();
            let capture = state.capture.as_mut()?;
            let (id, area, still_id) = (capture.id, capture.area, capture.next_still);
            if cause != Cause::Send && state.sending == Some(id) {
                drop(state);
                self.fail("Attn Lens is sending this capture, so no still was taken.".into());
                return None;
            }
            let covers_change =
                cause == Cause::Send || state.capture.as_ref().is_some_and(|c| c.unseen_change);
            if let Some(capture) = state.capture.as_mut() {
                capture.next_still += 1;
                capture.unseen_change = false;
            }
            // Counted before the state is released, so a send never sees the
            // still neither kept nor on the way.
            self.pending_stills.0.lock().unwrap().push(id);
            (id, area, still_id, covers_change)
        };
        let lens = self.clone();
        self.port.run(Box::new(move || {
            let frame = lens.port.still(area).map(|frame| {
                let thumb = bar::thumbnail(&frame);
                (frame, thumb)
            });
            {
                let mut state = lens.state.lock().unwrap();
                match frame {
                    Ok((frame, thumb)) => {
                        // A capture closed while its frame was on the way drops the frame.
                        if let Some(capture) = state.capture.as_mut().filter(|c| c.id == id) {
                            let still = Still {
                                id: still_id,
                                at,
                                cause,
                                area,
                                thumb,
                                doc: Document::new(frame),
                                times: vec![],
                            };
                            let index = capture.stills.partition_point(|s| s.at <= at);
                            capture.stills.insert(index, still);
                            join_finished_ink(capture);
                        }
                    }
                    Err(error) => {
                        // Ink with no frame under it cannot be sent; the next press retries.
                        let sending = state.sending == Some(id);
                        if let Some(capture) = state.capture.as_mut().filter(|c| c.id == id) {
                            // The change is still unseen, and a send must not go without it.
                            if covers_change {
                                capture.unseen_change = true;
                                if sending {
                                    capture.send_still_error = Some(error.clone());
                                }
                            }
                            capture.ink.retain(|m| m.still != still_id);
                            if capture.view_still == Some(still_id) {
                                capture.view_still = None;
                            }
                        }
                        state.error = Some(error);
                    }
                }
            }
            {
                let mut pending = lens.pending_stills.0.lock().unwrap();
                let index = pending.iter().position(|&c| c == id).unwrap();
                pending.swap_remove(index);
            }
            lens.pending_stills.1.notify_all();
            lens.changed();
        }));
        Some(still_id)
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
            // None kept and none on the way, as when × dropped every one.
            let id = capture.id;
            let in_flight = self.pending_stills.0.lock().unwrap().contains(&id);
            let needs_still = (capture.stills.is_empty() && !in_flight) || capture.unseen_change;
            state.send = SendStatus::Sending;
            state.sending = Some(id);
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
        while pending.contains(&id) {
            pending = self.pending_stills.1.wait(pending).unwrap();
        }
        drop(pending);
        let still_error = self
            .state
            .lock()
            .unwrap()
            .capture
            .as_mut()
            .filter(|c| c.id == id)
            .and_then(|c| c.send_still_error.take());
        let delivered = voice
            .and_then(|()| still_error.map_or(Ok(()), Err))
            .map_err(attn::Failure::Error)
            .and_then(|()| self.deliver(id));
        {
            let mut state = self.state.lock().unwrap();
            state.sending = None;
            state.send = match delivered {
                Ok(target) => {
                    if state.capture.as_ref().is_some_and(|c| c.id == id) {
                        state.capture = None;
                        state.input = Input::None;
                        state.watch = None;
                    }
                    SendStatus::Sent(target)
                }
                Err(attn::Failure::NotPaired(text)) => {
                    self.port.forget_token();
                    state.attn = AttnStatus::Unpaired(text.clone());
                    SendStatus::Failed(text)
                }
                Err(attn::Failure::Error(text)) => SendStatus::Failed(text),
            };
        }
        if !self.is_open() {
            self.port.capture_keys(false);
        }
        self.changed();
    }

    fn deliver(&self, id: u64) -> Result<String, attn::Failure> {
        let plain = attn::Failure::Error;
        let (docs, target, message) = {
            let state = self.state.lock().unwrap();
            let capture = state
                .capture
                .as_ref()
                .filter(|c| c.id == id)
                .ok_or_else(|| plain("The capture was closed before it was sent.".into()))?;
            (
                capture
                    .stills
                    .iter()
                    .map(|s| s.doc.clone())
                    .collect::<Vec<_>>(),
                capture
                    .target
                    .clone()
                    .ok_or_else(|| plain("The capture has no target session.".into()))?,
                capture.message.clone(),
            )
        };
        if docs.is_empty() {
            return Err(plain("The capture has no still to send.".into()));
        }
        let token = self
            .port
            .token()
            .ok_or_else(|| attn::Failure::NotPaired("Attn Lens is not paired with attn.".into()))?;
        let dir = tempfile::Builder::new()
            .prefix("attn-lens-send-")
            .tempdir()
            .map_err(|e| plain(e.to_string()))?;
        let mut files: Vec<PathBuf> = vec![];
        for (index, doc) in docs.iter().enumerate() {
            let path = dir.path().join(format!("frame-{:02}.png", index + 1));
            doc.export()
                .save_with_format(&path, image::ImageFormat::Png)
                .map_err(|e| plain(e.to_string()))?;
            files.push(path);
        }
        let bin = attn::locate()?;
        attn::send(&bin, &token, &target, &files, &message)?;
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
                display: c.display,
                started: c.started,
                stills: c
                    .stills
                    .iter()
                    .map(|s| StillSummary {
                        id: s.id,
                        at: s.at,
                        cause: s.cause,
                        marks: s.doc.marks.len(),
                        times: s.times.clone(),
                        thumb: s.thumb.clone(),
                        width: s.doc.base.width(),
                        height: s.doc.base.height(),
                    })
                    .collect(),
                ink: c
                    .ink
                    .iter()
                    .map(|m| InkSummary {
                        still: m.still,
                        fading: m.fade_from.is_some(),
                    })
                    .collect(),
                segments: c.segments.clone(),
                target: c.target.clone(),
                message: c.message.clone(),
            }),
            picking: state.picking.clone(),
            last_area: state.last_area,
            keys: state.keys,
            bar_key_requests: state.bar_key_requests,
            tool: state.tool,
            placing_label: state.label.is_some(),
            attn: state.attn.clone(),
            sessions: state.sessions.clone(),
            palette: state.palette,
            voice: state.voice.is_some(),
            resizing: state.resizing,
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
            "picking": summary.picking,
            "last_area": summary.last_area,
            "keys": format!("{:?}", summary.keys),
            "tool": format!("{:?}", summary.tool),
            "attn": format!("{:?}", summary.attn),
            "palette": summary.palette,
            "input": format!("{:?}", summary.input),
            "voice": summary.voice,
            "send": send,
            "error": summary.error,
            "capture": summary.capture.map(|c| json!({
                "id": c.id,
                "area": c.area,
                "started": c.started,
                "stills": c.stills.iter().map(|s| json!({
                    "id": s.id,
                    "at": s.at,
                    "cause": s.cause,
                    "marks": s.marks,
                    "times": s.times,
                    "width": s.width,
                    "height": s.height,
                })).collect::<Vec<_>>(),
                "ink": c.ink.iter().map(|m| json!({
                    "still": m.still,
                    "fading": m.fading,
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

/// Puts finished ink on the stills whose frames have arrived, so a stroke and its
/// frame may finish in either order and the frame always exists first.
fn join_finished_ink(capture: &mut Capture) {
    for ink in capture.ink.iter_mut().filter(|m| !m.committed) {
        if let Some(still) = capture.stills.iter_mut().find(|s| s.id == ink.still) {
            let scale = still.doc.base.width() as f32 / still.area.rect[2] as f32;
            still
                .doc
                .commit(ink::on_still(&ink.mark, still.area, scale));
            still.times.push(ink.times);
            ink.committed = true;
        }
    }
}

pub struct StillSummary {
    pub id: u64,
    pub at: session::HostTime,
    pub cause: Cause,
    pub marks: usize,
    pub times: Vec<(HostTime, HostTime)>,
    pub thumb: rune::Pixels,
    pub width: u32,
    pub height: u32,
}

pub struct InkSummary {
    pub still: u64,
    pub fading: bool,
}

pub struct CaptureSummary {
    pub id: u64,
    pub area: session::Area,
    pub display: DisplayFrame,
    pub started: session::HostTime,
    pub stills: Vec<StillSummary>,
    pub ink: Vec<InkSummary>,
    pub segments: Vec<Segment>,
    pub target: Option<String>,
    pub message: String,
}

pub struct Summary {
    pub picking: Option<Vec<DisplayFrame>>,
    pub last_area: Option<Area>,
    pub keys: KeyOwner,
    pub bar_key_requests: u64,
    pub tool: InkTool,
    pub placing_label: bool,
    pub attn: AttnStatus,
    pub sessions: Vec<attn::Session>,
    pub palette: Option<usize>,
    pub input: Input,
    pub resizing: bool,
    pub capture: Option<CaptureSummary>,
    pub voice: bool,
    pub send: SendStatus,
    pub error: Option<String>,
}
