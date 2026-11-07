//! The single entry point for editor actions. Triggers only construct data.
use super::{CopyFeedback, Editor, Message, NumberDraft, Operation, Panel, actions::Action};
use crate::{
    document::{Document, Tool, actions::DocumentAction, actions::Selection},
    style::Style,
};
use rune::{FileDialogOptions, Tx};
use std::time::Duration;

/// How long a Copied! confirmation stays in the toolbar.
const FEEDBACK: Duration = Duration::from_secs(2);

impl Editor {
    /// For triggers with nowhere else to report: a refusal lands in the status line.
    pub(crate) fn dispatch_ui(&mut self, action: Action, tx: &mut Tx) {
        self.commit_number(tx);
        if let Err(error) = self.dispatch(action, tx) {
            self.status = error;
        }
    }

    pub(crate) fn dispatch(&mut self, action: Action, tx: &mut Tx) -> Result<(), String> {
        validate(&action)?;
        let needs_image = !matches!(
            action,
            Action::OpenImage
                | Action::OpenPath(_)
                | Action::PasteImage
                | Action::Cancel
                | Action::Help
                | Action::Quit
                | Action::SelectTool(_)
                | Action::SetColor(_)
                | Action::SetRgb(_)
                | Action::SetStrokeWidth(_)
                | Action::SetAppearance(_)
                | Action::SetMagnifierZoom(_)
                | Action::SetCounterNumber(_)
                | Action::SetCropRatio(_)
                | Action::ToggleImageTools
                | Action::ToggleColorPicker
                | Action::PickScreenColor
                | Action::SetResizeScale(_)
                | Action::ToggleSmartResize
        );
        if needs_image && self.doc.is_none() {
            return Err("Open an image first.".into());
        }
        let harmless_while_busy = matches!(
            action,
            Action::Quit
                | Action::Help
                | Action::Cancel
                | Action::Fit
                | Action::ActualSize
                | Action::Zoom(_)
                | Action::ZoomAt { .. }
                | Action::PanBy(_)
        );
        if self.is_busy() && !harmless_while_busy {
            return Err("Editor is busy. Wait for the current operation to finish.".into());
        }
        match action {
            Action::OpenImage => {
                self.commit_text(tx);
                self.status = "Choose a PNG or JPEG…".into();
                let options = FileDialogOptions {
                    title: "Open an image".into(),
                    ..Default::default()
                }
                .filter("Images", &["png", "jpg", "jpeg"]);
                tx.open_file(options, |s: &mut Editor, result, tx| match result {
                    Ok(Some(path)) => s.dispatch_ui(Action::OpenPath(path), tx),
                    Ok(None) => s.status.clear(),
                    Err(error) => s.status = error.to_string(),
                });
            }
            Action::OpenPath(path) => {
                self.commit_text(tx);
                let id = self.start(Operation::Load);
                let mailbox = self.mailbox.clone();
                self.port.run(Box::new(move || {
                    let image = crate::platform::load(&path);
                    mailbox.post(Message::Image { id, image });
                }));
            }
            Action::PasteImage => {
                self.commit_text(tx);
                let id = self.start(Operation::Paste);
                let (mailbox, port) = (self.mailbox.clone(), self.port.clone());
                self.port.run(Box::new(move || {
                    let image = port.clipboard_image();
                    mailbox.post(Message::Image { id, image });
                }));
            }
            Action::SaveImage => {
                self.commit_text(tx);
                let options = FileDialogOptions {
                    title: "Save annotated screenshot".into(),
                    file_name: Some(format!("Screenshot {}.png", timestamp())),
                    directory: crate::platform::pictures_folder(),
                    ..Default::default()
                }
                .filter("PNG", &["png"]);
                self.status = "Choose where to save…".into();
                tx.save_file(options, |s: &mut Editor, result, _| match result {
                    Ok(Some(path)) => s.save_to(path),
                    Ok(None) => s.status.clear(),
                    Err(error) => s.status = error.to_string(),
                });
            }
            Action::CopyImage => {
                self.commit_text(tx);
                let id = self.start(Operation::Copy);
                self.status = "Copying…".into();
                let document = self.doc().render_snapshot();
                let (mailbox, port) = (self.mailbox.clone(), self.port.clone());
                self.port.run(Box::new(move || {
                    let result = port.copy_image(document.export());
                    mailbox.post(Message::Copied { id, result });
                }));
            }
            Action::CopyOcr => {
                self.commit_text(tx);
                self.cancel_gesture();
                let id = self.start(Operation::Ocr);
                self.status = "Reading text locally…".into();
                self.copy_feedback = Some(CopyFeedback::ReadingText);
                let image = self.doc().base.clone();
                let revision = self.revision;
                let (mailbox, port) = (self.mailbox.clone(), self.port.clone());
                self.port.run(Box::new(move || {
                    let text = port.ocr(&image, None);
                    mailbox.post(Message::Recognized { id, revision, text });
                }));
            }
            Action::Undo | Action::Redo => {
                self.commit_text(tx);
                self.cancel_gesture();
                self.edit(
                    if action == Action::Undo {
                        DocumentAction::Undo
                    } else {
                        DocumentAction::Redo
                    },
                    tx,
                );
            }
            Action::Delete => {
                self.cancel_gesture();
                let deletions = self.selected_indices();
                self.edit(
                    DocumentAction::EditAnnotations {
                        updates: vec![],
                        additions: vec![],
                        deletions,
                        remember: true,
                    },
                    tx,
                );
            }
            Action::DuplicateSelection => {
                self.cancel_gesture();
                let additions = self
                    .selected_indices()
                    .into_iter()
                    .map(|index| {
                        let mut mark = self.doc().marks[index].clone();
                        mark.translate(10., 10.);
                        mark
                    })
                    .collect();
                self.edit(
                    DocumentAction::EditAnnotations {
                        updates: vec![],
                        additions,
                        deletions: vec![],
                        remember: true,
                    },
                    tx,
                );
            }
            Action::SelectAll => {
                self.commit_text(tx);
                self.cancel_gesture();
                self.tool = Tool::Select;
                self.set_selection((0..self.doc().marks.len()).collect());
            }
            Action::SelectTool(tool) => {
                self.commit_text(tx);
                self.cancel_gesture();
                self.set_selection(vec![]);
                self.status.clear();
                self.tool = tool;
                self.panel = Panel::Tool;
                self.picker.open = false;
            }
            Action::SetColor(color) => {
                if matches!(
                    self.options_tool(),
                    Tool::Select | Tool::Crop | Tool::Pixelate
                ) {
                    return Err("Select an annotation or a tool with a color option".into());
                }
                self.prepare_option_edit(tx);
                self.defaults[self.options_tool().index()].color = color;
                let remember = !self.picker.continuing;
                self.update_selected(remember, |mark| mark.color = color, tx);
                if !self.picker.continuing {
                    self.picker.sync([color[0], color[1], color[2]]);
                }
            }
            Action::SetRgb([r, g, b]) => {
                let alpha = self.tool_settings().color[3];
                return self.dispatch(Action::SetColor([r, g, b, alpha]), tx);
            }
            Action::SetStrokeWidth(width) => {
                self.prepare_option_edit(tx);
                self.defaults[self.options_tool().index()].width = width;
                self.update_selected(true, |mark| mark.width = width, tx);
            }
            Action::SetAppearance(style) => {
                self.prepare_option_edit(tx);
                self.defaults[self.options_tool().index()].style = style;
                self.update_selected(true, |mark| mark.style = style, tx);
            }
            Action::SetMagnifierZoom(zoom) => {
                if self.options_tool() != Tool::Magnifier {
                    return Err("Choose a magnifier first".into());
                }
                self.cancel_gesture();
                self.defaults[Tool::Magnifier.index()].magnification = zoom;
                self.update_primary(|mark| mark.text = zoom.to_string(), tx);
            }
            Action::SetCounterNumber(number) => {
                if self.options_tool() != Tool::Counter {
                    return Err("Choose a step first".into());
                }
                self.cancel_gesture();
                if self.selected.is_empty() {
                    self.next_counter = Some(number);
                } else {
                    self.update_primary(|mark| mark.text = number.to_string(), tx);
                }
            }
            Action::SetCropRatio(ratio) => self.crop_ratio = ratio,
            Action::AddLinePoint => self.line_points(false, tx)?,
            Action::StraightenLine => self.line_points(true, tx)?,
            Action::NudgeSelection { delta, remember } => {
                self.cancel_gesture();
                self.update_selected(remember, |mark| mark.translate(delta.0, delta.1), tx);
            }
            Action::Fit | Action::ActualSize => {
                self.zoom = (action == Action::ActualSize).then_some(1.);
                self.pan = (0., 0.);
            }
            Action::Zoom(factor) => {
                self.zoom_at(factor, (self.canvas.0 * 0.5, self.canvas.1 * 0.5));
            }
            Action::ZoomAt { factor, anchor } => self.zoom_at(factor, anchor),
            Action::PanBy(delta) => {
                self.pan.0 += delta.0;
                self.pan.1 += delta.1;
            }
            Action::ToggleImageTools => {
                self.commit_text(tx);
                self.panel = match self.panel {
                    Panel::Image => Panel::Tool,
                    Panel::Tool => Panel::Image,
                };
                self.picker.open = false;
            }
            Action::ToggleColorPicker => {
                self.picker.open = !self.picker.open;
                let color = self.tool_settings().color;
                self.picker.sync([color[0], color[1], color[2]]);
            }
            Action::SetResizeScale(scale) => self.resize_scale = scale,
            Action::ToggleSmartResize => self.resize_smart = !self.resize_smart,
            Action::ApplyResize => {
                let (scale, smart) = (self.resize_scale, self.resize_smart);
                self.commit_text(tx);
                self.cancel_gesture();
                self.edit(DocumentAction::Resize { scale, smart }, tx);
            }
            Action::Rotate => {
                self.commit_text(tx);
                self.cancel_gesture();
                self.edit(DocumentAction::Rotate, tx);
            }
            Action::PickScreenColor => {
                self.commit_text(tx);
                let id = self.start(Operation::ColorSample);
                self.status = "Pick a screen color • Escape to cancel".into();
                let mailbox = self.mailbox.clone();
                self.port.pick_color(std::sync::Arc::new(move |color| {
                    mailbox.post(Message::ColorSampled { id, color });
                }));
            }
            Action::Cancel => {
                if self.text.take().is_some() {
                    self.status = "Text canceled".into();
                } else if self.picker.open {
                    self.picker.open = false;
                } else {
                    self.cancel_gesture();
                    self.set_selection(vec![]);
                }
            }
            Action::Help => {
                if let Err(error) =
                    tx.open_url("https://github.com/SolenesInc/attn-lens/blob/main/docs/usage.md")
                {
                    self.status = error.to_string();
                }
            }
            Action::Quit => tx.quit(),
        }
        Ok(())
    }

