//! macOS's input for [`super::Keys`]: event taps on their own run-loop thread,
//! and the machine on another, so a slow sink never stalls the taps.
//!
//! Adapted from Hex's `src/suppression.rs` (`run_event_tap`, `TapSet`,
//! `HealthTimer`; <https://github.com/anomalyco/hex>, commit 54d1ebf), MIT
//! License, Copyright (c) 2026 Christopher Langton. See THIRD_PARTY_NOTICES.md.
//! Every tap only listens: Lens never swallows a key.
use super::{InputEvent, Keys, Physical};
use crate::{
    lens::session::HostTime,
    platform_port::mac::host_time,
    platform_port::{KeyEvent, Sink},
};
use std::{
    ffi::c_void,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicPtr, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::Duration,
};

const EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const EVENT_KEY_DOWN: u32 = 10;
const EVENT_KEY_UP: u32 = 11;
const EVENT_FLAGS_CHANGED: u32 = 12;
const EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = u32::MAX - 1;
const EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = u32::MAX;
const KEYBOARD_EVENT_KEYCODE: u32 = 9;

// Assistive keyboards inject downstream of HID; observe them alongside hardware input.
const ANNOTATED_SESSION_EVENT_TAP: u32 = 2;
// Another app's global shortcut (a Carbon hotkey such as Spotlight's ⌘Space)
// consumes its key before the session, so only HID sees that press.
const HID_EVENT_TAP: u32 = 0;
const HEAD_INSERT_EVENT_TAP: u32 = 0;
const LISTEN_ONLY_EVENT_TAP: u32 = 1;
const HID_SYSTEM_STATE: u32 = 1;
const COMBINED_SESSION_STATE: u32 = 0;

const SESSION_MASK: u64 = (1 << EVENT_KEY_DOWN)
    | (1 << EVENT_KEY_UP)
    | (1 << EVENT_FLAGS_CHANGED)
    | (1 << EVENT_LEFT_MOUSE_DOWN)
    | (1 << EVENT_RIGHT_MOUSE_DOWN)
    | (1 << EVENT_OTHER_MOUSE_DOWN);
const HID_MASK: u64 = 1 << EVENT_KEY_DOWN;

/// How often the tap thread confirms its taps still run, and retries one it
/// could not create. macOS can disable or invalidate a tap without telling it
/// (after sleep, secure input, or an Accessibility change); without
/// Accessibility no tap can be created until the user allows it. Hex's
/// `TAP_HEALTH_INTERVAL_SECONDS`.
const TAP_HEALTH_INTERVAL_SECONDS: f64 = 5.0;
/// How often stale keys are sampled while some are tracked. Recovery needs two
/// neutral samples `STALE_KEY_NEUTRAL` (100 ms) apart, so it lands within 150 ms.
const RECOVERY_SAMPLE: Duration = Duration::from_millis(50);

type EventRef = *mut c_void;
type EventTapCallback = unsafe extern "C" fn(
    proxy: *mut c_void,
    event_type: u32,
    event: EventRef,
    user_info: *mut c_void,
) -> EventRef;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceFlagsState(state_id: u32) -> u64;
    fn CGEventSourceKeyState(state_id: u32, key: u16) -> bool;
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: EventTapCallback,
        user_info: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventTapIsEnabled(tap: *mut c_void) -> bool;
    fn CGEventGetFlags(event: EventRef) -> u64;
    fn CGEventGetTimestamp(event: EventRef) -> u64;
    fn CGEventGetIntegerValueField(event: EventRef, field: u32) -> i64;
    fn mach_absolute_time() -> u64;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: *mut c_void,
        order: isize,
    ) -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRemoveSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopRun();
    fn CFRunLoopTimerCreate(
        allocator: *const c_void,
        fire_date: f64,
        interval: f64,
        flags: usize,
        order: isize,
        callout: unsafe extern "C" fn(timer: *mut c_void, info: *mut c_void),
        context: *mut RunLoopTimerContext,
    ) -> *mut c_void;
    fn CFRunLoopAddTimer(run_loop: *mut c_void, timer: *mut c_void, mode: *const c_void);
    fn CFAbsoluteTimeGetCurrent() -> f64;
    fn CFMachPortIsValid(port: *mut c_void) -> u8;
    fn CFRelease(value: *const c_void);
    static kCFRunLoopCommonModes: *const c_void;
}

#[repr(C)]
struct RunLoopTimerContext {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
}

fn now() -> HostTime {
    // SAFETY: plain clock read.
    host_time(unsafe { mach_absolute_time() })
}

