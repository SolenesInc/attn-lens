//! Every window, driven through Rune's headless tooling against the fake port and
//! a stand-in `attn`. Trees are read with `--window KEY --tree out.json --at mark:NAME`.
use serde_json::Value;
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

/// A scratch folder with a stand-in `attn` that lists `sessions`, and records what
/// it is sent: its arguments in `args` and each PNG as it was then.
struct Run {
    dir: tempfile::TempDir,
    script: &'static str,
}

impl Run {
    fn new(script: &'static str, sessions: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(
            path.join("attn"),
            format!(
                r#"#!/bin/sh
cat > /dev/null
if [ "$2" = "sessions" ]; then echo '{sessions}'; exit 0; fi
echo "$@" > "{dir}/args"
for arg in "$@"; do case "$arg" in *.png) cp "$arg" "{dir}/";; esac; done
echo '{{"id":"cap"}}'
"#,
                dir = path.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(path.join("attn"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        image::RgbaImage::from_pixel(16, 10, image::Rgba([40, 44, 52, 255]))
            .save(path.join("backdrop.png"))
            .unwrap();
        Self { dir, script }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn command(&self, mark: &str) -> Command {
        let script: PathBuf = [env!("CARGO_MANIFEST_DIR"), "headless", self.script]
            .iter()
            .collect();
        let mut command = Command::new(env!("CARGO_BIN_EXE_attn-lens"));
        command
            .env("ATTN_BIN", self.path("attn"))
            .arg("--desktop")
            .arg(self.path("backdrop.png"))
            .arg("--script")
            .arg(script)
            .arg("--at")
            .arg(format!("mark:{mark}"));
        command
    }

    fn tree(&self, window: &str, mark: &str) -> Value {
        let out = self.path(&format!("{window}-{mark}.json"));
        let run = self
            .command(mark)
            .args(["--window", window, "--tree"])
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
}

fn window<'a>(tree: &'a Value, key: &str) -> &'a Value {
    tree["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["key"] == key)
        .unwrap_or_else(|| panic!("no window {key} in {tree}"))
}

fn visible(tree: &Value, key: &str) -> bool {
    window(tree, key)["visible"].as_bool().unwrap()
}

fn frame(tree: &Value, key: &str) -> Vec<f64> {
    let frame = &window(tree, key)["frame"];
    frame
        .as_array()
        .unwrap_or_else(|| panic!("{key} has no frame: {frame}"))
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect()
}

/// An element that is really there: Rune keeps removed ones in the tree while they fade.
fn element<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    tree["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["key"] == key && e["exiting"] != true)
}

fn text<'a>(tree: &'a Value, key: &str) -> Option<&'a str> {
    element(tree, key).and_then(|e| e["text"].as_str())
}

fn label<'a>(tree: &'a Value, key: &str) -> Option<&'a str> {
    element(tree, key).and_then(|e| e["label"].as_str())
}

const HANDLES: [&str; 4] = [
    "handle-top-left",
    "handle-top-right",
    "handle-bottom-left",
    "handle-bottom-right",
];

/// The outline on the ink layer, and a handle window at each corner, inside the area.
fn has_area_frame(tree: &Value) {
    assert!(element(tree, "area-outline").is_some(), "no outline");
    let ink = frame(tree, "ink");
    for key in HANDLES {
        assert!(visible(tree, key), "{key} is hidden");
        let handle = frame(tree, key);
        assert!(
            handle[0] >= ink[0]
                && handle[1] >= ink[1]
                && handle[0] + handle[2] <= ink[0] + ink[2]
                && handle[1] + handle[3] <= ink[1] + ink[3],
            "{key} at {handle:?} is outside the area {ink:?}"
        );
    }
}

const ONE_SESSION: &str = r#"[{"id":"s1","label":"one","agent":"claude","state":"idle"}]"#;
const TWO_SESSIONS: &str = r#"[{"id":"s1","label":"one","agent":"claude","state":"idle","last_focused_at":"2026-11-07T10:00:00Z"},{"id":"s2","label":"two","agent":"codex","state":"working"}]"#;

