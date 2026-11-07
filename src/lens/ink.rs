//! The ink layer over the area: its outline for the whole capture, and the marks
//! drawn while the draw key is held (D3, D4 and D5 on the plan). Marks are in points
//! on the display, so a moved area leaves them in place. The corner handles are
//! small windows of their own, so the ink layer takes no pointer while the key is up.
use super::{
    Lens, StrokeStart,
    session::{Area, DisplayFrame},
};
use crate::{
    arrow,
    document::{Mark, Tool},
    style,
};
use rune::{Color, Cursor, Cx, Drag, El, Phase, Shape, Tx, canvas, el, text};
use std::sync::Arc;

const INK: [u8; 4] = [255, 61, 46, 255];
const STROKE_POINTS: f32 = 4.;
const LABEL_POINTS: f32 = 3.2;
/// Glance's limit for a mark that was meant, not a slip of the pointer.
const MIN_TRAVEL: f32 = 8.;
/// How far a drag strays from its chord and still counts as straight: a feel
/// choice, judged in the user's session (C15), not a measured limit.
const STRAIGHT_MIN: f32 = 6.;
const STRAIGHT_RATIO: f32 = 0.08;
/// A longer path than this means the hand went back along the line.
const STRAIGHT_PATH_RATIO: f32 = 1.15;
/// `validate_mark`'s limit: past it the editor would refuse the mark.
const MAX_POINTS: usize = 2000;
const OUTLINE: f32 = 2.;
/// The corner handle's grab zone, inside the area so it stays on screen when the
/// area is the whole display.
pub const HANDLE_ZONE: f32 = 24.;
const HANDLE_DOT: f32 = 10.;

/// What the next stroke makes, set on the bar's tool row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InkTool {
    /// An arrow when the drag is straight, otherwise freehand; ⇧ makes a box.
    #[default]
    Stroke,
    Highlight,
    Label,
    Counter,
}

fn mark(tool: Tool, points: Vec<(f32, f32)>, width: f32, text: String) -> Mark {
    Mark {
        style: Default::default(),
        tool,
        curve: None,
        points,
        color: INK,
        width,
        text,
    }
}

fn is_straight(points: &[(f32, f32)]) -> bool {
    let (a, b) = (points[0], points[points.len() - 1]);
    let chord = (b.0 - a.0).hypot(b.1 - a.1);
    if chord == 0. {
        return false;
    }
    let stray = points
        .iter()
        .map(|p| ((b.0 - a.0) * (a.1 - p.1) - (a.0 - p.0) * (b.1 - a.1)).abs() / chord)
        .fold(0., f32::max);
    let path: f32 = points
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum();
    stray <= STRAIGHT_MIN.max(chord * STRAIGHT_RATIO) && path <= chord * STRAIGHT_PATH_RATIO
}

/// The mark a finished stroke makes, in the same space as `points`. None when
/// the stroke was too short to mean anything.
pub fn mark_from_stroke(
    start: &StrokeStart,
    points: &[(f32, f32)],
    counter: usize,
) -> Option<Mark> {
    let (first, last) = (*points.first()?, *points.last()?);
    if start.tool == InkTool::Counter {
        return Some(mark(
            Tool::Counter,
            vec![last],
            STROKE_POINTS,
            counter.to_string(),
        ));
    }
    let path: f32 = points
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum();
    if path < MIN_TRAVEL {
        return None;
    }
    let shaped = draft_mark(start, points).filter(|mark| mark.tool != Tool::Pen);
    Some(shaped.unwrap_or_else(|| {
        if is_straight(points) {
            return mark(Tool::Arrow, vec![first, last], STROKE_POINTS, String::new());
        }
        let mut points = points.to_vec();
        while points.len() > MAX_POINTS {
            points = points.iter().step_by(2).copied().collect();
        }
        mark(Tool::Pen, points, STROKE_POINTS, String::new())
    }))
}

