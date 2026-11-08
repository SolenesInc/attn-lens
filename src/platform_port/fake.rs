//! The headless port. A Rune script drives it with marks, on the virtual clock:
//!
//! ```text
//! key:draw-down  key:draw-up  key:capture  key:voice  key:keyboard  key:send
//! key:shift-down  key:shift-up
//! watch:click  watch:scroll  watch:switch  watch:change:0.7
//! say:0.5:1.2:the words      a segment from 0.5 s to 1.2 s, while voice runs
//! clipboard:PATH             the system clipboard now holds the image at PATH
//! ocr:the words              text recognition finds these words from now on
//! pick:#RRGGBB  pick:cancel  answers a Pick from screen that is waiting
//! ```
//!
//! Any other mark is left to the script. Work runs inline, so a tree read at a
//! mark sees every still and send that mark caused.
//!
//! When `ATTN_LENS_FAKE_CLIPBOARD` names a folder, what the app copies lands in
//! it as `image.png` and `text.txt`, so a test can read what a user would paste.
use super::{Chord, FAKE_TICK, KeyEvent, PlatformPort, Sink, Voice, Watch, WatchEvent};
use crate::lens::session::{Area, DisplayFrame, HostTime, Segment};
use image::{Rgba, RgbaImage};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// The display a fake capture covers, in points.
/// The menu bar's height on a Mac display without a notch.
const MENU_BAR: f64 = 24.;

pub const DISPLAY: Area = Area {
    display: 1,
    rect: [0., 0., 640., 400.],
};

type Sinks = Arc<Mutex<(Option<Sink<WatchEvent>>, Option<Sink<Segment>>)>>;

type PickSink = Sink<Result<Option<[u8; 3]>, String>>;

#[derive(Default)]
struct Script {
    marks: Vec<(String, f64)>,
    fired: usize,
    now: f64,
    keys: Option<Sink<KeyEvent>>,
    clipboard: Option<RgbaImage>,
    ocr: String,
    pick: Option<PickSink>,
    /// After a `jobs:hold` mark, jobs wait here until a `jobs:next` mark runs the oldest,
    /// a `jobs:last` mark the newest, or a `jobs:drain` mark every one waiting.
    held: Option<std::collections::VecDeque<Box<dyn FnOnce() + Send>>>,
}

pub struct FakePort {
    script: Mutex<Script>,
    sinks: Sinks,
    token: Mutex<Option<String>>,
    displays: Mutex<Vec<DisplayFrame>>,
    shift: AtomicBool,
}

impl FakePort {
    pub fn new(marks: Vec<(String, f64)>, token: Option<&str>) -> Self {
        Self {
            script: Mutex::new(Script {
                marks,
                ..Default::default()
            }),
            sinks: Default::default(),
            token: Mutex::new(token.map(str::to_string)),
            displays: Mutex::new(vec![DisplayFrame {
                id: DISPLAY.display,
                frame: DISPLAY.rect,
                menu_bar: MENU_BAR,
            }]),
            shift: AtomicBool::new(false),
        }
    }

    /// Script marks are read on the first tick: Rune publishes the run only
    /// after the window factory returns.
    pub fn from_headless_run() -> Self {
        Self::new(vec![], Some("fake-token"))
    }

    fn fire(&self, mark: &str) {
        if mark == "key:shift-down" || mark == "key:shift-up" {
            self.shift.store(mark == "key:shift-down", Ordering::SeqCst);
        } else if let Some(event) = key_event(mark) {
            let sink = self.script.lock().unwrap().keys.clone();
            if let Some(sink) = sink {
                sink(event);
            }
        } else if let Some(event) = watch_event(mark) {
            let sink = self.sinks.lock().unwrap().0.clone();
            if let Some(sink) = sink {
                sink(event);
            }
        } else if let Some(segment) = segment(mark) {
            let sink = self.sinks.lock().unwrap().1.clone();
            if let Some(sink) = sink {
                sink(segment);
            }
        } else if let Some(path) = mark.strip_prefix("clipboard:") {
            let image = image::open(path).expect("clipboard: mark names a readable image");
            self.script.lock().unwrap().clipboard = Some(image.to_rgba8());
        } else if let Some(text) = mark.strip_prefix("ocr:") {
            self.script.lock().unwrap().ocr = text.to_string();
        } else if mark == "jobs:hold" {
            self.script.lock().unwrap().held = Some(Default::default());
        } else if mark == "jobs:next" || mark == "jobs:last" {
            let job = self.script.lock().unwrap().held.as_mut().and_then(|held| {
                if mark == "jobs:next" {
                    held.pop_front()
                } else {
                    held.pop_back()
                }
            });
            if let Some(job) = job {
                job();
            }
        } else if mark == "jobs:drain" {
            let jobs = self
                .script
                .lock()
                .unwrap()
                .held
                .as_mut()
                .map(std::mem::take)
                .unwrap_or_default();
            for job in jobs {
                job();
            }
        } else if let Some(answer) = mark.strip_prefix("pick:") {
            let sink = self.script.lock().unwrap().pick.take();
            if let Some(sink) = sink {
                sink(Ok(picked(answer)));
            }
        }
    }
}

fn key_event(mark: &str) -> Option<KeyEvent> {
    Some(match mark.strip_prefix("key:")? {
        "draw-down" => KeyEvent::DrawDown,
        "draw-up" => KeyEvent::DrawUp,
        "capture" => KeyEvent::Pressed(Chord::Capture),
        "voice" => KeyEvent::Pressed(Chord::Voice),
        "keyboard" => KeyEvent::Pressed(Chord::Keyboard),
        "send" => KeyEvent::Pressed(Chord::Send),
        _ => return None,
    })
}

