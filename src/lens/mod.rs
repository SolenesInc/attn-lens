//! Attn Lens: capture the screen, mark it up in place, and send it to an attn
//! session. The controller owns the global shortcut, the one overlay, and the
//! corner tile a closed or sent capture shrinks into.
pub mod attn;
pub mod capture;
pub mod overlay;
mod tile;

use crate::document::Document;
use gpui::*;
use overlay::Overlay;
use tile::Tile;

/// One capture as the person left it: image, marks, message and target.
#[derive(Clone)]
pub struct Capture {
    pub display: u32,
    pub document: Document,
    pub message: String,
    pub target: Option<String>,
    /// The label of the session it was last sent to.
    pub sent_to: Option<String>,
}

#[derive(Default)]
struct Lens {
    overlay: Option<WindowHandle<Overlay>>,
    tile: Option<WindowHandle<Tile>>,
    /// The app that was frontmost when the overlay opened, to give focus back.
    previous_app: Option<mac::RunningApp>,
    _hotkeys: Option<global_hotkey::GlobalHotKeyManager>,
}
impl Global for Lens {}

/// ⌘⌥2 captures the display under the pointer, from anywhere.
pub fn install(cx: &mut App) -> Result<(), String> {
    use global_hotkey::{
        GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
        hotkey::{Code, HotKey, Modifiers},
    };
    let mut lens = Lens::default();
    let key = HotKey::new(Some(Modifiers::SUPER | Modifiers::ALT), Code::Digit2);
    let manager =
        GlobalHotKeyManager::new().map_err(|e| format!("Global shortcuts unavailable: {e}"))?;
    let registered = manager
        .register(key)
        .map_err(|e| format!("⌘⌥2 is unavailable: {e}"));
    lens._hotkeys = Some(manager);
    cx.set_global(lens);
    let (sender, receiver) = async_channel::unbounded::<()>();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.id == key.id() && event.state == HotKeyState::Pressed {
            let _ = sender.try_send(());
        }
    }));
    cx.spawn(async move |cx| {
        while receiver.recv().await.is_ok() {
            if cx.update(start_capture).is_err() {
                break;
            }
        }
    })
    .detach();
    registered
}

/// Captures the display under the pointer and opens the overlay on it.
pub fn start_capture(cx: &mut App) {
    if lens(cx).overlay.is_some() {
        return;
    }
    use crate::permission_guide::{Permission, granted, show};
    if !granted(Permission::ScreenRecording) {
        show(Permission::ScreenRecording);
        return;
    }
    let display = capture::display_under_pointer();
    let previous = mac::frontmost();
    cx.spawn(async move |cx| {
        let shot = cx.background_executor().spawn(async move { capture::screenshot(display) }).await;
        let _ = cx.update(|cx| match shot {
            Ok(image) => {
                lens(cx).previous_app = previous;
                open_overlay(
                    Capture { display, document: Document::new(image), message: String::new(), target: None, sent_to: None },
                    cx,
                );
            }
            Err(error) => notify(
                format!("{error}\nIf you just allowed Screen Recording, quit and reopen Attn Lens: macOS applies it to new processes only."),
                display,
                cx,
            ),
        });
    })
    .detach();
}

fn lens(cx: &mut App) -> &mut Lens {
    if !cx.has_global::<Lens>() {
        cx.set_global(Lens::default());
    }
    cx.global_mut::<Lens>()
}

pub fn open_overlay(capture: Capture, cx: &mut App) {
    close_tile(cx);
    let Some(display) = cx
        .displays()
        .into_iter()
        .find(|d| u32::from(d.id()) == capture.display)
        .or_else(|| cx.primary_display())
    else {
        return;
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(display.bounds())),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        display_id: Some(display.id()),
        window_background: WindowBackgroundAppearance::Opaque,
        ..Default::default()
    };
    cx.activate(true);
    match cx.open_window(options, |window, cx| {
        crate::theme::Theme::install(window, cx);
        cx.new(|cx| Overlay::new(capture, window, cx))
    }) {
        Ok(handle) => lens(cx).overlay = Some(handle),
        Err(error) => eprintln!("Attn Lens: could not open the overlay: {error}"),
    }
}

/// Closes the overlay, keeps the capture in the corner tile, and hands focus back.
pub fn close_overlay(capture: Capture, cx: &mut App) {
    let lens = lens(cx);
    let overlay = lens.overlay.take();
    let previous = lens.previous_app.take();
    cx.defer(move |cx| {
        if let Some(overlay) = overlay {
            let _ = overlay.update(cx, |_, window, _| window.remove_window());
        }
        open_tile(capture, cx);
        if let Some(app) = previous {
            app.activate();
        }
    });
}

fn open_tile(capture: Capture, cx: &mut App) {
    let display = capture.display;
    show_tile(tile::Content::Capture(capture), display, cx);
}

/// Shows `text` in the corner, for failures that have no capture to hold them.
pub fn notify(text: String, display: u32, cx: &mut App) {
    show_tile(tile::Content::Notice(text.into()), display, cx);
}