/// What a stroke shows while it is being drawn.
fn draft_mark(start: &StrokeStart, points: &[(f32, f32)]) -> Option<Mark> {
    let (first, last) = (*points.first()?, *points.last()?);
    Some(match (start.tool, start.boxed) {
        (InkTool::Highlight, _) => mark(
            Tool::Highlight,
            vec![first, last],
            STROKE_POINTS,
            String::new(),
        ),
        (InkTool::Stroke, true) => mark(
            Tool::Rectangle,
            vec![first, last],
            STROKE_POINTS,
            String::new(),
        ),
        (InkTool::Stroke, false) => mark(Tool::Pen, points.to_vec(), STROKE_POINTS, String::new()),
        (InkTool::Label | InkTool::Counter, _) => return None,
    })
}

pub fn label_mark(at: (f32, f32), text: &str) -> Mark {
    mark(Tool::Text, vec![at], LABEL_POINTS, text.to_string())
}

/// `mark` in its still's pixels. `scale` is pixels per point.
pub fn on_still(mark: &Mark, area: Area, scale: f32) -> Mark {
    let mut still = mark.clone();
    for point in &mut still.points {
        point.0 = (point.0 - area.rect[0] as f32) * scale;
        point.1 = (point.1 - area.rect[1] as f32) * scale;
    }
    still.width *= scale;
    still
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub const ALL: [Corner; 4] = [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomLeft,
        Corner::BottomRight,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Corner::TopLeft => "handle-top-left",
            Corner::TopRight => "handle-top-right",
            Corner::BottomLeft => "handle-bottom-left",
            Corner::BottomRight => "handle-bottom-right",
        }
    }

    fn zone(self, width: f32, height: f32) -> (f32, f32) {
        let right = width - HANDLE_ZONE;
        let bottom = height - HANDLE_ZONE;
        match self {
            Corner::TopLeft => (0., 0.),
            Corner::TopRight => (right, 0.),
            Corner::BottomLeft => (0., bottom),
            Corner::BottomRight => (right, bottom),
        }
    }

    /// The corner's own point on an area of this size.
    fn point(self, width: f32, height: f32) -> (f32, f32) {
        match self {
            Corner::TopLeft => (0., 0.),
            Corner::TopRight => (width, 0.),
            Corner::BottomLeft => (0., height),
            Corner::BottomRight => (width, height),
        }
    }

    /// The handle's window, in points from the display's top-left. It stays
    /// below the menu bar, which it would otherwise cover and take clicks from.
    pub fn rect(self, area: Area, display: DisplayFrame) -> [f32; 4] {
        let (x, y) = self.zone(area.rect[2] as f32, area.rect[3] as f32);
        let top = (area.rect[1] as f32 + y).max(display.menu_bar as f32);
        [area.rect[0] as f32 + x, top, HANDLE_ZONE, HANDLE_ZONE]
    }

    /// `rect` with this corner moved to the point `(x, y)`; the opposite corner stays.
    fn moved(self, rect: [f64; 4], x: f64, y: f64) -> [f64; 4] {
        let [left, top, width, height] = rect;
        let (right, bottom) = (left + width, top + height);
        let (x0, y0, x1, y1) = match self {
            Corner::TopLeft => (x, y, right, bottom),
            Corner::TopRight => (left, y, x, bottom),
            Corner::BottomLeft => (x, top, right, y),
            Corner::BottomRight => (left, top, x, y),
        };
        [x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()]
    }
}

struct Stroke {
    start: StrokeStart,
    points: Vec<(f32, f32)>,
}

pub struct Ink {
    pub lens: Arc<Lens>,
    stroke: Option<Stroke>,
}

impl Ink {
    pub fn new(lens: Arc<Lens>) -> Self {
        Self { lens, stroke: None }
    }
}

