//! The tool strip: while the draw key is down, the six tools stand beside the
//! point where the pointer was when it went down. Each shows, in ink, the mark it
//! makes, and the current one stands out. A click on one, or fn with its digit,
//! picks it. The strip is drawn on the ink layer, so it comes and goes with the
//! key and never holds up a stroke: a press anywhere else on the layer draws.
use super::ink::{self, Ink, InkTool};
use crate::document::{Mark, Tool};
use rune::{Color, El, Shape, Transition, canvas, el, row, text, tok};

const GLYPH: f32 = 20.;
const GLYPH_WIDTH: f32 = 2.2;
/// Space between the pointer and the strip, so the first stroke from the
/// pointer starts clear of it.
const CLEAR: f32 = 16.;

/// How the strip looks. Three treatments while the user compares them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    /// A capsule of six cells; the current tool sits on an ink disc.
    Capsule,
    /// Six chips fanned on an arc above the pointer, like ink thrown from it.
    Arc,
    /// A column of tabs; the current one slides out with its name.
    Tabs,
}

fn look() -> Look {
    match std::env::var("LENS_STRIP").as_deref() {
        Ok("arc") => Look::Arc,
        Ok("tabs") => Look::Tabs,
        _ => Look::Capsule,
    }
}

/// The mark `tool` makes, small, in a `GLYPH` square.
fn sample(tool: InkTool, color: [u8; 4]) -> Mark {
    let s = GLYPH;
    let (kind, points, text) = match tool {
        InkTool::Pen => (
            Tool::Pen,
            (0..=12)
                .map(|i| {
                    let x = 2. + i as f32 * (s - 4.) / 12.;
                    (x, s / 2. + (i as f32 * 0.9).sin() * s * 0.22)
                })
                .collect(),
            "",
        ),
        InkTool::Arrow => (Tool::Arrow, vec![(3., s - 3.), (s - 3., 3.)], ""),
        InkTool::Box => (Tool::Rectangle, vec![(3., 5.), (s - 3., s - 5.)], ""),
        InkTool::Highlight => (Tool::Highlight, vec![(1., 6.), (s - 1., s - 6.)], ""),
        InkTool::Label => (Tool::Text, vec![(2., 2.)], "Aa"),
        InkTool::Counter => (Tool::Counter, vec![(s / 2., s / 2.)], "1"),
    };
    Mark {
        style: Default::default(),
        tool: kind,
        curve: None,
        points,
        color,
        width: GLYPH_WIDTH,
        text: text.into(),
        drawn: None,
    }
}

/// `tool`'s mark in `color`, as the ink layer draws it.
fn glyph(tool: InkTool, color: [u8; 4], counter_text: Color) -> El {
    let mark = sample(tool, color);
    let base = el().size(GLYPH, GLYPH);
    match tool {
        InkTool::Label => base.center().child(
            text("Aa")
                .font(tok::TextStyle::new(14., 700.))
                .color(ink::color(color, 1.)),
        ),
        InkTool::Counter => base
            .child(
                canvas(ink::draw(Shape::new(), &mark, (0., 0.)))
                    .absolute(0., 0.)
                    .size(GLYPH, GLYPH),
            )
            .child(
                el().absolute(0., 0.).size(GLYPH, GLYPH).center().child(
                    text("1")
                        .font(tok::TextStyle::new(11., 700.))
                        .color(counter_text),
                ),
            ),
        _ => base.child(
            canvas(ink::draw(Shape::new(), &mark, (0., 0.)))
                .absolute(0., 0.)
                .size(GLYPH, GLYPH),
        ),
    }
}

/// One tool's hit target. A press picks the tool and draws nothing.
fn cell(tool: InkTool, size: f32) -> El {
    el().key(format!("tool-{}", tool.key()))
        .label(tool.name())
        .size(size, size)
        .center()
        .on_press(move |s: &mut Ink| s.lens.pick_tool(tool))
}

fn digit(tool: InkTool, color: Color) -> El {
    text(tool.digit().to_string())
        .font(tok::TextStyle::new(8.5, 600.))
        .color(color)
}

