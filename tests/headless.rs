//! Every window, driven through Rune's headless tooling against the fake port.
use serde_json::Value;
use std::{path::Path, process::Command};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/headless/foundation.json");

fn tree(dir: &Path, window: &str, mark: &str) -> Value {
    let out = dir.join(format!("{window}-{mark}.json"));
    let backdrop = dir.join("backdrop.png");
    if !backdrop.exists() {
        image::RgbaImage::from_pixel(16, 10, image::Rgba([40, 44, 52, 255]))
            .save(&backdrop)
            .unwrap();
    }
    let run = Command::new(env!("CARGO_BIN_EXE_attn-lens"))
        .arg("--desktop")
        .arg(&backdrop)
        .args(["--script", SCRIPT, "--window", window, "--at"])
        .arg(format!("mark:{mark}"))
        .arg("--tree")
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap()
}

fn visible(tree: &Value, window: &str) -> bool {
    tree["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["key"] == window)
        .unwrap_or_else(|| panic!("no window {window} in {tree}"))["visible"]
        .as_bool()
        .unwrap()
}

fn text<'a>(tree: &'a Value, key: &str) -> Option<&'a str> {
    tree["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["key"] == key)
        .and_then(|e| e["text"].as_str())
}

#[test]
fn the_app_exposes_its_windows() {
    let run = Command::new(env!("CARGO_BIN_EXE_attn-lens"))
        .arg("--list-windows")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "editor\nbar\n");
}

#[test]
fn launch_dock_and_capture_keys_reach_the_windows() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();

    // Lens starts with no window.
    let launched = tree(dir, "bar", "launched");
    assert!(!visible(&launched, "editor"));
    assert!(!visible(&launched, "bar"));

    // A Dock click opens the empty editor (D18).
    let dock = tree(dir, "editor", "dock-clicked");
    assert!(visible(&dock, "editor"));
    assert!(!visible(&dock, "bar"));
    assert!(text(&dock, "editor-empty").is_some());

    // ⌘⌥X opens a capture and takes no still yet.
    let open = tree(dir, "bar", "capture-open");
    assert!(visible(&open, "bar"));
    assert_eq!(text(&open, "input"), Some("input None"));
    assert_eq!(text(&open, "still-0"), None);

    // The draw key flips the input while it is held.
    let drawing = tree(dir, "bar", "drawing");
    assert_eq!(text(&drawing, "input"), Some("input All"));

    // ⌘⌥X during a capture, a click and a window switch each take a still, in
    // order; the draw key is up again; voice segments arrive.
    let stills = tree(dir, "bar", "stills-taken");
    assert_eq!(text(&stills, "input"), Some("input None"));
    assert_eq!(text(&stills, "still-0"), Some("Hotkey 1280×800"));
    assert_eq!(text(&stills, "still-1"), Some("Click 1280×800"));
    assert_eq!(text(&stills, "still-2"), Some("WindowSwitch 1280×800"));
    assert_eq!(text(&stills, "still-3"), None);
    assert_eq!(text(&stills, "voice"), Some("voice on"));
    assert_eq!(text(&stills, "segment-0"), Some("this button does nothing"));

    // ⌘⌥↩ without a session names what is missing and keeps the capture.
    let refused = tree(dir, "bar", "send-refused");
    assert!(visible(&refused, "bar"));
    assert_eq!(
        text(&refused, "error"),
        Some("The capture has no target session.")
    );
    assert!(text(&refused, "still-2").is_some());
}