#[test]
fn the_app_exposes_its_windows() {
    let run = Command::new(env!("CARGO_BIN_EXE_attn-lens"))
        .arg("--list-windows")
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&run.stdout);
    let listed: Vec<&str> = listed.lines().collect();
    assert_eq!(&listed[..2], ["editor", "bar"]);
    assert!(
        listed.contains(&"picker-0") && listed.contains(&"ink"),
        "{listed:?}"
    );
}

#[test]
fn launch_dock_and_capture_keys_reach_the_windows() {
    let run = Run::new("foundation.json", "[]");

    // Lens starts with no window.
    let launched = run.tree("bar", "launched");
    assert!(!visible(&launched, "editor"));
    assert!(!visible(&launched, "bar"));
    assert!(!visible(&launched, "picker-0"));
    assert!(!visible(&launched, "ink"));

    // A Dock click opens the empty editor (D18).
    let dock = run.tree("editor", "dock-clicked");
    assert!(visible(&dock, "editor"));
    assert!(!visible(&dock, "bar"));
    assert!(text(&dock, "editor-empty").is_some());

    // A pick opens a capture and takes no still yet.
    let open = run.tree("bar", "capture-open");
    assert!(visible(&open, "bar"));
    assert_eq!(label(&open, "still-0"), None);

    // ⌘⌥X during a capture, a click and a window switch each take a still, in
    // order; voice segments arrive.
    let stills = run.tree("bar", "stills-taken");
    assert_eq!(label(&stills, "still-0"), Some("Hotkey 1280×800"));
    assert_eq!(label(&stills, "still-1"), Some("Click 1280×800"));
    assert_eq!(label(&stills, "still-2"), Some("WindowSwitch 1280×800"));
    assert_eq!(label(&stills, "still-3"), None);
    assert_eq!(text(&stills, "voice"), Some("Listening · ⌘⌥S to stop"));
    assert_eq!(text(&stills, "segment-0"), Some("this button does nothing"));

    // ⌘⌥↩ without a session names what is missing and keeps the capture.
    let refused = run.tree("bar", "send-refused");
    assert!(visible(&refused, "bar"));
    assert_eq!(label(&refused, "chip"), Some("No sessions in attn"));
    assert_eq!(
        text(&refused, "error"),
        Some("The capture has no target session.")
    );
    assert!(label(&refused, "still-2").is_some());
}

#[test]
fn the_picker_covers_every_display_and_says_both_ways_in() {
    let run = Run::new("pick.json", ONE_SESSION);

    let picking = run.tree("picker-1", "picking");
    assert_eq!(frame(&picking, "picker-0"), [0., 0., 1280., 800.]);
    assert_eq!(frame(&picking, "picker-1"), [1280., 0., 1024., 768.]);
    assert!(visible(&picking, "picker-0"));
    assert!(visible(&picking, "picker-1"));
    assert!(!visible(&picking, "picker-2"), "no third display");
    let hint = text(&picking, "picker-hint").unwrap();
    assert!(hint.contains("Drag for an area"), "{hint}");
    assert!(hint.contains("Click for the whole display"), "{hint}");
    assert!(!hint.contains("last area"), "no last area yet: {hint}");

    // ↩ with no last area leaves the picker open; Esc closes it and starts nothing.
    let enter = run.tree("picker-0", "enter-without-last");
    assert!(visible(&enter, "picker-0"));
    let cancelled = run.tree("bar", "cancelled");
    assert!(!visible(&cancelled, "picker-0"));
    assert!(!visible(&cancelled, "picker-1"));
    assert!(!visible(&cancelled, "bar"));
    assert!(!visible(&cancelled, "ink"));

    // A drag on the first display makes the area, and the bar sits outside it.
    let picked = run.tree("ink", "area-display-1");
    assert!(!visible(&picked, "picker-0"));
    assert_eq!(frame(&picked, "ink"), [200., 100., 400., 300.]);
    has_area_frame(&picked);
    let bar = frame(&picked, "bar");
    assert!(bar[1] >= 400., "the bar overlaps the area: {bar:?}");

    // Esc in the bar closes the capture; the picker now offers the last area.
    let closed = run.tree("bar", "capture-closed");
    assert!(!visible(&closed, "ink"));
    assert!(!visible(&closed, "bar"));
    let again = run.tree("picker-0", "picking-again");
    assert!(text(&again, "picker-hint").unwrap().contains("↩ last area"));
    let last = run.tree("ink", "last-area");
    assert_eq!(frame(&last, "ink"), [200., 100., 400., 300.]);
    assert!(visible(&last, "ink"));
}

