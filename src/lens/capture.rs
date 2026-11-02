//! Stills through ScreenCaptureKit in-process (Hex's capture-core). Glance's
//! `screencapture` subprocess adds macOS 26's private-window-picker consent on
//! top of Screen Recording; this path does not.
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

/// One still of `display` at its native pixels, leaving out Lens's own windows.
pub fn screenshot(display: u32) -> Result<RgbaImage, String> {
    let exclude = capture_core::Exclusions {
        windows: vec![],
        apps: vec![std::process::id() as i32],
    };
    let shot = capture_core::screenshot(&capture_core::Source::Display(display), &exclude, None)
        .map_err(|e| format!("Screen capture failed: {e}"))?;
    RgbaImage::from_raw(shot.width, shot.height, shot.rgba)
        .ok_or_else(|| "Screen capture returned a buffer that does not match its size".into())
}