    fn prepare_option_edit(&mut self, tx: &mut Tx) {
        self.commit_text(tx);
        self.cancel_gesture();
    }

    fn zoom_at(&mut self, factor: f32, anchor: (f32, f32)) {
        if self.gesture.is_active() {
            return;
        }
        let center = (self.canvas.0 * 0.5, self.canvas.1 * 0.5);
        if let Some((zoom, pan)) = crate::navigation::anchored_zoom(
            self.zoom.unwrap_or(self.layout().scale),
            self.pan,
            center,
            anchor,
            factor,
        ) {
            self.zoom = Some(zoom);
            self.pan = pan;
        }
    }

    fn start(&mut self, operation: Operation) -> u64 {
        self.next_operation += 1;
        self.operation = Some((self.next_operation, operation));
        self.next_operation
    }

    fn save_to(&mut self, path: std::path::PathBuf) {
        let path = match crate::platform::checked_export_path(path, "png") {
            Ok(path) => path,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let id = self.start(Operation::Save);
        self.status = "Saving…".into();
        let document = self.doc().render_snapshot();
        let mailbox = self.mailbox.clone();
        self.port.run(Box::new(move || {
            let result = crate::platform::save_png(&document.export(), &path).map(|()| path);
            mailbox.post(Message::Saved { id, result });
        }));
    }

    /// Applies an edit; a refusal goes to the status line. Crop, resize and rotate
    /// work on a copy off the UI thread and land through [`Message::Transformed`].
    pub(crate) fn edit(&mut self, edit: DocumentAction, _tx: &mut Tx) {
        if matches!(
            edit,
            DocumentAction::Crop { .. } | DocumentAction::Resize { .. } | DocumentAction::Rotate
        ) {
            let id = self.start(Operation::Transform);
            self.status = match edit {
                DocumentAction::Crop { .. } => "Cropping…",
                _ => "Working…",
            }
            .into();
            let mut document = self.doc().clone();
            let mailbox = self.mailbox.clone();
            self.port.run(Box::new(move || {
                let document = edit.apply(&mut document).map(|_| Box::new(document));
                mailbox.post(Message::Transformed { id, document });
            }));
            return;
        }
        match edit.apply(self.doc_mut()) {
            Ok(outcome) => {
                match outcome.selection {
                    Selection::Keep => {}
                    Selection::Clear => self.set_selection(vec![]),
                    Selection::Select(index) => self.set_selection(vec![index]),
                    Selection::SelectMany(indices) => self.set_selection(indices),
                }
                if outcome.reset_view {
                    self.zoom = None;
                    self.pan = (0., 0.);
                }
                if outcome.changed {
                    self.changed();
                }
            }
            Err(error) => self.status = error,
        }
    }

    fn update_selected(
        &mut self,
        remember: bool,
        mut update: impl FnMut(&mut crate::document::Mark),
        tx: &mut Tx,
    ) {
        let updates: Vec<_> = self
            .selected_indices()
            .into_iter()
            .map(|index| {
                let mut mark = self.doc().marks[index].clone();
                update(&mut mark);
                (index, mark)
            })
            .collect();
        if updates.is_empty() {
            return;
        }
        self.edit(
            DocumentAction::EditAnnotations {
                updates,
                additions: vec![],
                deletions: vec![],
                remember,
            },
            tx,
        );
    }

    fn update_primary(&mut self, update: impl FnOnce(&mut crate::document::Mark), tx: &mut Tx) {
        let Some(&index) = self.selected.last() else {
            return;
        };
        let mut mark = self.doc().marks[index].clone();
        update(&mut mark);
        self.edit(DocumentAction::UpdateAnnotation { index, mark }, tx);
    }

    fn line_points(&mut self, straighten: bool, tx: &mut Tx) -> Result<(), String> {
        let index = self
            .selected
            .last()
            .copied()
            .filter(|i| {
                self.doc()
                    .marks
                    .get(*i)
                    .is_some_and(|m| m.tool == Tool::Arrow)
            })
            .ok_or("Select a line to edit its points")?;
        self.cancel_gesture();
        let mut mark = self.doc().marks[index].clone();
        let end = *mark.points.last().unwrap();
        if straighten {
            mark.points = vec![mark.points[0], end];
        } else {
            if mark.points.len() >= 32 {
                return Err("Maximum 32 line points".into());
            }
            if mark.curve.is_some() {
                mark.points = vec![mark.points[0], crate::arrow::at(&mark, 0.5), end];
            } else {
                let longest = mark
                    .points
                    .windows(2)
                    .enumerate()
                    .max_by(|(_, a), (_, b)| {
                        (a[1].0 - a[0].0)
                            .hypot(a[1].1 - a[0].1)
                            .total_cmp(&(b[1].0 - b[0].0).hypot(b[1].1 - b[0].1))
                    })
                    .map_or(0, |(i, _)| i);
                let (a, b) = (mark.points[longest], mark.points[longest + 1]);
                mark.points
                    .insert(longest + 1, ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5));
            }
        }
        mark.curve = None;
        self.edit(DocumentAction::UpdateAnnotation { index, mark }, tx);
        Ok(())
    }

