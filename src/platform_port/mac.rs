//! The macOS port. Keys come from Carbon hotkeys and `NSEvent` flags monitors, so
//! Lens needs no Accessibility or Input Monitoring; stills come from capture-core.
//! AppKit and Carbon calls run on the main thread.
#![allow(unexpected_cfgs)] // objc 0.2 macros use a legacy cargo-clippy feature.
use super::{
    Chord, KeyEvent, PlatformPort, Sink, Voice, VoiceEvent, VoiceSource, Watch, WatchEvent,
};
use crate::{
    lens::{
        attn, capture,
        session::{Area, DisplayFrame, HostTime},
        voice::{Clock, Speech},
    },
    permission_guide::{self, Permission},
};
use block2::RcBlock;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use image::RgbaImage;
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject},
};
use std::{
    cell::RefCell,
    ffi::c_void,
    ptr::NonNull,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

/// The key held to draw: fn. `NSEventModifierFlagFunction`.
const DRAW_KEY_FLAG: u64 = 0x80_0000;
/// `NSEventMaskFlagsChanged`.
const FLAGS_CHANGED_MASK: u64 = 1 << 12;
/// `NSEventModifierFlagShift`.
const SHIFT_FLAG: u64 = 0x2_0000;

const COMMAND_OPTION: Modifiers = Modifiers::SUPER.union(Modifiers::ALT);

thread_local! {
    /// Lives on the main thread, where Carbon registers hotkeys.
    static HOTKEYS: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
}

fn capture_chord() -> HotKey {
    HotKey::new(Some(COMMAND_OPTION), Code::KeyX)
}

/// The keys that exist only during a capture (D4 on the plan).
fn capture_only_chords() -> [(HotKey, Chord); 3] {
    [
        (HotKey::new(Some(COMMAND_OPTION), Code::KeyS), Chord::Voice),
        (
            HotKey::new(Some(COMMAND_OPTION), Code::KeyC),
            Chord::Keyboard,
        ),
        (HotKey::new(Some(COMMAND_OPTION), Code::Enter), Chord::Send),
    ]
}

#[link(name = "System")]
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut Timebase) -> i32;
    fn pthread_main_np() -> i32;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Timebase {
    numer: u32,
    denom: u32,
}

/// Runs `job` on the main thread: now if this is it, otherwise soon.
fn on_main(job: impl FnOnce() + Send + 'static) {
    extern "C" fn trampoline(context: *mut c_void) {
        // SAFETY: `context` is the box leaked below, and this runs once.
        let job = unsafe { Box::from_raw(context.cast::<Box<dyn FnOnce() + Send>>()) };
        job();
    }
    // SAFETY: plain libc query.
    if unsafe { pthread_main_np() } != 0 {
        return job();
    }
    let job: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(job));
    // SAFETY: the main queue outlives the process, and the trampoline takes ownership.
    unsafe {
        dispatch_async_f(
            &raw const _dispatch_main_q,
            Box::into_raw(job).cast(),
            trampoline,
        );
    }
}

pub struct MacPort {
    speech: Arc<Speech>,
}

impl MacPort {
    pub fn new() -> Self {
        Self {
            speech: Arc::default(),
        }
    }

    fn clock(&self) -> Clock {
        // SAFETY: plain clock read.
        Arc::new(|| host_time(unsafe { mach_absolute_time() }))
    }
}

/// `mach_absolute_time` ticks as host time, the clock every capture time uses.
pub fn host_time(ticks: u64) -> HostTime {
    static TIMEBASE: OnceLock<Timebase> = OnceLock::new();
    let base = TIMEBASE.get_or_init(|| {
        let mut base = Timebase::default();
        // SAFETY: `base` is a valid out pointer.
        unsafe { mach_timebase_info(&mut base) };
        base
    });
    HostTime(ticks * u64::from(base.numer) / u64::from(base.denom))
}

impl Default for MacPort {
    fn default() -> Self {
        Self::new()
    }
}

fn install_hotkeys(sink: Sink<KeyEvent>) -> Result<(), String> {
    let manager =
        GlobalHotKeyManager::new().map_err(|e| format!("Global shortcuts unavailable: {e}"))?;
    let capture = capture_chord();
    manager
        .register(capture)
        .map_err(|e| format!("⌘⌥X is unavailable: {e}"))?;
    let capture_id = capture.id();
    let others = capture_only_chords().map(|(key, chord)| (key.id(), chord));
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state != HotKeyState::Pressed {
            return;
        }
        if event.id == capture_id {
            sink(KeyEvent::Pressed(Chord::Capture));
        } else if let Some((_, chord)) = others.iter().find(|(id, _)| *id == event.id) {
            sink(KeyEvent::Pressed(*chord));
        }
    }));
    HOTKEYS.with(|hotkeys| *hotkeys.borrow_mut() = Some(manager));
    Ok(())
}

