//! One capture and what it holds, on one clock.
use crate::document::Document;
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
    pub at: HostTime,
    pub cause: Cause,
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

/// One live session from ⌘⌥X to Send or Esc, over one area.
pub struct Capture {
    pub id: u64,
    pub area: Area,
    pub started: HostTime,
    pub stills: Vec<Still>,
    pub segments: Vec<Segment>,
    /// The session id the capture will be sent to.
    pub target: Option<String>,
    pub message: String,
}
