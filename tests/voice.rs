//! Voice end to end, short of the microphone: a fixture WAV from `speech/tests/fixtures`
//! plays at real speed through the real speech helper, and each segment lands on
//! the clock within 300 ms of when its phrase was spoken.
//!
//! Needs Metal, the helper (`cargo build --release --manifest-path speech/Cargo.toml`)
//! and the models. Opt in with a cache that holds them:
//! `ATTN_LENS_SPEECH_CACHE=… cargo test --test voice -- --ignored`
use attn_lens::{
    lens::{session::HostTime, voice::Speech},
    platform_port::{VoiceEvent, VoiceSource, VoiceStatus},
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

const TOLERANCE_MS: i64 = 300;

fn fixture(name: &str) -> (PathBuf, Value) {
    let dir: PathBuf = [env!("CARGO_MANIFEST_DIR"), "speech/tests/fixtures"]
        .iter()
        .collect();
    let truth = std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap();
    (
        dir.join(format!("{name}.wav")),
        serde_json::from_str(&truth).unwrap(),
    )
}

#[test]
#[ignore = "needs Metal, the speech helper and the models in ATTN_LENS_SPEECH_CACHE"]
fn wav_voice_puts_each_phrase_on_the_clock_and_finish_keeps_the_last() {
    assert!(
        std::env::var_os("ATTN_LENS_SPEECH_CACHE").is_some(),
        "ATTN_LENS_SPEECH_CACHE names a cache that holds the speech models"
    );
    let (wav, truth) = fixture("no-pause-before-finish");
    let origin = Instant::now();
    let clock = Arc::new(move || HostTime(origin.elapsed().as_nanos() as u64));
    let (events, received) = mpsc::channel();
    let events = Mutex::new(events);
    let speech = Arc::new(Speech::default());
    let voice = speech.start(
        VoiceSource::Wav(wav),
        clock.clone(),
        Arc::new(move |event| events.lock().unwrap().send(event).unwrap()),
    );

    // Audio starts once the model is ready; the clock reads then.
    let started = loop {
        match received.recv_timeout(Duration::from_secs(120)).unwrap() {
            VoiceEvent::Status(VoiceStatus::Listening) => break clock(),
            VoiceEvent::Failed(error) => panic!("{error}"),
            _ => {}
        }
    };
    // Finish right as the last phrase ends: no pause closes it first.
    let audio = Duration::from_millis(truth["audio_ms"].as_u64().unwrap() + 100);
    std::thread::sleep(audio.saturating_sub(clock().since(started)));
    voice.finish().unwrap();

    let segments: Vec<_> = received
        .try_iter()
        .filter_map(|event| match event {
            VoiceEvent::Segment(segment) => Some(segment),
            VoiceEvent::Failed(error) => panic!("{error}"),
            VoiceEvent::Status(_) => None,
        })
        .collect();
    let phrases = truth["phrases"].as_array().unwrap();
    assert_eq!(segments.len(), phrases.len(), "{segments:?}");
    let ms = |t: HostTime| t.0 as i64 / 1_000_000;
    for (segment, phrase) in segments.iter().zip(phrases) {
        // The WAV's first sample played when the first chunk was read: `started`
        // is up to one 100 ms chunk after it.
        let start = ms(segment.start) - phrase["start_ms"].as_i64().unwrap();
        let end = ms(segment.end) - phrase["end_ms"].as_i64().unwrap();
        let begun = ms(started);
        eprintln!(
            "{:?}: start {:+} ms, end {:+} ms from the clock at Listening",
            segment.text,
            start - begun,
            end - begun
        );
        assert!((start - begun).abs() <= TOLERANCE_MS, "{segment:?}");
        assert!((end - begun).abs() <= TOLERANCE_MS, "{segment:?}");
    }
    assert!(
        segments
            .last()
            .unwrap()
            .text
            .to_lowercase()
            .contains("this error shows up"),
        "{segments:?}"
    );
    speech.release();
}

/// The first voice can leave the microphone prompt up; Send then finishes a voice
/// whose source is still opening. A FIFO holds the WAV source open the same way.
#[test]
#[ignore = "needs Metal, the speech helper and the models in ATTN_LENS_SPEECH_CACHE"]
fn finish_while_the_source_is_still_opening_returns_and_stops() {
    assert!(std::env::var_os("ATTN_LENS_SPEECH_CACHE").is_some());
    let (wav, _) = fixture("short-phrases");
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("voice.wav");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    let origin = Instant::now();
    let (events, received) = mpsc::channel();
    let events = Mutex::new(events);
    let speech = Arc::new(Speech::default());
    let voice = speech.start(
        VoiceSource::Wav(fifo.clone()),
        Arc::new(move || HostTime(origin.elapsed().as_nanos() as u64)),
        Arc::new(move |event| events.lock().unwrap().send(event).unwrap()),
    );
    // Warming up is reported just before the source opens.
    loop {
        match received.recv_timeout(Duration::from_secs(120)).unwrap() {
            VoiceEvent::Status(VoiceStatus::WarmingUp) => break,
            VoiceEvent::Failed(error) => panic!("{error}"),
            _ => {}
        }
    }
    let (done, finished) = mpsc::channel();
    let finishing = std::thread::spawn(move || done.send(voice.finish()).unwrap());
    // finish waits for the source; the "user" answers the prompt a moment later.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        finished.try_recv().is_err(),
        "finish waits while the source opens"
    );
    std::fs::write(&fifo, std::fs::read(wav).unwrap()).unwrap();
    finished
        .recv_timeout(Duration::from_secs(30))
        .expect("finish returns once the source opened")
        .unwrap();
    finishing.join().unwrap();
    // Voice is over: no more events once it has stopped.
    std::thread::sleep(Duration::from_millis(500));
    let late: Vec<_> = received
        .try_iter()
        .filter(|event| matches!(event, VoiceEvent::Segment(_) | VoiceEvent::Failed(_)))
        .collect();
    assert!(late.is_empty(), "{late:?}");
    speech.release();
}

#[test]
#[ignore = "needs Metal, the speech helper and the models in ATTN_LENS_SPEECH_CACHE"]
fn finish_reports_a_source_that_fails_to_open() {
    assert!(std::env::var_os("ATTN_LENS_SPEECH_CACHE").is_some());
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("voice.wav");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    let origin = Instant::now();
    let (events, received) = mpsc::channel();
    let events = Mutex::new(events);
    let speech = Arc::new(Speech::default());
    let voice = speech.start(
        VoiceSource::Wav(fifo.clone()),
        Arc::new(move || HostTime(origin.elapsed().as_nanos() as u64)),
        Arc::new(move |event| events.lock().unwrap().send(event).unwrap()),
    );
    loop {
        match received.recv_timeout(Duration::from_secs(120)).unwrap() {
            VoiceEvent::Status(VoiceStatus::WarmingUp) => break,
            VoiceEvent::Failed(error) => panic!("{error}"),
            _ => {}
        }
    }
    let finishing = std::thread::spawn(move || voice.finish());
    // The source is still opening, as the microphone is while its prompt shows;
    // bytes that are no WAV make it fail, as a denied prompt does.
    std::fs::write(&fifo, b"not a wav file").unwrap();
    let outcome = finishing.join().unwrap();
    assert!(outcome.is_err(), "{outcome:?}");
    speech.release();
}
