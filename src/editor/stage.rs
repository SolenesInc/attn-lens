//! What frames the image on the canvas: the backdrop behind it while editing, and
//! whole frames while the Animation panel plays an entrance. Frames render one at a
//! time on the port's lane, and the newest request wins, as the raster in
//! `preview.rs` does. A frame stays shown until the next one lands, so playback
//! never flashes empty.
use super::{Editor, Message, Panel};
use crate::{animation::Renderer, backdrop::Backdrop};
use image::RgbaImage;
use rune::Pixels;
use std::sync::Arc;

/// The longest side of a backdrop frame, and of an entrance frame while it plays:
/// Glance's preview size, and the GIF export's. A backdrop is a gradient or a soft
/// moving field, so the canvas scales it up without visible loss.
pub(crate) const PLAYING_EDGE: u32 = 960;
/// The longest side of an entrance frame while paused: the MP4 export's, so a frame
/// being inspected shows what the video holds.
pub(crate) const PAUSED_EDGE: u32 = 1920;
/// Frames per second of preview time, as in MP4 export.
const FPS: u32 = 30;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Scene {
    /// The background and shadow; the canvas draws the image over them.
    Backdrop {
        backdrop: Backdrop,
        source: (u32, u32),
    },
    /// The whole picture, image and marks included, of document `revision`.
    Entrance { revision: u64, edge: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameKey {
    pub scene: Scene,
    pub tick: u32,
}

pub(crate) struct Frame {
    key: FrameKey,
    image: RgbaImage,
    renderer: Arc<Renderer>,
}

pub(crate) struct Stage {
    pub pixels: Pixels,
    pub shown: Option<FrameKey>,
    pending: Option<FrameKey>,
    renderer: Option<(Scene, Arc<Renderer>)>,
    lane: crate::platform_port::Lane,
}

impl Stage {
    pub fn new(lane: crate::platform_port::Lane) -> Self {
        Self {
            pixels: Pixels::new(),
            shown: None,
            pending: None,
            renderer: None,
            lane,
        }
    }
}

/// Preview time. It advances only while the window is on screen, playing, and showing
/// the frame it asks for, so playback waits for a slow frame instead of skipping past it.
pub(crate) struct Playback {
    /// Seconds into the cycle or the clip.
    pub position: f32,
    pub playing: bool,
    /// The clock at the last poll while visible; None after the window hid, so time
    /// spent hidden does not count.
    last: Option<f64>,
}

/// A moving backdrop plays from the start.
impl Default for Playback {
    fn default() -> Self {
        Self {
            position: 0.,
            playing: true,
            last: None,
        }
    }
}

impl Editor {
    /// What the canvas shows around or instead of the image now.
    pub(crate) fn scene(&self) -> Option<Scene> {
        let doc = self.doc.as_ref()?;
        if self.panel == Panel::Animation && doc.image_animation.enabled() {
            return Some(Scene::Entrance {
                revision: self.revision,
                edge: if self.playback.playing {
                    PLAYING_EDGE
                } else {
                    PAUSED_EDGE
                },
            });
        }
        doc.backdrop.map(|backdrop| Scene::Backdrop {
            backdrop,
            source: doc.base.dimensions(),
        })
    }

    /// The backdrop the picture is laid out in: the document's, or the plain one an
    /// entrance exports with.
    pub(crate) fn frame_backdrop(&self) -> Option<Backdrop> {
        let doc = self.doc.as_ref()?;
        match self.scene() {
            Some(Scene::Entrance { .. }) => Some(doc.animation_backdrop()),
            _ => doc.backdrop,
        }
    }

    /// Whether preview time moves: an entrance being inspected, or a moving backdrop.
    pub(crate) fn animates(&self) -> bool {
        match self.scene() {
            Some(Scene::Entrance { .. }) => true,
            Some(Scene::Backdrop { backdrop, .. }) => {
                backdrop.motion != crate::animation::Motion::Still
            }
            None => false,
        }
    }

    pub(crate) fn clip_seconds(&self) -> f32 {
        self.doc.as_ref().map_or(5, |d| d.animation_seconds()) as f32
    }

    /// Where preview time sits in the cycle, 0 to 1. An entrance clip stops at its end;
    /// a backdrop loops.
    pub(crate) fn phase(&self) -> f32 {
        let seconds = self.clip_seconds();
        match self.scene() {
            Some(Scene::Entrance { .. }) => (self.playback.position / seconds).clamp(0., 1.),
            _ => (self.playback.position / seconds).rem_euclid(1.),
        }
    }

    /// The entrance and backdrop phases a PNG or copy captures: the frame being
    /// inspected in the Animation panel, otherwise the image fully revealed over the
    /// backdrop frame on the canvas.
    pub(crate) fn export_phase(&self) -> (f32, f32) {
        let phase = self.phase();
        let Some(doc) = &self.doc else {
            return (phase, phase);
        };
        let a = doc.image_animation;
        if !a.enabled() || self.panel == Panel::Animation {
            (phase, phase)
        } else {
            let revealed = (a.delay_ms + a.duration_ms) as f32 / 1000. / a.seconds as f32;
            (revealed, phase)
        }
    }

    fn wanted_frame(&self) -> Option<FrameKey> {
        let scene = self.scene()?;
        let tick = if self.animates() {
            let frames = self.clip_seconds() as u32 * FPS;
            ((self.phase() * frames as f32).floor() as u32).min(frames)
        } else {
            0
        };
        Some(FrameKey { scene, tick })
    }

    /// True while the frame preview time asks for is not on screen yet.
    pub(crate) fn preparing(&self) -> bool {
        self.wanted_frame()
            .is_some_and(|key| self.stage.shown != Some(key))
    }

    /// Advances preview time from the clock, then asks for the frame it now needs.
    /// Returns how long until the next frame is due. A closed editor is only hidden and
    /// Rune still polls it, so while closed it neither advances, renders nor asks to be
    /// woken.
    pub(crate) fn advance(&mut self, now: f64) -> Option<std::time::Duration> {
        if !self.open {
            self.playback.last = None;
            return None;
        }
        let elapsed = self
            .playback
            .last
            .map_or(0., |last| (now - last).max(0.) as f32);
        self.playback.last = Some(now);
        let mut next = None;
        if self.playback.playing && self.animates() {
            if !self.preparing() {
                self.playback.position += elapsed;
            }
            let seconds = self.clip_seconds();
            if matches!(self.scene(), Some(Scene::Entrance { .. })) {
                if self.playback.position >= seconds {
                    self.playback.position = seconds;
                    self.playback.playing = false;
                }
            } else {
                self.playback.position = self.playback.position.rem_euclid(seconds);
            }
            if self.playback.playing {
                next = Some(std::time::Duration::from_secs_f64(1. / FPS as f64));
            }
        }
        self.request_frame();
        next
    }

    /// Starts the clip from the beginning.
    pub(crate) fn replay(&mut self) {
        self.playback.position = 0.;
        self.playback.playing = true;
        self.playback.last = Some(self.now);
    }

    pub(crate) fn toggle_playback(&mut self) {
        let finished = matches!(self.scene(), Some(Scene::Entrance { .. }))
            && self.playback.position >= self.clip_seconds();
        if finished {
            self.replay();
        } else {
            self.playback.playing = !self.playback.playing;
            self.playback.last = Some(self.now);
        }
    }

    /// Asks for the frame preview time needs, unless it is shown or on its way. Only
    /// one renders at a time; the next request goes out when it lands.
    pub(crate) fn request_frame(&mut self) {
        let Some(key) = self.wanted_frame() else {
            self.stage.renderer = None;
            self.stage.shown = None;
            return;
        };
        if self.stage.shown == Some(key) || self.stage.pending.is_some() {
            return;
        }
        self.stage.pending = Some(key);
        let cached = self
            .stage
            .renderer
            .as_ref()
            .filter(|(scene, _)| *scene == key.scene)
            .map(|(_, renderer)| renderer.clone());
        let source = match (cached.is_some(), key.scene) {
            (false, Scene::Entrance { .. }) => Some(self.doc().render_snapshot()),
            _ => None,
        };
        let frames = self.clip_seconds() as u32 * FPS;
        let mailbox = self.mailbox.clone();
        self.stage.lane.run(Box::new(move || {
            let renderer = cached.unwrap_or_else(|| {
                Arc::new(match (key.scene, source) {
                    (Scene::Backdrop { backdrop, source }, _) => {
                        Renderer::backdrop(source, backdrop, Some(PLAYING_EDGE))
                    }
                    (Scene::Entrance { edge, .. }, Some(document)) => {
                        Renderer::for_document(&document, Some(edge))
                    }
                    (Scene::Entrance { .. }, None) => unreachable!("an entrance has its source"),
                })
            });
            let image = renderer.frame(key.tick as f32 / frames.max(1) as f32);
            mailbox.post(Message::Frame(Box::new(Frame {
                key,
                image,
                renderer,
            })));
        }));
    }

    /// Shows a frame of the scene still on screen; a frame of an older one is dropped.
    pub(crate) fn receive_frame(&mut self, frame: Frame) {
        self.stage.pending = None;
        let current = self.scene() == Some(frame.key.scene);
        if current {
            let (width, height) = frame.image.dimensions();
            self.stage.pixels.set(width, height, frame.image.into_raw());
            self.stage.shown = Some(frame.key);
            self.stage.renderer = Some((frame.key.scene, frame.renderer));
        }
        self.request_frame();
    }

    /// The picture's rect in canvas points: the backdrop's, or the whole frame's.
    pub(crate) fn stage_rect(&self) -> Option<[f32; 4]> {
        let b = self.frame_backdrop()?;
        let doc = self.doc.as_ref()?;
        let source = doc.base.dimensions();
        let frame = b.layout(source);
        let l = self.layout();
        Some([
            l.x - frame.origin.0 as f32 * l.scale,
            l.y - frame.origin.1 as f32 * l.scale,
            frame.dimensions.0 as f32 * l.scale,
            frame.dimensions.1 as f32 * l.scale,
        ])
    }
}
