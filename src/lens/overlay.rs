//! The in-place Send surface (plan D9) on the ink layer (D10): a capture fills
//! the display it came from, marks paint over it, and a bar at the bottom holds
//! the target session and the message. Tab opens the palette; ⌘E opens the editor.
use super::{Capture, attn};
use crate::document::{Mark, Tool};
use crate::prompt_input::{self, PromptInput};
use crate::{Layout, theme::Theme};
use gpui::{prelude::*, *};
use std::{cell::Cell, rc::Rc, sync::Arc};

/// Ink colour and widths, in screen points; marks store image pixels.
const INK: [u8; 4] = [255, 61, 46, 255];
const STROKE_POINTS: f32 = 4.;
const LABEL_POINTS: f32 = 3.2;

/// What the overlay can do. Pointer and keys produce these, and so does
/// automation (`attn-lens overlay …`), so both go through one path.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// A mark in image pixels: `tool` is arrow, rectangle or text.
    AddMark {
        tool: Tool,
        points: Vec<(f32, f32)>,
        #[serde(default)]
        text: String,
    },
    Undo,
    SetMessage {
        text: String,
    },
    ChooseSession {
        id: String,
    },
    TogglePalette,
    Send,
    Pair,
    Copy,
    Save,
    Edit,
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Loading,
    Unpaired(String),
    Pairing {
        code: Option<String>,
    },
    /// attn could not list the sessions; ↩ and Tab ask again.
    Unlisted(String),
    Ready,
    Sending,
    Failed(attn::Failure),
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Session(usize),
    Copy,
    Save,
    Edit,
}

pub struct Overlay {
    pub(super) capture: Capture,
    frame: Arc<RenderImage>,
    focus: FocusHandle,
    message: Entity<PromptInput>,
    label: Option<((f32, f32), Entity<PromptInput>)>,
    label_armed: bool,
    draft: Option<Mark>,
    pub(super) sessions: Vec<attn::Session>,
    pub(super) status: Status,
    palette: Option<usize>,
    note: Option<SharedString>,
    layout: Rc<Cell<Layout>>,
    _subscriptions: Vec<Subscription>,
}

impl Overlay {
    pub fn new(capture: Capture, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let message =
            cx.new(|cx| PromptInput::new(focus.clone(), "Message for the agent (optional)", cx));
        message.update(cx, |input, cx| input.set_text(&capture.message, cx));
        let subscription = cx.subscribe(&message, |this, _, event, cx| match event {
            prompt_input::Event::Changed(text) => this.capture.message = text.clone(),
            prompt_input::Event::Submit => this.dispatch(Action::Send, cx),
            prompt_input::Event::Close => {}
        });
        message.read(cx).focus(window);
        let frame = crate::editor::render_image((*capture.document.base).clone());
        let mut this = Self {
            capture,
            frame,
            focus,
            message,
            label: None,
            label_armed: false,
            draft: None,
            sessions: vec![],
            status: Status::Loading,
            palette: None,
            note: None,
            layout: Rc::new(Cell::new(Layout::default())),
            _subscriptions: vec![subscription],
        };
        this.load_sessions(cx);
        this
    }