#[test]
fn a_click_takes_the_whole_second_display_and_keeps_its_outline() {
    let run = Run::new("pick.json", ONE_SESSION);
    let whole = run.tree("ink", "whole-display-2");
    assert_eq!(frame(&whole, "ink"), [1280., 0., 1024., 768.]);
    has_area_frame(&whole);
    // The outline stays inside the display, where it can be seen.
    let outline = element(&whole, "area-outline").unwrap();
    assert_eq!(outline["rect"], serde_json::json!([0., 0., 1024., 768.]));
    assert_eq!(
        frame(&whole, "handle-bottom-right"),
        [2280., 744., 24., 24.]
    );
    // No room outside a whole display, so the bar stays on it, inside the bottom edge.
    let bar = frame(&whole, "bar");
    assert!(bar[0] >= 1280., "the bar left the display: {bar:?}");
    assert!(bar[1] + bar[3] <= 768., "the bar left the display: {bar:?}");

    // The outline and handles stay for the rest of the capture: a still changes nothing.
    let still = run.tree("ink", "still-taken");
    has_area_frame(&still);
    let strip = run.tree("bar", "still-taken");
    assert_eq!(label(&strip, "still-0"), Some("Hotkey 2048×1536"));
}

#[test]
fn an_area_on_the_second_display_adjusts_by_its_corners() {
    let run = Run::new("pick.json", ONE_SESSION);
    let picked = run.tree("ink", "area-display-2");
    assert_eq!(frame(&picked, "ink"), [1400., 100., 300., 200.]);
    assert!(frame(&picked, "bar")[1] >= 300.);
    has_area_frame(&picked);

    let bottom_right = run.tree("ink", "bottom-right-moved");
    assert_eq!(frame(&bottom_right, "ink"), [1400., 100., 350., 240.]);
    has_area_frame(&bottom_right);

    // Moving the top-left corner moves the window under the pointer, and it still lands.
    let top_left = run.tree("ink", "top-left-moved");
    assert_eq!(frame(&top_left, "ink"), [1380., 90., 370., 250.]);
    has_area_frame(&top_left);

    // On the display with the menu bar, the top handles sit below it, where they
    // cannot take the Apple menu's or the clock's clicks.
    let whole = run.tree("ink", "whole-display-1");
    assert_eq!(frame(&whole, "ink"), [0., 0., 1280., 800.]);
    assert_eq!(frame(&whole, "handle-top-left"), [0., 24., 24., 24.]);
    assert_eq!(frame(&whole, "handle-top-right"), [1256., 24., 24., 24.]);
    assert_eq!(
        frame(&whole, "handle-bottom-right"),
        [1256., 776., 24., 24.]
    );
}

