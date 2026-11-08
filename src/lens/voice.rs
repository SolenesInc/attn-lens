//! Voice: the speech helper (`speech/`, protocol 3) as a child process, fed from
//! the microphone or, for driven checks, a WAV file. The helper times each phrase
//! from the recording's first sample; this module puts those times on the capture
//! clock through the host time of that first sample.
//!
//! The helper starts on the first ⌘⌥S of a capture and stays warm, model loaded,
//! until the capture ends ([`Speech::release`]).
#[cfg(target_os = "macos")]
mod mic;

use super::session::{HostTime, Segment};
use crate::platform_port::{Sink, Voice, VoiceEvent, VoiceSource, VoiceStatus};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{ChildStdin, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

/// The helper protocol this build speaks.
const PROTOCOL: u64 = 3;
/// The rate the helper takes: 16 kHz mono f32.
pub const SAMPLE_RATE: u32 = 16_000;
/// Audio per WAV chunk, as the microphone tap delivers it: 100 ms.
const CHUNK_FRAMES: usize = 1_600;

/// The host clock, read when audio starts.
pub type Clock = Arc<dyn Fn() -> HostTime + Send + Sync>;

/// Audio arrives here: samples at [`SAMPLE_RATE`], and with the first buffer the
/// host time of its first sample.
pub type Feed = Box<dyn FnMut(&[f32], Option<HostTime>) + Send>;

/// A running audio source. It stops delivering once `stop` returns.
pub trait Source: Send {
    fn stop(self: Box<Self>);
}

/// The speech helper for this app, started on demand. Each running voice holds
/// the helper too, so it exits once released and the last voice has stopped.
#[derive(Default)]
pub struct Speech {
    helper: Mutex<Option<Arc<Helper>>>,
}

impl Speech {
    /// Starts voice from `source`. Progress, segments and failure reach `sink`
    /// from a thread of its own; this returns at once.
    pub fn start(
        self: &Arc<Self>,
        source: VoiceSource,
        clock: Clock,
        sink: Sink<VoiceEvent>,
    ) -> Voice {
        let run = Arc::new(Run::default());
        let speech = self.clone();
        let thread_run = run.clone();
        std::thread::spawn(move || {
            if let Err(error) = speech.listen(&thread_run, &source, &clock, &sink)
                && !thread_run.stopped()
            {
                sink(VoiceEvent::Failed(error));
            }
            thread_run.set(Phase::Done);
        });
        Voice(Box::new(move || run.finish()))
    }

    /// Lets go of the helper and its model. Its stdin closes, and it exits,
    /// once no voice holds it.
    pub fn release(&self) {
        self.helper.lock().unwrap().take();
    }

    fn helper(&self) -> Result<Arc<Helper>, String> {
        let mut helper = self.helper.lock().unwrap();
        if let Some(running) = helper.as_ref().filter(|h| !h.gone.load(Ordering::SeqCst)) {
            return Ok(running.clone());
        }
        let started = Arc::new(Helper::spawn(&helper_path()?, &cache_dir()?)?);
        *helper = Some(started.clone());
        Ok(started)
    }

    fn listen(
        &self,
        run: &Run,
        source: &VoiceSource,
        clock: &Clock,
        sink: &Sink<VoiceEvent>,
    ) -> Result<(), String> {
        let helper = self.helper()?;
        if !helper.prepared.load(Ordering::SeqCst) {
            let id = helper.id();
            let answers = helper.route(id);
            if !run.advance(Phase::Preparing {
                id,
                helper: helper.clone(),
            }) {
                return Ok(());
            }
            helper.request(&json!({"op": "prepare", "id": id}))?;
            prepare(&answers, sink)?;
            helper.unroute(id);
            helper.prepared.store(true, Ordering::SeqCst);
        }
        if !run.advance(Phase::Opening) {
            return Ok(());
        }
        let id = helper.id();
        let answers = helper.route(id);
        helper.request(&json!({"op": "listen", "id": id}))?;
        sink(VoiceEvent::Status(VoiceStatus::WarmingUp));
        let base: Arc<Mutex<Option<HostTime>>> = Arc::default();
        // Closed by `finish` before it asks for the transcript, so no audio
        // arrives after it, whatever the source has in flight.
        let gate = Arc::new(Mutex::new(true));
        let feed: Feed = {
            let (helper, base, sink, gate) =
                (helper.clone(), base.clone(), sink.clone(), gate.clone());
            Box::new(move |samples, first| {
                let open = gate.lock().unwrap();
                if !*open {
                    return;
                }
                if let Some(first) = first {
                    *base.lock().unwrap() = Some(first);
                    sink(VoiceEvent::Status(VoiceStatus::Listening));
                }
                // A failed write means the helper has gone; its route reports it.
                let _ = helper.audio(id, samples);
            })
        };
        let failed: Sink<String> = {
            let helper = helper.clone();
            Arc::new(move |error| helper.fail(id, &error))
        };
        let opened = open(source, clock, feed, failed);
        let source = match opened {
            Ok(source) => source,
            Err(error) => {
                let _ = helper.request(&json!({"op": "cancel", "target": id}));
                return Err(error);
            }
        };
        run.set(Phase::Listening {
            id,
            helper: helper.clone(),
            source: Some(source),
            gate,
        });
        let result = deliver(&answers, &base, sink);
        helper.unroute(id);
        // Stopped by an error rather than by finish: the source is still open.
        run.stop_source();
        run.done(result.clone());
        result
    }
}

/// Reports a prepare's progress until the model is ready.
fn prepare(answers: &mpsc::Receiver<Value>, sink: &Sink<VoiceEvent>) -> Result<(), String> {
    loop {
        let answer = answers.recv().map_err(|_| gone())?;
        match answer["event"].as_str() {
            Some("progress") => match answer["stage"].as_str() {
                Some("loading") => sink(VoiceEvent::Status(VoiceStatus::Loading)),
                Some("downloading" | "verifying") => {
                    sink(VoiceEvent::Status(VoiceStatus::Downloading {
                        bytes: answer["bytes"].as_u64().unwrap_or(0),
                        total: answer["total"].as_u64().unwrap_or(0),
                    }));
                }
                _ => {}
            },
            Some("ready") => return Ok(()),
            Some("error") => return Err(helper_error(&answer)),
            _ => {}
        }
    }
}

/// Puts each segment on the capture clock until the recording's transcript.
fn deliver(
    answers: &mpsc::Receiver<Value>,
    base: &Mutex<Option<HostTime>>,
    sink: &Sink<VoiceEvent>,
) -> Result<(), String> {
    loop {
        let answer = answers.recv().map_err(|_| gone())?;
        match answer["event"].as_str() {
            Some("segment") => {
                // Segments come only from audio, so the base is known by now.
                let base = base.lock().unwrap().unwrap_or_default();
                let at = |ms: &Value| HostTime(base.0 + ms.as_u64().unwrap_or(0) * 1_000_000);
                sink(VoiceEvent::Segment(Segment {
                    start: at(&answer["start_ms"]),
                    end: at(&answer["end_ms"]),
                    text: answer["text"].as_str().unwrap_or_default().to_string(),
                }));
            }
            Some("transcript") => return Ok(()),
            Some("error") => return Err(helper_error(&answer)),
            _ => {}
        }
    }
}

fn helper_error(answer: &Value) -> String {
    format!(
        "Voice stopped: {}",
        answer["message"]
            .as_str()
            .unwrap_or("the speech helper failed")
    )
}

fn gone() -> String {
    "Voice stopped: the speech helper quit.".into()
}

/// Where one voice has got. `finish` acts on it.
enum Phase {
    Starting,
    Preparing {
        id: u64,
        helper: Arc<Helper>,
    },
    Opening,
    Listening {
        id: u64,
        helper: Arc<Helper>,
        source: Option<Box<dyn Source>>,
        gate: Arc<Mutex<bool>>,
    },
    /// `finish` came before listening began.
    Stopped,
    Done,
}

/// One voice's phase and, once its transcript or error arrived, its outcome,
/// under one lock so `finish` can wait for either.
struct Run {
    state: Mutex<RunState>,
    changed: Condvar,
}

struct RunState {
    phase: Phase,
    outcome: Option<Result<(), String>>,
}

impl Default for Run {
    fn default() -> Self {
        Self {
            state: Mutex::new(RunState {
                phase: Phase::Starting,
                outcome: None,
            }),
            changed: Condvar::new(),
        }
    }
}

impl Run {
    fn set(&self, phase: Phase) {
        self.state.lock().unwrap().phase = phase;
        self.changed.notify_all();
    }

    /// Moves on to `phase` unless `finish` already stopped this voice.
    fn advance(&self, phase: Phase) -> bool {
        let mut state = self.state.lock().unwrap();
        if matches!(state.phase, Phase::Stopped) {
            return false;
        }
        state.phase = phase;
        self.changed.notify_all();
        true
    }

    fn stopped(&self) -> bool {
        matches!(self.state.lock().unwrap().phase, Phase::Stopped)
    }

    fn stop_source(&self) {
        let source = match &mut self.state.lock().unwrap().phase {
            Phase::Listening { source, .. } => source.take(),
            _ => None,
        };
        if let Some(source) = source {
            source.stop();
        }
    }

    fn done(&self, outcome: Result<(), String>) {
        self.state.lock().unwrap().outcome = Some(outcome);
        self.changed.notify_all();
    }

    /// Stops the audio and waits for the last segment.
    fn finish(&self) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        // Opening ends in Listening or with an error; the first time, it waits
        // for the user to answer the microphone prompt.
        while matches!(state.phase, Phase::Opening) {
            state = self.changed.wait(state).unwrap();
        }
        match std::mem::replace(&mut state.phase, Phase::Stopped) {
            Phase::Starting => Ok(()),
            Phase::Preparing { id, helper } => {
                drop(state);
                // The helper keeps what it downloaded, so the next ⌘⌥S resumes.
                helper.request(&json!({"op": "cancel", "target": id}))
            }
            Phase::Listening {
                id,
                helper,
                source,
                gate,
            } => {
                drop(state);
                if let Some(source) = source {
                    source.stop();
                }
                *gate.lock().unwrap() = false;
                helper.request(&json!({"op": "finish", "target": id}))?;
                let mut state = self.state.lock().unwrap();
                loop {
                    if let Some(outcome) = state.outcome.take() {
                        return outcome;
                    }
                    state = self.changed.wait(state).unwrap();
                }
            }
            // Voice failed on its own and reported it already.
            Phase::Opening | Phase::Stopped | Phase::Done => Ok(()),
        }
    }
}

