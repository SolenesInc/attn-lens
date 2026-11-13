//! The bar beside the area: the session chip, the message, voice, the tool row,
//! the still strip, Edit and Send, and the Tab palette. It takes keys without
//! activating Lens, and shows who has them (D6 on the plan).
use super::{
    AttnStatus, KeyOwner, Lens, SendStatus, Summary, VoiceState, attn, ink::InkTool,
    session::DisplayFrame,
};
use crate::platform_port::VoiceStatus;
use image::RgbaImage;
use rune::{Color, Cx, El, Key, KeyEvent, TextEdit, Tx, col, el, row, text, text_input, tok};
use std::sync::Arc;

pub const WIDTH: f32 = 760.;
/// Every control on the bar stands this tall.
const CONTROL: f32 = 32.;
const GAP: f32 = 8.;
const PADDING: f32 = 10.;
const STATUS: f32 = 44.;
const PALETTE_ROW: f32 = 36.;
/// How many rows the palette shows before it scrolls.
const PALETTE_ROWS: usize = 8;
const THUMB: (u32, u32) = (104, 64);
/// Keeps the bar off the area and the display's edge.
const MARGIN: f64 = 8.;

/// A still's small picture for the strip.
pub fn thumbnail(frame: &RgbaImage) -> rune::Pixels {
    let pixels = rune::Pixels::new();
    draw_thumbnail(&pixels, frame);
    pixels
}

/// Draws `frame` small into `pixels`, which a strip may already be showing.
pub fn draw_thumbnail(pixels: &rune::Pixels, frame: &RgbaImage) {
    let scale = (f64::from(THUMB.0) / f64::from(frame.width().max(1)))
        .min(f64::from(THUMB.1) / f64::from(frame.height().max(1)));
    let width = ((f64::from(frame.width()) * scale) as u32).max(1);
    let height = ((f64::from(frame.height()) * scale) as u32).max(1);
    let small = image::imageops::thumbnail(frame, width, height);
    pixels.set(width, height, small.into_raw());
}

/// What stops or follows a send, for the line under the controls.
fn status(summary: &Summary) -> Option<String> {
    match &summary.attn {
        AttnStatus::Unpaired(why) => {
            return Some(format!(
                "{why} Press ↩ to pair: attn asks you to allow “{}”.",
                attn::SENDER_NAME
            ));
        }
        AttnStatus::Pairing(None) => return Some("Asking attn to pair…".into()),
        AttnStatus::Pairing(Some(code)) => {
            return Some(format!(
                "In attn, check the code is {code} and click Allow."
            ));
        }
        AttnStatus::Unlisted(why) => {
            return Some(format!("{why} Press ↩ to list the sessions again."));
        }
        AttnStatus::Loading | AttnStatus::Ready => {}
    }
    match &summary.send {
        SendStatus::Sending => Some("Sending…".into()),
        SendStatus::Failed(error) => Some(error.clone()),
        SendStatus::Idle | SendStatus::Sent(_) => None,
    }
}

/// The bar's height for what it shows now.
pub fn height(summary: &Summary) -> f32 {
    let mut height = PADDING * 2. + CONTROL * 2. + GAP;
    if status(summary).is_some() || summary.error.is_some() {
        height += STATUS;
    }
    if summary.palette.is_some() {
        height += palette_height(summary);
    }
    height
}

fn palette_height(summary: &Summary) -> f32 {
    PALETTE_ROW * summary.sessions.len().clamp(1, PALETTE_ROWS) as f32 + GAP
}

/// Where the bar goes: under the area if it fits, otherwise over it, otherwise
/// (a whole display) inside the bottom edge. Stills leave Lens's windows out.
pub fn placement(area: [f64; 4], display: DisplayFrame, height: f32) -> [f32; 4] {
    let (display_w, display_h) = (display.frame[2], display.frame[3]);
    let width = f64::from(WIDTH).min(display_w - MARGIN * 2.);
    let height = f64::from(height);
    let below = area[1] + area[3] + MARGIN;
    let above = area[1] - MARGIN - height;
    let y = if below + height + MARGIN <= display_h {
        below
    } else if above >= MARGIN {
        above
    } else {
        display_h - height - MARGIN
    };
    let x = area[0].clamp(MARGIN, (display_w - width - MARGIN).max(MARGIN));
    [x as f32, y as f32, width as f32, height as f32]
}

