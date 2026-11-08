//! The Rune app: a Dock app whose windows read the Lens state. It starts with no
//! window, ⌘⌥X opens the area picker, and a Dock click opens the empty editor.
use crate::{
    editor::{self, Editor},
    lens::{
        Input, KeyOwner, Lens, Summary, automation,
        bar::{self, Bar},
        ink::{Corner, Handle, Ink, handle_view, ink_view},
        select::{self, PICKER_SLOTS, Picker},
        session::{Area, DisplayFrame},
    },
    platform_port,
};
use rune::{
    AnyWindowSpec, App, AppHandle, AppSpec, Cx, DisplayTarget, El, KeyEvent, KeyPolicy,
    NativeDisplayId, OverlayHandle, OverlayInput, OverlayLevel, OverlayOptions, OverlayPlacement,
    OverlaySpec, Tx, WindowOptions, WindowSpec,
};
use std::sync::{Arc, Mutex};

const EDITOR_SIZE: (f32, f32) = (1220., 860.);
/// Room for the toolbar and the sidebar's longest option list.
const EDITOR_MIN_SIZE: (f32, f32) = (900., 560.);

/// A view that is an overlay on the Mac and an ordinary window when headless,
/// where Rune draws no overlays (D2 on the plan). This is the only platform branch.
enum Surface<T: 'static> {
    Overlay(OverlayHandle<T>),
    Window(AppHandle<T>),
}

impl<T: 'static> Surface<T> {
    fn show(&self) {
        match self {
            Surface::Overlay(handle) => handle.show(),
            Surface::Window(handle) => handle.show(),
        }
    }
    fn hide(&self) {
        match self {
            Surface::Overlay(handle) => handle.hide(),
            Surface::Window(handle) => handle.hide(),
        }
    }
    fn take_keys(&self) {
        match self {
            Surface::Overlay(handle) => handle.request_keys(),
            Surface::Window(handle) => handle.focus(),
        }
    }
    fn place(&self, display: u32, placement: OverlayPlacement) {
        let target = DisplayTarget::Native(NativeDisplayId::MacOS(display));
        match self {
            Surface::Overlay(handle) => {
                handle.set_display(target);
                handle.set_placement(Some(placement));
            }
            Surface::Window(handle) => handle.place(target, placement),
        }
    }
    /// Headless, a window stays clickable and the view ignores what it should not take.
    fn set_input(&self, input: OverlayInput) {
        if let Surface::Overlay(handle) = self {
            handle.set_input(input);
        }
    }
    fn refresh(&self) {
        match self {
            Surface::Overlay(handle) => handle.update(|_, _| {}),
            Surface::Window(handle) => handle.update(|_, _| {}),
        }
    }
}

type KeyHandler<T> = fn(&mut T, &KeyEvent, &mut Tx) -> bool;

/// Adds `view` to the app `owner` as a surface over the screen, left out of stills.
fn host<O: 'static, T: 'static>(
    owner: &mut App<O>,
    windows: &mut Vec<AnyWindowSpec>,
    key: String,
    view: fn(&T, &mut Cx<T>) -> El,
    on_key: Option<KeyHandler<T>>,
    options: OverlayOptions,
    state: T,
) -> Surface<T> {
    if rune::tooling::is_headless() {
        let app = App::new(AppSpec {
            on_key,
            ..AppSpec::new("Attn Lens Layer", (64., 64.), || unreachable!(), view)
        })
        .windowless();
        let handle = app.handle();
        let app = app.init(move || state);
        windows.push(
            WindowSpec::from_app(key, app)
                .options(WindowOptions {
                    transparent: true,
                    borderless: true,
                    visible: false,
                    always_on_top: true,
                    min_size: Some((0., 0.)),
                    ..Default::default()
                })
                .into(),
        );
        return Surface::Window(handle);
    }
    let mut spec = OverlaySpec::new("attn-lens-layer", (64., 64.), view)
        .options(options)
        .release_gpu_when_hidden();
    if let Some(on_key) = on_key {
        spec = spec.on_key(on_key);
    }
    Surface::Overlay(owner.overlay(spec, state))
}

fn overlay_input(input: Input) -> OverlayInput {
    match input {
        // The page gets the pointer; the handles are windows of their own.
        Input::None => OverlayInput::None,
        Input::All => OverlayInput::All,
    }
}

