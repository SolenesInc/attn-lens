//! The corner tile a capture shrinks into when it is sent or closed. Clicking
//! it reopens the capture with its marks, message and target.
use super::Capture;
use crate::theme::Theme;
use gpui::{prelude::*, *};
use std::sync::Arc;

pub enum Content {
    Capture(Box<Capture>),
    Notice(SharedString),
}

pub struct Tile {
    content: Content,
    thumbnail: Option<Arc<RenderImage>>,
}

impl Tile {
    /// The capture this tile reopens, if it holds one.
    pub fn capture(&self) -> Option<&Capture> {
        match &self.content {
            Content::Capture(capture) => Some(capture),
            Content::Notice(_) => None,
        }
    }

    pub fn new(content: Content) -> Self {
        let thumbnail = match &content {
            Content::Capture(capture) => Some(crate::editor::render_image(
                image::DynamicImage::ImageRgba8(capture.document.render(None))
                    .thumbnail(240, 140)
                    .to_rgba8(),
            )),
            Content::Notice(_) => None,
        };
        Self { content, thumbnail }
    }
}

impl Render for Tile {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx);
        let (title, detail, ok): (SharedString, SharedString, bool) = match &self.content {
            Content::Capture(capture) => match &capture.sent_to {
                Some(label) => (
                    format!("Sent to {label}").into(),
                    "Click to reopen".into(),
                    true,
                ),
                None => ("Not sent".into(), "Click to reopen".into(), false),
            },
            Content::Notice(text) => ("Attn Lens".into(), text.clone(), false),
        };
        div()
            .id("lens-tile")
            .size_full()
            .p_2()
            .flex()
            .gap_2()
            .rounded_lg()
            .bg(rgb(theme.chrome))
            .border_1()
            .border_color(rgb(match (&self.content, ok) {
                (_, true) => theme.positive,
                (Content::Notice(_), _) => theme.error,
                _ => theme.border,
            }))
            .text_xs()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(capture) = this.capture() {
                    let capture = capture.clone();
                    cx.defer(move |cx| super::open_overlay(capture, cx));
                }
            }))
            .children(
                self.thumbnail
                    .clone()
                    .map(|t| img(t).w(px(108.)).h_full().rounded_md()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(if ok { theme.positive } else { theme.text }))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme.muted))
                            .whitespace_normal()
                            .child(detail),
                    ),
            )
            .child(
                div()
                    .id("lens-tile-close")
                    .text_color(rgb(theme.muted))
                    .child("×")
                    .on_click(cx.listener(|_, _, window, cx| {
                        cx.stop_propagation();
                        window.remove_window();
                    })),
            )
    }
}