fn watch_event(mark: &str) -> Option<WatchEvent> {
    let event = mark.strip_prefix("watch:")?;
    Some(match event {
        "click" => WatchEvent::Click,
        "scroll" => WatchEvent::Scroll,
        "switch" => WatchEvent::WindowSwitch,
        _ => WatchEvent::Change(event.strip_prefix("change:")?.parse().ok()?),
    })
}

fn segment(mark: &str) -> Option<Segment> {
    let mut parts = mark.strip_prefix("say:")?.splitn(3, ':');
    let start = parts.next()?.parse().ok()?;
    let end = parts.next()?.parse().ok()?;
    Some(Segment {
        start: HostTime::from_secs(start),
        end: HostTime::from_secs(end),
        text: parts.next()?.to_string(),
    })
}

fn picked(answer: &str) -> Option<[u8; 3]> {
    let hex = u32::from_str_radix(answer.strip_prefix('#')?, 16).ok()?;
    Some([(hex >> 16) as u8, (hex >> 8) as u8, hex as u8])
}

/// Where the fake's clipboard writes land, when a test asked for them.
fn clipboard_file(name: &str) -> Option<std::path::PathBuf> {
    Some(std::path::Path::new(&std::env::var_os("ATTN_LENS_FAKE_CLIPBOARD")?).join(name))
}

impl PlatformPort for FakePort {
    fn now(&self) -> HostTime {
        HostTime::from_secs(self.script.lock().unwrap().now)
    }
    fn keys(&self, sink: Sink<KeyEvent>) {
        self.script.lock().unwrap().keys = Some(sink);
    }
    fn capture_keys(&self, _open: bool) {}
    fn pointer_display(&self) -> Result<Area, String> {
        Ok(DISPLAY)
    }
    fn displays(&self) -> Vec<DisplayFrame> {
        self.displays.lock().unwrap().clone()
    }
    /// The first display is the main one and has the menu bar, as on a Mac.
    fn launch_displays(&self, mut displays: Vec<DisplayFrame>) {
        if let Some(main) = displays.first_mut() {
            main.menu_bar = MENU_BAR;
        }
        *self.displays.lock().unwrap() = displays;
    }
    fn shift_held(&self) -> bool {
        self.shift.load(Ordering::SeqCst)
    }
    fn screen_recording(&self) -> bool {
        true
    }
    fn still(&self, area: Area) -> Result<RgbaImage, String> {
        // Two pixels per point, as on a Retina display.
        let (width, height) = ((area.rect[2] * 2.) as u32, (area.rect[3] * 2.) as u32);
        Ok(RgbaImage::from_fn(width, height, |x, y| {
            Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        }))
    }
    fn watch(&self, _area: Area, sink: Sink<WatchEvent>) -> Watch {
        self.sinks.lock().unwrap().0 = Some(sink);
        let sinks = self.sinks.clone();
        Watch(Some(Box::new(move || sinks.lock().unwrap().0 = None)))
    }
    fn voice(&self, sink: Sink<Segment>) -> Result<Voice, String> {
        self.sinks.lock().unwrap().1 = Some(sink);
        let sinks = self.sinks.clone();
        Ok(Voice(Box::new(move || {
            sinks.lock().unwrap().1 = None;
            Ok(())
        })))
    }
    fn token(&self) -> Option<String> {
        self.token.lock().unwrap().clone()
    }
    fn store_token(&self, token: &str) -> Result<(), String> {
        *self.token.lock().unwrap() = Some(token.to_string());
        Ok(())
    }
    fn forget_token(&self) {
        *self.token.lock().unwrap() = None;
    }
    fn copy_image(&self, image: RgbaImage) -> Result<(), String> {
        if let Some(path) = clipboard_file("image.png") {
            image.save(path).map_err(|e| e.to_string())?;
        }
        self.script.lock().unwrap().clipboard = Some(image);
        Ok(())
    }
    fn clipboard_image(&self) -> Result<RgbaImage, String> {
        self.script
            .lock()
            .unwrap()
            .clipboard
            .clone()
            .ok_or_else(|| "The clipboard doesn’t contain an image.".into())
    }
    fn copy_text(&self, text: &str) -> Result<(), String> {
        if let Some(path) = clipboard_file("text.txt") {
            std::fs::write(path, text).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn ocr(&self, _image: &RgbaImage, _rectangle: Option<[u32; 4]>) -> Result<String, String> {
        Ok(self.script.lock().unwrap().ocr.clone())
    }
    fn exposes_state(&self) -> bool {
        true
    }
    fn pick_color(&self, sink: PickSink) {
        self.script.lock().unwrap().pick = Some(sink);
    }
    fn run(&self, job: Box<dyn FnOnce() + Send>) {
        let mut script = self.script.lock().unwrap();
        match &mut script.held {
            Some(held) => held.push_back(job),
            None => {
                drop(script);
                job();
            }
        }
    }
    fn tick(&self, now: f64) -> Option<Duration> {
        let run = rune::tooling::headless_run();
        loop {
            let mark = {
                let mut script = self.script.lock().unwrap();
                script.now = script.now.max(now);
                if script.marks.is_empty()
                    && let Some(run) = run
                {
                    script.marks = run.marks().map(|(n, t)| (n.to_string(), t)).collect();
                }
                let fired = script.fired;
                match script.marks.get(fired).filter(|(_, t)| *t <= now).cloned() {
                    Some(mark) => {
                        script.fired += 1;
                        mark.0
                    }
                    None => return Some(FAKE_TICK),
                }
            };
            self.fire(&mark);
        }
    }
}