pub struct Bar {
    pub lens: Arc<Lens>,
    pub seen: u64,
    capture: Option<u64>,
    placing_label: bool,
    message: TextEdit,
    label: TextEdit,
}

impl Bar {
    pub fn new(lens: Arc<Lens>) -> Self {
        let mut message = TextEdit::new("");
        message.focused = true;
        Self {
            lens,
            seen: 0,
            capture: None,
            placing_label: false,
            message,
            label: TextEdit::new(""),
        }
    }

    /// Follows the shared state: a new capture starts with an empty message, and
    /// placing a label moves the typing to its field.
    /// A label being placed is finished first: the editor is about to take the still.
    fn finish_label(&mut self) {
        if self.placing_label {
            self.lens.finish_label(&self.label.text);
        }
    }

    pub fn follow(&mut self) {
        let (capture, message, placing) = self.lens.read(|summary| {
            let capture = summary.capture.as_ref();
            (
                capture.map(|c| c.id),
                capture.map_or(String::new(), |c| c.message.clone()),
                summary.placing_label,
            )
        });
        if capture != self.capture {
            self.capture = capture;
            self.message = TextEdit::new(&message);
        }
        if placing != self.placing_label {
            self.placing_label = placing;
            self.label = TextEdit::new("");
        }
        self.label.focused = placing;
        self.message.focused = !placing;
    }
}

fn button(key: &str, content: String, on: bool, t: &tok::Theme) -> El {
    el().key(key)
        .h(CONTROL)
        .px(10.)
        .center()
        .radius(8.)
        .fill(if on { t.indicator } else { t.control_track })
        .child(
            text(content)
                .font(tok::type_::LABEL)
                .color(if on { Color::WHITE } else { t.text }),
        )
}

fn tool_button(key: &'static str, name: &str, tool: InkTool, armed: InkTool, t: &tok::Theme) -> El {
    button(key, name.to_string(), armed == tool, t)
        .on_press(move |s: &mut Bar| s.lens.arm_tool(tool))
}

/// What the chip says: the target session, and where it runs once there is one.
fn chip_parts(summary: &Summary) -> (String, Option<String>) {
    let target = summary
        .capture
        .as_ref()
        .and_then(|c| c.target.as_ref())
        .and_then(|id| summary.sessions.iter().find(|s| &s.id == id));
    let why = match (&summary.attn, target) {
        (AttnStatus::Unpaired(_) | AttnStatus::Pairing(_), _) => "Not paired with attn",
        (AttnStatus::Unlisted(_), _) => "Sessions unavailable",
        (_, Some(session)) => {
            return (format!("→ {}", session.label), Some(session.host_label()));
        }
        (AttnStatus::Loading, None) => "Loading sessions…",
        (AttnStatus::Ready, None) if summary.sessions.is_empty() => "No sessions in attn",
        (AttnStatus::Ready, None) => "Choose a session",
    };
    (why.into(), None)
}

/// The session chip. A long session label ends in "…" inside the chip, and the
/// host after it stays whole; the full text is the chip's label and, while cut, its tooltip.
fn chip(summary: &Summary, t: &tok::Theme) -> El {
    let (target, host) = chip_parts(summary);
    let full = match &host {
        Some(host) => format!("{target} · {host}"),
        None => target.clone(),
    };
    let on = summary.palette.is_some();
    let color = if on { Color::WHITE } else { t.text };
    row()
        .key("chip")
        .label(full.clone())
        .h(CONTROL)
        .max_w(240.)
        .px(10.)
        .gap(4.)
        .center_y()
        .radius(8.)
        .fill(if on { t.indicator } else { t.control_track })
        .child(
            text(target)
                .key("chip-target")
                .font(tok::type_::LABEL)
                .color(color)
                .truncate()
                .help_when_truncated(full),
        )
        .children(host.map(|host| {
            text(format!("· {host}"))
                .key("chip-host")
                .font(tok::type_::LABEL)
                .color(color)
        }))
        .on_press(|s: &mut Bar| s.lens.toggle_palette())
}