/// The helper process. Its stdin closes when this drops, and it exits.
struct Helper {
    stdin: Mutex<ChildStdin>,
    routes: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>,
    next_id: AtomicU64,
    prepared: AtomicBool,
    gone: Arc<AtomicBool>,
}

impl Helper {
    fn spawn(path: &Path, cache: &Path) -> Result<Self, String> {
        let mut child = Command::new(path)
            .args(["serve", "--cache"])
            .arg(cache)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("The speech helper {} did not start: {e}", path.display()))?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let mut lines = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
        let hello: Value = lines
            .next()
            .and_then(Result::ok)
            .and_then(|line| serde_json::from_str(&line).ok())
            .ok_or("The speech helper quit before it said hello.")?;
        if hello["event"] != "hello" || hello["protocol"] != PROTOCOL {
            return Err(format!(
                "The speech helper speaks protocol {}, and Attn Lens needs {PROTOCOL}.",
                hello["protocol"]
            ));
        }
        let routes: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>> = Arc::default();
        let gone = Arc::new(AtomicBool::new(false));
        {
            let (routes, gone) = (routes.clone(), gone.clone());
            std::thread::spawn(move || {
                for line in lines.map_while(Result::ok) {
                    let Ok(answer) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    let Some(id) = answer["id"].as_u64().or(answer["target"].as_u64()) else {
                        continue;
                    };
                    if let Some(route) = routes.lock().unwrap().get(&id) {
                        let _ = route.send(answer);
                    }
                }
                gone.store(true, Ordering::SeqCst);
                // Every waiting voice hears that the helper has gone.
                routes.lock().unwrap().clear();
                let _ = child.wait();
            });
        }
        Ok(Self {
            stdin: Mutex::new(stdin),
            routes,
            next_id: AtomicU64::new(1),
            prepared: AtomicBool::new(false),
            gone,
        })
    }

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    fn route(&self, id: u64) -> mpsc::Receiver<Value> {
        let (send, receive) = mpsc::channel();
        if !self.gone.load(Ordering::SeqCst) {
            self.routes.lock().unwrap().insert(id, send);
        }
        receive
    }

    fn unroute(&self, id: u64) {
        self.routes.lock().unwrap().remove(&id);
    }

    /// Ends recording `id` with `error`, as if the helper had reported it.
    fn fail(&self, id: u64, error: &str) {
        let _ = self.request(&json!({"op": "cancel", "target": id}));
        if let Some(route) = self.routes.lock().unwrap().get(&id) {
            let _ = route.send(json!({"event": "error", "message": error}));
        }
    }

    fn request(&self, request: &Value) -> Result<(), String> {
        let mut stdin = self.stdin.lock().unwrap();
        writeln!(stdin, "{request}").map_err(|_| gone())
    }

    fn audio(&self, target: u64, samples: &[f32]) -> Result<(), String> {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut stdin = self.stdin.lock().unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"op": "audio", "target": target, "bytes": bytes.len()})
        )
        .and_then(|()| stdin.write_all(&bytes))
        .map_err(|_| gone())
    }
}

