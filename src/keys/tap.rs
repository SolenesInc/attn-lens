//! macOS's input for [`super::Keys`]: event taps on their own run-loop thread,
//! and the machine on another, so a slow sink never stalls the taps.
//!
//! Adapted from Hex's `src/suppression.rs` (`run_event_tap`, `TapSet`,
//! `HealthTimer`; <https://github.com/anomalyco/hex>, commit 54d1ebf), MIT
//! License, Copyright (c) 2026 Christopher Langton. See THIRD_PARTY_NOTICES.md.
//! The HID tap only listens. The session tap passes every event on, except ↩
//! and Esc while a dictation's pill waits for them (`pill_keys`, F7 on the plan),
//! and fn with 1 to 6 while the ink layer takes the pointer (`tool_keys`): those
//! it takes, so ↩ never also runs a line in the terminal in front, and a digit
//! that picks a tool is not also typed.
use super::{COMMAND, CONTROL, InputEvent, Keys, OPTION, Physical, SHIFT};
use crate::{
    lens::session::HostTime,
    platform_port::mac::host_time,
    platform_port::{KeyEvent, PillKey, Sink},
};
use std::{
    ffi::c_void,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicPtr, AtomicU16, Ordering},
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
const DEFAULT_EVENT_TAP: u32 = 0;
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

/// How often the tap thread confirms its taps still run, re-creates one macOS
/// invalidated (after sleep, secure input, or an Accessibility change), and,
/// until Lens is trusted, checks for an Accessibility grant. Hex's
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

/// The keys the session tap may take, by key code, and what each becomes: for
/// the pill, Return, keypad Enter and Escape; for the tool strip, 1 to 6 on the
/// main row.
const TAKEN_KEYS: [(u16, KeyEvent); 9] = [
    (36, KeyEvent::Pill(PillKey::Enter)),
    (76, KeyEvent::Pill(PillKey::Enter)),
    (53, KeyEvent::Pill(PillKey::Esc)),
    (18, KeyEvent::Tool(1)),
    (19, KeyEvent::Tool(2)),
    (20, KeyEvent::Tool(3)),
    (21, KeyEvent::Tool(4)),
    (23, KeyEvent::Tool(5)),
    (22, KeyEvent::Tool(6)),
];

/// What the taps hand the engine thread.
enum Tapped {
    Input(InputEvent, HostTime),
    /// A key the session tap took from the app in front.
    Taken(KeyEvent),
}

/// Which keys Lens takes now, set from its state and read on the tap thread.
#[derive(Clone, Default)]
struct Takes {
    /// ↩ and Esc, while a dictation's pill waits for them.
    pill: Arc<AtomicBool>,
    /// fn with 1 to 6, while the ink layer takes the pointer.
    tools: Arc<AtomicBool>,
}

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
    fn AXIsProcessTrusted() -> bool;
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
    takes: Takes,
}

