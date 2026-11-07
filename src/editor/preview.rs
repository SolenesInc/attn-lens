//! The raster under the canvas: rendered off the UI thread, newest revision wins.
use super::{Editor, Message};
use rune::Pixels;

/// The longest side the view holds. Rune's `Pixels` refuses anything past this, so a
/// larger image shows downscaled and still exports at full size.
pub(crate) const MAX_VIEW_SIDE: u32 = 8192;

pub(crate) struct Rendered {
    revision: u64,
    marks: usize,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct Preview {
    pub pixels: Pixels,
    /// Marks already in `pixels`. Later ones are drawn live.
    pub marks: usize,
    pub size: (u32, u32),
    rendering: bool,
}

impl Editor {
    /// Rendering is one job at a time; a result for an old revision asks for another.
    pub(crate) fn schedule_preview(&mut self) {
        if self.preview.rendering || self.doc.is_none() {
            return;
        }
        self.preview.rendering = true;
        let mut document = self.doc().render_snapshot();
        if let Some(index) = self.gesture.first_drag_index() {
            document.marks.truncate(index);
        }
        let revision = self.revision;
        let mailbox = self.mailbox.clone();
        self.port.run(Box::new(move || {
            let marks = document.marks.len();
            let image = document.render(None);
            let image = if image.width().max(image.height()) > MAX_VIEW_SIDE {
                image::imageops::thumbnail(
                    &image,
                    (image.width() as u64 * MAX_VIEW_SIDE as u64
                        / image.width().max(image.height()) as u64)
                        .max(1) as u32,
                    (image.height() as u64 * MAX_VIEW_SIDE as u64
                        / image.width().max(image.height()) as u64)
                        .max(1) as u32,
                )
            } else {
                image
            };
            let (width, height) = image.dimensions();
            mailbox.post(Message::Preview(Rendered {
                revision,
                marks,
                width,
                height,
                rgba: image.into_raw(),
            }));
        }));
    }

    pub(crate) fn receive_preview(&mut self, rendered: Rendered) {
        self.preview.rendering = false;
        if rendered.revision != self.revision {
            self.schedule_preview();
            return;
        }
        // The lens only shows while a magnifier is live, and now the raster has it.
        if !self.gesture.is_active() {
            self.live_source = None;
        }
        self.preview.marks = rendered.marks;
        self.preview.size = (rendered.width, rendered.height);
        self.preview
            .pixels
            .set(rendered.width, rendered.height, rendered.rgba);
    }

    /// The document changed: new revision, new raster.
    pub(crate) fn changed(&mut self) {
        self.revision += 1;
        self.schedule_preview();
    }
}
