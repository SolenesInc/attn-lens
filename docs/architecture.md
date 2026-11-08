# Architecture

Attn Lens is a Rust app on the Rune UI toolkit. It has two halves: Lens, the live
capture, and the editor, which holds one image document. A capture's still opens in
the editor as a document.

## App and platform

`src/app.rs` is the Rune app and its windows. It is a Dock app: **⌘Q** quits and a
Dock click opens the editor. When `rune::tooling::is_headless()` is true, overlays
are hosted as ordinary windows; that is the only branch on the platform.

`src/platform_port.rs` defines `PlatformPort`, everything the UI asks of the
machine. `platform_port/mac.rs` is the real one and `platform_port/fake.rs` is
driven by script marks. Attn Lens runs on macOS; elsewhere it runs only headless,
for tests. `src/platform.rs` holds dialogs, fonts and clipboard.

## Lens

`src/lens/mod.rs` is `Lens`, the state every window and the socket read. Each
change wakes the windows, because a Rune window does not redraw from another's
change.

| Module | Owns |
| --- | --- |
| `session.rs` | Capture, Still, Segment, HostTime |
| `select.rs`, `ink.rs`, `bar.rs` | The area picker, the ink layer and the bar |
| `tile.rs` | The corner tile a closed capture shrinks to |
| `capture.rs` | Stills through Hex's capture-core (ScreenCaptureKit) |
| `watch.rs` | macOS's `PlatformPort::watch` |
| `voice.rs`, `voice/mic.rs` | The speech helper as a child process, fed by the microphone or a WAV |
| `deliver.rs` | What Send delivers: the message, contact sheets and frames |
| `attn.rs` | The only contract with attn: `attn send` JSON and the Keychain token |
| `automation.rs` | The development socket, present only with `--automation` |

`app.rs` hosts the picker, the ink layer and the four corner handles as `Surface`s.
The ink layer takes the pointer only while the draw key is held. While the editor
holds a still, the live surfaces hide and the watch stops; `EditorCall` is how Lens
reaches the editor, and `Editor::leave_still` gives the still back.

`deliver.rs` writes images only from `Document::export()`, so a send never carries a
raw copy of an edited still.

The speech helper is the separate crate `speech/`, built by `scripts/bundle.sh`. It
times each phrase from the recording's first sample; `voice.rs` puts those times on
the capture clock.

## Editor

Every toolbar button, menu, key and gesture reaches `Editor::dispatch` in
`src/editor/dispatch.rs` as a typed `Action`. New triggers construct an action; they
do not fabricate key events.

| Module | Owns |
| --- | --- |
| `gesture.rs` | The pointer on the canvas: one `Gesture` at a time |
| `keys.rs`, `menus.rs` | Window keys and native menus |
| `view.rs`, `canvas.rs`, `panels/` | The window, canvas and sidebar |
| `preview.rs` | The raster, rendered off the UI thread |
| `live.rs` | Marks not yet in the raster |
| `stage.rs` | The backdrop behind the image, and whole frames while the Animation panel plays |
| `export.rs` | GIF and MP4 export |

`stage.rs` renders on the port's `Lane`, one thread on the Mac, so Metal keeps its
thread-local state. Worker results carry an operation ID or document revision, and
a stale one cannot affect newer work.

## Document and rendering

`src/document.rs` and `document/actions.rs` are the image model, validated edits and
undo. Screenshot pixels are immutable and shared across undo states. Geometry and
annotations use physical image pixels.

`src/arrow.rs`, `backdrop.rs`, `effects.rs`, `style.rs`, `selection.rs` and
`navigation.rs` are mark geometry and raster painting, free of any UI toolkit. Inside
padding repeats the nearest screenshot edge pixels before rounding and shadow.

`src/animation.rs`, `src/motion_shader.rs` and `src/shaders/motion.metal` are
backdrops for preview and export, with a CPU fallback. `src/animation/entrance.rs`
evaluates the image's entrance, and `entrance/gpu.rs` accelerates it in Metal.

`src/ocr.rs` runs text recognition off the UI thread through `native/ocr.swift`
(Apple Vision), bounding the helper's output and runtime. Revision checks reject a
stale result before it reaches the clipboard. `native/video_encoder.swift` and
`native/video_frame.swift` are AVFoundation helpers for MP4.

## Tests

`headless/` holds Rune scripts: `tests/headless.rs` runs them for Lens and
`tests/editor.rs` runs `headless/editor/*.json`, one script per tool option and
usage table. `tests/voice.rs` checks phrase times against fixture audio, and
`scripts/lens-vm.sh` runs driven checks in a macOS VM.

[Performance notes](../PERFORMANCE.md) · [QA](../QA.md) ·
[Original development history](../PLAN.md)