    pub fn dispatch(&mut self, action: Action, cx: &mut Context<Self>) {
        self.note = None;
        match action {
            Action::AddMark { tool, points, text } => {
                if matches!(tool, Tool::Arrow | Tool::Rectangle | Tool::Text) && !points.is_empty()
                {
                    let mark = self.mark(tool, points, text);
                    self.capture.document.commit(mark);
                }
            }
            Action::Undo => self.capture.document.undo(),
            Action::SetMessage { text } => {
                self.capture.message = text.clone();
                self.message
                    .update(cx, |input, cx| input.set_text(&text, cx));
            }
            Action::ChooseSession { id } => {
                if self.sessions.iter().any(|s| s.id == id) {
                    self.capture.target = Some(id);
                    self.palette = None;
                }
            }
            Action::TogglePalette => {
                self.palette = match self.palette {
                    Some(_) => None,
                    None => Some(0),
                };
                if self.palette.is_some() && matches!(self.status, Status::Unlisted(_)) {
                    self.load_sessions(cx);
                }
            }
            Action::Send => self.send(cx),
            Action::Pair => self.pair(cx),
            Action::Copy => {
                let image = self.capture.document.export();
                self.note = Some("Copying…".into());
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { crate::platform::copy(image) })
                        .await;
                    this.update(cx, |this, cx| {
                        this.note =
                            Some(result.map_or_else(|e| e.into(), |_| "Copied the image".into()));
                        cx.notify();
                    })
                })
                .detach();
            }
            Action::Save => {
                let image = self.capture.document.export();
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { crate::platform::save(image) })
                        .await;
                    this.update(cx, |this, cx| {
                        this.note = match result {
                            Ok(Some(path)) => Some(format!("Saved {}", path.display()).into()),
                            Ok(None) => None,
                            Err(e) => Some(e.into()),
                        };
                        cx.notify();
                    })
                })
                .detach();
            }
            Action::Edit => {
                let document = self.capture.document.clone();
                cx.defer(move |cx| super::open_editor(document, cx));
                super::close_overlay(self.capture.clone(), cx);
            }
            Action::Close => super::close_overlay(self.capture.clone(), cx),
        }
        cx.notify();
    }

    fn mark(&self, tool: Tool, points: Vec<(f32, f32)>, text: String) -> Mark {
        // Image pixels per screen point; 1 until the first layout.
        let scale = self.layout.get().scale;
        let pixels_per_point = if scale > 0. { scale.recip() } else { 1. };
        Mark {
            style: Default::default(),
            tool,
            curve: None,
            points,
            color: INK,
            width: if tool == Tool::Text {
                LABEL_POINTS
            } else {
                STROKE_POINTS
            } * pixels_per_point,
            text,
        }
    }

    fn load_sessions(&mut self, cx: &mut Context<Self>) {
        let Some(token) = attn::token() else {
            self.status = Status::Unpaired("Attn Lens is not paired with attn yet.".into());
            return;
        };
        self.status = Status::Loading;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { attn::locate().and_then(|bin| attn::sessions(&bin, &token)) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(sessions) => {
                        let keep = this
                            .capture
                            .target
                            .as_ref()
                            .filter(|id| sessions.iter().any(|s| &s.id == *id))
                            .cloned();
                        this.capture.target = keep.or_else(|| {
                            attn::default_session(&sessions).map(|i| sessions[i].id.clone())
                        });
                        this.sessions = sessions;
                        this.status = Status::Ready;
                    }
                    Err(attn::Failure::NotPaired(text)) => {
                        attn::forget_token();
                        this.status = Status::Unpaired(text);
                    }
                    Err(attn::Failure::Error(text)) => this.status = Status::Unlisted(text),
                }
                cx.notify();
            })
        })
        .detach();
    }

    fn pair(&mut self, cx: &mut Context<Self>) {
        if matches!(self.status, Status::Pairing { .. } | Status::Sending) {
            return;
        }
        self.status = Status::Pairing { code: None };
        let (codes, code) = async_channel::bounded(1);
        // The token is stored here, not in the overlay, so a pairing finished
        // after the person closed the overlay still counts.
        let task = cx.background_executor().spawn(async move {
            let bin = attn::locate()?;
            let token = attn::pair(&bin, |request| {
                let _ = codes.try_send(request.code);
            })?;
            attn::store_token(&token)
        });
        cx.spawn(async move |this, cx| {
            if let Ok(code) = code.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.status = Status::Pairing { code: Some(code) };
                    cx.notify();
                });
            }
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.load_sessions(cx),
                    Err(attn::Failure::NotPaired(text) | attn::Failure::Error(text)) => {
                        this.status = Status::Unpaired(text)
                    }
                }
                cx.notify();
            })
        })
        .detach();
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        match &self.status {
            Status::Unpaired(_) => return self.pair(cx),
            Status::Failed(attn::Failure::NotPaired(_)) => return self.pair(cx),
            Status::Unlisted(_) => return self.load_sessions(cx),
            Status::Sending | Status::Pairing { .. } | Status::Loading => return,
            Status::Ready | Status::Failed(_) => {}
        }
        let Some(session) = self.target().cloned() else {
            self.note = Some("Choose a session first: press Tab.".into());
            return;
        };
        let Some(token) = attn::token() else {
            self.status = Status::Unpaired("Attn Lens is not paired with attn yet.".into());
            return;
        };
        self.status = Status::Sending;
        let image = self.capture.document.export();
        let message = self.capture.message.clone();
        let id = session.id.clone();
        let task = cx.background_executor().spawn(async move {
            let dir = tempfile::Builder::new()
                .prefix("attn-lens-")
                .tempdir()
                .map_err(|e| {
                    attn::Failure::Error(format!("Could not make a folder for the image: {e}"))
                })?;
            let file = dir.path().join("screenshot.png");
            image
                .save_with_format(&file, image::ImageFormat::Png)
                .map_err(|e| attn::Failure::Error(format!("Could not write the image: {e}")))?;
            let bin = attn::locate()?;
            let sent = attn::send(&bin, &token, &id, &file, &message);
            if let Err(attn::Failure::NotPaired(_)) = sent {
                attn::forget_token();
            }
            sent
        });
        // The result goes to Lens, which finds the capture whether the overlay
        // is still open or the person closed it into the tile.
        let (capture, label) = (self.capture.id, session.label.clone());
        cx.spawn(async move |_, cx| {
            let result = task.await;
            cx.update(|cx| super::finish_send(capture, label, result, cx))
        })
        .detach();
    }

    /// Applies a send's result if this overlay shows capture `id`.
    pub(super) fn finish_send(
        &mut self,
        id: u64,
        label: &str,
        result: &Result<attn::Sent, attn::Failure>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.capture.id != id {
            return false;
        }
        match result {
            Ok(_) => {
                self.capture.sent_to = Some(label.to_owned());
                super::close_overlay(self.capture.clone(), cx);
            }
            Err(failure) => self.status = Status::Failed(failure.clone()),
        }
        cx.notify();
        true
    }

    fn target(&self) -> Option<&attn::Session> {
        let id = self.capture.target.as_ref()?;
        self.sessions.iter().find(|s| &s.id == id)
    }

    /// Palette rows: the chosen session first, then the rest grouped by host, then actions.
    fn rows(&self) -> Vec<Row> {
        let chosen = self
            .capture
            .target
            .as_ref()
            .and_then(|id| self.sessions.iter().position(|s| &s.id == id));
        let mut rest: Vec<usize> = (0..self.sessions.len())
            .filter(|i| Some(*i) != chosen)
            .collect();
        rest.sort_by(|a, b| {
            let (a, b) = (&self.sessions[*a], &self.sessions[*b]);
            (
                a.host.is_some(),
                a.host.as_deref(),
                b.last_focused_at.as_deref(),
            )
                .cmp(&(
                    b.host.is_some(),
                    b.host.as_deref(),
                    a.last_focused_at.as_deref(),
                ))
        });
        chosen
            .into_iter()
            .chain(rest)
            .map(Row::Session)
            .chain([Row::Copy, Row::Save, Row::Edit])
            .collect()
    }

    fn run_row(&mut self, row: Row, cx: &mut Context<Self>) {
        let action = match row {
            Row::Session(i) => Action::ChooseSession {
                id: self.sessions[i].id.clone(),
            },
            Row::Copy => Action::Copy,
            Row::Save => Action::Save,
            Row::Edit => Action::Edit,
        };
        self.palette = None;
        self.dispatch(action, cx);
    }

    fn image_point(&self, position: Point<Pixels>) -> (f32, f32) {
        let l = self.layout.get();
        let w = self.capture.document.base.width() as f32;
        let h = self.capture.document.base.height() as f32;
        (
            ((f32::from(position.x) - l.x) / l.scale).clamp(0., w),
            ((f32::from(position.y) - l.y) / l.scale).clamp(0., h),
        )
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = e.keystroke.key.as_str();
        let m = e.keystroke.modifiers;
        if self.message.read(cx).buffer.marked.is_some() {
            return;
        }
        if let Some((at, input)) = self.label.clone() {
            match key {
                "enter" => {
                    let text = input.read(cx).buffer.text().trim().to_owned();
                    self.label = None;
                    self.message.read(cx).focus(window);
                    if !text.is_empty() {
                        self.dispatch(
                            Action::AddMark {
                                tool: Tool::Text,
                                points: vec![at],
                                text,
                            },
                            cx,
                        );
                    }
                    cx.stop_propagation();
                }
                "escape" => {
                    self.label = None;
                    self.message.read(cx).focus(window);
                    cx.notify();
                    cx.stop_propagation();
                }
                _ => {}
            }
            return;
        }
        let selection = !self.message.read(cx).buffer.selection().is_empty();
        let action = match (key, m.platform, m.alt, m.shift) {
            ("tab", false, false, _) => Some(Action::TogglePalette),
            ("escape", ..) if self.palette.is_some() => Some(Action::TogglePalette),
            ("escape", ..) => Some(Action::Close),
            ("up" | "down", false, false, false) if self.palette.is_some() => {
                let rows = self.rows().len();
                let at = self.palette.unwrap_or(0);
                self.palette = Some(if key == "down" {
                    (at + 1) % rows
                } else {
                    (at + rows - 1) % rows
                });
                cx.notify();
                cx.stop_propagation();
                return;
            }
            ("enter", false, false, false) if self.palette.is_some() => {
                let rows = self.rows();
                if let Some(row) = self.palette.and_then(|i| rows.get(i).copied()) {
                    self.run_row(row, cx);
                }
                cx.stop_propagation();
                return;
            }
            ("enter", false, false, false) => Some(Action::Send),
            ("z", true, false, false) => Some(Action::Undo),
            ("c", true, false, false) if !selection => Some(Action::Copy),
            ("s", true, false, false) => Some(Action::Save),
            ("e", true, false, false) => Some(Action::Edit),
            ("t", false, true, false) => {
                self.label_armed = !self.label_armed;
                cx.notify();
                cx.stop_propagation();
                return;
            }
            _ => None,
        };
        if let Some(action) = action {
            cx.stop_propagation();
            self.dispatch(action, cx);
        }
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.image_point(e.position);
        if self.label_armed {
            self.label_armed = false;
            let input = cx.new(|cx| PromptInput::new(self.focus.clone(), "Label", cx));
            input.read(cx).focus(window);
            self.label = Some((p, input));
        } else {
            let tool = if e.modifiers.shift {
                Tool::Rectangle
            } else {
                Tool::Arrow
            };
            self.draft = Some(self.mark(tool, vec![p, p], String::new()));
        }
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if e.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let p = self.image_point(e.position);
        if let Some(draft) = &mut self.draft {
            draft.points[1] = p;
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        let (a, b) = (draft.points[0], draft.points[1]);
        let points = self.layout.get().scale * ((a.0 - b.0).hypot(a.1 - b.1));
        if points > 8. {
            self.dispatch(
                Action::AddMark {
                    tool: draft.tool,
                    points: draft.points,
                    text: String::new(),
                },
                cx,
            );
        }
        cx.notify();
    }
}

