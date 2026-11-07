//! Marks drawn live over the raster, in canvas points. A pointer move repaints these
//! without touching pixels; they settle into the raster when the gesture ends.
use super::Editor;
use crate::{
    document::{Mark, Tool},
    effects,
    navigation::Layout,
    style::{self, Dash, Fill},
};
use rune::{Color, El, Shape, TextStyle, canvas, el, text};

/// An sRGB byte colour as Rune's linear one.
pub(crate) fn rgba(c: [u8; 4]) -> Color {
    Color {
        r: rune::color::srgb_decode(c[0] as f32 / 255.),
        g: rune::color::srgb_decode(c[1] as f32 / 255.),
        b: rune::color::srgb_decode(c[2] as f32 / 255.),
        a: c[3] as f32 / 255.,
    }
}

pub(crate) const ACCENT: [u8; 4] = [0x4c, 0x8d, 0xff, 255];

fn map(l: Layout, (x, y): (f32, f32)) -> [f32; 2] {
    [l.x + x * l.scale, l.y + y * l.scale]
}

fn rectangle(a: [f32; 2], b: [f32; 2]) -> Vec<[f32; 2]> {
    vec![a, [b[0], a[1]], b, [a[0], b[1]]]
}

fn disc(shape: Shape, center: [f32; 2], radius: f32, color: Color) -> Shape {
    shape.fill(circle(center, radius), color)
}

fn circle(center: [f32; 2], radius: f32) -> Vec<[f32; 2]> {
    (0..48)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / 48.;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        })
        .collect()
}

fn outline(shape: Shape, points: Vec<[f32; 2]>, width: f32, color: Color) -> Shape {
    shape.stroke_with(points, true, width, rune::LineCap::Round, color)
}

fn strokes(
    mut shape: Shape,
    l: Layout,
    points: &[(f32, f32)],
    dash: Dash,
    width: f32,
    color: Color,
) -> Shape {
    let screen_width = width.max(0.5) * l.scale;
    for run in style::strokes(points, dash, width) {
        let mut mapped: Vec<_> = run.iter().map(|p| map(l, *p)).collect();
        // A single point is a dot: a tap with the pen.
        if mapped.len() == 1 {
            mapped.push([mapped[0][0] + 0.01, mapped[0][1]]);
        }
        shape = shape.stroke(mapped, screen_width, color);
    }
    shape
}

fn paths(mut shape: Shape, mark: &Mark, l: Layout) -> Shape {
    let color = rgba(mark.color);
    match mark.tool {
        Tool::Pen => strokes(
            shape,
            l,
            &style::pen_points(mark),
            Dash::Solid,
            mark.width,
            color,
        ),
        Tool::Rectangle => {
            let points = style::box_points(mark);
            if mark.style.fill == Fill::Filled {
                shape.fill(points.iter().map(|p| map(l, *p)).collect::<Vec<_>>(), color)
            } else {
                strokes(shape, l, &points, mark.style.dash, mark.width, color)
            }
        }
        Tool::Arrow => {
            shape = strokes(
                shape,
                l,
                &style::shaft(mark),
                mark.style.dash,
                mark.width,
                color,
            );
            for head in style::heads(mark) {
                shape = shape.fill(head.iter().map(|p| map(l, *p)).collect::<Vec<_>>(), color);
            }
            for dot in style::dots(mark) {
                shape = disc(
                    shape,
                    map(l, dot),
                    (mark.width * 2.).max(3.) * l.scale,
                    color,
                );
            }
            shape
        }
        _ => shape,
    }
}

/// One mark's vector part.
fn paint(mut shape: Shape, mark: &Mark, l: Layout) -> Shape {
    let Some(&first) = mark.points.first() else {
        return shape;
    };
    let last = *mark.points.last().unwrap();
    let (a, b) = (map(l, first), map(l, last));
    let color = rgba(mark.color);
    match mark.tool {
        Tool::Select | Tool::Text => shape,
        Tool::Pen | Tool::Arrow | Tool::Rectangle => paths(shape, mark, l),
        Tool::Highlight => shape.fill(rectangle(a, b), color.alpha(0.35 * color.a)),
        Tool::Pixelate | Tool::Crop => {
            let tint = rgba(if mark.tool == Tool::Crop {
                [255, 255, 255, 255]
            } else {
                [0x74, 0x83, 0x9a, 255]
            });
            shape = shape.fill(rectangle(a, b), tint.alpha(0.12));
            shape = outline(shape, rectangle(a, b), 1., tint);
            for corner in rectangle(a, b) {
                let (lo, hi) = (
                    [corner[0] - 3., corner[1] - 3.],
                    [corner[0] + 3., corner[1] + 3.],
                );
                shape = shape.fill(rectangle(lo, hi), rgba([255, 255, 255, 255]));
                shape = outline(shape, rectangle(lo, hi), 1., rgba([0x64, 0x74, 0x8b, 255]));
            }
            shape
        }
        Tool::Counter => disc(shape, a, (mark.width * 3.6).max(1.) * l.scale, color),
        Tool::Spotlight => {
            let image = (map(l, (0., 0.)), map(l, (l.width, l.height)));
            let (left, right) = (a[0].min(b[0]), a[0].max(b[0]));
            let (top, bottom) = (a[1].min(b[1]), a[1].max(b[1]));
            let dim = Color {
                r: 0.,
                g: 0.,
                b: 0.,
                a: mark.style.dim,
            };
            for (from, to) in [
                (image.0, [image.1[0], top]),
                ([image.0[0], bottom], image.1),
                ([image.0[0], top], [left, bottom]),
                ([right, top], [image.1[0], bottom]),
            ] {
                shape = shape.fill(rectangle(from, to), dim);
            }
            let mut border = mark.clone();
            border.tool = Tool::Rectangle;
            border.width = 2. / l.scale;
            border.style.fill = Fill::Outline;
            paths(shape, &border, l)
        }
        Tool::Magnifier => {
            if mark.points.len() < 2 {
                return shape;
            }
            let radius = effects::radius(mark) * l.scale;
            shape = shape.line(a, b, l.scale, rune::LineCap::Round, color);
            shape = outline(
                shape,
                circle(a, radius / effects::zoom(mark)),
                1.5 * l.scale,
                color,
            );
            outline(shape, circle(b, radius), 3. * l.scale, color)
        }
    }
}

