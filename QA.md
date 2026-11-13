# Attn Lens QA

Use synthetic images for acceptance checks. Record the build/commit, macOS version,
hardware, checks performed, and failures with each result.

## Automated checks

Run the [contributor checks](CONTRIBUTING.md#make-a-pull-request). The regular suite
covers the document model, rendering, the attn contract against a stand-in `attn`,
and the windows through Rune's headless scripts: `tests/headless.rs` runs
`headless/*.json` and `tests/editor.rs` runs `headless/editor/*.json`.

### Opt-in checks

Read each ignored test before running it. Build the [native helpers](BUILD.md)
for video and OCR checks. These tests use synthetic content; media-generating
checks write under `target/`. Run only the checks relevant to the change:

| Check | Command |
| --- | --- |
| Native MP4 encode/frame decode | `cargo test --locked native_video_roundtrip -- --ignored --nocapture` |
| Local OCR on synthetic text, source crop and blank image | `cargo test --locked native_ocr_recognizes_synthetic_text_and_regions -- --ignored --nocapture` |
| Voice phrase times against fixture audio | `ATTN_LENS_SPEECH_CACHE=… cargo test --locked --test voice -- --ignored` |
| Image entrance export samples | `cargo test --release --locked image_entrance_export_qa -- --ignored --nocapture` |
| Focus effects and loop samples | `cargo test --release --locked focus_and_loop_demo_qa -- --ignored --nocapture` |
| Native motion export samples | `cargo test --release --locked native_motion_export_qa -- --ignored --nocapture` |
| Undo-history commit benchmark | `cargo test --release --locked document_commit_benchmark -- --ignored --nocapture` |
| Foreground sampling benchmark | `cargo test --locked foreground_sampling_benchmark -- --ignored --nocapture` |

See [PERFORMANCE.md](PERFORMANCE.md) for what the benchmarks measure.

## Driven checks in a VM

`scripts/lens-vm.sh` runs a macOS VM, so synthetic keys never reach this Mac. Run
`setup` once and `push` to install this checkout, then:

| Command | Covers |
| --- | --- |
| `scripts/lens-vm.sh check OUT` | The capture key (right ⌘) to the bar's window, then idle CPU and memory |
| `scripts/lens-vm.sh live OUT` | The picker, pass-through to a page, the keyboard's owner, drawing with the draw key, corner handles and Send, against the stand-in `attn` |
| `scripts/lens-vm.sh voice OUT` | Voice with a speech fixture |
| `scripts/lens-vm.sh editor OUT` | Menus, Copy as OCR, Pick from screen, scroll and zoom, light and dark |
| `scripts/lens-vm.sh tile OUT` | Tile hover and click, Back not counted as a change |

`scripts/lens-vm.sh stop` frees the VM's 8 GB. Speed and the feel of drawing are
checked on a real Mac; VM numbers do not count for them.

## Capture and send

- With Screen Recording missing, check the guide opens System Settings at launch
  and on the capture key, and the grant works after relaunch.
- With Accessibility missing, check the guide opens its list at launch and lists
  it, and that the keys work within five seconds of the grant, with no relaunch.
- Start with a tap of **right ⌘**: drag an area, click for a display, **↩** for the last area,
  **Esc** to cancel. Drag a corner to adjust the area.
- Hold **fn** and draw an arrow, a freehand stroke and, with **⇧**, a box. Check
  the page keeps taking clicks, scrolls and typing.
- Check **⌘⌥C** returns the keyboard to the bar and **Tab** opens the palette.
- Tap **right ⌥** with no model cached: check download progress, then phrases. Interrupt
  a download and check the next tap resumes it. Check the Microphone prompt. Hold
  **right ⌥** and check voice stops on release.
- Check **right ⌘+C** copies and opens nothing, **right ⌥+e** types é and starts no
  voice, and a hold of right ⌘ does nothing.
- Send with **↩** and with **⌘⌥↩**. Check the tile, a click on it reopening the
  capture, and **Esc** keeping a tile while a still exists.
- Remove Lens under attn's **Settings → Paired senders** and check pairing recovers.
- Stop attn and check the bar shows attn's error and recovers.

## Editor

- Open a capture's still with **Edit** and from a thumbnail. Check **‹ Back**, closing
  the window, the capture key and **Send ↩**, and that opening or pasting another image is
  refused while a still is open.
- Test toolbar and menu shortcuts, clipboard import/copy, file drop, open/save
  cancellation, overwrite confirmation and extension handling.
- Draw each annotation. Select, move, restyle, duplicate, delete, and undo/redo.
  Check curved-arrow handles, spotlight corners, and magnifier source/lens handles.
- Place steps, including multi-digit values; deselect and wait for the preview to
  settle. Numbers must stay visible through zoom, drag/release, undo/redo and
  PNG/clipboard export.
- Box-select several marks; test Shift-click/drag, group edits and one-step undo.
  Verify Select All works in the canvas, inline text, numeric fields, and hex fields.
- Test text selection, Unicode/IME, copy/paste, caret placement, and tool shortcuts
  while editing. Test zoom/pan, pinch, smart zoom, and wheel momentum.
- At minimum window size, inspect toolbar/sidebar fit, numeric labels/values,
  scrolling, tool help, and exact-value edits. Check backdrop and annotation color
  pickers, screen sampling/cancellation, opacity, and undo.
- Test framing, crop, resize, and rotation; inspect PNG/clipboard output at full resolution.
- Click **Copy as OCR** on synthetic multiline text and paste into a text editor.
  Confirm the toolbar shows reading/copied feedback, no result panel opens, and
  image history is untouched. Empty images and failures preserve the clipboard.
- Check accessible toolbar/inspector/menu discovery, exact values, and VoiceOver navigation.

## Animation and export

- Preview all eight motions, Randomize, custom colors, and duration changes.
- Check each entrance over still/moving/absent backdrops, timing, Hold/Exit,
  replay, pause, seeking, and return to annotation editing.
- With a large image, replay and seek while preparing. Playback must wait for the
  requested frame; paused playback stays paused.
- Compare inspected-frame PNG/copy with time-zero GIF/MP4 exports. Check dimensions,
  duration, matte, stable foreground, and loop seams visually.
- Close or scroll the inspector during export: progress stays visible. Cancel and
  confirm existing files survive and temporary files are cleaned up.