impl Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx);
        let viewport = window.viewport_size();
        let (iw, ih) = self.capture.document.base.dimensions();
        let scale =
            (f32::from(viewport.width) / iw as f32).min(f32::from(viewport.height) / ih as f32);
        let layout = Layout {
            x: (f32::from(viewport.width) - iw as f32 * scale) / 2.,
            y: (f32::from(viewport.height) - ih as f32 * scale) / 2.,
            scale,
            width: iw as f32 * scale,
            height: ih as f32 * scale,
        };
        self.layout.set(layout);
        let frame = self.frame.clone();
        let marks: Vec<Mark> = self
            .capture
            .document
            .marks
            .iter()
            .cloned()
            .chain(self.draft.clone())
            .collect();
        let ink = canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                let image = Bounds::new(
                    point(px(layout.x), px(layout.y)),
                    size(px(layout.width), px(layout.height)),
                );
                let _ = window.paint_image(image, Corners::default(), frame, 0, false);
                for mark in &marks {
                    crate::drawing::paint(mark, layout, window, cx);
                }
            },
        )
        .size_full();
        let mut root = div()
            .id("lens-overlay")
            .size_full()
            .relative()
            .bg(rgb(0x050507))
            .track_focus(&self.focus)
            .key_context("LensOverlay")
            .capture_key_down(cx.listener(Self::key))
            .cursor(if self.label_armed {
                CursorStyle::IBeam
            } else {
                CursorStyle::Crosshair
            })
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .child(ink)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .border_3()
                    .border_color(rgb(theme.accent)),
            );
        if let Some(((x, y), input)) = &self.label {
            root = root.child(
                div()
                    .absolute()
                    .left(px(layout.x + x * layout.scale))
                    .top(px(layout.y + y * layout.scale))
                    .w(px(260.))
                    .child(input.clone()),
            );
        }
        root.child(self.bar(&theme, cx)).children(
            self.palette
                .map(|selected| self.palette_view(selected, &theme, cx)),
        )
    }
}

