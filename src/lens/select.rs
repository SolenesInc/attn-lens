//! The area picker: a crosshair over each display. A drag makes an area, a click
//! takes the whole display, ↩ takes the last area, and Esc cancels (D3 on the plan).
use super::{Lens, session::DisplayFrame};
use rune::{Color, Cursor, Cx, Drag, El, Key, KeyEvent, Phase, Tx, col, el, text, tok};
use std::sync::Arc;

/// One picker per connected display. Hidden pickers hold no window, so this costs
/// little past the 8 displays an M2 Ultra drives; more than that names itself in
/// each picker's hint.
pub const PICKER_SLOTS: usize = 8;
/// The three 12 pt grab zones a side needs, so a small area keeps its handles apart.
const MIN_AREA: f64 = 36.;
/// A press that travels no further is a click.
const CLICK_TRAVEL: f32 = 8.;

/// Keeps `rect` on its display and big enough to adjust.
pub fn clamp(rect: [f64; 4], display: DisplayFrame) -> [f64; 4] {
    let (display_w, display_h) = (display.frame[2], display.frame[3]);
    let width = rect[2].clamp(MIN_AREA.min(display_w), display_w);
    let height = rect[3].clamp(MIN_AREA.min(display_h), display_h);
    [
        rect[0].clamp(0., display_w - width),
        rect[1].clamp(0., display_h - height),
        width,
        height,
    ]
}

pub struct Picker {
    pub lens: Arc<Lens>,
    pub slot: usize,
    pub drag: Option<[f32; 4]>,
}

impl Picker {
    fn display(&self) -> Option<DisplayFrame> {
        self.lens
            .read(|summary| summary.picking.as_ref()?.get(self.slot).copied())
    }
}

fn hint(has_last: bool, extra_displays: usize) -> String {
    let mut hint = "Drag for an area · Click for the whole display".to_string();
    if has_last {
        hint.push_str(" · ↩ last area");
    }
    hint.push_str(" · Esc cancel");
    if extra_displays > 0 {
        hint.push_str(&format!(
            "\nAttn Lens picks on {PICKER_SLOTS} displays; {extra_displays} more have no picker."
        ));
    }
    hint
}

pub fn picker_view(s: &Picker, _: &mut Cx<Picker>) -> El {
    let t = tok::theme(true);
    let (has_last, extra) = s.lens.read(|summary| {
        (
            summary.last_area.is_some(),
            summary
                .picking
                .as_ref()
                .map_or(0, |d| d.len().saturating_sub(PICKER_SLOTS)),
        )
    });
    let mut root = el()
        .key("picker")
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(0.18))
        .cursor(Cursor::Crosshair)
        .child(
            col().w_full().pt(48.).center().child(
                el().px(14.).py(8.).radius(10.).fill(t.card).child(
                    text(hint(has_last, extra))
                        .key("picker-hint")
                        .font(tok::type_::BODY)
                        .color(t.text),
                ),
            ),
        );
    if let Some(d) = s.drag {
        root = root.child(
            el().key("picker-drag")
                .absolute(d[0], d[1])
                .size(d[2], d[3])
                .fill(Color::WHITE.alpha(0.08))
                .border(1.5, Color::WHITE),
        );
    }
    root.on_drag(|s: &mut Picker, d: &mut Drag, _: &mut Tx| match d.phase {
        Phase::Down => s.drag = None,
        Phase::Move => {
            s.drag = Some([
                d.start_x.min(d.x),
                d.start_y.min(d.y),
                (d.x - d.start_x).abs(),
                (d.y - d.start_y).abs(),
            ]);
        }
        Phase::Up => {
            let drag = s.drag.take();
            if d.cancelled() {
                return;
            }
            let Some(display) = s.display() else { return };
            let travel = (d.x - d.start_x).hypot(d.y - d.start_y);
            let rect = drag
                .filter(|_| travel > CLICK_TRAVEL)
                .map(|r| r.map(f64::from));
            if let Err(error) = s.lens.pick(display.id, rect) {
                eprintln!("Attn Lens: {error}");
            }
        }
    })
}

pub fn picker_key(s: &mut Picker, event: &KeyEvent, _: &mut Tx) -> bool {
    match event.key {
        Key::Escape => {
            s.lens.cancel_pick();
            true
        }
        Key::Enter => {
            s.lens.pick_last();
            true
        }
        _ => false,
    }
}