#[test]
fn the_draw_key_gives_an_arrow_a_freehand_stroke_and_a_box() {
    let run = Run::new("capture.json", ONE_SESSION);

    // With the draw key up, nothing is drawn.
    let open = run.tree("ink", "area-picked");
    assert_eq!(label(&open, "mark-0"), None);

    // A straight drag is an arrow, and the first press took the still, before ink.
    let arrow = run.tree("ink", "arrow-drawn");
    assert_eq!(label(&arrow, "mark-0"), Some("arrow"));
    let bar = run.tree("bar", "arrow-drawn");
    assert_eq!(label(&bar, "still-0"), Some("Draw 800×600"));

    // During a stroke, what is drawn so far shows.
    let mid = run.tree("ink", "mid-stroke");
    assert_eq!(label(&mid, "draft"), Some("freehand"));
    // The handles are off while drawing, so a stroke can start on a corner.
    for key in HANDLES {
        assert!(!visible(&mid, key), "{key} shows while drawing");
    }

    // A crooked drag stays freehand, and joins the still that press began.
    let freehand = run.tree("ink", "freehand-drawn");
    assert_eq!(label(&freehand, "mark-1"), Some("freehand"));
    assert_eq!(label(&run.tree("bar", "freehand-drawn"), "still-1"), None);

    // ⇧ with the draw key makes a box.
    let boxed = run.tree("ink", "box-drawn");
    assert_eq!(label(&boxed, "mark-2"), Some("box"));
    assert!(element(&boxed, "draft").is_none());

    // With the key up again, a press draws nothing and takes no still.
    let up = run.tree("ink", "press-with-key-up");
    assert_eq!(label(&up, "mark-3"), None);
    has_area_frame(&up);
    assert_eq!(
        label(&run.tree("bar", "press-with-key-up"), "still-1"),
        None
    );

    // A ⌘⌥X still ends the view, so the next stroke takes a still of its own.
    let bar = run.tree("bar", "hotkey-still");
    assert_eq!(label(&bar, "still-1"), Some("Hotkey 800×600"));
    let bar = run.tree("bar", "drawn-after-still");
    assert_eq!(label(&bar, "still-2"), Some("Draw 800×600"));

    // So does moving a corner: the next stroke's still covers the new area.
    let resized = run.tree("ink", "drawn-after-resize");
    assert_eq!(frame(&resized, "ink"), [200., 100., 440., 330.]);
    assert_eq!(label(&resized, "mark-4"), Some("arrow"));
    let bar = run.tree("bar", "drawn-after-resize");
    assert_eq!(label(&bar, "still-3"), Some("Draw 880×660"));

    // With every still dropped, ⌘⌥↩ takes one of the area and sends it (D7).
    let dropped = run.tree("bar", "all-dropped");
    assert_eq!(label(&dropped, "still-0"), None);
    let sent = run.tree("bar", "sent-after-drops");
    assert!(!visible(&sent, "bar"), "the capture stayed open");
    let args = std::fs::read_to_string(run.path("args")).unwrap();
    assert!(args.starts_with("send s1 "), "{args}");
    assert!(run.path("frame-01.png").exists(), "{args}");
}

#[test]
fn a_still_left_from_a_closed_capture_does_not_stop_the_next_from_sending() {
    // Capture A asks for a still and closes before it lands; capture B, with no
    // still of its own, sends while A's is still on the way.
    let run = Run::new("stale-still.json", TWO_SESSIONS);
    let closed = run.tree("bar", "a-closed");
    assert!(!visible(&closed, "bar"), "capture A stayed open");
    let sent = run.tree("bar", "b-sent");
    assert!(!visible(&sent, "bar"), "capture B stayed open");
    let args = std::fs::read_to_string(run.path("args")).unwrap();
    assert!(args.starts_with("send s1 "), "{args}");
    let frame = image::open(run.path("frame-01.png")).unwrap().to_rgba8();
    assert_eq!((frame.width(), frame.height()), (400, 300), "not B's area");
}

#[test]
fn a_corner_drag_keeps_the_opposite_corner_where_it_was() {
    // The area spans 100–200 across. The top-left corner, dragged past the
    // minimum and back, moves only its own edges; so does the bottom-right
    // corner dragged off the display and back.
    let run = Run::new("corners.json", TWO_SESSIONS);
    assert_eq!(
        frame(&run.tree("bar", "picked"), "ink"),
        [100., 100., 100., 200.]
    );
    assert_eq!(
        frame(&run.tree("bar", "past-anchor"), "ink"),
        [164., 100., 36., 200.]
    );
    assert_eq!(
        frame(&run.tree("bar", "back-from-anchor"), "ink"),
        [150., 100., 50., 200.]
    );
    assert_eq!(
        frame(&run.tree("bar", "off-display"), "ink"),
        [150., 100., 1130., 700.]
    );
    assert_eq!(
        frame(&run.tree("bar", "back-on-display"), "ink"),
        [150., 100., 150., 300.]
    );
}