/// Whether a tracked key still reads as held. Combined-session state can keep a
/// key down for good after a system shortcut swallows its release (Tab after
/// ⌘Tab), so HID state, the physical keyboard, must agree.
fn key_held(code: u16) -> bool {
    // SAFETY: read-only CoreGraphics state queries.
    unsafe {
        CGEventSourceKeyState(COMBINED_SESSION_STATE, code)
            && CGEventSourceKeyState(HID_SYSTEM_STATE, code)
    }
}

fn session_flags() -> u64 {
    // SAFETY: a read-only query of a documented event-source state.
    unsafe { CGEventSourceFlagsState(COMBINED_SESSION_STATE) }
}

/// The running key engine. Its threads live as long as the app.
pub struct Tap {
    sink: Sink<KeyEvent>,
    /// The draw key is reported only while a capture is open.
    open: Arc<AtomicBool>,
    draw: Arc<AtomicBool>,
}

impl Tap {
    /// Starts the taps and the machine. Without Accessibility the taps cannot be
    /// created; they are retried every few seconds, so a grant takes effect
    /// without a relaunch.
    pub fn start(sink: Sink<KeyEvent>) -> Result<Self, String> {
        let (send, events) = mpsc::channel();
        std::thread::Builder::new()
            .name("lens-key-tap".into())
            .spawn(move || run_taps(send))
            .map_err(|e| format!("the key tap cannot start: {e}"))?;
        let open = Arc::new(AtomicBool::new(false));
        let draw = Arc::new(AtomicBool::new(false));
        let (engine_open, engine_draw, engine_sink) = (open.clone(), draw.clone(), sink.clone());
        std::thread::Builder::new()
            .name("lens-keys".into())
            .spawn(move || {
                run_keys(events, |event| {
                    match event {
                        KeyEvent::DrawDown | KeyEvent::DrawUp => {
                            engine_draw.store(event == KeyEvent::DrawDown, Ordering::Release);
                            if !engine_open.load(Ordering::Acquire) {
                                return;
                            }
                        }
                        KeyEvent::Pressed(_) | KeyEvent::Voice(_) => {}
                    }
                    engine_sink(event);
                })
            })
            .map_err(|e| format!("the key engine cannot start: {e}"))?;
        Ok(Self { sink, open, draw })
    }

    /// Reports the draw key from now on, or stops. A draw key already down when
    /// the capture opens counts at once.
    pub fn capture_open(&self, open: bool) {
        self.open.store(open, Ordering::Release);
        if open && self.draw.load(Ordering::Acquire) {
            (self.sink)(KeyEvent::DrawDown);
        }
    }
}

fn run_keys(events: Receiver<(InputEvent, HostTime)>, emit: impl Fn(KeyEvent)) {
    let mut keys = Keys::new(Physical {
        flags: session_flags,
        key_down: key_held,
    });
    loop {
        let wait = [
            keys.hold_due().map(|due| due.since(now())),
            keys.recoverable().then_some(RECOVERY_SAMPLE),
        ]
        .into_iter()
        .flatten()
        .min();
        let first = match wait {
            Some(wait) => match events.recv_timeout(wait) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match events.recv() {
                Ok(event) => Some(event),
                Err(_) => return,
            },
        };
        for (event, at) in first.into_iter().chain(events.try_iter()) {
            super::log(format_args!(
                "{event:?} stamped {} µs before it was read",
                now().0.saturating_sub(at.0) / 1_000
            ));
            keys.feed(event, at).into_iter().for_each(&emit);
        }
        keys.tick(now()).into_iter().for_each(&emit);
        keys.recover_stale_keys(now(), session_flags, key_held);
    }
}

struct Context {
    send: Sender<(InputEvent, HostTime)>,
    taps: [AtomicPtr<c_void>; 2],
}

impl Context {
    fn send(&self, event: InputEvent, at: HostTime) {
        let _ = self.send.send((event, at));
    }

