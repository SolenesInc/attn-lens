//! Feeds the fixtures in `tests/fixtures` through the real helper and model and
//! checks each phrase's times against when it was really spoken.
//!
//! Needs Metal and the models. Opt in with a cache that holds them, such as one
//! a first ⌘⌥S filled:
//! `ATTN_SPEECH_CACHE=~/Library/Application\ Support/dev.attn.lens/speech cargo test --release -- --ignored`
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// How far a segment's start or end may fall from the true speech (C6).
const TOLERANCE_MS: i64 = 300;
/// One microphone tap's worth of audio per request, as Lens sends it.
const CHUNK: usize = 1_600;

struct Phrase {
    text: String,
    start_ms: i64,
    end_ms: i64,
}

fn fixture(name: &str) -> (Vec<f32>, Vec<Phrase>) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let samples = hound::WavReader::open(dir.join(format!("{name}.wav")))
        .unwrap()
        .into_samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / f32::from(i16::MAX))
        .collect();
    let truth: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap())
            .unwrap();
    let phrases = truth["phrases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| Phrase {
            text: p["text"].as_str().unwrap().into(),
            start_ms: p["start_ms"].as_i64().unwrap(),
            end_ms: p["end_ms"].as_i64().unwrap(),
        })
        .collect();
    (samples, phrases)
}

fn words(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

struct Heard {
    segments: Vec<Value>,
    transcript: Value,
}

/// Runs one recording of `samples` through a fresh helper and finishes it.
fn hear(samples: &[f32]) -> Heard {
    let cache = PathBuf::from(
        std::env::var_os("ATTN_SPEECH_CACHE")
            .expect("ATTN_SPEECH_CACHE names a cache that holds the speech models"),
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_attn-speech"))
        .args(["serve", "--cache"])
        .arg(&cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut next = || serde_json::from_str::<Value>(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(next()["protocol"], 3);
    writeln!(stdin, "{}", json!({"op": "prepare", "id": 1})).unwrap();
    loop {
        let event = next();
        match event["event"].as_str() {
            Some("ready") => break,
            Some("error") => panic!("prepare failed: {event}"),
            _ => {}
        }
    }
    writeln!(stdin, "{}", json!({"op": "listen", "id": 2})).unwrap();
    for chunk in samples.chunks(CHUNK) {
        let bytes: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
        writeln!(
            stdin,
            "{}",
            json!({"op": "audio", "target": 2, "bytes": bytes.len()})
        )
        .unwrap();
        stdin.write_all(&bytes).unwrap();
    }
    writeln!(stdin, "{}", json!({"op": "finish", "target": 2})).unwrap();
    let mut segments = Vec::new();
    let transcript = loop {
        let event = next();
        match event["event"].as_str() {
            Some("segment") => {
                assert_eq!(event["target"], 2);
                segments.push(event);
            }
            Some("transcript") => break event,
            Some("error") => panic!("recording failed: {event}"),
            _ => {}
        }
    };
    drop(stdin);
    child.wait().unwrap();
    Heard {
        segments,
        transcript,
    }
}

fn check(name: &str) -> Heard {
    let (samples, phrases) = fixture(name);
    let heard = hear(&samples);
    eprintln!("{name}:");
    for segment in &heard.segments {
        eprintln!("  {segment}");
    }
    assert_eq!(
        heard.segments.len(),
        phrases.len(),
        "{name}: one segment per phrase"
    );
    for (segment, phrase) in heard.segments.iter().zip(&phrases) {
        let start = segment["start_ms"].as_i64().unwrap();
        let end = segment["end_ms"].as_i64().unwrap();
        eprintln!(
            "  {:?}: start {:+} ms, end {:+} ms",
            phrase.text,
            start - phrase.start_ms,
            end - phrase.end_ms
        );
        assert!(
            (start - phrase.start_ms).abs() <= TOLERANCE_MS,
            "{name}: {:?} starts at {start} ms, spoken at {} ms",
            phrase.text,
            phrase.start_ms
        );
        assert!(
            (end - phrase.end_ms).abs() <= TOLERANCE_MS,
            "{name}: {:?} ends at {end} ms, spoken until {} ms",
            phrase.text,
            phrase.end_ms
        );
        assert_eq!(
            words(segment["text"].as_str().unwrap()),
            words(&phrase.text),
            "{name}"
        );
    }
    heard
}

#[test]
#[ignore = "needs Metal and the speech models in ATTN_SPEECH_CACHE"]
fn phrases_under_three_seconds_are_each_their_own_segment() {
    check("short-phrases");
}

#[test]
#[ignore = "needs Metal and the speech models in ATTN_SPEECH_CACHE"]
fn a_long_silent_lead_in_does_not_move_the_start() {
    check("long-lead-in");
}

#[test]
#[ignore = "needs Metal and the speech models in ATTN_SPEECH_CACHE"]
fn finish_returns_the_sentence_still_being_spoken() {
    let heard = check("no-pause-before-finish");
    assert_eq!(
        words(heard.transcript["text"].as_str().unwrap()),
        "the page loads and then this error shows up"
    );
}

#[test]
#[ignore = "needs Metal and the speech models in ATTN_SPEECH_CACHE"]
fn a_hesitation_inside_a_sentence_keeps_it_one_segment() {
    check("hesitation");
}