fn strip(summary: &Summary, t: &tok::Theme) -> El {
    let mut strip = row().key("strip").grow(1.).h(CONTROL).gap(6.).scroll_x();
    let stills = summary.capture.as_ref().map_or(&[][..], |c| &c.stills);
    for (index, still) in stills.iter().enumerate() {
        let id = still.id;
        strip = strip.child(
            el().key(format!("still-{index}"))
                .label(format!(
                    "{:?} {}×{}",
                    still.cause, still.width, still.height
                ))
                .size(52., CONTROL)
                .radius(4.)
                .fill(t.control_track)
                .on_press(move |s: &mut Bar| {
                    s.finish_label();
                    s.lens.edit_still(id);
                })
                .child(el().w_full().h_full().pixels(&still.thumb))
                .child(
                    el().key(format!("drop-{index}"))
                        .label("Drop this still")
                        .absolute(36., 0.)
                        .size(16., 16.)
                        .center()
                        .radius(8.)
                        .fill(Color::BLACK.alpha(0.6))
                        .child(text("×").font(tok::type_::LABEL).color(Color::WHITE))
                        .on_press(move |s: &mut Bar| s.lens.drop_still(id)),
                ),
        );
    }
    strip
}

fn palette(summary: &Summary, selected: usize, t: &tok::Theme) -> El {
    let mut list = col()
        .key("palette")
        .w_full()
        .h(palette_height(summary) - GAP)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    if summary.sessions.is_empty() {
        let empty = match &summary.attn {
            AttnStatus::Loading => "Loading sessions…",
            AttnStatus::Ready => "No live sessions in attn. Start one there.",
            _ => "Attn Lens cannot list the sessions; the line above says why.",
        };
        return list.child(
            text(empty)
                .key("palette-empty")
                .font(tok::type_::BODY)
                .color(t.text_secondary),
        );
    }
    for (index, session) in summary.sessions.iter().enumerate() {
        list = list.child(
            row()
                .key(format!("palette-row-{index}"))
                .w_full()
                .h(PALETTE_ROW)
                .px(8.)
                .gap(12.)
                .center_y()
                .radius(6.)
                .fill(if index == selected {
                    t.indicator
                } else {
                    Color::TRANSPARENT
                })
                .child(
                    text(session.label.clone())
                        .key(format!("palette-label-{index}"))
                        .grow(1.)
                        .font(tok::type_::BODY)
                        .color(t.text)
                        .truncate()
                        .help_when_truncated(session.label.clone()),
                )
                .child(
                    text(format!(
                        "{} · {} · {}",
                        session.agent,
                        session.state_label(),
                        session.host_label()
                    ))
                    .key(format!("palette-meta-{index}"))
                    .font(tok::type_::LABEL)
                    .color(t.text_secondary),
                )
                .on_press(move |s: &mut Bar| s.lens.choose_session(Some(index))),
        );
    }
    list
}

/// Keys the field does not take for itself. Returns whether the key was used.
pub fn bar_key(s: &mut Bar, event: &KeyEvent, _: &mut Tx) -> bool {
    let (palette_open, placing) = s
        .lens
        .read(|summary| (summary.palette.is_some(), summary.placing_label));
    match event.key {
        Key::Enter if placing => s.lens.finish_label(&s.label.text),
        Key::Escape if placing => s.lens.finish_label(""),
        Key::Enter if palette_open => s.lens.choose_session(None),
        Key::Escape if palette_open => s.lens.toggle_palette(),
        Key::Up if palette_open => s.lens.move_palette(-1),
        Key::Down if palette_open => s.lens.move_palette(1),
        Key::Enter => s.lens.send_or_recover(),
        Key::Escape => s.lens.end_capture(),
        Key::Tab => s.lens.toggle_palette(),
        _ => return false,
    }
    true
}

