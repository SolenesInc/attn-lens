//! The floating guide beside System Settings' Privacy & Security pane: it
//! opens the pane for one permission and offers Attn Lens as a drag tile,
//! closing once the permission is granted or System Settings closes.
//!
//! Adapted from Hex's `src/desktop/permission_guide.rs`
//! (<https://github.com/anomalyco/hex>, commit 54d1ebf), MIT License,
//! Copyright (c) 2026 Christopher Langton. See THIRD_PARTY_NOTICES.md.

unsafe extern "C" {
    fn lens_show_permission_guide(permission: i32);
    fn lens_permission_granted(permission: i32) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permission {
    ScreenRecording,
    Microphone,
    /// The capture, voice and draw keys: the key engine's event taps need it.
    Accessibility,
}

impl Permission {
    /// Matches `LensPermission` in native/permission_guide.m.
    const fn code(self) -> i32 {
        match self {
            Self::ScreenRecording => 1,
            Self::Microphone => 2,
            Self::Accessibility => 3,
        }
    }
}

/// Opens System Settings at `permission`'s list and, inside a `.app` bundle,
/// floats the guide beside it. Safe from any thread; the UI runs on the main
/// thread.
pub fn show(permission: Permission) {
    unsafe { lens_show_permission_guide(permission.code()) };
}

/// Whether `permission` is granted as System Settings shows it now. A Screen
/// Recording grant still needs a relaunch before captures see it.
pub fn granted(permission: Permission) -> bool {
    unsafe { lens_permission_granted(permission.code()) }
}