/// The strip for `current`, its pointer at `at` in the layer's points, kept
/// inside a layer of `size`. `drawing` while a stroke is under way: the strip
/// steps back so the ink reads first.
pub fn tool_strip(
    current: InkTool,
    at: (f32, f32),
    size: (f32, f32),
    drawing: bool,
    t: &tok::Theme,
) -> El {
    let ink_rgba = ink::INK;
    let ink_color = ink::color(ink_rgba, 1.);
    let white = [255, 255, 255, 255];
    let body = match look() {
        Look::Capsule => {
            let cells = InkTool::ALL.iter().map(|&tool| {
                let on = tool == current;
                cell(tool, 34.)
                    .radius(17.)
                    .fill(if on { ink_color } else { Color::TRANSPARENT })
                    .child(glyph(
                        tool,
                        if on { white } else { ink_rgba },
                        if on { ink_color } else { Color::WHITE },
                    ))
                    .child(el().absolute(25., 23.).child(digit(
                        tool,
                        if on {
                            Color::WHITE.alpha(0.8)
                        } else {
                            t.text_secondary
                        },
                    )))
            });
            row()
                .padding(4.)
                .gap(2.)
                .radius(21.)
                .fill(t.card)
                .border(1., t.separator)
                .shadow(8., 24., t.shadow)
                .children(cells)
                .child(
                    el().h(34.)
                        .px(8.)
                        .center_y()
                        .child(text(current.name()).font(tok::type_::LABEL).color(t.text)),
                )
        }
        Look::Arc => {
            const RADIUS: f32 = 84.;
            const CHIP: f32 = 30.;
            const LABEL: f32 = 26.;
            let span = (InkTool::ALL.len() - 1) as f32;
            // The pointer is at the arc's centre, the box's bottom middle.
            let mut fan = el().size(RADIUS * 2. + CHIP * 2., LABEL + RADIUS + CHIP + 4.);
            for (i, &tool) in InkTool::ALL.iter().enumerate() {
                let on = tool == current;
                // From left of straight up to right of it, centred on the pointer.
                let angle = (-165. + 150. * i as f32 / span).to_radians();
                let (x, y) = (
                    RADIUS + CHIP + RADIUS * angle.cos() - CHIP / 2.,
                    LABEL + RADIUS + CHIP + RADIUS * angle.sin() - CHIP / 2.,
                );
                let chip = cell(tool, CHIP)
                    .absolute(x, y)
                    .radius(CHIP / 2.)
                    .fill(if on { ink_color } else { t.card })
                    .border(1., if on { ink_color } else { t.separator })
                    .shadow(4., 12., t.shadow)
                    .scale(if on { 1.22 } else { 1. })
                    .child(glyph(
                        tool,
                        if on { white } else { ink_rgba },
                        if on { ink_color } else { Color::WHITE },
                    ))
                    .transition(Transition::scale(0.4).combined(Transition::offset(
                        (RADIUS + CHIP - x) * 0.6,
                        (LABEL + RADIUS + CHIP - y) * 0.6,
                    )))
                    .enter_delay(i as f32 * 0.012);
                fan = fan.child(chip);
            }
            fan.child(
                el().absolute(0., 0.)
                    .w(RADIUS * 2. + CHIP * 2.)
                    .center()
                    .child(
                        el().px(8.).py(2.).radius(8.).fill(t.card).child(
                            text(format!("{} · fn {}", current.name(), current.digit()))
                                .font(tok::type_::LABEL)
                                .color(t.text),
                        ),
                    ),
            )
        }
        Look::Tabs => {
            let mut column = rune::col().gap(4.);
            for &tool in &InkTool::ALL {
                let on = tool == current;
                let tab = el()
                    .h(30.)
                    .radius(15.)
                    .fill(if on { ink_color } else { t.card })
                    .border(1., if on { ink_color } else { t.separator })
                    .shadow(3., 10., t.shadow)
                    .offset(if on { 10. } else { 0. }, 0.)
                    .child(
                        row()
                            .h(30.)
                            .pl(5.)
                            .pr(if on { 12. } else { 5. })
                            .gap(6.)
                            .center_y()
                            .child(cell(tool, 20.).child(glyph(
                                tool,
                                if on { white } else { ink_rgba },
                                if on { ink_color } else { Color::WHITE },
                            )))
                            .children(on.then(|| {
                                text(tool.name())
                                    .font(tok::type_::LABEL)
                                    .color(Color::WHITE)
                            }))
                            .child(digit(
                                tool,
                                if on {
                                    Color::WHITE.alpha(0.8)
                                } else {
                                    t.text_secondary
                                },
                            )),
                    );
                column = column.child(tab);
            }
            column
        }
    };
    let (width, height) = match look() {
        Look::Capsule => (6. * 36. + 8. + 80., 42.),
        Look::Arc => (84. * 2. + 60., 26. + 84. + 30. + 4.),
        Look::Tabs => (130., 6. * 34.),
    };
    // Up and right of the pointer, or wherever it fits inside the layer.
    let (x, y) = match look() {
        Look::Arc => (at.0 - width / 2., at.1 - (26. + 84. + 30.)),
        Look::Capsule => (at.0 + CLEAR, at.1 - CLEAR - height),
        Look::Tabs => (at.0 + CLEAR, at.1 - height / 2.),
    };
    let x = x.clamp(0., (size.0 - width).max(0.));
    let y = y.clamp(0., (size.1 - height).max(0.));
    el().key("tool-strip")
        .label(current.name())
        .absolute(x, y)
        .opacity(if drawing { 0.3 } else { 1. })
        .transition(Transition::scale(0.92).combined(Transition::offset(0., 6.)))
        .child(body)
}