    /// Finishes the label being typed. An empty one is discarded.
    pub(crate) fn commit_text(&mut self, tx: &mut Tx) {
        let Some(draft) = self.text.take() else {
            return;
        };
        if self.text_edit.text.trim().is_empty() {
            self.status = "Empty label discarded".into();
            return;
        }
        let mut mark = draft.mark;
        mark.text = self.text_edit.text.replace(['\n', '\r'], " ");
        self.status = "Text label added".into();
        self.edit(DocumentAction::AddAnnotation { mark }, tx);
    }

    /// Applies the number being typed, if it parses within limits.
    pub(crate) fn commit_number(&mut self, tx: &mut Tx) {
        let Some(NumberDraft { label }) = self.number.take() else {
            return;
        };
        let limits = super::panels::number_limits(label);
        match super::panels::parse_number(&self.number_edit.text, limits) {
            Some(value) => {
                let action = self.number_action(label, value);
                self.dispatch_ui(action, tx);
            }
            None => {
                self.status = format!(
                    "{label} must be a number from {} to {}.",
                    limits.0, limits.1
                );
            }
        }
    }

    pub(crate) fn number_action(&self, label: &str, value: f32) -> Action {
        let settings = self.tool_settings();
        match label {
            "Thickness" => Action::SetStrokeWidth(value),
            "Font size" => Action::SetStrokeWidth(value / 7.),
            "Block size" => Action::SetStrokeWidth(value / 4.),
            "Badge diameter" => Action::SetStrokeWidth(value / 7.2),
            "Lens diameter" => Action::SetStrokeWidth(value / 24.),
            "Number" | "Next number" => Action::SetCounterNumber(value.round() as u32),
            "Dim surroundings" => Action::SetAppearance(Style {
                dim: value / 100.,
                ..settings.style
            }),
            "Corner radius" => Action::SetAppearance(Style {
                radius: value,
                ..settings.style
            }),
            "Opacity" | "Intensity" => {
                let mut color = settings.color;
                color[3] = (value / 100. * 255.).round() as u8;
                Action::SetColor(color)
            }
            _ => unreachable!("registered numeric field {label}"),
        }
    }

