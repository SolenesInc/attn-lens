//! GIF and MP4 export: a save dialog, then frames rendered and encoded off the UI
//! thread with progress, until done or cancelled from the progress bar, the Export
//! menu or Escape.
use super::{Editor, Message, Operation, Panel, actions::AnimationFormat};
use crate::{animation::Motion, document::actions::DocumentAction};
use rune::{FileDialogOptions, Tx};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct Export {
    pub id: u64,
    pub format: AnimationFormat,
    pub percent: u32,
    cancel: Arc<AtomicBool>,
}

impl AnimationFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Mp4 => "mp4",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Gif => "GIF",
            Self::Mp4 => "MP4",
        }
    }
}

impl Editor {
    /// Asks where to save, then exports. With nothing that moves, it adds a Flow
    /// backdrop and opens the Backdrop panel instead, so the user sees what will move.
    pub(crate) fn export_animation(&mut self, format: AnimationFormat, tx: &mut Tx) {
        self.commit_text(tx);
        self.cancel_gesture();
        let doc = self.doc();
        let moves = doc.image_animation.enabled()
            || doc.backdrop.is_some_and(|b| b.motion != Motion::Still);
        if !moves {
            let mut b = doc.backdrop.or(self.backdrop_off).unwrap_or_default();
            b.motion = Motion::Flow;
            b.gradient = true;
            self.edit(
                DocumentAction::SetBackdrop {
                    backdrop: Some(b),
                    remember: true,
                },
                tx,
            );
            self.panel = Panel::Backdrop;
            self.status = format!(
                "Added a moving backdrop • export {} again to save it",
                format.label()
            );
            return;
        }
        let (name, directory) = match format {
            AnimationFormat::Gif => ("Animated Screenshot", crate::platform::pictures_folder()),
            AnimationFormat::Mp4 => ("Animated Screenshot", crate::platform::movies_folder()),
        };
        let extension = format.extension();
        let options = FileDialogOptions {
            title: format!("Export {}", format.label()),
            file_name: Some(format!(
                "{name} {}.{extension}",
                super::dispatch::timestamp()
            )),
            directory,
            ..Default::default()
        }
        .filter(format.label(), &[extension]);
        self.status = "Choose where to export…".into();
        tx.save_file(options, move |s: &mut Editor, result, _| match result {
            Ok(Some(path)) => s.start_export(format, path),
            Ok(None) => s.status.clear(),
            Err(error) => s.status = error.to_string(),
        });
    }

    fn start_export(&mut self, format: AnimationFormat, path: std::path::PathBuf) {
        if self.is_busy() {
            self.status = "Editor is busy. Wait for the current operation to finish.".into();
            return;
        }
        let path = match crate::platform::checked_export_path(path, format.extension()) {
            Ok(path) => path,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let encoder = match format {
            AnimationFormat::Mp4 => match self.port.video_encoder() {
                Ok(encoder) => Some(encoder),
                Err(error) => {
                    self.status = error;
                    return;
                }
            },
            AnimationFormat::Gif => None,
        };
        let id = self.start(Operation::Export);
        let cancel = Arc::new(AtomicBool::new(false));
        self.export = Some(Export {
            id,
            format,
            percent: 0,
            cancel: cancel.clone(),
        });
        self.status = format!("Exporting {}…", format.label());
        let document = self.doc().render_snapshot();
        let mailbox = self.mailbox.clone();
        self.port.run(Box::new(move || {
            let progress = |percent| mailbox.post(Message::ExportProgress { id, percent });
            // Exports begin at time zero.
            let result = match encoder {
                Some(encoder) => {
                    crate::video::encode(&document, &encoder, &path, 0., &cancel, progress)
                }
                None => crate::gif_export::encode(&document, &path, 0., &cancel, progress),
            };
            let result = result.map(|finished| finished.then_some(path));
            mailbox.post(Message::Exported { id, result });
        }));
    }

    /// True when an export was running and is now told to stop.
    pub(crate) fn cancel_export(&mut self) -> bool {
        let Some(export) = &self.export else {
            return false;
        };
        export.cancel.store(true, Ordering::Relaxed);
        self.status = "Canceling export…".into();
        true
    }

    pub(crate) fn receive_progress(&mut self, id: u64, percent: u32) {
        if let Some(export) = self.export.as_mut().filter(|e| e.id == id) {
            export.percent = percent;
        }
    }

    pub(crate) fn receive_export(&mut self, result: Result<Option<std::path::PathBuf>, String>) {
        let format = self.export.take().map(|e| e.format.label()).unwrap_or("");
        self.status = match result {
            Ok(Some(path)) => format!("Exported {format} {}", path.display()),
            Ok(None) => "Export canceled".into(),
            Err(error) => error,
        };
    }
}
