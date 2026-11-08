//! The sidebar's small controls.
use super::super::{Editor, actions::Action, live::rgba};
use rune::{Cursor, Cx, El, Icon, Key, Tx, col, el, icon, row, text, text_input, tok};

fn theme(cx: &Cx<Editor>) -> tok::Theme {
    tok::theme(cx.system_dark())
}

pub(crate) fn slug(label: &str) -> String {
    label.to_lowercase().replace(' ', "-")
}

pub(crate) fn field(cx: &Cx<Editor>, label: &str, control: El) -> El {
    col()
        .gap(6.)
        .child(
            text(label.to_string())
                .font(tok::type_::LABEL)
                .color(theme(cx).text_secondary),
        )
        .child(control)
}

pub(crate) fn button(
    cx: &Cx<Editor>,
    key: &str,
    label: &str,
    enabled: bool,
    on_press: impl Fn(&mut Editor, &mut Tx) + 'static,
) -> El {
    let t = theme(cx);
    let mut button = row()
        .key(key.to_string())
        .role("button")
        .label(label.to_string())
        .px(10.)
        .py(6.)
        .radius(6.)
        .center()
        .fill(t.control_off)
        .disabled(!enabled)
        .child(
            text(label.to_string())
                .font(tok::type_::LABEL)
                .color(if enabled { t.text } else { t.text_secondary })
                .inert(),
        );
    if enabled {
        button = button
            .cursor(Cursor::Pointer)
            .hover(t.hover)
            .press(t.press)
            .on_press_tx(on_press);
    }
    button
}

/// An icon button in the toolbar; `active` marks the tool in use.
pub(crate) fn icon_button(
    cx: &Cx<Editor>,
    key: &str,
    label: String,
    glyph: &'static Icon,
    active: bool,
    action: Action,
) -> El {
    let t = theme(cx);
    let accent = rgba(super::super::live::ACCENT);
    let mut button = el()
        .key(key.to_string())
        .role("button")
        .label(label.clone())
        .help(label)
        .checked(active)
        .size(32., 32.)
        .radius(7.)
        .center()
        .cursor(Cursor::Pointer)
        .hover(t.hover)
        .press(t.press)
        .on_press_tx(move |s: &mut Editor, tx| s.dispatch_ui(action.clone(), tx))
        .child(
            icon(glyph, 18.)
                .color(if active { accent } else { t.text })
                .inert(),
        );
    if active {
        button = button.fill(accent.alpha(0.16));
    }
    button
}

/// One choice among several. The active one reads as checked.
pub(crate) fn choices<T>(
    cx: &Cx<Editor>,
    group: &str,
    options: impl IntoIterator<Item = (T, Action, bool)>,
) -> El
where
    T: Into<String>,
{
    let t = theme(cx);
    let accent = rgba(super::super::live::ACCENT);
    let mut bar = row()
        .key(format!("{group}-choices"))
        .gap(2.)
        .radius(7.)
        .fill(t.control_track)
        .flex_wrap();
    for (label, action, active) in options {
        let label: String = label.into();
        let mut choice = el()
            .key(format!("{group}-{}", slug(&label)))
            .role("radio")
            .label(label.clone())
            .checked(active)
            .px(9.)
            .py(5.)
            .radius(6.)
            .center()
            .cursor(Cursor::Pointer)
            .hover(t.hover)
            .on_press_tx(move |s: &mut Editor, tx| s.dispatch_ui(action.clone(), tx))
            .child(
                text(label)
                    .font(tok::type_::LABEL)
                    .color(if active { t.text } else { t.text_secondary })
                    .inert(),
            );
        if active {
            choice = choice.fill(accent.alpha(0.22));
        }
        bar = bar.child(choice);
    }
    bar
}