    /// Takes in what a worker finished.
    pub(crate) fn receive(&mut self, message: Message, tx: &mut Tx) {
        let id = match &message {
            Message::Preview(_) | Message::LensSource(_) => None,
            Message::Image { id, .. }
            | Message::Transformed { id, .. }
            | Message::Saved { id, .. }
            | Message::Copied { id, .. }
            | Message::Recognized { id, .. }
            | Message::ColorSampled { id, .. } => Some(*id),
        };
        if let Some(id) = id {
            if self.operation.is_none_or(|(active, _)| active != id) {
                return;
            }
            self.operation = None;
            self.status.clear();
        }
        match message {
            Message::Preview(rendered) => self.receive_preview(rendered),
            Message::LensSource(image) => {
                if self.gesture.is_active() {
                    self.live_source = Some(image);
                    self.refresh_lens();
                }
            }
            Message::Image { image, .. } => match image {
                Ok(image) => self.load(Document::new(image)),
                Err(error) => self.status = error,
            },
            Message::Transformed { document, .. } => match document {
                Ok(document) => {
                    self.doc = Some(*document);
                    self.set_selection(vec![]);
                    self.zoom = None;
                    self.pan = (0., 0.);
                    self.changed();
                }
                Err(error) => self.status = error,
            },
            Message::Saved { result, .. } => match result {
                Ok(path) => self.status = format!("Saved {}", path.display()),
                Err(error) => self.status = error,
            },
            Message::Copied { result, .. } => match result {
                Ok(()) => {
                    self.status = "Copied to clipboard".into();
                    self.confirm(CopyFeedback::Copied, tx);
                }
                Err(error) => self.status = error,
            },
            Message::Recognized { revision, text, .. } => {
                self.copy_feedback = None;
                if revision != self.revision {
                    self.status = "Image changed during text extraction; extract again".into();
                    return;
                }
                match text {
                    Ok(text) if text.trim().is_empty() => {
                        self.status = "No text found".into();
                        self.confirm(CopyFeedback::NoText, tx);
                    }
                    Ok(text) => match self.port.copy_text(&text) {
                        Ok(()) => {
                            self.status = "Text copied!".into();
                            self.confirm(CopyFeedback::TextCopied, tx);
                        }
                        Err(error) => self.status = error,
                    },
                    Err(error) => self.status = error,
                }
            }
            Message::ColorSampled { color, .. } => match color {
                Ok(Some(rgb)) => {
                    self.dispatch_ui(Action::SetRgb(rgb), tx);
                }
                Ok(None) => {}
                Err(error) => self.status = error,
            },
        }
    }

