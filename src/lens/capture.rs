//! Stills through ScreenCaptureKit in-process (Hex's capture-core). Glance's
//! `screencapture` subprocess adds macOS 26's private-window-picker consent on
//! top of Screen Recording; this path does not.
use super::session::{Area, DisplayFrame};
use image::RgbaImage;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *const std::ffi::c_void) -> *const std::ffi::c_void;
    fn CGEventGetLocation(event: *const std::ffi::c_void) -> CGPoint;
    fn CGGetDisplaysWithPoint(point: CGPoint, max: u32, displays: *mut u32, count: *mut u32)
    -> i32;
    fn CGMainDisplayID() -> u32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const std::ffi::c_void);
}

/// Where the pointer is, in global points with the origin at the main display's
/// top left. None if the system gives no event to read it from.
pub fn pointer() -> Option<(f64, f64)> {
    // SAFETY: a new event from no source only reads the current pointer; it is released.
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some((point.x, point.y))
    }
}

/// The display the pointer is on, as a `CGDirectDisplayID`.
pub fn display_under_pointer() -> u32 {
    let Some((x, y)) = pointer() else {
        // SAFETY: no arguments; always returns a display id.
        return unsafe { CGMainDisplayID() };
    };
    let (mut display, mut count) = (0u32, 0u32);
    // SAFETY: room for one display id and its count.
    unsafe {
        if CGGetDisplaysWithPoint(CGPoint { x, y }, 1, &mut display, &mut count) == 0 && count == 1
        {
            display
        } else {
            CGMainDisplayID()
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    origin: [f64; 2],
    size: [f64; 2],
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGDisplayBounds(display: u32) -> Rect;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
}

/// The size of `display`, in points.
pub fn display_size(display: u32) -> (f64, f64) {
    // SAFETY: CoreGraphics returns an empty rect for an unknown display.
    let bounds = unsafe { CGDisplayBounds(display) };
    (bounds.size[0], bounds.size[1])
}

unsafe extern "C" {
    /// From `native/displays.m`.
    fn lens_menu_bar_points(display: u32) -> f64;
    fn lens_dock_points(display: u32) -> f64;
}

/// The displays connected now. CoreGraphics reports how many there are first.
pub fn displays() -> Vec<DisplayFrame> {
    let mut count = 0u32;
    // SAFETY: a null list asks only for the count.
    if unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) } != 0 {
        return vec![];
    }
    let mut ids = vec![0u32; count as usize];
    // SAFETY: `ids` holds `count` entries.
    if unsafe { CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut count) } != 0 {
        return vec![];
    }
    ids.truncate(count as usize);
    ids.into_iter()
        .map(|id| {
            // SAFETY: as in `display_size`.
            let bounds = unsafe { CGDisplayBounds(id) };
            DisplayFrame {
                id,
                frame: [
                    bounds.origin[0],
                    bounds.origin[1],
                    bounds.size[0],
                    bounds.size[1],
                ],
                // SAFETY: reads NSScreen's list; no other effect.
                menu_bar: unsafe { lens_menu_bar_points(id) },
                // SAFETY: as above.
                dock: unsafe { lens_dock_points(id) },
            }
        })
        .collect()
}

/// `area` in global top-left points: `[left, top, right, bottom]`.
pub fn global_bounds(area: Area) -> [f64; 4] {
    // SAFETY: as in `display_size`.
    let display = unsafe { CGDisplayBounds(area.display) };
    let left = display.origin[0] + area.rect[0];
    let top = display.origin[1] + area.rect[1];
    [left, top, left + area.rect[2], top + area.rect[3]]
}

/// One still of `area` at its native pixels, leaving out Lens's own windows.
pub fn screenshot(area: Area) -> Result<RgbaImage, String> {
    frame(area, None)
}

/// A frame of `area` with its long edge at most `long_edge` pixels, leaving out
/// Lens's own windows.
pub fn frame(area: Area, long_edge: Option<u32>) -> Result<RgbaImage, String> {
    let exclude = capture_core::Exclusions {
        windows: vec![],
        apps: vec![std::process::id() as i32],
    };
    let source = capture_core::Source::Region {
        display: area.display,
        rect: capture_core::Rect {
            x: area.rect[0],
            y: area.rect[1],
            width: area.rect[2],
            height: area.rect[3],
        },
    };
    let shot = capture_core::screenshot(&source, &exclude, long_edge)
        .map_err(|e| format!("Screen capture failed: {e}"))?;
    RgbaImage::from_raw(shot.width, shot.height, shot.rgba)
        .ok_or_else(|| "Screen capture returned a buffer that does not match its size".into())
}