fn show_tile(content: tile::Content, display: u32, cx: &mut App) {
    close_tile(cx);
    let Some(display) = cx
        .displays()
        .into_iter()
        .find(|d| u32::from(d.id()) == display)
        .or_else(|| cx.primary_display())
    else {
        return;
    };
    let area = display.bounds();
    let size = size(
        px(340.),
        px(if matches!(content, tile::Content::Notice(_)) {
            120.
        } else {
            84.
        }),
    );
    let origin = point(
        area.right() - size.width - px(22.),
        area.bottom() - size.height - px(22.),
    );
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, size))),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        display_id: Some(display.id()),
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    if let Ok(handle) = cx.open_window(options, |window, cx| {
        crate::theme::Theme::install(window, cx);
        if let Some(panel) = mac::ns_window(window) {
            mac::never_key(panel);
        }
        cx.new(|_| Tile::new(content))
    }) {
        lens(cx).tile = Some(handle);
    }
}

fn close_tile(cx: &mut App) {
    if let Some(tile) = lens(cx).tile.take() {
        let _ = tile.update(cx, |_, window, _| window.remove_window());
    }
}

/// Opens a capture in the editor window for crop, pixelate, backdrop or export.
pub fn open_editor(document: Document, cx: &mut App) {
    crate::open_editor(Some(document), cx);
    cx.activate(true);
}

/// The editor's Send: back to the in-place bar with the edited image.
pub fn send_from_editor(document: &Document, cx: &mut App) {
    let display = capture::display_under_pointer();
    lens(cx).previous_app = None;
    open_overlay(
        Capture {
            display,
            document: Document::new(document.export()),
            message: String::new(),
            target: None,
            sent_to: None,
        },
        cx,
    );
}

/// Runs an overlay action from automation; errors name what is missing.
pub fn automate(action: overlay::Action, cx: &mut App) -> Result<(), String> {
    let handle = lens(cx)
        .overlay
        .ok_or("No capture is open. Start one with ⌘⌥2 or `capture`.")?;
    handle
        .update(cx, |overlay, _, cx| overlay.dispatch(action, cx))
        .map_err(|e| e.to_string())
}

/// What automation reads back: the open capture, its status and its sessions.
pub fn state(cx: &mut App) -> serde_json::Value {
    let lens = lens(cx);
    let tile = lens.tile;
    let Some(handle) = lens.overlay else {
        return serde_json::json!({ "overlay": null, "tile": tile.is_some() });
    };
    handle
        .read_with(cx, |o, _| {
            serde_json::json!({
                "overlay": {
                    "marks": o.capture.document.marks.len(),
                    "message": o.capture.message,
                    "target": o.capture.target,
                    "status": format!("{:?}", o.status),
                    "sessions": o.sessions.iter().map(|s| serde_json::json!({"id": s.id, "label": s.label, "host": s.host})).collect::<Vec<_>>(),
                },
                "tile": tile.is_some(),
            })
        })
        .unwrap_or(serde_json::Value::Null)
}

pub(crate) mod mac {
    #![allow(unexpected_cfgs)] // objc 0.2 macros use a legacy cargo-clippy feature.
    use cocoa::base::{BOOL, NO, id, nil};
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// A retained `NSRunningApplication`.
    pub struct RunningApp(id);
    impl RunningApp {
        pub fn activate(self) {
            unsafe {
                let _: BOOL = msg_send![self.0, activateWithOptions: 0usize];
            }
        }
    }
    impl Drop for RunningApp {
        fn drop(&mut self) {
            unsafe {
                let _: () = msg_send![self.0, release];
            }
        }
    }

    pub fn frontmost() -> Option<RunningApp> {
        unsafe {
            let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
            let app: id = msg_send![ws, frontmostApplication];
            let me: id = msg_send![class!(NSRunningApplication), currentApplication];
            let same: BOOL = if app == nil {
                NO
            } else {
                msg_send![app, isEqual: me]
            };
            if app == nil || same != NO {
                return None;
            }
            let _: id = msg_send![app, retain];
            Some(RunningApp(app))
        }
    }

    pub fn ns_window(window: &gpui::Window) -> Option<id> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        let view = handle.ns_view.as_ptr() as id;
        Some(unsafe { msg_send![view, window] })
    }

    /// GPUI's panel class can become key; the corner tile must not take typing
    /// from the app the person returned to.
    pub fn never_key(window: id) {
        use objc::declare::ClassDecl;
        use objc::runtime::{Class, Object, Sel, object_getClass};
        extern "C" fn no(_: &Object, _: Sel) -> BOOL {
            NO
        }
        unsafe extern "C" {
            fn object_setClass(obj: *mut Object, cls: *const Class) -> *const Class;
        }
        unsafe {
            let current = object_getClass(window as *const Object);
            let name = "AttnLensNonKeyPanel";
            let class = Class::get(name).unwrap_or_else(|| {
                let mut decl = ClassDecl::new(name, &*current).expect("a new panel class");
                decl.add_method(
                    sel!(canBecomeKeyWindow),
                    no as extern "C" fn(&Object, Sel) -> BOOL,
                );
                decl.add_method(
                    sel!(canBecomeMainWindow),
                    no as extern "C" fn(&Object, Sel) -> BOOL,
                );
                decl.register()
            });
            object_setClass(window, class);
        }
    }
}