#[test]
fn the_bar_arms_tools_and_owns_the_keyboard() {
    let run = Run::new("bar.json", TWO_SESSIONS);

    // The bar starts on the session last focused in attn, with the keys.
    let open = run.tree("bar", "open");
    assert_eq!(label(&open, "chip"), Some("→ one · this Mac"));
    assert_eq!(text(&open, "keys"), Some("Typing here"));
    for key in [
        "message",
        "tool-highlight",
        "tool-label",
        "tool-counter",
        "edit",
        "send",
    ] {
        assert!(element(&open, key).is_some(), "{key} is missing");
    }
    // Edit, Send and the tools stand at one height.
    let height = |key: &str| element(&open, key).unwrap()["rect"][3].as_f64().unwrap();
    assert_eq!(height("edit"), height("send"));
    assert_eq!(height("tool-label"), height("send"));
    assert_eq!(height("chip"), height("send"));

    assert_eq!(text(&run.tree("bar", "typed"), "message"), Some("hi"));

    // A tool lasts for one stroke.
    let highlighted = run.tree("ink", "highlighted");
    assert_eq!(label(&highlighted, "mark-0"), Some("highlight"));

    let counted = run.tree("ink", "counted");
    assert_eq!(label(&counted, "mark-1"), Some("counter"));
    assert_eq!(label(&counted, "mark-2"), Some("counter"));

    // A label takes its text in the bar, then typing returns to the message.
    let placing = run.tree("bar", "label-placed");
    assert!(element(&placing, "label").is_some());
    assert!(element(&placing, "message").is_none());
    let labelled = run.tree("ink", "labelled");
    assert_eq!(label(&labelled, "mark-3"), Some("label"));
    assert_eq!(text(&labelled, "mark-3"), Some("here"));
    let message = run.tree("bar", "labelled");
    assert_eq!(text(&message, "message"), Some("hi"));

    // A page click gives the page the keys, and the field says so. ⌘⌥C takes them back.
    let page = run.tree("bar", "page-clicked");
    assert_eq!(text(&page, "keys"), Some("Typing on the page"));
    assert_eq!(
        text(&run.tree("bar", "keys-back"), "keys"),
        Some("Typing here")
    );
    assert_eq!(
        text(&run.tree("bar", "typed-again"), "message"),
        Some("hi!")
    );
}

#[test]
fn the_strip_drops_stills_the_palette_chooses_and_send_delivers() {
    let run = Run::new("bar.json", TWO_SESSIONS);

    let stills = run.tree("bar", "three-stills");
    assert_eq!(label(&stills, "still-0"), Some("Draw 800×600"));
    assert_eq!(label(&stills, "still-1"), Some("Click 800×600"));
    assert_eq!(label(&stills, "still-2"), Some("Hotkey 800×600"));
    assert!(element(&stills, "drop-2").is_some());

    let dropped = run.tree("bar", "still-dropped");
    assert!(label(&dropped, "still-1").is_some());
    assert_eq!(label(&dropped, "still-2"), None);

    let palette = run.tree("bar", "palette-open");
    assert!(element(&palette, "palette-row-0").is_some());
    assert!(element(&palette, "palette-row-1").is_some());
    let closed = frame(&dropped, "bar");
    assert!(
        frame(&palette, "bar")[3] > closed[3],
        "the bar did not grow"
    );

    // ↓ moves the selection, and the selected row stands out.
    let moved = run.tree("bar", "palette-moved");
    let fill = |tree: &Value, key: &str| element(tree, key).unwrap()["fill"].clone();
    assert_ne!(fill(&moved, "palette-row-1"), fill(&moved, "palette-row-0"));

    let chosen = run.tree("bar", "session-chosen");
    assert!(element(&chosen, "palette-row-0").is_none());
    assert_eq!(
        frame(&chosen, "bar")[3],
        closed[3],
        "the bar did not shrink"
    );

    // ⌘⌥↩ sends from anywhere: the chosen session, the message, and the kept stills.
    let sent = run.tree("bar", "sent");
    assert!(!visible(&sent, "bar"));
    assert!(!visible(&sent, "ink"));
    let args = std::fs::read_to_string(run.path("args")).unwrap();
    assert!(args.starts_with("send s2 "), "{args}");
    assert!(args.ends_with("-m hi!\n"), "{args}");
    assert!(run.path("frame-02.png").exists());
    assert!(
        !Path::new(&run.path("frame-03.png")).exists(),
        "the dropped still was sent"
    );

    // The sent frame carries the marks; the area's outline never reaches it.
    let frame_one = image::open(run.path("frame-01.png")).unwrap().to_rgba8();
    assert_eq!((frame_one.width(), frame_one.height()), (800, 600));
    let untouched = |x: u32, y: u32| [(x % 256) as u8, (y % 256) as u8, 128, 255];
    for corner in [(0, 0), (799, 0), (0, 599), (799, 599), (12, 12)] {
        assert_eq!(
            frame_one.get_pixel(corner.0, corner.1).0,
            untouched(corner.0, corner.1)
        );
    }
    let (x, y) = (2 * (300 - 200), 2 * (250 - 100));
    assert_ne!(
        frame_one.get_pixel(x, y).0,
        untouched(x, y),
        "the counter is not on the frame"
    );
}