/// What the windows show. Applying a change only when it differs matters: showing
/// a window or asking for keys takes the keyboard from the page.
#[derive(Clone, Default, PartialEq)]
struct Shown {
    picking: Option<Vec<DisplayFrame>>,
    area: Option<Area>,
    input: Input,
    /// Each corner handle's window, while the draw key is up.
    handles: Option<[[f32; 4]; 4]>,
    resizing: bool,
    bar_rect: Option<[f32; 4]>,
    bar_key_requests: u64,
}

impl Shown {
    fn of(summary: &Summary) -> Self {
        let capture = summary.capture.as_ref();
        Self {
            picking: summary.picking.clone(),
            area: capture.map(|c| c.area),
            input: summary.input,
            handles: capture
                .filter(|_| summary.input == Input::None)
                .map(|c| Corner::ALL.map(|corner| corner.rect(c.area, c.display))),
            resizing: summary.resizing,
            bar_rect: capture.map(|c| bar::placement(c.area.rect, c.display, bar::height(summary))),
            bar_key_requests: summary.bar_key_requests,
        }
    }
}

struct Surfaces {
    pickers: Vec<Surface<Picker>>,
    ink: Surface<Ink>,
    handles: Vec<Surface<Handle>>,
    bar: AppHandle<Bar>,
}

impl Surfaces {
    fn apply(&self, lens: &Lens, was: &Shown, now: &Shown) {
        if was.picking != now.picking {
            for (slot, picker) in self.pickers.iter().enumerate() {
                match now.picking.as_ref().and_then(|d| d.get(slot)) {
                    Some(display) => {
                        picker.place(display.id, OverlayPlacement::FullScreen);
                        picker.show();
                    }
                    None => picker.hide(),
                }
            }
            if let Some(displays) = &now.picking {
                let under_pointer = lens.port().pointer_display().map(|area| area.display);
                let slot = displays
                    .iter()
                    .position(|d| Ok(d.id) == under_pointer)
                    .unwrap_or(0);
                if let Some(picker) = self.pickers.get(slot) {
                    picker.take_keys();
                }
            }
        }
        if was.area != now.area {
            match now.area {
                Some(area) => {
                    let [x, y, width, height] = area.rect.map(|v| v as f32);
                    self.ink.place(
                        area.display,
                        OverlayPlacement::Rect {
                            x,
                            y,
                            width,
                            height,
                        },
                    );
                    if was.area.is_none() {
                        self.ink.show();
                    }
                }
                None => self.ink.hide(),
            }
        }
        if was.input != now.input {
            self.ink.set_input(overlay_input(now.input));
        }
        // A handle that owns a drag must not move under it; they land when it ends.
        let landed = was.resizing && !now.resizing;
        if !now.resizing && (was.handles != now.handles || landed) {
            for (index, handle) in self.handles.iter().enumerate() {
                match (now.area, now.handles) {
                    (Some(area), Some(rects)) => {
                        let [x, y, width, height] = rects[index];
                        handle.place(
                            area.display,
                            OverlayPlacement::Rect {
                                x,
                                y,
                                width,
                                height,
                            },
                        );
                        if was.handles.is_none() {
                            handle.show();
                        }
                    }
                    _ => handle.hide(),
                }
            }
        }
        if let (Some(area), true) = (now.area, was.bar_rect != now.bar_rect)
            && let Some([x, y, width, height]) = now.bar_rect
        {
            self.bar.place(
                DisplayTarget::Native(NativeDisplayId::MacOS(area.display)),
                OverlayPlacement::Rect {
                    x,
                    y,
                    width,
                    height,
                },
            );
        }
        if now.area.is_none() && was.area.is_some() {
            self.bar.hide();
        }
        if now.area.is_some() && was.bar_key_requests != now.bar_key_requests {
            self.bar.open();
        }
    }

    fn refresh(&self) {
        for picker in &self.pickers {
            picker.refresh();
        }
        self.ink.refresh();
        for handle in &self.handles {
            handle.refresh();
        }
        self.bar.wake();
    }
}