/// Text and step labels, which a Shape cannot draw.
fn label(mark: &Mark, l: Layout) -> Option<El> {
    let &first = mark.points.first()?;
    let a = map(l, first);
    let style = |size: f32| TextStyle {
        line_height: size,
        ..TextStyle::new(size, 400.)
    };
    match mark.tool {
        Tool::Text => {
            let size = (mark.width * 7.).max(1.) * l.scale;
            Some(
                text(mark.text.replace('\n', " "))
                    .font(style(size))
                    .color(rgba(mark.color))
                    .absolute(a[0], a[1])
                    .inert(),
            )
        }
        Tool::Counter => {
            let size = (mark.width * 4.4).max(1.) * l.scale;
            let radius = (mark.width * 3.6).max(1.) * l.scale;
            let dark = mark.color[..3].iter().map(|c| *c as u32).sum::<u32>() > 600;
            Some(
                el().absolute(a[0] - radius, a[1] - radius)
                    .size(radius * 2., radius * 2.)
                    .center()
                    .inert()
                    .child(
                        text(mark.text.clone())
                            .font(style(size))
                            .color(rgba(if dark {
                                [0x20, 0x22, 0x2a, 255]
                            } else {
                                [255, 255, 255, 255]
                            })),
                    ),
            )
        }
        _ => None,
    }
}

impl Editor {
    /// The marks not yet in the raster, the one being drawn last.
    pub(crate) fn live_marks(&self) -> Vec<&Mark> {
        let Some(doc) = &self.doc else {
            return vec![];
        };
        let mut marks: Vec<&Mark> = doc
            .marks
            .iter()
            .enumerate()
            .skip(self.preview.marks)
            .map(|(index, mark)| self.gesture.moved_mark(index).unwrap_or(mark))
            .collect();
        marks.extend(self.gesture.draft());
        marks
    }

    /// The live marks as elements over the canvas.
    pub(crate) fn live_layer(&self, shape: Shape, l: Layout) -> (Shape, Vec<El>) {
        let mut shape = shape;
        let mut labels = vec![];
        for mark in self.live_marks() {
            shape = paint(shape, mark, l);
            labels.extend(label(mark, l));
        }
        (shape, labels)
    }

    /// The lens of a magnifier being drawn or dragged: its pixels and where it sits.
    pub(crate) fn lens(&self, l: Layout) -> Option<El> {
        let mark = self
            .live_marks()
            .into_iter()
            .rfind(|m| m.tool == Tool::Magnifier && m.points.len() >= 2)?;
        self.live_source.as_ref()?;
        let center = map(l, *mark.points.last()?);
        let radius = effects::radius(mark) * l.scale;
        Some(
            canvas(Shape::new())
                .key("lens")
                .absolute(center[0] - radius, center[1] - radius)
                .size(radius * 2., radius * 2.)
                .radius(radius)
                .clip_soft(0.)
                .pixels(&self.live_lens)
                .inert(),
        )
    }

    /// Flattens the image off the UI thread for the lens to sample.
    pub(crate) fn flatten_for_lens(&mut self) {
        self.lens_request += 1;
        self.live_source = None;
        let request = self.lens_request;
        let document = self.doc().render_snapshot();
        let mailbox = self.mailbox.clone();
        self.port.run(Box::new(move || {
            let image = std::sync::Arc::new(document.plain(None));
            mailbox.post(super::Message::LensSource { request, image });
        }));
    }

    /// Re-samples the live lens after the magnifier moved.
    pub(crate) fn refresh_lens(&mut self) {
        let Some(source) = &self.live_source else {
            return;
        };
        let moved = match &self.gesture {
            super::gesture::Gesture::Moving { drag, .. } => Some(&drag.moved),
            super::gesture::Gesture::Drawing(mark) => Some(mark),
            _ => None,
        };
        if let Some(mark) = moved.filter(|m| m.tool == Tool::Magnifier) {
            let tile = effects::tile(source, mark);
            let (width, height) = tile.dimensions();
            self.live_lens.set(width, height, tile.into_raw());
        }
    }
}