/// A number with step arrows; a press on it opens it for typing.
pub(crate) fn number(
    s: &Editor,
    cx: &Cx<Editor>,
    label: &'static str,
    value: f32,
    unit: &'static str,
) -> El {
    let t = theme(cx);
    let (min, max, step) = super::number_limits(label);
    let key = slug(label);
    let shown = super::shown_number(value);
    let editing = s.number.as_ref().is_some_and(|d| d.label == label);
    let center: El = if editing {
        text_input(&s.number_edit, tok::type_::LABEL)
            .key(format!("{key}-input"))
            .label(label)
            .single_line()
            .grow(1.)
            .px(6.)
            .on_edit(|s: &mut Editor| &mut s.number_edit)
            .on_submit(|s: &mut Editor, tx| {
                s.commit_number(tx);
                tx.blur();
            })
            .on_input_key(|s: &mut Editor, key, tx| {
                if key.key == Key::Escape {
                    s.number = None;
                    tx.blur();
                    return true;
                }
                false
            })
    } else {
        row()
            .key(format!("{key}-value"))
            .grow(1.)
            .center()
            .cursor(Cursor::Text)
            .on_press_tx(move |s: &mut Editor, tx| s.begin_number(label, value, tx))
            .child(
                text(if unit.is_empty() {
                    shown.clone()
                } else {
                    format!("{shown} {unit}")
                })
                .font(tok::type_::LABEL)
                .color(t.text)
                .inert(),
            )
    };
    let arrow = |id: String, glyph: &'static str, delta: f32| {
        el().key(id)
            .role("button")
            .label(glyph)
            .size(26., 26.)
            .radius(6.)
            .center()
            .cursor(Cursor::Pointer)
            .hover(t.hover)
            .on_press_tx(move |s: &mut Editor, tx| {
                let next = (value + delta).clamp(min, max);
                let action = s.number_action(label, next);
                s.number = None;
                s.dispatch_ui(action, tx);
            })
            .child(text(glyph).font(tok::type_::BODY).color(t.text).inert())
    };
    field(
        cx,
        label,
        row()
            .key(format!("{key}-field"))
            .role("slider")
            .label(label)
            .range_step(step as f64)
            .range_value(value as f64, min as f64, max as f64)
            .a11y_value(shown)
            .on_step(move |s: &mut Editor, delta, tx| {
                let next = (value + delta as f32 * step).clamp(min, max);
                let action = s.number_action(label, next);
                s.dispatch_ui(action, tx);
            })
            .h(30.)
            .center_y()
            .radius(7.)
            .fill(t.control_track)
            .child(arrow(format!("{key}-dec"), "−", -step))
            .child(center)
            .child(arrow(format!("{key}-inc"), "+", step)),
    )
}

/// A slider's track: the sidebar's width inside its padding.
const TRACK: f32 = super::WIDTH - 28.;

/// What a slider asks for: the value, and whether it starts a new undo step.
pub(crate) type SliderAction = fn(u32, bool) -> Action;

/// A value along a track, with its reading beside the label. A drag is one undo step;
/// arrows, Page Up and Down, Home and End step it as a native slider does.
pub(crate) fn slider(
    cx: &Cx<Editor>,
    label: &str,
    value: u32,
    (min, max, step): (u32, u32, u32),
    shown: String,
    action: SliderAction,
) -> El {
    let t = theme(cx);
    let accent = rgba(super::super::live::ACCENT);
    let key = slug(label);
    let fraction = (value.saturating_sub(min) as f32 / (max - min).max(1) as f32).clamp(0., 1.);
    let at = move |x: f32, width: f32| {
        let raw = min as f32 + (x / width.max(1.)).clamp(0., 1.) * (max - min) as f32;
        let stepped = (raw / step as f32).round() as u32 * step;
        stepped.clamp(min, max)
    };
    let track = el()
        .key(format!("{key}-slider"))
        .role("slider")
        .label(label.to_string())
        .range_step(step as f64)
        .range_value(value as f64, min as f64, max as f64)
        .a11y_value(shown.clone())
        .w(TRACK)
        .h(20.)
        .cursor(Cursor::Pointer)
        .on_drag(move |s: &mut Editor, drag, tx| {
            if matches!(drag.phase, rune::Phase::Down) {
                s.continuing = false;
            }
            let revision = s.revision;
            s.dispatch_ui(action(at(drag.x, drag.w), !s.continuing), tx);
            // The undo step starts with the first move that changes something.
            let started = s.continuing || s.revision != revision;
            s.continuing = started && !matches!(drag.phase, rune::Phase::Up);
        })
        .on_step(move |s: &mut Editor, delta, tx| {
            let next = (value as i64 + delta as i64 * step as i64).clamp(min as i64, max as i64);
            s.dispatch_ui(action(next as u32, true), tx);
        })
        .on_step_limit(move |s: &mut Editor, end, tx| {
            s.dispatch_ui(action(if end { max } else { min }, true), tx);
        })
        .child(
            el().absolute(0., 8.)
                .w(TRACK)
                .h(4.)
                .radius(2.)
                .fill(t.control_track)
                .inert(),
        )
        .child(
            el().absolute(0., 8.)
                .w(TRACK * fraction)
                .h(4.)
                .radius(2.)
                .fill(accent)
                .inert(),
        )
        .child(
            el().absolute((TRACK - 14.) * fraction, 3.)
                .size(14., 14.)
                .radius(7.)
                .fill(rgba([255, 255, 255, 255]))
                .border(1., t.card_border)
                .inert(),
        );
    col()
        .key(format!("{key}-field"))
        .gap(4.)
        .child(
            row()
                .justify_between()
                .child(
                    text(label.to_string())
                        .font(tok::type_::LABEL)
                        .color(t.text_secondary),
                )
                .child(
                    text(shown)
                        .key(format!("{key}-reading"))
                        .font(tok::type_::LABEL)
                        .color(t.text),
                ),
        )
        .child(track)
}