impl Tap {
    /// Starts the taps and the machine. Without Accessibility no tap is made;
    /// trust is checked again every few seconds, so a grant takes effect
    /// without a relaunch.
    pub fn start(sink: Sink<KeyEvent>) -> Result<Self, String> {
        let (send, events) = mpsc::channel();
        let takes = Takes::default();
        let tap_takes = takes.clone();
        std::thread::Builder::new()
            .name("lens-key-tap".into())
            .spawn(move || run_taps(send, tap_takes))
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
                        KeyEvent::Pressed(_)
                        | KeyEvent::Voice(_)
                        | KeyEvent::Pill(_)
                        | KeyEvent::Tool(_) => {}
                    }
                    engine_sink(event);
                })
            })
            .map_err(|e| format!("the key engine cannot start: {e}"))?;
        Ok(Self {
            sink,
            open,
            draw,
            takes,
        })
    }

    /// Takes ↩ and Esc from the app in front from now on, or gives them back.
    pub fn pill_keys(&self, on: bool) {
        self.takes.pill.store(on, Ordering::Release);
    }

    /// Takes fn with 1 to 6 from the app in front from now on, or gives them back.
    pub fn tool_keys(&self, on: bool) {
        self.takes.tools.store(on, Ordering::Release);
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

fn run_keys(events: Receiver<Tapped>, emit: impl Fn(KeyEvent)) {
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
        for tapped in first.into_iter().chain(events.try_iter()) {
            let (event, at) = match tapped {
                Tapped::Input(event, at) => (event, at),
                Tapped::Taken(key) => {
                    super::log(format_args!("took {key:?}"));
                    emit(key);
                    continue;
                }
            };
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
    send: Sender<Tapped>,
    taps: [AtomicPtr<c_void>; 2],
    takes: Takes,
    /// The keys whose press the session tap took, one bit per entry of
    /// [`TAKEN_KEYS`], so their repeats and release are taken too.
    taken: AtomicU16,
    /// Whether fn is down, from the modifier events this tap has passed on. The
    /// key events that follow arrive in the same order, so a digit typed just
    /// after fn comes up is never taken while Lens has yet to hear of the release.
    fn_down: AtomicBool,
}

impl Context {
    fn send(&self, event: InputEvent, at: HostTime) {
        let _ = self.send.send(Tapped::Input(event, at));
    }

    /// Whether the session tap takes this key event from the app in front: a
    /// bare ↩ or Esc pressed while the pill waits, or a digit pressed with fn
    /// (and maybe ⇧) while the ink layer takes the pointer, and its repeats and
    /// release.
    fn take(&self, code: u16, down: bool, flags: u64) -> bool {
        let Some(index) = TAKEN_KEYS.iter().position(|(c, _)| *c == code) else {
            return false;
        };
        let bit = 1u16 << index;
        let taken = self.taken.load(Ordering::Acquire) & bit != 0;
        if !down {
            self.taken.fetch_and(!bit, Ordering::AcqRel);
            return taken;
        }
        if taken {
            return true;
        }
        let key = TAKEN_KEYS[index].1;
        let takes = match key {
            KeyEvent::Pill(_) => {
                flags & (SHIFT | CONTROL | OPTION | COMMAND) == 0
                    && self.takes.pill.load(Ordering::Acquire)
            }
            _ => {
                flags & (CONTROL | OPTION | COMMAND) == 0
                    && self.fn_down.load(Ordering::Acquire)
                    && self.takes.tools.load(Ordering::Acquire)
            }
        };
        if !takes {
            return false;
        }
        self.taken.fetch_or(bit, Ordering::AcqRel);
        let _ = self.send.send(Tapped::Taken(key));
        true
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
    let (location, option, mask, callback): (u32, u32, u64, EventTapCallback) = match kind {
        Kind::Session => (
            ANNOTATED_SESSION_EVENT_TAP,
            DEFAULT_EVENT_TAP,
            SESSION_MASK,
            session_callback,
        ),
        Kind::Hid => (HID_EVENT_TAP, LISTEN_ONLY_EVENT_TAP, HID_MASK, hid_callback),
    };
    // SAFETY: the callback only reads `context`, which the caller keeps alive.
    let tap = unsafe {
        CGEventTapCreate(
            location,
            HEAD_INSERT_EVENT_TAP,
            option,
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
        // Without Accessibility macOS still creates a listen-only tap, but never
        // feeds it, not even once access is granted. Creating taps only once the
        // process is trusted lets a grant reach the running app.
        // SAFETY: a read-only query.
        if self.taps.iter().all(Option::is_none) && !unsafe { AXIsProcessTrusted() } {
            if !self.failing {
                eprintln!(
                    "Attn Lens: the capture, voice and draw keys cannot be read: \
                     allow Attn Lens in Accessibility"
                );
            }
            self.failing = true;
            return;
        }
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

fn run_taps(send: Sender<Tapped>, takes: Takes) {
    let context = Box::new(Context {
        send,
        taps: [
            AtomicPtr::new(ptr::null_mut()),
            AtomicPtr::new(ptr::null_mut()),
        ],
        takes,
        taken: AtomicU16::new(0),
        fn_down: AtomicBool::new(false),
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
        EVENT_FLAGS_CHANGED => {
            context
                .fn_down
                .store(flags & super::FUNCTION != 0, Ordering::Release);
            InputEvent::Flags(flags)
        }
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
    // The engine still sees a taken key, so a chord with it counts as one.
    if let InputEvent::Key { code, down, flags } = input
        && context.take(code, down, flags)
    {
        return ptr::null_mut();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn context(pill: bool, tools: bool) -> (Context, Receiver<Tapped>) {
        let (send, events) = mpsc::channel();
        let takes = Takes::default();
        takes.pill.store(pill, Ordering::Release);
        takes.tools.store(tools, Ordering::Release);
        let context = Context {
            send,
            taps: [
                AtomicPtr::new(ptr::null_mut()),
                AtomicPtr::new(ptr::null_mut()),
            ],
            takes,
            taken: AtomicU16::new(0),
            fn_down: AtomicBool::new(true),
        };
        (context, events)
    }

    fn taken(events: &Receiver<Tapped>) -> Vec<KeyEvent> {
        events
            .try_iter()
            .filter_map(|t| match t {
                Tapped::Taken(key) => Some(key),
                Tapped::Input(..) => None,
            })
            .collect()
    }

    const FN: u64 = super::super::FUNCTION;
    const DIGIT_2: u16 = 19;

    #[test]
    fn a_digit_is_taken_only_while_tools_are_on_and_without_command_keys() {
        // (pill, tools, flags, taken?)
        let cases = [
            (false, true, FN, true),
            (false, true, FN | SHIFT, true),
            (false, true, FN | COMMAND, false),
            (false, true, FN | OPTION, false),
            (false, true, FN | CONTROL, false),
            (false, false, FN, false),
            (true, false, 0, false),
        ];
        for (pill, tools, flags, expect) in cases {
            let (context, events) = context(pill, tools);
            assert_eq!(context.take(DIGIT_2, true, flags), expect, "{flags:#x}");
            assert_eq!(
                context.take(DIGIT_2, false, flags),
                expect,
                "release {flags:#x}"
            );
            let want: &[KeyEvent] = if expect { &[KeyEvent::Tool(2)] } else { &[] };
            assert_eq!(taken(&events), want);
        }
    }

    #[test]
    fn a_taken_digit_keeps_its_repeats_and_release_after_tools_go_off() {
        let (context, events) = context(false, true);
        assert!(context.take(DIGIT_2, true, FN));
        context.takes.tools.store(false, Ordering::Release);
        assert!(context.take(DIGIT_2, true, FN), "repeat");
        assert!(context.take(DIGIT_2, false, 0), "release");
        assert!(
            !context.take(DIGIT_2, true, 0),
            "a new press goes to the app"
        );
        assert_eq!(taken(&events), [KeyEvent::Tool(2)]);
    }

    #[test]
    fn a_digit_typed_after_fn_comes_up_reaches_the_app_before_lens_hears_of_it() {
        // Lens still takes tool keys: the release is on its way to it.
        let (context, events) = context(false, true);
        context.fn_down.store(false, Ordering::Release);
        assert!(!context.take(DIGIT_2, true, 0));
        assert!(!context.take(DIGIT_2, false, 0));
        assert!(taken(&events).is_empty());
    }

    #[test]
    fn enter_is_taken_for_the_pill_and_not_for_the_tools() {
        let (context, events) = context(false, true);
        assert!(!context.take(36, true, FN));
        let (context2, events2) = super::tests::context(true, false);
        assert!(context2.take(36, true, 0));
        assert!(taken(&events).is_empty());
        assert_eq!(taken(&events2), [KeyEvent::Pill(PillKey::Enter)]);
    }
}
