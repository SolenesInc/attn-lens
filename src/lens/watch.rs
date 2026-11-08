//! macOS's `PlatformPort::watch`: what happens on screen during a capture. Clicks
//! and scrolls in the area come from global `NSEvent` monitors, which need no
//! permission and do not see events aimed at Lens's own windows. A window switch is
//! a new front window, looked for when an app activates and on a poll, because
//! ⌘` inside one app activates nothing. A sampling loop compares small frames.
//!
//! Set `LENS_WATCH_LOG=1` to print every event to stderr: that is how the
//! thresholds below were measured.
#![allow(unexpected_cfgs)] // objc 0.2 macros use a legacy cargo-clippy feature.
use super::{
    capture,
    session::{Area, HostTime},
};
use crate::platform_port::{
    Sink, Watch, WatchEvent,
    mac::{host_now, queue_on_main},
};
use block2::RcBlock;
use image::RgbaImage;
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::c_void,
    ptr::{self, NonNull},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

/// F23 on the plan: a call took 0.6 ms, about 1% of a core at this rate, and it
/// caught every ⌘` switch.
const WINDOW_POLL: Duration = Duration::from_millis(66);
/// Sampling at this rate and size cost 1.5–2.0% of a core in the Lens VM (C5 on
/// s-dqjzpt, with `CHANGE_FRACTION` measured at them). A new screen fades ink within
/// a quarter of a second.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(250);
const SAMPLE_LONG_EDGE: u32 = 640;
/// How far a channel must move for a pixel of a sample to count as changed. Past
/// the noise: an untouched page measured at most 0.0008 of the area.
const PIXEL_TOLERANCE: u8 = 16;

/// NSEventMask: left, right and other mouse down.
const CLICK_MASK: u64 = (1 << 1) | (1 << 3) | (1 << 25);
const SCROLL_MASK: u64 = 1 << 22;

/// `kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements`.
const ON_SCREEN_WINDOWS: u32 = 1 | (1 << 4);
/// `kCFNumberSInt64Type`.
const CF_NUMBER_I64: i32 = 4;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGWindowLayer: *const c_void;
    static kCGWindowNumber: *const c_void;
    static kCGWindowOwnerPID: *const c_void;
    fn CGEventGetLocation(event: *const c_void) -> CGPoint;
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: *const c_void, kind: i32, value: *mut i64) -> bool;
    fn CFRelease(cf: *const c_void);
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSWorkspaceDidActivateApplicationNotification: *const AnyObject;
}

/// The window number of the frontmost normal window that is not Lens's.
fn front_window() -> Option<i64> {
    fn field(window: *const c_void, key: *const c_void) -> Option<i64> {
        let mut value = 0;
        // SAFETY: `window` is a live dictionary from the list below; these keys hold numbers.
        unsafe {
            let number = CFDictionaryGetValue(window, key);
            (!number.is_null() && CFNumberGetValue(number, CF_NUMBER_I64, &mut value))
                .then_some(value)
        }
    }
    let own = i64::from(std::process::id());
    // SAFETY: the list is released below, and its windows are read only before that.
    unsafe {
        let list = CGWindowListCopyWindowInfo(ON_SCREEN_WINDOWS, 0);
        if list.is_null() {
            return None;
        }
        // The list runs front to back. Layer 0 holds normal windows.
        let front = (0..CFArrayGetCount(list))
            .map(|index| CFArrayGetValueAtIndex(list, index))
            .find(|&window| {
                field(window, kCGWindowLayer) == Some(0)
                    && field(window, kCGWindowOwnerPID) != Some(own)
            })
            .and_then(|window| field(window, kCGWindowNumber));
        CFRelease(list);
        front
    }
}

/// Reports a new front window once, whichever signal notices it first.
struct WindowSwitches {
    front: Mutex<Option<i64>>,
    sink: Sink<WatchEvent>,
}

impl WindowSwitches {
    fn look(&self) {
        let Some(now) = front_window() else { return };
        if self.front.lock().unwrap().replace(now) != Some(now) {
            (self.sink)(WatchEvent::WindowSwitch);
        }
    }
}

/// The fraction of pixels that moved by more than [`PIXEL_TOLERANCE`] in a channel.
fn change_fraction(before: &RgbaImage, after: &RgbaImage) -> f32 {
    let changed = before
        .pixels()
        .zip(after.pixels())
        .filter(|(a, b)| {
            a.0.iter()
                .zip(b.0)
                .any(|(a, b)| a.abs_diff(b) > PIXEL_TOLERANCE)
        })
        .count();
    changed as f32 / before.len().max(1) as f32 * 4.
}

/// Polls the front window and compares frames until `stop` drops its sender.
fn poll(area: Area, switches: Arc<WindowSwitches>, stop: mpsc::Receiver<()>) {
    let mut previous: Option<(RgbaImage, HostTime)> = None;
    let mut sampled_at: Option<Instant> = None;
    let mut reported_failure = false;
    while let Err(mpsc::RecvTimeoutError::Timeout) = stop.recv_timeout(WINDOW_POLL) {
        switches.look();
        if sampled_at.is_some_and(|at| at.elapsed() < SAMPLE_INTERVAL) {
            continue;
        }
        sampled_at = Some(Instant::now());
        let taken = host_now();
        match capture::frame(area, Some(SAMPLE_LONG_EDGE)) {
            Ok(frame) => {
                if let Some((before, baseline)) = previous
                    .as_ref()
                    .filter(|(b, _)| b.dimensions() == frame.dimensions())
                {
                    let fraction = change_fraction(before, &frame);
                    (switches.sink)(WatchEvent::Change(fraction, *baseline));
                }
                previous = Some((frame, taken));
            }
            Err(error) if !reported_failure => {
                reported_failure = true;
                eprintln!("Attn Lens: screen watching stopped comparing frames: {error}");
            }
            Err(_) => {}
        }
    }
}