#[test]
fn a_failed_listing_names_itself_and_enter_asks_again() {
    let run = Run::new("capture.json", "not json");
    let bar = run.tree("bar", "area-picked");
    assert_eq!(label(&bar, "chip"), Some("Sessions unavailable"));
    let status = text(&bar, "status").unwrap();
    assert!(status.contains("JSON Lens cannot read"), "{status}");
    assert!(status.contains("list the sessions again"), "{status}");
}

#[test]
fn an_unpaired_lens_pairs_from_the_bar() {
    let run = Run::new("pair.json", ONE_SESSION);
    // attn refuses the token until `attn send pair` has run.
    let attn = std::fs::read_to_string(run.path("attn")).unwrap();
    let dir = run.dir.path().display();
    std::fs::write(
        run.path("attn"),
        attn.replacen(
            "cat > /dev/null\n",
            &format!(
                r#"cat > /dev/null
if [ "$2" = "pair" ]; then
  touch "{dir}/paired"
  echo '{{"id":"p1","name":"Attn Lens","code":"4242","requested_at":"2026-11-03T00:00:00Z"}}'
  echo '{{"sender":{{"id":"s","name":"Attn Lens","created_at":"2026-11-03T00:00:00Z"}},"token":"tok"}}'
  exit 0
fi
if [ ! -e "{dir}/paired" ]; then echo 'token unknown: run attn send pair' >&2; exit 1; fi
"#
            ),
            1,
        ),
    )
    .unwrap();

    let unpaired = run.tree("bar", "unpaired");
    assert_eq!(label(&unpaired, "chip"), Some("Not paired with attn"));
    assert!(
        text(&unpaired, "status")
            .unwrap()
            .contains("Press ↩ to pair")
    );

    let paired = run.tree("bar", "paired");
    assert_eq!(label(&paired, "chip"), Some("→ one · this Mac"));
    assert_eq!(text(&paired, "status"), None);
}

