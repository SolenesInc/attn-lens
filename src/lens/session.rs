//! One capture and what it holds, on one clock.
use crate::document::{Document, Mark};
use serde::Serialize;

/// Nanoseconds on the host clock. There is one clock: the macOS port reads
/// `mach_absolute_time`, and the headless fake reads Rune's virtual clock, so
/// headless runs repeat exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct HostTime(pub u64);

impl HostTime {
    pub fn from_secs(seconds: f64) -> Self {
        Self((seconds * 1e9) as u64)
    }
    pub fn since(self, earlier: HostTime) -> std::time::Duration {
        std::time::Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }
}

/// The rectangle of one display that a capture covers, in points from the
/// display's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Area {
    /// `CGDirectDisplayID` on macOS.
    pub display: u32,
    pub rect: [f64; 4],
}

/// One display as it stands now: its id and its frame in points, in the one
/// global space macOS uses for displays.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DisplayFrame {
    pub id: u32,
    pub frame: [f64; 4],
    /// Points the menu bar takes at the display's top. The corner handles stay
    /// below it, since they sit above the menu bar and would take its clicks.
    pub menu_bar: f64,
}

impl DisplayFrame {
    /// The whole display as an area.
    pub fn whole(self) -> Area {
        Area {
            display: self.id,
            rect: [0., 0., self.frame[2], self.frame[3]],
        }
    }
}

/// Why a still was taken (D7 on the plan).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    Draw,
    Hotkey,
    Click,
    WindowSwitch,
    Send,
}

/// One full-resolution frame of the area, with its marks and their times.
pub struct Still {
    pub id: u64,
    pub at: HostTime,
    pub cause: Cause,
    /// The area when the frame was asked for: a corner handle may move it later.
    pub area: Area,
    pub thumb: rune::Pixels,
    pub doc: Document,
    /// One span per mark, by index. A mark added in the editor has none.
    pub times: Vec<(HostTime, HostTime)>,
}

/// One phrase of transcript, from its first speech frame to its last.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Segment {
    pub start: HostTime,
    pub end: HostTime,
    pub text: String,
}

/// A mark on the ink layer, in points on its display. It joins its still's
/// `Document` once both the stroke and the frame are finished.
#[derive(Clone)]
pub struct InkMark {
    pub still: u64,
    pub mark: Mark,
    pub times: (HostTime, HostTime),
    pub committed: bool,
}

/// One live session from ⌘⌥X to Send or Esc, over one area.
pub struct Capture {
    pub id: u64,
    pub area: Area,
    pub display: DisplayFrame,
    pub started: HostTime,
    pub stills: Vec<Still>,
    pub ink: Vec<InkMark>,
    /// The still that strokes on the current view join (D7 on the plan).
    pub view_still: Option<u64>,
    pub next_still: u64,
    pub segments: Vec<Segment>,
    /// The session id the capture will be sent to.
    pub target: Option<String>,
    pub message: String,
}