    /// A new image replaces the document and resets the view.
    pub(crate) fn load(&mut self, document: Document) {
        self.doc = Some(document);
        self.set_selection(vec![]);
        self.gesture = Default::default();
        self.text = None;
        self.next_counter = None;
        self.zoom = None;
        self.pan = (0., 0.);
        self.preview.marks = 0;
        self.changed();
    }

    fn confirm(&mut self, feedback: CopyFeedback, tx: &mut Tx) {
        self.copy_feedback = Some(feedback);
        self.feedback_until = self.now + FEEDBACK.as_secs_f64();
        tx.wake_after(FEEDBACK);
    }
}

/// Rejected here, before any state changes.
fn validate(action: &Action) -> Result<(), String> {
    match action {
        Action::SetStrokeWidth(width) if !width.is_finite() || !(0.5..=64.).contains(width) => {
            Err("Stroke width must be 0.5..64".into())
        }
        Action::SetAppearance(style) => style.validate(),
        Action::SetMagnifierZoom(zoom) if !zoom.is_finite() || !(1.5..=4.).contains(zoom) => {
            Err("Magnification must be 1.5..4".into())
        }
        Action::SetCounterNumber(number) if !(1..=999).contains(number) => {
            Err("Step number must be 1..999".into())
        }
        Action::SetCropRatio(Some(ratio)) if !ratio.is_finite() || !(0.1..=10.).contains(ratio) => {
            Err("Crop ratio must be 0.1..10".into())
        }
        Action::SetResizeScale(scale) if !scale.is_finite() || !(0.1..=4.).contains(scale) => {
            Err("Resize scale must be 0.1..4".into())
        }
        Action::Zoom(factor) | Action::ZoomAt { factor, .. }
            if !factor.is_finite() || *factor <= 0. =>
        {
            Err("Zoom factor must be positive and finite".into())
        }
        _ => Ok(()),
    }
}

/// Local time with filename-safe separators, like macOS screenshots.
fn timestamp() -> String {
    std::process::Command::new("date")
        .arg("+%Y-%m-%d at %H.%M.%S")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|time| time.trim().to_string())
        .unwrap_or_default()
}