/// The helper beside the app's executable, as `scripts/bundle.sh` places it.
/// `ATTN_LENS_SPEECH` names another; a development build also finds
/// `speech/target/release/attn-speech`.
fn helper_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("ATTN_LENS_SPEECH") {
        return Ok(path.into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let beside = exe.with_file_name("attn-speech");
    if beside.exists() {
        return Ok(beside);
    }
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("speech/target/release/attn-speech");
    if cfg!(debug_assertions) && built.exists() {
        return Ok(built);
    }
    Err(format!(
        "The speech helper is missing: expected {}. Build the app with scripts/bundle.sh.",
        beside.display()
    ))
}

/// Where the model downloads (D9 on the plan). `ATTN_LENS_SPEECH_CACHE` names another.
fn cache_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("ATTN_LENS_SPEECH_CACHE") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable")?;
    Ok(PathBuf::from(home).join("Library/Application Support/dev.attn.lens/speech"))
}

fn open(
    source: &VoiceSource,
    clock: &Clock,
    feed: Feed,
    failed: Sink<String>,
) -> Result<Box<dyn Source>, String> {
    match source {
        VoiceSource::Wav(path) => Wav::open(path, clock.clone(), feed),
        #[cfg(target_os = "macos")]
        VoiceSource::Microphone => mic::open(feed, failed),
        #[cfg(not(target_os = "macos"))]
        VoiceSource::Microphone => {
            drop((feed, failed));
            Err("Voice uses the Mac's microphone.".into())
        }
    }
}

