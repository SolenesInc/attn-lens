use std::path::Path;
use std::time::Instant;

use transcribe_cpp::{
    Backend, CancelToken, DeviceType, Model, ModelOptions, RunOptions, Session, SessionOptions,
    TimestampKind, VadSession, VadSessionOptions,
};

use crate::audio;

const SPEECH_PROBABILITY: f32 = 0.4;
const QUIET_PROBABILITY: f32 = SPEECH_PROBABILITY - 0.15;
const MIN_SPEECH_MS: usize = 160;
/// Quiet this long after speech ends a phrase, which becomes one timeline row.
/// The fixtures in `tests/fixtures` split at 1 s gaps and keep a 350 ms
/// hesitation inside its sentence, which a 250 ms pause would split.
const PAUSE_MS: usize = 500;
const EXPECTED_ARCHITECTURE: &str = "parakeet";
const EXPECTED_VARIANT: &str = "unified-en-0.6b";

pub struct Engine {
    model: Model,
    session: Option<Session>,
    session_options: SessionOptions,
    run_options: RunOptions,
    max_chunk_samples: Option<usize>,
    vad: VadSession,
    frame_samples: usize,
}

#[derive(Default)]
pub struct Recording {
    open: Vec<f32>,
    heard: usize,
    voice: Voice,
    texts: Vec<String>,
    segments: usize,
    speech: bool,
    asr_ms: u128,
}

/// The speech in the open segment, by VAD frame since the recording began.
#[derive(Default)]
struct Voice {
    speech_frames: usize,
    quiet_frames: usize,
    /// The first frame of the run that confirmed speech, once one has.
    first: Option<usize>,
    /// The last frame scored as speech.
    last: usize,
}

/// One phrase, from its first speech frame to its last, in milliseconds since
/// the recording's first sample.
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

pub struct Transcript {
    pub text: String,
    pub speech: bool,
    pub audio_ms: u64,
    pub segments: usize,
    pub asr_ms: u128,
}

impl Voice {
    fn observe(&mut self, frame: usize, probability: f32, speech_frames_needed: usize) {
        if probability >= SPEECH_PROBABILITY {
            self.speech_frames += 1;
            if self.speech_frames >= speech_frames_needed {
                self.first.get_or_insert(frame + 1 - self.speech_frames);
            }
            self.last = frame;
        } else {
            self.speech_frames = 0;
        }
        self.quiet_frames = if probability < QUIET_PROBABILITY {
            self.quiet_frames + 1
        } else {
            0
        };
    }
}

impl Engine {
    pub fn load(model_path: &Path, vad_path: &Path) -> Result<Self, String> {
        let model = Model::load_with(
            model_path,
            &ModelOptions {
                backend: Backend::Metal,
                ..Default::default()
            },
        )
        .map_err(|error| format!("could not load {}: {error}", model_path.display()))?;
        let (architecture, variant) = (model.arch(), model.variant());
        if architecture != EXPECTED_ARCHITECTURE || variant != EXPECTED_VARIANT {
            return Err(format!(
                "{} contains {architecture}/{variant}, expected {EXPECTED_ARCHITECTURE}/{EXPECTED_VARIANT}",
                model_path.display()
            ));
        }
        let device = model.device().map_err(|error| error.to_string())?;
        if device.kind != "metal" {
            return Err(format!(
                "the speech model selected {} ({}) instead of Metal",
                device.kind, device.name
            ));
        }
        let capabilities = model.capabilities().map_err(|error| error.to_string())?;
        let session_options = match device.device_type {
            DeviceType::Gpu | DeviceType::Igpu => SessionOptions {
                n_threads: 1,
                ..Default::default()
            },
            _ => SessionOptions::default(),
        };
        let session = model
            .session_with(&session_options)
            .map_err(|error| error.to_string())?;
        let (vad, frame_samples) = Model::load_with(
            vad_path,
            &ModelOptions {
                backend: Backend::Cpu,
                ..Default::default()
            },
        )
        .and_then(|vad| {
            let frame_samples = vad.vad_info()?.frame_samples;
            Ok((
                vad.vad_session_with(&VadSessionOptions { n_threads: 1 })?,
                frame_samples,
            ))
        })
        .map_err(|error| format!("could not load {}: {error}", vad_path.display()))?;
        let frame_samples = usize::try_from(frame_samples)
            .ok()
            .filter(|samples| *samples > 0)
            .ok_or_else(|| {
                format!(
                    "{} reports {frame_samples} samples per frame",
                    vad_path.display()
                )
            })?;
        let max_chunk_samples = usize::try_from(capabilities.max_audio_ms)
            .ok()
            .filter(|ms| *ms > 0)
            .map(|ms| ms * audio::SAMPLE_RATE as usize / 1000);
        Ok(Self {
            model,
            session: Some(session),
            session_options,
            run_options: RunOptions {
                timestamps: TimestampKind::None,
                language: Some("en".into()),
                ..Default::default()
            },
            max_chunk_samples,
            vad,
            frame_samples,
        })
    }

    pub fn warm(&mut self) -> Result<(), String> {
        let mut silence = Vec::new();
        audio::pad_for_model(&mut silence);
        self.run(&silence, &CancelToken::new()).map(|_| ())
    }

    pub fn listen(&mut self) -> Recording {
        self.vad.stream_reset();
        Recording::default()
    }

