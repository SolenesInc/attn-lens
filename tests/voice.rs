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