/// A WAV file played at real speed, then silence until voice stops, like a
/// microphone in a quiet room.
struct Wav {
    stop: Arc<AtomicBool>,
}

impl Wav {
    fn open(path: &Path, clock: Clock, mut feed: Feed) -> Result<Box<dyn Source>, String> {
        let samples = read_wav(path)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        std::thread::spawn(move || {
            let period = Duration::from_secs_f64(CHUNK_FRAMES as f64 / f64::from(SAMPLE_RATE));
            let silence = vec![0.0; CHUNK_FRAMES];
            let begun = Instant::now();
            let first = clock();
            for index in 0u32.. {
                // Each chunk goes once it has played, as a microphone tap delivers it.
                std::thread::sleep(
                    (begun + period * (index + 1)).saturating_duration_since(Instant::now()),
                );
                if stopping.load(Ordering::SeqCst) {
                    return;
                }
                let start = index as usize * CHUNK_FRAMES;
                let chunk = samples
                    .get(start..(start + CHUNK_FRAMES).min(samples.len()))
                    .filter(|chunk| !chunk.is_empty())
                    .unwrap_or(&silence);
                feed(chunk, (index == 0).then_some(first));
            }
        });
        Ok(Box::new(Self { stop }))
    }
}

impl Source for Wav {
    fn stop(self: Box<Self>) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// The longest WAV voice plays: 10 minutes, past any driven check (the fixtures
/// run 5 to 13 s), at 38 MB of samples.
const MAX_WAV_SECONDS: u32 = 600;

/// Reads a 16 kHz mono WAV of 16-bit or float samples, up to [`MAX_WAV_SECONDS`].
fn read_wav(path: &Path) -> Result<Vec<f32>, String> {
    let reader = hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = reader.spec();
    let limit = (MAX_WAV_SECONDS * SAMPLE_RATE) as usize;
    if reader.duration() as usize > limit {
        return Err(format!(
            "{} runs {} s; voice plays at most {MAX_WAV_SECONDS} s.",
            path.display(),
            reader.duration() / SAMPLE_RATE
        ));
    }
    if spec.sample_rate != SAMPLE_RATE || spec.channels != 1 {
        return Err(format!(
            "{} is {} Hz with {} channels; voice plays 16000 Hz mono.",
            path.display(),
            spec.sample_rate,
            spec.channels
        ));
    }
    let read: Result<Vec<f32>, _> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => reader
            .into_samples::<i16>()
            .take(limit)
            .map(|s| s.map(|s| f32::from(s) / f32::from(i16::MAX)))
            .collect(),
        (hound::SampleFormat::Float, 32) => reader.into_samples::<f32>().take(limit).collect(),
        (format, bits) => {
            return Err(format!(
                "{} holds {bits}-bit {format:?} samples; voice plays 16-bit or 32-bit float.",
                path.display()
            ));
        }
    };
    read.map_err(|e| format!("{}: {e}", path.display()))
}