/// Reads the draw key from the global monitor while another app is key, and the
/// local one while a Lens window is. Neither needs a permission.
fn install_draw_key_monitors(sink: Sink<KeyEvent>) {
    static HELD: AtomicBool = AtomicBool::new(false);
    fn flags_changed(event: &AnyObject, sink: &Sink<KeyEvent>) {
        // SAFETY: `event` is the NSEvent AppKit passes to the monitor.
        let flags: u64 = unsafe { msg_send![event, modifierFlags] };
        let held = flags & DRAW_KEY_FLAG != 0;
        if HELD.swap(held, Ordering::SeqCst) != held {
            sink(if held {
                KeyEvent::DrawDown
            } else {
                KeyEvent::DrawUp
            });
        }
    }
    let class = AnyClass::get(c"NSEvent").expect("AppKit is linked");
    let global_sink = sink.clone();
    let global = RcBlock::new(move |event: NonNull<AnyObject>| {
        // SAFETY: AppKit passes a live event.
        flags_changed(unsafe { event.as_ref() }, &global_sink);
    });
    // SAFETY: the mask and block match the method's signature; the monitors and
    // their blocks live for the whole process.
    unsafe {
        let monitor: Option<Retained<AnyObject>> = msg_send![class, addGlobalMonitorForEventsMatchingMask: FLAGS_CHANGED_MASK, handler: &*global];
        std::mem::forget((monitor, global));
    }
    let local = RcBlock::new(move |event: NonNull<AnyObject>| -> *mut AnyObject {
        // SAFETY: AppKit passes a live event.
        flags_changed(unsafe { event.as_ref() }, &sink);
        event.as_ptr()
    });
    // SAFETY: as above.
    unsafe {
        let monitor: Option<Retained<AnyObject>> = msg_send![class, addLocalMonitorForEventsMatchingMask: FLAGS_CHANGED_MASK, handler: &*local];
        std::mem::forget((monitor, local));
    }
}

impl PlatformPort for MacPort {
    fn now(&self) -> HostTime {
        // SAFETY: plain clock read.
        host_time(unsafe { mach_absolute_time() })
    }

    fn keys(&self, sink: Sink<KeyEvent>) {
        on_main(move || {
            if let Err(error) = install_hotkeys(sink.clone()) {
                eprintln!("Attn Lens: {error}");
            }
            install_draw_key_monitors(sink);
        });
    }

    fn capture_keys(&self, open: bool) {
        on_main(move || {
            HOTKEYS.with(|hotkeys| {
                let hotkeys = hotkeys.borrow();
                let Some(manager) = hotkeys.as_ref() else {
                    return;
                };
                for (key, _) in capture_only_chords() {
                    let result = if open {
                        manager.register(key)
                    } else {
                        manager.unregister(key)
                    };
                    if let Err(error) = result {
                        eprintln!("Attn Lens: {key:?}: {error}");
                    }
                }
            });
        });
    }

    fn pointer_display(&self) -> Result<Area, String> {
        let display = capture::display_under_pointer();
        let (width, height) = capture::display_size(display);
        Ok(Area {
            display,
            rect: [0., 0., width, height],
        })
    }

    fn displays(&self) -> Vec<DisplayFrame> {
        capture::displays()
    }

    fn shift_held(&self) -> bool {
        let class = AnyClass::get(c"NSEvent").expect("AppKit is linked");
        // SAFETY: a class property that needs no permission and works off the main thread.
        let flags: u64 = unsafe { msg_send![class, modifierFlags] };
        flags & SHIFT_FLAG != 0
    }

    fn screen_recording(&self) -> bool {
        let granted = permission_guide::granted(Permission::ScreenRecording);
        if !granted {
            on_main(|| permission_guide::show(Permission::ScreenRecording));
        }
        granted
    }

    fn still(&self, area: Area) -> Result<RgbaImage, String> {
        capture::screenshot(area)
    }

    /// No events yet: nothing about the screen reaches a capture but keys.
    fn watch(&self, _area: Area, _sink: Sink<WatchEvent>) -> Watch {
        Watch(None)
    }

    fn voice(&self, source: VoiceSource, sink: Sink<VoiceEvent>) -> Result<Voice, String> {
        Ok(self.speech.start(source, self.clock(), sink))
    }

    fn end_voice(&self) {
        self.speech.release();
    }

    fn token(&self) -> Option<String> {
        attn::token()
    }

    fn store_token(&self, token: &str) -> Result<(), String> {
        attn::store_token(token).map_err(|failure| match failure {
            attn::Failure::NotPaired(text) | attn::Failure::Error(text) => text,
        })
    }

    fn forget_token(&self) {
        attn::forget_token();
    }

    fn copy_image(&self, image: RgbaImage) -> Result<(), String> {
        crate::platform::copy(image)
    }

    fn clipboard_image(&self) -> Result<RgbaImage, String> {
        crate::platform::clipboard_image()
    }

    fn copy_text(&self, text: &str) -> Result<(), String> {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text))
            .map_err(|e| e.to_string())
    }

    fn ocr(&self, image: &RgbaImage, rectangle: Option<[u32; 4]>) -> Result<String, String> {
        crate::ocr::extract(image, rectangle)
    }

    fn pick_color(&self, sink: Sink<Result<Option<[u8; 3]>, String>>) {
        // AppKit shows the sampler on the main thread; this thread waits for its answer.
        on_main(move || match crate::platform::sample_screen_color() {
            Ok(answer) => {
                std::thread::spawn(move || {
                    sink(
                        answer
                            .recv_blocking()
                            .unwrap_or_else(|_| Err("Screen sampler closed".into())),
                    );
                });
            }
            Err(error) => sink(Err(error)),
        });
    }
}
