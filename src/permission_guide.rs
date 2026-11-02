//! The floating guide beside System Settings' Privacy & Security pane: it
//! opens the pane for one permission and offers Attn Lens as a drag tile (or,
//! for a refused microphone, asks to switch Lens on), closing once the
//! permission is granted or System Settings closes.
//!
//! Adapted from Hex's `src/desktop/permission_guide.rs`
//! (<https://github.com/anomalyco/hex>, commit 54d1ebf), MIT License,
//! Copyright (c) 2026 Christopher Langton. See THIRD_PARTY_NOTICES.md.

use std::ffi::CStr;
use std::process::Command;

unsafe extern "C" {
    fn lens_show_permission_guide(permission: i32);
    fn lens_permission_granted(permission: i32) -> bool;
    fn lens_permission_list_name(permission: i32) -> *const std::ffi::c_char;
    fn lens_running_app_bundle_id(buf: *mut u8, cap: usize) -> usize;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permission {
    ScreenRecording,
    Microphone,
}

impl Permission {
    /// Matches `LensPermission` in native/permission_guide.m.
    const fn code(self) -> i32 {
        match self {
            Self::ScreenRecording => 1,
            Self::Microphone => 2,
        }
    }

    /// The TCC service `tccutil reset` takes for this permission. The
    /// Microphone list only holds apps that asked, so it has no stuck row to
    /// clear.
    const fn tcc_service(self) -> Option<&'static str> {
        match self {
            Self::ScreenRecording => Some("ScreenCapture"),
            Self::Microphone => None,
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

/// What System Settings calls `permission`'s list, for instructions in Lens.
pub fn list_name(permission: Permission) -> &'static str {
    let name = unsafe { CStr::from_ptr(lens_permission_list_name(permission.code())) };
    name.to_str().unwrap_or("Privacy & Security")
}

/// The running `.app` bundle's `CFBundleIdentifier`, or `None` outside a
/// bundle (a development binary has no row of its own in the list).
pub fn running_bundle_id() -> Option<String> {
    let mut buf = [0u8; 256];
    let len = unsafe { lens_running_app_bundle_id(buf.as_mut_ptr(), buf.len()) };
    if len == 0 {
        return None;
    }
    std::str::from_utf8(&buf[..len]).ok().map(str::to_owned)
}

/// Clears this bundle's TCC row for `permission` off the UI thread, then
/// calls `done` so the caller can show the guide again. A row left from an
/// earlier signature stays grey in System Settings while the new binary is
/// refused; resetting it lets the drag add a working row.
pub fn reset_and_reopen_async(
    permission: Permission,
    done: impl FnOnce(Permission) + Send + 'static,
) {
    let (Some(service), Some(bundle_id)) = (permission.tcc_service(), running_bundle_id()) else {
        done(permission);
        return;
    };
    if let Err(error) = std::thread::Builder::new()
        .name("lens-tcc-reset".into())
        .spawn(move || {
            reset_bundle_service(service, &bundle_id);
            done(permission);
        })
    {
        eprintln!("Permission guide: could not start the TCC reset: {error}");
    }
}

fn reset_bundle_service(service: &str, bundle_id: &str) {
    let output = Command::new("/usr/bin/tccutil")
        .args(["reset", service, bundle_id])
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            eprintln!("Permission guide: could not run tccutil reset {service}: {error}");
            return;
        }
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    // 64 (OSStatus -10814): no bundle on disk, so System Settings already
    // hides the row and there is nothing to clear.
    let nothing_to_clear = output.status.code() == Some(64)
        || stderr.contains("-10814")
        || stderr.contains("No such bundle identifier");
    if !output.status.success() && !nothing_to_clear {
        eprintln!(
            "Permission guide: tccutil reset {service} {bundle_id} failed ({:?}): {}",
            output.status.code(),
            stderr.trim()
        );
    }
}
