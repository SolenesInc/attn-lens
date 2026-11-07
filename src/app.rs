//! The Rune app: a Dock app whose windows read the Lens state. It starts with no
//! window, ⌘⌥X opens a capture, and a Dock click opens the empty editor.
use crate::{
    editor::{self, Editor},
    lens::{Input, Lens, SendStatus, Summary, automation},
    platform_port,
};
use rune::{App, AppSpec, Cx, El, Tx, WindowOptions, WindowSpec, row, text, tok};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const EDITOR_SIZE: (f32, f32) = (1220., 860.);
/// Room for the toolbar and the sidebar's longest option list.
const EDITOR_MIN_SIZE: (f32, f32) = (900., 560.);
const BAR_SIZE: (f32, f32) = (560., 64.);

struct Bar {
    lens: Arc<Lens>,
    seen: u64,
}

fn poll_lens(lens: &Lens, seen: &mut u64, now: f64, tx: &mut Tx) -> bool {
    if let Some(next) = lens.port().tick(now) {
        tx.wake_after(next);
    }
    let generation = lens.generation();
    let changed = generation != *seen;
    *seen = generation;
    changed
}

fn bar_view(s: &Bar, cx: &mut Cx<Bar>) -> El {
    let t = tok::theme(cx.system_dark());
    let label = |content: String, key: String| {
        text(content)
            .key(key)
            .font(tok::type_::LABEL)
            .color(t.text_secondary)
    };
    s.lens.read(|summary: &Summary| {
        let mut bar = row()
            .key("bar")
            .w_full()
            .h_full()
            .px(14.)
            .gap(10.)
            .center_y()
            .fill(t.card)
            .child(label(
                match summary.input {
                    Input::All => "input All".into(),
                    Input::None => "input None".into(),
                },
                "input".into(),
            ));
        if let Some(capture) = &summary.capture {
            for (index, still) in capture.stills.iter().enumerate() {
                bar = bar.child(label(
                    format!("{:?} {}×{}", still.cause, still.width, still.height),
                    format!("still-{index}"),
                ));
            }
            for (index, segment) in capture.segments.iter().enumerate() {
                bar = bar.child(label(segment.text.clone(), format!("segment-{index}")));
            }
        }
        if summary.voice {
            bar = bar.child(label("voice on".into(), "voice".into()));
        }
        match &summary.send {
            SendStatus::Idle => {}
            SendStatus::Sending => bar = bar.child(label("Sending".into(), "send".into())),
            SendStatus::Sent(target) => {
                bar = bar.child(label(format!("Sent to {target}"), "send".into()))
            }
            SendStatus::Failed(error) => bar = bar.child(label(error.clone(), "send".into())),
        }
        if let Some(error) = &summary.error {
            bar = bar.child(label(error.clone(), "error".into()));
        }
        bar
    })
}

/// Runs the app until it quits. Returns an error only if it cannot start.
pub fn run() -> Result<(), String> {
    let lens = Lens::new(platform_port::select()?);
    if std::env::args().any(|arg| arg == "--automation") {
        automation::listen(lens.clone(), &automation::socket_path()?)?;
    }
    rune::run_windows(move |_displays| {
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
        let port = lens.port().clone();
        let closing = me.clone();
        port.close_button(
            "editor",
            Arc::new(move |()| closing.update(|s: &mut Editor, _| s.on_window_close())),
        );
        let editor = editor::menus::install(editor)
            .init(move || Editor::new(me, port))
            .on_focus(|s, focused, _| s.on_window_focus(focused))
            .on_close(|s, _| s.on_window_close())
            .on_reopen(|s, _, _| {
                s.open_window();
                true
            });

        let bar = App::new(AppSpec {
            poll: Some(|s: &mut Bar, now, tx: &mut Tx| {
                let lens = s.lens.clone();
                poll_lens(&lens, &mut s.seen, now, tx)
            }),
            ..AppSpec::new("Attn Lens Bar", BAR_SIZE, || unreachable!(), bar_view)
        })
        .windowless();
        let handle = bar.handle();
        let visible = handle.clone();
        lens.report_window("bar", move || visible.is_visible());
        let waker = handle.clone();
        lens.on_change(move || waker.wake());
        let shown = lens.clone();
        let was_open = AtomicBool::new(false);
        // Only on a change: showing the bar takes the keys from the page.
        lens.on_change(move || {
            let open = shown.read(|summary| summary.capture.is_some());
            if was_open.swap(open, Ordering::SeqCst) != open {
                if open {
                    handle.show();
                } else {
                    handle.hide();
                }
            }
        });
        let state = lens.clone();
        let bar = bar.init(move || Bar {
            lens: state,
            seen: 0,
        });

        vec![
            rune::AnyWindowSpec::from(WindowSpec::from_app("editor", editor)),
            WindowSpec::from_app("bar", bar)
                .configured_options(WindowOptions::launcher_panel())
                .into(),
        ]
    });
    Ok(())
}
