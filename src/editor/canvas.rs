//! The canvas: the raster, live marks on top, then selection and text being typed.
use super::{
    Editor,
    actions::Action,
    gesture::Gesture,
    live::{ACCENT, rgba},
    preview::MAX_VIEW_SIDE,
};
use crate::{
    arrow,
    document::{Mark, Tool},
    navigation::Layout,
};
use rune::{
    Cursor, Cx, El, Key, PointerButton, Shape, TextStyle, canvas, col, el, text, text_input, tok,
};

/// Labels hold one line, so a pasted line break becomes a space.
fn one_line(text: &str) -> String {
    text.split(['\r', '\n', '\u{2028}', '\u{2029}'])
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn at(l: Layout, x: f32, y: f32) -> [f32; 2] {
    [l.x + x * l.scale, l.y + y * l.scale]
}

fn dashed_box(shape: Shape, a: [f32; 2], b: [f32; 2], width: f32, color: rune::Color) -> Shape {
    shape.stroke_with(
        vec![a, [b[0], a[1]], b, [a[0], b[1]]],
        true,
        width,
        rune::LineCap::Round,
        color,
    )
}

fn handle(shape: Shape, p: [f32; 2]) -> Shape {
    let box_of = |r: f32| {
        vec![
            [p[0] - r, p[1] - r],
            [p[0] + r, p[1] - r],
            [p[0] + r, p[1] + r],
            [p[0] - r, p[1] + r],
        ]
    };
    let shape = shape.fill(box_of(4.), rgba([255, 255, 255, 255]));
    shape.stroke_with(box_of(4.), true, 1.5, rune::LineCap::Round, rgba(ACCENT))
}

/// Single-mark handles when the mark has any, otherwise a box around each.
fn selection(mut shape: Shape, marks: &[&Mark], l: Layout) -> Shape {
    let accent = rgba(ACCENT);
    if let [mark] = marks
        && matches!(mark.tool, Tool::Arrow | Tool::Magnifier | Tool::Spotlight)
    {
        for (x, y) in arrow::handles(mark) {
            shape = handle(shape, at(l, x, y));
        }
        return shape;
    }
    for mark in marks {
        let (left, top, right, bottom) = mark.bounds();
        let a = at(l, left, top);
        let b = at(l, right, bottom);
        shape = dashed_box(
            shape,
            [a[0] - 4., a[1] - 4.],
            [b[0] + 4., b[1] + 4.],
            1.,
            accent,
        );
    }
    shape
}

impl Editor {
    fn cursor(&self) -> Cursor {
        if matches!(self.gesture, Gesture::Panning { .. }) {
            Cursor::Grabbing
        } else if self.space_down {
            Cursor::Grab
        } else if self.tool == Tool::Select {
            Cursor::Default
        } else if self.tool == Tool::Text {
            Cursor::Text
        } else {
            Cursor::Crosshair
        }
    }

    fn annotation_nodes(&self, l: Layout) -> Vec<El> {
        let Some(doc) = &self.doc else {
            return vec![];
        };
        doc.marks
            .iter()
            .enumerate()
            .map(|(index, mark)| {
                let (left, top, right, bottom) = mark.bounds();
                let a = at(l, left, top);
                let b = at(l, right, bottom);
                el().key(format!("mark-{index}"))
                    .absolute(a[0], a[1])
                    .w((b[0] - a[0]).max(1.))
                    .h((b[1] - a[1]).max(1.))
                    .role("image")
                    .label(format!("{} annotation", mark.tool.label()))
                    .a11y_value(self.describe(index, mark))
            })
            .collect()
    }

    /// What a screen reader says about a mark besides its kind.
    fn describe(&self, index: usize, mark: &Mark) -> String {
        let (left, top, right, bottom) = mark.bounds();
        let mut value = format!(
            "{} by {} pixels at {}, {}",
            (right - left).round(),
            (bottom - top).round(),
            left.round(),
            top.round()
        );
        if matches!(mark.tool, Tool::Text | Tool::Counter) {
            value = format!("{}, {value}", mark.text);
        }
        if self.selected.contains(&index) {
            value.push_str(", selected");
        }
        value
    }

    /// The document as JSON for scripts, on the fake port only.
    pub(crate) fn state_probe(&self) -> Option<El> {
        if !self.port.exposes_state() {
            return None;
        }
        let marks: Vec<_> = self
            .doc
            .iter()
            .flat_map(|doc| doc.marks.iter().enumerate())
            .map(|(index, mark)| {
                serde_json::json!({
                    "tool": mark.tool.label(),
                    "points": mark.points,
                    "curve": mark.curve,
                    "color": mark.color,
                    "width": mark.width,
                    "text": mark.text,
                    "style": mark.style,
                    "selected": self.selected.contains(&index),
                })
            })
            .collect();
        let state = serde_json::json!({
            "marks": marks,
            "preview": {
                "behind": self.revision - self.preview.shown,
                "marks": self.preview.marks,
                "size": self.preview.size,
            },
            "lens": {
                "shown": self.lens.shown.is_some(),
                "sampling": self.lens.pending.is_some(),
                "behind": self.lens_behind(),
            },
        });
        Some(el().key("editor-state").label(state.to_string()).inert())
    }

    fn text_field(&self, l: Layout, cx: &Cx<Editor>) -> Option<El> {
        let draft = self.text.as_ref()?;
        let point = at(l, draft.mark.points[0].0, draft.mark.points[0].1);
        let size = (draft.mark.width * 7.).max(1.) * l.scale;
        let t = tok::theme(cx.system_dark());
        let style = TextStyle {
            line_height: size,
            ..TextStyle::new(size, 400.)
        };
        Some(
            text_input(&self.text_edit, style)
                .key("text-editor")
                .label("Annotation text")
                .single_line()
                .color(rgba(draft.mark.color))
                .absolute(point[0], point[1])
                .min_w(size.max(24.) * 8.)
                .border(1., t.indicator)
                .on_edit(|s: &mut Editor| &mut s.text_edit)
                .on_input_paste(|s: &mut Editor, content, _| {
                    s.text_edit.insert(&one_line(content.text()));
                    true
                })
                .on_submit(|s: &mut Editor, tx| {
                    s.commit_text(tx);
                    tx.blur();
                })
                .on_input_key(|s: &mut Editor, key, tx| {
                    if key.key == Key::Escape {
                        s.dispatch_ui(Action::Cancel, tx);
                        tx.blur();
                        return true;
                    }
                    false
                }),
        )
    }

    pub(crate) fn canvas_view(&self, cx: &mut Cx<Editor>) -> El {
        let t = tok::theme(cx.system_dark());
        let l = self.layout();
        let mut root = col()
            .key("canvas")
            .grow(1.)
            .h_full()
            .clip_soft(0.)
            .instant_changes()
            .fill(t.control_track)
            .cursor(self.cursor())
            .on_layout(|s: &mut Editor, rect| s.canvas = (rect[2], rect[3]))
            .pointer_buttons([PointerButton::Middle])
            .on_drag(|s: &mut Editor, drag, tx| {
                if tx.button() == PointerButton::Middle {
                    s.on_canvas_pan_drag(drag);
                } else {
                    s.on_canvas_drag(drag, tx);
                }
            })
            .on_wheel(|s: &mut Editor, wheel, tx| s.on_canvas_wheel(wheel, tx))
            .on_magnify(|s: &mut Editor, magnify, tx| {
                let anchor = (magnify.x, magnify.y);
                s.dispatch_ui(
                    Action::ZoomAt {
                        factor: 1. + magnify.delta,
                        anchor,
                    },
                    tx,
                );
                true
            })
            .on_smart_magnify(|s: &mut Editor, _, _, tx| {
                let action = if s.zoom.is_some() {
                    Action::Fit
                } else {
                    Action::ActualSize
                };
                s.dispatch_ui(action, tx);
                true
            });
        let Some(doc) = &self.doc else {
            return root.center().child(
                col()
                    .gap(8.)
                    .center()
                    .inert()
                    .child(
                        text("Open an image to edit it")
                            .key("editor-empty")
                            .font(tok::type_::TITLE)
                            .color(t.text),
                    )
                    .child(
                        text("Drop a PNG or JPEG here, paste one, or press ⌘O.")
                            .font(tok::type_::BODY)
                            .color(t.text_secondary),
                    ),
            );
        };
        let (width, height) = doc.base.dimensions();
        root = root.label(format!(
            "Screenshot canvas, {width} by {height} pixels, {} annotations",
            doc.marks.len()
        ));
        if self.preview.size.0 > 0 {
            root = root.child(
                el().key("image")
                    .absolute(l.x, l.y)
                    .w(l.width * l.scale)
                    .h(l.height * l.scale)
                    .pixels(&self.preview.pixels)
                    .inert(),
            );
        }
        let (mut shape, labels) = self.live_layer(Shape::new(), l);
        let selected: Vec<&Mark> = self
            .selected_indices()
            .into_iter()
            .map(|i| self.gesture.moved_mark(i).unwrap_or(&doc.marks[i]))
            .collect();
        shape = selection(shape, &selected, l);
        if let Gesture::Selecting {
            origin, current, ..
        } = self.gesture
        {
            let a = at(l, origin.0, origin.1);
            let b = at(l, current.0, current.1);
            shape = shape.fill(
                vec![a, [b[0], a[1]], b, [a[0], b[1]]],
                rgba([0x4c, 0x8d, 0xff, 0x22]),
            );
            shape = dashed_box(shape, a, b, 1., rgba(ACCENT));
        }
        root = root.child(
            canvas(shape)
                .key("overlay")
                .absolute(0., 0.)
                .w_full()
                .h_full()
                .inert(),
        );
        root = root.children(labels);
        root = root.children(self.lens(l));
        root = root.children(self.annotation_nodes(l));
        root = root.children(self.text_field(l, cx));
        let downscaled = width.max(height) > MAX_VIEW_SIDE;
        root.children(downscaled.then(|| {
            text(format!(
                "View {}×{}, image {width}×{height}, downscaled",
                self.preview.size.0, self.preview.size.1,
            ))
            .key("view-size")
            .font(tok::type_::LABEL)
            .color(t.text_secondary)
            .absolute(10., 8.)
            .inert()
        }))
    }

    fn on_canvas_wheel(&mut self, wheel: rune::WheelEvent, tx: &mut rune::Tx) -> bool {
        if self.gesture.is_active() {
            return true;
        }
        if wheel.mods.cmd || wheel.mods.ctrl {
            self.dispatch_ui(
                Action::ZoomAt {
                    factor: (wheel.dy * 0.008).exp(),
                    anchor: (wheel.x, wheel.y),
                },
                tx,
            );
        } else if wheel.mods.shift && wheel.dx == 0. {
            self.dispatch_ui(Action::PanBy((wheel.dy, 0.)), tx);
        } else {
            self.dispatch_ui(Action::PanBy((wheel.dx, wheel.dy)), tx);
        }
        true
    }
}