pub fn bar_view(s: &Bar, cx: &mut Cx<Bar>) -> El {
    let t = tok::theme(cx.system_dark());
    s.lens.read(|summary: &Summary| {
        let keys_here = summary.keys == KeyOwner::Bar;
        let field = if summary.placing_label {
            text_input(&s.label, tok::type_::BODY.lh(22.))
                .key("label")
                .placeholder("Label text, then ↩", t.text_secondary)
                .on_edit(|s: &mut Bar| &mut s.label)
        } else {
            text_input(&s.message, tok::type_::BODY.lh(22.))
                .key("message")
                .placeholder(
                    if keys_here {
                        "Say what you mean"
                    } else {
                        "The page has the keys · ⌘⌥C to type here"
                    },
                    t.text_secondary,
                )
                .on_edit(|s: &mut Bar| &mut s.message)
                .on_input_change(|s: &mut Bar, _: &mut Tx| {
                    s.lens.set_message(&s.message.text);
                })
        }
        .single_line()
        .grow(1.)
        .color(t.text)
        .on_input_key(bar_key);

        let voice = voice_text(summary.voice);
        let segments = summary.capture.as_ref().map_or(&[][..], |c| &c.segments);
        let controls = row()
            .w_full()
            .h(CONTROL)
            .gap(GAP)
            .center_y()
            .child(chip(summary, &t))
            .child(
                row()
                    .grow(1.)
                    .h(CONTROL)
                    .px(10.)
                    .center_y()
                    .radius(8.)
                    .border(1., if keys_here { t.indicator } else { t.separator })
                    .child(field),
            )
            .child(
                text(if keys_here {
                    "Typing here"
                } else {
                    "Typing on the page"
                })
                .key("keys")
                .font(tok::type_::LABEL)
                .color(t.text_secondary),
            );
        let tools = row()
            .w_full()
            .h(CONTROL)
            .gap(GAP)
            .center_y()
            .child(tool_button(
                "tool-highlight",
                "Highlight",
                InkTool::Highlight,
                summary.tool,
                &t,
            ))
            .child(tool_button(
                "tool-label",
                "Label",
                InkTool::Label,
                summary.tool,
                &t,
            ))
            .child(tool_button(
                "tool-counter",
                "Counter",
                InkTool::Counter,
                summary.tool,
                &t,
            ))
            .child(strip(summary, &t))
            .children(segments.last().map(|segment| {
                text(segment.text.clone())
                    .key(format!("segment-{}", segments.len() - 1))
                    .max_w(160.)
                    .font(tok::type_::LABEL)
                    .color(t.text_secondary)
            }))
            .child(
                text(voice)
                    .key("voice")
                    .font(tok::type_::LABEL)
                    .color(t.text_secondary),
            )
            .child(
                button("edit", "Edit".into(), false, &t).on_press(|s: &mut Bar| {
                    s.finish_label();
                    s.lens.edit_latest();
                }),
            )
            .child(
                button(
                    "send",
                    match summary.attn {
                        AttnStatus::Unpaired(_) => "Pair ↩".into(),
                        _ if summary.send == SendStatus::Sending => "Sending…".into(),
                        _ => "Send ↩".to_string(),
                    },
                    true,
                    &t,
                )
                .on_press(|s: &mut Bar| s.lens.send_or_recover()),
            );
        let mut bar = col()
            .key("bar")
            .w_full()
            .h_full()
            .padding(PADDING)
            .gap(GAP)
            .radius(14.)
            .fill(t.card)
            .child(controls)
            .child(tools);
        if let Some(line) = status(summary) {
            bar = bar.child(
                text(line)
                    .key("status")
                    .font(tok::type_::LABEL)
                    .color(t.text_secondary),
            );
        }
        if let Some(error) = &summary.error {
            bar = bar.child(
                text(error.clone())
                    .key("error")
                    .font(tok::type_::LABEL)
                    .color(t.text_secondary),
            );
        }
        if let Some(selected) = summary.palette {
            bar = bar.child(palette(summary, selected, &t));
        }
        bar
    })
}

/// Voice in the bar: what the voice key does now, or how far voice has got.
fn voice_text(voice: VoiceState) -> String {
    match voice {
        VoiceState::Off => "right ⌥ voice".into(),
        VoiceState::Starting => "Starting voice…".into(),
        VoiceState::Running(VoiceStatus::Downloading { bytes, total }) => format!(
            "Downloading speech model {}%",
            bytes.saturating_mul(100) / total.max(1)
        ),
        VoiceState::Running(VoiceStatus::Loading) => "Loading speech model…".into(),
        VoiceState::Running(VoiceStatus::WarmingUp) => "Starting microphone…".into(),
        VoiceState::Running(VoiceStatus::Listening) => "Listening · right ⌥ to stop".into(),
        VoiceState::Finishing => "Finishing…".into(),
    }
}