/// What AppKit holds for one watch. Lives on the main thread.
struct Installed {
    monitors: Vec<Retained<AnyObject>>,
    observer: Retained<AnyObject>,
}

thread_local! {
    static INSTALLED: RefCell<HashMap<u64, Installed>> = RefCell::new(HashMap::new());
}

fn workspace_center() -> Retained<AnyObject> {
    let class = AnyClass::get(c"NSWorkspace").expect("AppKit is linked");
    // SAFETY: class and instance properties that return live objects.
    unsafe {
        let workspace: Retained<AnyObject> = msg_send![class, sharedWorkspace];
        msg_send![&workspace, notificationCenter]
    }
}

fn pointer_in(event: &AnyObject, bounds: [f64; 4]) -> bool {
    // SAFETY: `event` is a live NSEvent, and its CGEvent lives as long as it does.
    let point = unsafe {
        let event: *const c_void = msg_send![event, CGEvent];
        CGEventGetLocation(event)
    };
    (bounds[0]..bounds[2]).contains(&point.x) && (bounds[1]..bounds[3]).contains(&point.y)
}

fn install(bounds: [f64; 4], switches: Arc<WindowSwitches>) -> Installed {
    let add_monitor = |mask: u64, respond: Box<dyn Fn(&AnyObject)>| {
        let class = AnyClass::get(c"NSEvent").expect("AppKit is linked");
        let block = RcBlock::new(move |event: NonNull<AnyObject>| {
            // SAFETY: AppKit passes a live event.
            respond(unsafe { event.as_ref() });
        });
        // SAFETY: the mask and block match the method's signature; AppKit copies the block.
        let monitor: Option<Retained<AnyObject>> = unsafe {
            msg_send![class, addGlobalMonitorForEventsMatchingMask: mask, handler: &*block]
        };
        monitor
    };
    let clicks = {
        let sink = switches.sink.clone();
        add_monitor(
            CLICK_MASK,
            Box::new(move |event| {
                if pointer_in(event, bounds) {
                    sink(WatchEvent::Click);
                }
            }),
        )
    };
    let scrolls = {
        let sink = switches.sink.clone();
        add_monitor(
            SCROLL_MASK,
            Box::new(move |event| {
                // SAFETY: both are NSEvent properties of a scroll event. Fingers
                // resting on a trackpad send scroll events that move nothing.
                let moved = unsafe {
                    let dx: f64 = msg_send![event, scrollingDeltaX];
                    let dy: f64 = msg_send![event, scrollingDeltaY];
                    dx != 0. || dy != 0.
                };
                if moved && pointer_in(event, bounds) {
                    sink(WatchEvent::Scroll);
                }
            }),
        )
    };
    let activated = RcBlock::new(move |_note: NonNull<AnyObject>| switches.look());
    let none: *const AnyObject = ptr::null();
    // SAFETY: the name is AppKit's constant; with no object and queue the block runs
    // for every activation on the posting (main) thread, and AppKit copies it.
    let observer: Retained<AnyObject> = unsafe {
        msg_send![&workspace_center(), addObserverForName: NSWorkspaceDidActivateApplicationNotification, object: none, queue: none, usingBlock: &*activated]
    };
    Installed {
        monitors: [clicks, scrolls].into_iter().flatten().collect(),
        observer,
    }
}

fn uninstall(installed: Installed) {
    let class = AnyClass::get(c"NSEvent").expect("AppKit is linked");
    for monitor in installed.monitors {
        // SAFETY: `monitor` is a token from the add call in `install`.
        let () = unsafe { msg_send![class, removeMonitor: &*monitor] };
    }
    // SAFETY: `observer` is the token from `install`.
    let () = unsafe { msg_send![&workspace_center(), removeObserver: &*installed.observer] };
}

pub fn start(area: Area, sink: Sink<WatchEvent>) -> Watch {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let sink: Sink<WatchEvent> = if std::env::var_os("LENS_WATCH_LOG").is_some() {
        Arc::new(move |event| {
            eprintln!("watch {event:?}");
            sink(event);
        })
    } else {
        sink
    };
    let switches = Arc::new(WindowSwitches {
        front: Mutex::new(front_window()),
        sink,
    });
    let bounds = capture::global_bounds(area);
    let main_switches = switches.clone();
    // Install and removal both queue, so removal always follows its install.
    queue_on_main(move || {
        let installed = install(bounds, main_switches);
        INSTALLED.with(|all| all.borrow_mut().insert(id, installed));
    });
    let (stop, stopped) = mpsc::channel();
    std::thread::spawn(move || poll(area, switches, stopped));
    Watch(Some(Box::new(move || {
        drop(stop);
        queue_on_main(move || {
            if let Some(installed) = INSTALLED.with(|all| all.borrow_mut().remove(&id)) {
                uninstall(installed);
            }
        });
    })))
}