fn color(rgba: [u8; 4], alpha: f32) -> Color {
    rune::color::hex(&format!("#{:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2]))
        .expect("six hex digits")
        .alpha(rgba[3] as f32 / 255. * alpha)
}

fn at(point: (f32, f32), origin: (f32, f32)) -> [f32; 2] {
    [point.0 - origin.0, point.1 - origin.1]
}

/// Draws `mark` in the same shapes the still's raster paints.
fn draw(mut shape: Shape, mark: &Mark, origin: (f32, f32)) -> Shape {
    let ink = color(mark.color, 1.);
    let path = |points: Vec<(f32, f32)>| -> Vec<[f32; 2]> {
        points.into_iter().map(|p| at(p, origin)).collect()
    };
    match mark.tool {
        Tool::Pen => shape = shape.stroke(path(style::pen_points(mark)), mark.width, ink),
        Tool::Rectangle => shape = shape.stroke(path(style::box_points(mark)), mark.width, ink),
        Tool::Arrow => {
            shape = shape
                .stroke(path(style::shaft(mark)), mark.width, ink)
                .fill(path(arrow::head(mark).to_vec()), ink);
        }
        Tool::Highlight => {
            let (a, b) = (at(mark.points[0], origin), at(mark.points[1], origin));
            shape = shape.fill(
                vec![a, [b[0], a[1]], b, [a[0], b[1]]],
                color(mark.color, 0.35),
            );
        }
        Tool::Counter => {
            let center = at(mark.points[0], origin);
            let radius = mark.width * 3.6;
            let circle: Vec<[f32; 2]> = (0..32)
                .map(|i| {
                    let angle = i as f32 / 32. * std::f32::consts::TAU;
                    [
                        center[0] + radius * angle.cos(),
                        center[1] + radius * angle.sin(),
                    ]
                })
                .collect();
            shape = shape.fill(circle, ink);
        }
        _ => {}
    }
    shape
}

/// The word a mark is listed by, so the tree says what was drawn.
fn kind(mark: &Mark) -> &'static str {
    match mark.tool {
        Tool::Pen => "freehand",
        Tool::Arrow => "arrow",
        Tool::Rectangle => "box",
        Tool::Highlight => "highlight",
        Tool::Counter => "counter",
        Tool::Text => "label",
        _ => "mark",
    }
}

fn mark_element(index: usize, mark: &Mark, origin: (f32, f32)) -> El {
    let a = at(mark.points[0], origin);
    let key = format!("mark-{index}");
    match mark.tool {
        Tool::Text => text(mark.text.clone())
            .key(key)
            .label(kind(mark))
            .absolute(a[0], a[1])
            .font(rune::tok::TextStyle::new(mark.width * 7., 500.))
            .color(color(mark.color, 1.)),
        Tool::Counter => {
            let radius = mark.width * 3.6;
            el().key(key)
                .label(kind(mark))
                .absolute(a[0] - radius, a[1] - radius)
                .size(radius * 2., radius * 2.)
                .center()
                .child(
                    text(mark.text.clone())
                        .font(rune::tok::type_::LABEL)
                        .color(Color::WHITE),
                )
        }
        _ => el()
            .key(key)
            .label(kind(mark))
            .absolute(a[0], a[1])
            .size(1., 1.),
    }
}

pub fn ink_view(s: &Ink, _: &mut Cx<Ink>) -> El {
    let area = s
        .lens
        .read(|summary| summary.capture.as_ref().map(|c| c.area));
    let Some(area) = area else {
        return el().key("ink").w_full().h_full();
    };
    let origin = (area.rect[0] as f32, area.rect[1] as f32);
    let (width, height) = (area.rect[2] as f32, area.rect[3] as f32);
    let marks = s.lens.ink_marks();
    let mut shape = Shape::new();
    for mark in &marks {
        shape = draw(shape, mark, origin);
    }
    let draft = s
        .stroke
        .as_ref()
        .and_then(|stroke| draft_mark(&stroke.start, &stroke.points));
    if let Some(mark) = &draft {
        shape = draw(shape, mark, origin);
    }
    let mut root = el()
        .key("ink")
        .w_full()
        .h_full()
        .child(canvas(shape).absolute(0., 0.).size(width, height))
        .child(
            el().key("area-outline")
                .absolute(0., 0.)
                .size(width, height)
                .border(OUTLINE, accent()),
        );
    for (index, mark) in marks.iter().enumerate() {
        root = root.child(mark_element(index, mark, origin));
    }
    if let Some(mark) = &draft {
        root = root.child(
            el().key("draft")
                .label(kind(mark))
                .absolute(0., 0.)
                .size(1., 1.),
        );
    }
    // The layer takes the pointer only while the draw key is held (D5).
    root.on_drag(|s: &mut Ink, d: &mut Drag, _: &mut Tx| {
        let area = s
            .lens
            .read(|summary| summary.capture.as_ref().map(|c| c.area));
        let Some(area) = area else { return };
        let point = (d.x + area.rect[0] as f32, d.y + area.rect[1] as f32);
        match d.phase {
            Phase::Down => {
                s.stroke = s.lens.begin_stroke().map(|start| Stroke {
                    start,
                    points: vec![point],
                });
            }
            Phase::Move => {
                if let Some(stroke) = &mut s.stroke {
                    stroke.points.push(point);
                }
            }
            Phase::Up => {
                if let Some(stroke) = s.stroke.take()
                    && !d.cancelled()
                {
                    s.lens.finish_stroke(stroke.start, stroke.points);
                }
            }
        }
    })
}

fn accent() -> Color {
    rune::color::hex("#0a84ff").expect("six hex digits")
}

/// One corner handle's window. It holds still while any corner is dragged, so
/// the drag it owns stays in its own points.
pub struct Handle {
    pub lens: Arc<Lens>,
    pub corner: Corner,
    grab: Option<Grab>,
}

/// Where the handle's window sat when pressed, and how far the press sat from
/// the corner, so the corner moves by the pointer's travel and does not jump.
struct Grab {
    window: (f32, f32),
    offset: (f32, f32),
}

impl Handle {
    pub fn new(lens: Arc<Lens>, corner: Corner) -> Self {
        Self {
            lens,
            corner,
            grab: None,
        }
    }
}

pub fn handle_view(s: &Handle, _: &mut Cx<Handle>) -> El {
    let resizing = s.lens.read(|summary| summary.resizing);
    let mut handle = el()
        .key(s.corner.key())
        .w_full()
        .h_full()
        .center()
        .hit()
        .cursor(Cursor::Crosshair);
    // The outline shows the corner while it moves; the dots return where it lands.
    if !resizing {
        handle = handle.child(
            el().size(HANDLE_DOT, HANDLE_DOT)
                .radius(2.)
                .fill(Color::WHITE)
                .border(OUTLINE, accent()),
        );
    }
    handle.on_drag(|s: &mut Handle, d: &mut Drag, _: &mut Tx| {
        let capture = s
            .lens
            .read(|summary| summary.capture.as_ref().map(|c| (c.area, c.display)));
        let Some((area, display)) = capture else {
            return;
        };
        match d.phase {
            Phase::Down => {
                let [x, y, ..] = s.corner.rect(area, display);
                let (width, height) = (area.rect[2] as f32, area.rect[3] as f32);
                let corner = s.corner.point(width, height);
                let press = (x + d.x, y + d.y);
                s.grab = Some(Grab {
                    window: (x, y),
                    offset: (
                        area.rect[0] as f32 + corner.0 - press.0,
                        area.rect[1] as f32 + corner.1 - press.1,
                    ),
                });
                s.lens.set_resizing(true);
            }
            Phase::Move => {
                if let Some(grab) = &s.grab {
                    let x = grab.window.0 + d.x + grab.offset.0;
                    let y = grab.window.1 + d.y + grab.offset.1;
                    s.lens
                        .set_area(s.corner.moved(area.rect, f64::from(x), f64::from(y)));
                }
            }
            Phase::Up => {
                s.grab = None;
                s.lens.set_resizing(false);
            }
        }
    })
}
