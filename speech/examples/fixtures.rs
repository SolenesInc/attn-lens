#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)] // Sample arithmetic on short synthetic clips.

//! Writes the speech fixtures in `tests/fixtures`: phrases spoken by macOS `say`,
//! placed between silences at known times, with a faint noise floor. Each WAV gets
//! a JSON file with the true start and end of every phrase.
//!
//! Run on a Mac: `cargo run --example fixtures`.
use std::path::Path;
use std::process::Command;

const RATE: u32 = 16_000;
/// A sample counts as speech above this level (about -40 dBFS). `say` leaves
/// digital silence around a phrase, so its quietest consonants clear it.
const SPEECH_LEVEL: f32 = 0.01;
/// The noise floor between phrases, about -60 dBFS.
const NOISE: f32 = 0.001;

enum Part {
    Silence(f32),
    Say(&'static str),
}

const FIXTURES: [(&str, &[Part]); 4] = [
    (
        "short-phrases",
        &[
            Part::Silence(1.0),
            Part::Say("Open the settings."),
            Part::Silence(1.2),
            Part::Say("Click save."),
            Part::Silence(1.2),
            Part::Say("It fails again."),
            Part::Silence(1.5),
        ],
    ),
    (
        "long-lead-in",
        &[
            Part::Silence(9.0),
            Part::Say("This button does nothing when I click it."),
            Part::Silence(1.5),
        ],
    ),
    (
        "hesitation",
        &[
            Part::Silence(1.0),
            Part::Say("When I click save, [[slnc 350]] nothing happens."),
            Part::Silence(1.5),
        ],
    ),
    (
        "no-pause-before-finish",
        &[
            Part::Silence(1.0),
            Part::Say("The page loads."),
            Part::Silence(1.0),
            Part::Say("And then this error shows up"),
        ],
    ),
];

fn main() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let scratch = tempfile::tempdir().unwrap();
    let mut seed = 0x2545_f491_u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * NOISE
    };
    for (name, parts) in FIXTURES {
        let mut samples: Vec<f32> = Vec::new();
        let mut phrases = Vec::new();
        for part in parts {
            match part {
                Part::Silence(seconds) => {
                    samples.extend((0..(seconds * RATE as f32) as usize).map(|_| noise()));
                }
                Part::Say(text) => {
                    let spoken = say(scratch.path(), text);
                    let first = spoken.iter().position(|s| s.abs() > SPEECH_LEVEL).unwrap();
                    let last = spoken.iter().rposition(|s| s.abs() > SPEECH_LEVEL).unwrap();
                    let start = samples.len();
                    samples.extend(spoken[first..=last].iter().map(|s| s + noise()));
                    phrases.push(serde_json::json!({
                        "text": text.replace("[[slnc 350]] ", ""),
                        "start_ms": start as u64 * 1000 / u64::from(RATE),
                        "end_ms": samples.len() as u64 * 1000 / u64::from(RATE),
                    }));
                }
            }
        }
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut wav = hound::WavWriter::create(out.join(format!("{name}.wav")), spec).unwrap();
        for sample in &samples {
            wav.write_sample((sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
                .unwrap();
        }
        wav.finalize().unwrap();
        let truth = serde_json::json!({
            "audio_ms": samples.len() as u64 * 1000 / u64::from(RATE),
            "phrases": phrases,
        });
        std::fs::write(
            out.join(format!("{name}.json")),
            serde_json::to_string_pretty(&truth).unwrap() + "\n",
        )
        .unwrap();
        println!("{name}: {truth}");
    }
}

fn say(scratch: &Path, text: &str) -> Vec<f32> {
    let path = scratch.join("phrase.wav");
    let status = Command::new("say")
        .args([
            "-v",
            "Samantha",
            "--file-format=WAVE",
            "--data-format=LEI16@16000",
            "-o",
        ])
        .arg(&path)
        .arg(text)
        .status()
        .unwrap();
    assert!(status.success(), "say failed for {text:?}");
    hound::WavReader::open(&path)
        .unwrap()
        .into_samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / f32::from(i16::MAX))
        .collect()
}
