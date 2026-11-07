//! Stills through ScreenCaptureKit in-process (Hex's capture-core). Glance's
//! `screencapture` subprocess adds macOS 26's private-window-picker consent on
//! top of Screen Recording; this path does not.
use super::session::Area;
use image::RgbaImage;

/// The display the pointer is on, as a `CGDirectDisplayID`.
pub fn display_under_pointer() -> u32 {
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
        fn CGGetDisplaysWithPoint(
            point: CGPoint,
            max: u32,
            displays: *mut u32,
            count: *mut u32,
        ) -> i32;
        fn CGMainDisplayID() -> u32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const std::ffi::c_void);
    }
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return CGMainDisplayID();
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        let (mut display, mut count) = (0u32, 0u32);
        if CGGetDisplaysWithPoint(point, 1, &mut display, &mut count) == 0 && count == 1 {
            display
        } else {
            CGMainDisplayID()
        }
    }
}

/// The size of `display`, in points.
pub fn display_size(display: u32) -> (f64, f64) {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        origin: [f64; 2],
        size: [f64; 2],
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGDisplayBounds(display: u32) -> Rect;
    }
    // SAFETY: CoreGraphics returns an empty rect for an unknown display.
    let bounds = unsafe { CGDisplayBounds(display) };
    (bounds.size[0], bounds.size[1])
}

/// One still of `area` at its native pixels, leaving out Lens's own windows.
pub fn screenshot(area: Area) -> Result<RgbaImage, String> {
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
    let shot = capture_core::screenshot(&source, &exclude, None)
        .map_err(|e| format!("Screen capture failed: {e}"))?;
    RgbaImage::from_raw(shot.width, shot.height, shot.rgba)
        .ok_or_else(|| "Screen capture returned a buffer that does not match its size".into())
}