/// Runs the app until it quits. Returns an error only if it cannot start.
pub fn run() -> Result<(), String> {
    let lens = Lens::new(platform_port::select()?);
    if std::env::args().any(|arg| arg == "--automation") {
        automation::listen(lens.clone(), &automation::socket_path()?)?;
    }
    rune::run_windows(move |displays| {
        lens.port().launch_displays(
            displays
                .iter()
                .enumerate()
                .map(|(index, display)| DisplayFrame {
                    id: index as u32 + 1,
                    frame: [
                        f64::from(display.position.0) / display.scale,
                        f64::from(display.position.1) / display.scale,
                        f64::from(display.size.0) / display.scale,
                        f64::from(display.size.1) / display.scale,
                    ],
                    // A headless display has no menu bar; the fake gives one.
                    menu_bar: 0.,
                })
                .collect(),
        );
        lens.listen();

        let editor = App::new(AppSpec {
            poll: Some(|s: &mut Editor, now, tx: &mut Tx| s.poll(now, tx)),
            on_key: Some(|s: &mut Editor, event, tx: &mut Tx| s.on_key(event, tx)),
            min_size: Some(EDITOR_MIN_SIZE),
            ..AppSpec::new("Attn Lens", EDITOR_SIZE, || unreachable!(), editor::view)
        })
        .windowless();
        let me = editor.handle();
        let visible = me.clone();
        lens.report_window("editor", move || visible.is_visible());
        let opener = me.clone();
        let port = lens.port().clone();
        let editor = editor::menus::install(editor)
            .init(move || Editor::new(me, port))
            .on_focus(|s, focused, _| s.on_window_focus(focused))
            .on_reopen(|s, _, _| {
                s.open_window();
                true
            });

        let mut bar = App::new(AppSpec {
            poll: Some(|s: &mut Bar, now, tx: &mut Tx| {
                let lens = s.lens.clone();
                if let Some(next) = lens.port().tick(now) {
                    tx.wake_after(next);
                }
                let generation = lens.generation();
                let changed = generation != s.seen;
                s.seen = generation;
                s.follow();
                changed
            }),
            ..AppSpec::new(
                "Attn Lens Bar",
                (bar::WIDTH, 100.),
                || unreachable!(),
                bar::bar_view,
            )
        })
        .windowless();
        let handle = bar.handle();
        let visible = handle.clone();
        lens.report_window("bar", move || visible.is_visible());

        let mut windows: Vec<AnyWindowSpec> = vec![];
        let pickers: Vec<Surface<Picker>> = (0..PICKER_SLOTS)
            .map(|slot| {
                host(
                    &mut bar,
                    &mut windows,
                    format!("picker-{slot}"),
                    select::picker_view,
                    Some(select::picker_key),
                    OverlayOptions {
                        placement: Some(OverlayPlacement::FullScreen),
                        exclude_from_capture: true,
                        keys: KeyPolicy::OnDemand,
                        input: OverlayInput::All,
                        level: OverlayLevel::AboveStatus,
                        ..Default::default()
                    },
                    Picker {
                        lens: lens.clone(),
                        slot,
                        drag: None,
                    },
                )
            })
            .collect();
        let ink = host(
            &mut bar,
            &mut windows,
            "ink".into(),
            ink_view,
            None,
            OverlayOptions {
                exclude_from_capture: true,
                input: OverlayInput::None,
                ..Default::default()
            },
            Ink::new(lens.clone()),
        );
        // After the ink layer, so they sit above it.
        let handles = Corner::ALL
            .into_iter()
            .map(|corner| {
                host(
                    &mut bar,
                    &mut windows,
                    corner.key().into(),
                    handle_view,
                    None,
                    OverlayOptions {
                        exclude_from_capture: true,
                        input: OverlayInput::All,
                        ..Default::default()
                    },
                    Handle::new(lens.clone(), corner),
                )
            })
            .collect();

        let surfaces = Surfaces {
            pickers,
            ink,
            handles,
            bar: handle.clone(),
        };
        let shown = Mutex::new(Shown::default());
        let reader = lens.clone();
        lens.on_change(move || {
            let now = reader.read(Shown::of);
            let mut was = shown.lock().unwrap();
            surfaces.apply(&reader, &was, &now);
            *was = now;
            surfaces.refresh();
        });

        let state = lens.clone();
        let open_editor: Arc<dyn Fn() + Send + Sync> = Arc::new(move || opener.open());
        let bar = bar
            .init(move || Bar::new(state, open_editor))
            .on_focus(move |s, focused, _| {
                s.lens.set_key_owner(if focused {
                    KeyOwner::Bar
                } else {
                    KeyOwner::Page
                });
            });

        let mut specs = vec![
            AnyWindowSpec::from(WindowSpec::from_app("editor", editor)),
            WindowSpec::from_app("bar", bar)
                .configured_options(WindowOptions::launcher_panel())
                .into(),
        ];
        specs.extend(windows);
        specs
    });
    Ok(())
}