    /// Feeds audio, and returns each phrase that a pause closed.
    pub fn hear(
        &mut self,
        recording: &mut Recording,
        samples: &[f32],
        cancel: &CancelToken,
    ) -> Result<Vec<Segment>, String> {
        let scored = self
            .vad
            .stream_feed(samples)
            .map_err(|error| format!("speech detection failed: {error}"))?;
        recording.open.extend_from_slice(samples);
        recording.heard += samples.len();
        let first_frame = usize::try_from(scored.first_frame).unwrap_or_default();
        let mut closed = Vec::new();
        for (index, probability) in scored.probs.iter().enumerate() {
            let frame = first_frame + index;
            recording
                .voice
                .observe(frame, *probability, self.frames(MIN_SPEECH_MS));
            if recording.voice.quiet_frames < self.frames(PAUSE_MS) {
                continue;
            }
            let dropped = recording.heard - recording.open.len();
            if recording.voice.first.is_none() {
                // Quiet with no phrase yet is let go, keeping half a pause before
                // whatever comes next, so minutes of silence before speech are
                // neither held nor recognized.
                let keep_from = (frame + 1 - self.frames(PAUSE_MS) / 2) * self.frame_samples;
                recording
                    .open
                    .drain(..keep_from.saturating_sub(dropped).min(recording.open.len()));
                continue;
            }
            // The audio is cut in the middle of the pause, so recognition hears
            // the phrase whole; its times come from its speech frames.
            let pause_middle = frame + 1 - recording.voice.quiet_frames / 2;
            let length = pause_middle * self.frame_samples - dropped;
            let rest = recording.open.split_off(length.min(recording.open.len()));
            let audio = std::mem::replace(&mut recording.open, rest);
            closed.extend(self.close_segment(recording, audio, cancel)?);
        }
        Ok(closed)
    }

    /// Closes the phrase in progress, returned with the whole transcript.
    pub fn finish(
        &mut self,
        mut recording: Recording,
        cancel: &CancelToken,
    ) -> Result<(Option<Segment>, Transcript), String> {
        let scored = self
            .vad
            .stream_flush()
            .map_err(|error| format!("speech detection failed: {error}"))?;
        let first_frame = usize::try_from(scored.first_frame).unwrap_or_default();
        for (index, probability) in scored.probs.into_iter().enumerate() {
            recording
                .voice
                .observe(first_frame + index, probability, self.frames(MIN_SPEECH_MS));
        }
        let tail = std::mem::take(&mut recording.open);
        let last = self.close_segment(&mut recording, tail, cancel)?;
        Ok((
            last,
            Transcript {
                text: recording.texts.join(" "),
                speech: recording.speech,
                audio_ms: (recording.heard * 1000 / audio::SAMPLE_RATE as usize) as u64,
                segments: recording.segments,
                asr_ms: recording.asr_ms,
            },
        ))
    }

    pub fn stop_listening(&mut self) {
        self.vad.stream_reset();
    }

    fn frames(&self, ms: usize) -> usize {
        (ms * audio::SAMPLE_RATE as usize / 1000).div_ceil(self.frame_samples)
    }

    fn frame_ms(&self, frame: usize) -> u64 {
        (frame * self.frame_samples * 1000 / audio::SAMPLE_RATE as usize) as u64
    }

    fn close_segment(
        &mut self,
        recording: &mut Recording,
        audio: Vec<f32>,
        cancel: &CancelToken,
    ) -> Result<Option<Segment>, String> {
        let voice = std::mem::take(&mut recording.voice);
        let Some(first) = voice.first else {
            return Ok(None);
        };
        let started = Instant::now();
        let text = self.recognize(audio, cancel)?;
        recording.asr_ms += started.elapsed().as_millis();
        recording.segments += 1;
        recording.speech = true;
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        recording.texts.push(text.to_string());
        Ok(Some(Segment {
            start_ms: self.frame_ms(first),
            end_ms: self.frame_ms(voice.last + 1),
            text: text.to_string(),
        }))
    }

    fn recognize(&mut self, mut samples: Vec<f32>, cancel: &CancelToken) -> Result<String, String> {
        audio::pad_for_model(&mut samples);
        let Some(max) = self.max_chunk_samples.filter(|max| samples.len() > *max) else {
            return self.run(&samples, cancel);
        };
        let mut parts = Vec::new();
        for chunk in samples.chunks(max) {
            let mut chunk = chunk.to_vec();
            audio::pad_chunk(&mut chunk);
            let text = self.run(&chunk, cancel)?;
            if !text.trim().is_empty() {
                parts.push(text.trim().to_string());
            }
        }
        Ok(parts.join(" "))
    }

    fn run(&mut self, samples: &[f32], cancel: &CancelToken) -> Result<String, String> {
        let mut session = match self.session.take() {
            Some(session) => session,
            None => self
                .model
                .session_with(&self.session_options)
                .map_err(|error| error.to_string())?,
        };
        session.set_cancel_token(cancel);
        let result = match session.run(samples, &self.run_options) {
            Ok(_) if cancel.is_cancelled() => Err("transcription canceled".into()),
            Ok(transcript) => Ok(transcript.text),
            Err(transcribe_cpp::Error::OutputRepetition {
                partial: Some(partial),
                ..
            }) => Ok(partial.text),
            Err(transcribe_cpp::Error::Aborted { .. }) => Err("transcription canceled".into()),
            Err(error) => Err(format!("transcription failed: {error}")),
        };
        drop(session);
        self.session = self.model.session_with(&self.session_options).ok();
        result
    }
}