    /// Re-enables both taps after macOS disables one.
    fn reenable(&self) {
        for slot in &self.taps {
            let tap = slot.load(Ordering::Acquire);
            if !tap.is_null() {
                // SAFETY: `tap` is a CFMachPort returned by `CGEventTapCreate`.
                unsafe { CGEventTapEnable(tap, true) };
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    /// Keys, modifiers and clicks, as the session sees them.
    Session,
    /// Key presses at the HID level: evidence of chords another app's shortcut hides.
    Hid,
}

impl Kind {
    const ALL: [Self; 2] = [Self::Session, Self::Hid];

    fn index(self) -> usize {
        self as usize
    }
}

struct AttachedTap {
    tap: *mut c_void,
    source: *mut c_void,
    run_loop: *mut c_void,
}

impl Drop for AttachedTap {
    fn drop(&mut self) {
        // SAFETY: both were created by `attach` and are owned here.
        unsafe {
            CGEventTapEnable(self.tap, false);
            CFRunLoopRemoveSource(self.run_loop, self.source, kCFRunLoopCommonModes);
            CFRelease(self.source.cast_const());
            CFRelease(self.tap.cast_const());
        }
    }
}

/// # Safety
/// `context` and `run_loop` must outlive the returned tap.
unsafe fn attach(
    kind: Kind,
    context: *const Context,
    run_loop: *mut c_void,
) -> Option<AttachedTap> {
    let (location, mask, callback): (u32, u64, EventTapCallback) = match kind {
        Kind::Session => (ANNOTATED_SESSION_EVENT_TAP, SESSION_MASK, session_callback),
        Kind::Hid => (HID_EVENT_TAP, HID_MASK, hid_callback),
    };
    // SAFETY: the callback only reads `context`, which the caller keeps alive.
    let tap = unsafe {
        CGEventTapCreate(
            location,
            HEAD_INSERT_EVENT_TAP,
            LISTEN_ONLY_EVENT_TAP,
            mask,
            callback,
            context.cast_mut().cast(),
        )
    };
    if tap.is_null() {
        return None;
    }
    // SAFETY: `tap` is a valid CFMachPort.
    let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null(), tap, 0) };
    if source.is_null() {
        // SAFETY: we own `tap`.
        unsafe { CFRelease(tap.cast_const()) };
        return None;
    }
    // SAFETY: the caller keeps `context` alive, and the slot lives in it.
    unsafe { &*context }.taps[kind.index()].store(tap, Ordering::Release);
    // SAFETY: `run_loop` is this thread's run loop.
    unsafe {
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
    }
    Some(AttachedTap {
        tap,
        source,
        run_loop,
    })
}

/// The taps on the tap thread, checked by its health timer.
struct TapSet {
    context: *const Context,
    run_loop: *mut c_void,
    taps: [Option<AttachedTap>; 2],
    /// Whether the last attempt to create the session tap failed, so a failure
    /// is reported once and its recovery once.
    failing: bool,
}

impl TapSet {
    /// Re-enables a disabled tap and creates an invalidated or missing one.
    fn check(&mut self) {
        // SAFETY: `context` outlives this set (see `run_taps`).
        let context = unsafe { &*self.context };
        let mut interrupted = false;
        for kind in Kind::ALL {
            let slot = &mut self.taps[kind.index()];
            let recreate = match slot {
                None => true,
                // SAFETY: a live CFMachPort owned by `AttachedTap`.
                Some(tap) if unsafe { CFMachPortIsValid(tap.tap) } == 0 => true,
                // SAFETY: as above.
                Some(tap) if !unsafe { CGEventTapIsEnabled(tap.tap) } => {
                    // SAFETY: as above.
                    unsafe { CGEventTapEnable(tap.tap, true) };
                    interrupted |= matches!(kind, Kind::Session);
                    // SAFETY: as above.
                    !unsafe { CGEventTapIsEnabled(tap.tap) }
                }
                Some(_) => false,
            };
            if !recreate {
                continue;
            }
            context.taps[kind.index()].store(ptr::null_mut(), Ordering::Release);
            let was_attached = slot.take().is_some();
            interrupted |= was_attached && matches!(kind, Kind::Session);
            // SAFETY: `context` and the run loop outlive this set.
            *slot = unsafe { attach(kind, self.context, self.run_loop) };
            if matches!(kind, Kind::Session) {
                let failing = slot.is_none();
                if failing && !self.failing {
                    eprintln!(
                        "Attn Lens: the capture, voice and draw keys cannot be read: \
                         allow Attn Lens in Accessibility"
                    );
                } else if !failing && self.failing {
                    eprintln!("Attn Lens: the keys are read again");
                }
                self.failing = failing;
            }
        }
        if interrupted {
            context.send(InputEvent::TapDisabled, now());
        }
    }
}

fn run_taps(send: Sender<(InputEvent, HostTime)>) {
    let context = Box::new(Context {
        send,
        taps: [
            AtomicPtr::new(ptr::null_mut()),
            AtomicPtr::new(ptr::null_mut()),
        ],
    });
    // SAFETY: this is the tap thread's own run loop.
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    let mut taps = Box::new(TapSet {
        context: &raw const *context,
        run_loop,
        taps: [None, None],
        failing: false,
    });
    taps.check();
    // App Nap would throttle this thread while Lens sits in the background, and
    // stretch the health timer. Idle system sleep stays allowed.
    let _awake = objc2_foundation::NSProcessInfo::processInfo().beginActivityWithOptions_reason(
        objc2_foundation::NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
        &objc2_foundation::NSString::from_str("Attn Lens listens for its keys"),
    );
    let mut timer_context = RunLoopTimerContext {
        version: 0,
        info: (&raw mut *taps).cast(),
        retain: ptr::null(),
        release: ptr::null(),
        copy_description: ptr::null(),
    };
    // SAFETY: CoreFoundation copies the context; `taps` outlives the timer,
    // which lives as long as this thread's run loop.
    unsafe {
        let timer = CFRunLoopTimerCreate(
            ptr::null(),
            CFAbsoluteTimeGetCurrent() + TAP_HEALTH_INTERVAL_SECONDS,
            TAP_HEALTH_INTERVAL_SECONDS,
            0,
            0,
            check_taps,
            &raw mut timer_context,
        );
        if timer.is_null() {
            eprintln!("Attn Lens: the key taps cannot be checked; a disabled tap stays off");
        } else {
            CFRunLoopAddTimer(run_loop, timer, kCFRunLoopCommonModes);
        }
        // This thread exists to run the taps; the run loop never returns.
        CFRunLoopRun();
    }
    drop(taps);
    drop(context);
}

unsafe extern "C" fn check_taps(_timer: *mut c_void, info: *mut c_void) {
    // SAFETY: `info` is the `TapSet` that outlives this timer, used only on the
    // tap thread.
    unsafe { &mut *info.cast::<TapSet>() }.check();
}

/// HID timestamps arrive as Mach ticks on Apple Silicon and as nanoseconds on
/// some events; keep whichever reading is recent, else `now` (Hex's
/// `physical_event_instant`).
fn hid_instant(raw: u64, now: HostTime) -> HostTime {
    const RECENT: Duration = Duration::from_secs(5);
    [host_time(raw), HostTime(raw)]
        .into_iter()
        .find(|at| *at <= now && now.since(*at) <= RECENT)
        .unwrap_or(now)
}

unsafe extern "C" fn session_callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: EventRef,
    user_info: *mut c_void,
) -> EventRef {
    // SAFETY: `user_info` is the `Context` that `run_taps` keeps alive.
    let context = unsafe { &*user_info.cast::<Context>() };
    if matches!(
        event_type,
        EVENT_TAP_DISABLED_BY_TIMEOUT | EVENT_TAP_DISABLED_BY_USER_INPUT
    ) {
        context.reenable();
        context.send(InputEvent::TapDisabled, now());
        return event;
    }
    if event.is_null() {
        return event;
    }
    // SAFETY: CoreGraphics supplied a valid event to this callback.
    let flags = unsafe { CGEventGetFlags(event) };
    let input = match event_type {
        EVENT_FLAGS_CHANGED => InputEvent::Flags(flags),
        EVENT_KEY_DOWN | EVENT_KEY_UP => InputEvent::Key {
            // SAFETY: a valid keyboard event.
            code: unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) } as u16,
            down: event_type == EVENT_KEY_DOWN,
            flags,
        },
        EVENT_LEFT_MOUSE_DOWN | EVENT_RIGHT_MOUSE_DOWN | EVENT_OTHER_MOUSE_DOWN => {
            InputEvent::MouseDown
        }
        _ => return event,
    };
    // Session taps stamp events in nanoseconds on the host clock.
    // SAFETY: a valid event.
    context.send(input, HostTime(unsafe { CGEventGetTimestamp(event) }));
    event
}

unsafe extern "C" fn hid_callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: EventRef,
    user_info: *mut c_void,
) -> EventRef {
    // SAFETY: `user_info` is the `Context` that `run_taps` keeps alive.
    let context = unsafe { &*user_info.cast::<Context>() };
    if matches!(
        event_type,
        EVENT_TAP_DISABLED_BY_TIMEOUT | EVENT_TAP_DISABLED_BY_USER_INPUT
    ) {
        // Missing evidence only lets a chord count as a press, so just resume.
        context.reenable();
        return event;
    }
    if event.is_null() || event_type != EVENT_KEY_DOWN {
        return event;
    }
    // SAFETY: a valid keyboard event.
    let code = unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) } as u16;
    // SAFETY: a valid event.
    let at = hid_instant(unsafe { CGEventGetTimestamp(event) }, now());
    context.send(InputEvent::PhysicalKeyDown { code }, at);
    event
}
