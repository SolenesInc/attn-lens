//! The headless port. A Rune script drives it with marks, on the virtual clock:
//!
//! ```text
//! key:draw-down  key:draw-up  key:capture  key:voice  key:keyboard  key:send
//! watch:click  watch:scroll  watch:switch  watch:change:0.7
//! say:0.5:1.2:the words      a segment from 0.5 s to 1.2 s, while voice runs
//! ```
//!
//! Any other mark is left to the script. Work runs inline, so a tree read at a
//! mark sees every still and send that mark caused.
use super::{Chord, FAKE_TICK, KeyEvent, PlatformPort, Sink, Voice, Watch, WatchEvent};
use crate::lens::session::{Area, HostTime, Segment};
use image::{Rgba, RgbaImage};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// The display a fake capture covers, in points.
pub const DISPLAY: Area = Area {
    display: 1,
    rect: [0., 0., 640., 400.],
};

type Sinks = Arc<Mutex<(Option<Sink<WatchEvent>>, Option<Sink<Segment>>)>>;

#[derive(Default)]
struct Script {
    marks: Vec<(String, f64)>,
    fired: usize,
    now: f64,
    keys: Option<Sink<KeyEvent>>,
}

pub struct FakePort {
    script: Mutex<Script>,
    sinks: Sinks,
    token: Option<String>,
}

impl FakePort {
    pub fn new(marks: Vec<(String, f64)>, token: Option<&str>) -> Self {
        Self {
            script: Mutex::new(Script {
                marks,
                ..Default::default()
            }),
            sinks: Default::default(),
            token: token.map(str::to_string),
        }
    }

    /// Script marks are read on the first tick: Rune publishes the run only
    /// after the window factory returns.
    pub fn from_headless_run() -> Self {
        Self::new(vec![], Some("fake-token"))
    }

    fn fire(&self, mark: &str) {
        if let Some(event) = key_event(mark) {
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
        self.token.clone()
    }
    fn run(&self, job: Box<dyn FnOnce() + Send>) {
        job();
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