impl Overlay {
    fn chip(&self, theme: &Theme) -> Div {
        let body: SharedString = match (&self.status, self.target()) {
            (Status::Unpaired(_) | Status::Pairing { .. }, _) => "Not paired with attn".into(),
            (Status::Loading, _) => "Loading sessions…".into(),
            (Status::Unlisted(_), _) => "Sessions unavailable".into(),
            (_, Some(s)) => format!("{} · {}", s.label, s.host_label()).into(),
            (_, None) if self.sessions.is_empty() => "No sessions in attn".into(),
            (_, None) => "Choose a session".into(),
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .max_w(px(300.))
            .rounded_md()
            .bg(rgb(theme.surface))
            .border_1()
            .border_color(rgb(theme.border))
            .text_color(rgb(theme.text))
            .child(
                div()
                    .text_color(rgb(theme.accent))
                    .font_weight(FontWeight::BOLD)
                    .child("→"),
            )
            .child(
                div()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(body),
            )
    }

    fn bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let marks = self.capture.document.marks.len();
        let line: Option<(SharedString, u32)> = match &self.status {
            Status::Unpaired(why) => Some((
                format!(
                    "{why} Press ↩ to pair: attn asks you to allow “{}”.",
                    attn::SENDER_NAME
                )
                .into(),
                theme.secondary,
            )),
            Status::Pairing { code: None } => {
                Some(("Asking attn to pair…".into(), theme.secondary))
            }
            Status::Pairing { code: Some(code) } => Some((
                format!("In attn, check the code is {code} and click Allow.").into(),
                theme.accent,
            )),
            Status::Sending => Some(("Sending…".into(), theme.secondary)),
            Status::Failed(attn::Failure::NotPaired(text)) => Some((
                format!("{text}\nPress ↩ to pair again.").into(),
                theme.error,
            )),
            Status::Failed(attn::Failure::Error(text)) => Some((text.clone().into(), theme.error)),
            Status::Unlisted(text) => Some((
                format!("{text}\nPress ↩ to list the sessions again.").into(),
                theme.error,
            )),
            Status::Loading | Status::Ready => None,
        };
        let hints = format!(
            "{marks} mark{} · drag arrow · ⇧drag box · ⌥T label · Tab sessions · ⌘E edit · Esc close",
            if marks == 1 { "" } else { "s" }
        );
        div()
            .absolute()
            .bottom(px(28.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("lens-bar")
                    .w(px(720.))
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_lg()
                    .bg(rgb(theme.chrome))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .shadow_lg()
                    .text_sm()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(self.chip(theme).id("lens-chip").cursor_pointer().on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.dispatch(Action::TogglePalette, cx)
                                }),
                            ))
                            .when(self.palette.is_none(), |row| {
                                row.child(div().flex_1().child(self.message.clone()))
                            })
                            .child(
                                div()
                                    .id("lens-send")
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(theme.accent_fill))
                                    .text_color(rgb(theme.on_fill))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .cursor_pointer()
                                    .child(match self.status {
                                        Status::Unpaired(_)
                                        | Status::Failed(attn::Failure::NotPaired(_)) => "Pair ↩",
                                        Status::Sending => "Sending…",
                                        _ => "Send ↩",
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.dispatch(Action::Send, cx)
                                    })),
                            ),
                    )
                    .children(line.map(|(text, color)| {
                        div().text_color(rgb(color)).whitespace_normal().child(text)
                    }))
                    .children(
                        self.note
                            .clone()
                            .map(|text| div().text_color(rgb(theme.secondary)).child(text)),
                    )
                    .child(div().text_xs().text_color(rgb(theme.muted)).child(hints)),
            )
    }

    fn palette_view(
        &self,
        selected: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rows = self.rows();
        let mut host: Option<Option<String>> = None;
        let mut list = div()
            .id("lens-palette-list")
            .flex()
            .flex_col()
            .max_h(px(392.))
            .overflow_y_scroll()
            .py_1();
        for (i, row) in rows.iter().enumerate() {
            let heading: Option<SharedString> = match row {
                Row::Session(s)
                    if i == 0
                        && self.capture.target.as_deref()
                            == Some(self.sessions[*s].id.as_str()) =>
                {
                    host = Some(None);
                    Some("Sending to".into())
                }
                Row::Session(s) => {
                    let h = self.sessions[*s].host.clone();
                    (host.as_ref() != Some(&h) || host == Some(None) && i == 1).then(|| {
                        host = Some(h.clone());
                        h.unwrap_or_else(|| "This Mac".into()).into()
                    })
                }
                Row::Copy => Some("Capture".into()),
                _ => None,
            };
            if let Some(heading) = heading {
                list = list.child(
                    div()
                        .px_3()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(rgb(theme.muted))
                        .child(heading),
                );
            }
            let (title, detail): (SharedString, SharedString) = match row {
                Row::Session(s) => {
                    let s = &self.sessions[*s];
                    (
                        s.label.clone().into(),
                        format!("{} · {} · {}", s.agent, s.state, s.host_label()).into(),
                    )
                }
                Row::Copy => ("Copy image".into(), "⌘C".into()),
                Row::Save => ("Save PNG…".into(), "⌘S".into()),
                Row::Edit => (
                    "Edit or export…".into(),
                    "⌘E · crop, pixelate, backdrop, GIF/MP4".into(),
                ),
            };
            let row = *row;
            list = list.child(
                div()
                    .id(("lens-row", i))
                    .mx_1()
                    .px_2()
                    .py_1()
                    .flex()
                    .gap_3()
                    .rounded_md()
                    .when(i == selected, |d| d.bg(rgb(theme.pressed)))
                    .hover(|d| d.bg(rgb(theme.hover)))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(rgb(theme.text))
                            .child(title),
                    )
                    .child(div().text_xs().text_color(rgb(theme.muted)).child(detail))
                    .on_click(cx.listener(move |this, _, _, cx| this.run_row(row, cx))),
            );
        }
        if self.sessions.is_empty() {
            let text = match &self.status {
                Status::Ready => {
                    "No live sessions in attn. Start one there, or copy or save the capture."
                }
                Status::Loading => "Loading sessions…",
                Status::Unlisted(text) => text,
                _ => "Pair with attn to list its sessions.",
            };
            list = list.child(
                div()
                    .p_3()
                    .text_color(rgb(theme.muted))
                    .child(text.to_owned()),
            );
        }
        div()
            .absolute()
            .inset_0()
            .bg(rgba(0x03030580))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.palette = None;
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .absolute()
                    .top(px(96.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("lens-palette")
                            .w(px(700.))
                            .flex()
                            .flex_col()
                            .rounded_lg()
                            .bg(rgb(theme.chrome))
                            .border_1()
                            .border_color(rgb(theme.border))
                            .shadow_lg()
                            .text_sm()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .p_2()
                                    .border_b_1()
                                    .border_color(rgb(theme.divider))
                                    .child(self.message.clone()),
                            )
                            .child(list)
                            .child(
                                div()
                                    .px_3()
                                    .py_1()
                                    .border_t_1()
                                    .border_color(rgb(theme.divider))
                                    .text_xs()
                                    .text_color(rgb(theme.muted))
                                    .child("↑↓ move · ↩ choose · Esc back"),
                            ),
                    ),
            )
    }
}
