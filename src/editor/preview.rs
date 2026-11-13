//! The raster under the canvas: rendered off the UI thread, newest revision wins.
use super::{Editor, Message};
use crate::document::Document;
use image::RgbaImage;
use rune::Pixels;
use std::sync::Arc;

/// The longest side the view holds. Rune's `Pixels` refuses anything past this, so a
/// larger image shows downscaled and still exports at full size.
pub(crate) const MAX_VIEW_SIDE: u32 = 8192;

pub(crate) struct Rendered {
    pub revision: u64,
    /// The image under the marks, so a raster of a replaced image is never shown.
    base: Arc<RgbaImage>,
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
    /// The document revision `pixels` shows.
    pub shown: u64,
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
            let mut rendered = render(&document);
            rendered.revision = revision;
            mailbox.post(Message::Preview(rendered));
        }));
    }

    /// Shows any raster of the current image, even an old one, then asks for the next.
    /// The one exception is a raster that has a mark being dragged in it, which is also
    /// drawn live.
    pub(crate) fn receive_preview(&mut self, rendered: Rendered) {
        self.preview.rendering = false;
        let Some(doc) = &self.doc else {
            return;
        };
        let current = rendered.revision == self.revision;
        let same_image = Arc::ptr_eq(&rendered.base, &doc.base);
        let overlaps_drag = self
            .gesture
            .first_drag_index()
            .is_some_and(|index| rendered.marks > index);
        if same_image && !overlaps_drag {
            self.show(rendered);
        }
        if !current {
            self.schedule_preview();
        }
    }

    fn show(&mut self, rendered: Rendered) {
        // The lens only shows while a magnifier is live, and now the raster has it.
        if !self.gesture.is_active() {
            self.live_source = None;
        }
        self.preview.shown = rendered.revision;
        self.preview.marks = rendered.marks;
        self.preview.size = (rendered.width, rendered.height);
        self.preview
            .pixels
            .set(rendered.width, rendered.height, rendered.rgba);
    }

    /// A new image, or a crop, resize or rotation, arrives with its raster, and both
    /// replace what was there at once.
    pub(crate) fn install(&mut self, document: Document, mut rendered: Rendered) {
        self.doc = Some(document);
        self.cancel_gesture();
        self.set_selection(vec![]);
        self.zoom = None;
        self.pan = (0., 0.);
        self.revision += 1;
        rendered.revision = self.revision;
        self.show(rendered);
    }

    /// The document changed: new revision, new raster.
    pub(crate) fn changed(&mut self) {
        self.revision += 1;
        self.schedule_preview();
    }
}

/// Flattens a document for the view, downscaled past [`MAX_VIEW_SIDE`]. Its revision
/// is set by whoever asked.
pub(crate) fn render(document: &Document) -> Rendered {
    let marks = document.marks.len();
    let image = document.render(None);
    // The canvas frames the image as exported: edges extended by the inside padding.
    let image = match document.backdrop {
        Some(b) if b.inside_padding > 0 => b.extend_edges(&image).into_owned(),
        _ => image,
    };
    let longest = image.width().max(image.height());
    let image = if longest > MAX_VIEW_SIDE {
        let fit = |side: u32| (side as u64 * MAX_VIEW_SIDE as u64 / longest as u64).max(1) as u32;
        image::imageops::thumbnail(&image, fit(image.width()), fit(image.height()))
    } else {
        image
    };
    let (width, height) = image.dimensions();
    Rendered {
        revision: 0,
        base: document.base.clone(),
        marks,
        width,
        height,
        rgba: image.into_raw(),
    }
}