#[test]
fn the_bar_shows_how_far_voice_has_got_and_waits_for_the_last_sentence() {
    let run = Run::new("voice.json", ONE_SESSION);
    let voice = |mark| text(&run.tree("bar", mark), "voice").map(str::to_string);

    assert_eq!(voice("open").as_deref(), Some("⌘⌥S voice"));
    assert_eq!(voice("starting").as_deref(), Some("Starting voice…"));
    // The first ⌘⌥S fetches the model, with progress.
    assert_eq!(
        voice("downloading").as_deref(),
        Some("Downloading speech model 25%")
    );
    assert_eq!(voice("loading").as_deref(), Some("Loading speech model…"));
    assert_eq!(voice("warming").as_deref(), Some("Starting microphone…"));
    assert_eq!(
        voice("listening").as_deref(),
        Some("Listening · ⌘⌥S to stop")
    );
    assert_eq!(
        text(&run.tree("bar", "segment"), "segment-0"),
        Some("open the settings")
    );

    // Stopping waits for the sentence in progress, and ⌘⌥S then does nothing.
    assert_eq!(voice("finishing").as_deref(), Some("Finishing…"));
    assert_eq!(voice("still-finishing").as_deref(), Some("Finishing…"));
    assert_eq!(voice("stopped").as_deref(), Some("⌘⌥S voice"));

    // Voice that fails on its own says why and leaves the capture open.
    let failed = run.tree("bar", "failed");
    assert_eq!(text(&failed, "voice"), Some("⌘⌥S voice"));
    assert_eq!(
        text(&failed, "error"),
        Some("Voice stopped: the speech helper quit.")
    );
    assert!(visible(&failed, "bar"));

    // Send waits for voice to finish, and the bar says so.
    let finishing = run.tree("bar", "send-finishing");
    assert_eq!(text(&finishing, "voice"), Some("Finishing…"));
    assert!(visible(&finishing, "bar"));
    assert!(
        !run.path("args").exists(),
        "nothing sent before voice finished"
    );
    assert!(!visible(&run.tree("bar", "sent"), "bar"));
    let args = std::fs::read_to_string(run.path("args")).unwrap();
    assert!(args.starts_with("send s1 "), "{args}");

    // ⌘⌥S while a send is delivering starts voice on that capture; the send
    // closing the capture stops it, so the next capture's ⌘⌥S starts afresh.
    assert_eq!(
        text(&run.tree("bar", "voice-during-send"), "voice"),
        Some("Starting voice…")
    );
    let again = run.tree("bar", "sent-again");
    assert!(
        !visible(&again, "bar"),
        "{:?} {:?} {:?}",
        text(&again, "error"),
        text(&again, "status"),
        text(&again, "voice")
    );
    let after = run.tree("bar", "voice-after-send");
    assert_eq!(text(&after, "voice"), Some("Starting voice…"));
    assert_eq!(text(&after, "error"), None);
}

#[test]
fn voice_on_a_new_capture_does_not_wait_for_the_last_one_to_stop() {
    let run = Run::new("voice-new-capture.json", ONE_SESSION);

    // Esc closed the capture and its voice is still stopping, held in a job.
    let second = run.tree("bar", "second-voice");
    assert_eq!(text(&second, "voice"), Some("Listening · ⌘⌥S to stop"));
    assert_eq!(text(&second, "error"), None);

    // The old voice ending leaves the new one running.
    let later = run.tree("bar", "old-voice-finished");
    assert_eq!(text(&later, "voice"), Some("Listening · ⌘⌥S to stop"));
}

#[test]
fn send_right_after_voice_stops_waits_for_its_last_sentence() {
    let run = Run::new("voice-send.json", ONE_SESSION);

    // ⌘⌥S stopped voice and Send came while it finished: the send's still is
    // taken, but nothing goes to attn until the last sentence is in.
    let waiting = run.tree("bar", "still-while-finishing");
    assert_eq!(text(&waiting, "voice"), Some("Finishing…"));
    assert_eq!(text(&waiting, "segment-0"), None);
    assert!(!run.path("args").exists(), "sent before voice finished");

    let finished = run.tree("bar", "voice-finished");
    assert_eq!(text(&finished, "segment-0"), Some("and then it crashes"));
    assert_eq!(text(&finished, "voice"), Some("⌘⌥S voice"));
    assert!(!run.path("args").exists(), "the send runs after the finish");

    assert!(!visible(&run.tree("bar", "sent"), "bar"));
    let args = std::fs::read_to_string(run.path("args")).unwrap();
    assert!(args.starts_with("send s1 "), "{args}");

    // A finish that fails stops the send, which would lose the sentence, and says why.
    let failed = run.tree("bar", "send-failed");
    assert!(visible(&failed, "bar"));
    assert_eq!(
        text(&failed, "status"),
        Some("Voice stopped: transcription failed")
    );
}
