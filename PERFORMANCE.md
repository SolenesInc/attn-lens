# Editor performance

How the editor keeps pixel work off the UI thread. This file holds design, not
numbers: speed is measured on a real Mac, against the current build, and VM runs do
not count.

## Drawing

A pointer move appends image-coordinate points to the gesture and repaints the marks
in `src/editor/live.rs`, in canvas points. It does not touch screenshot pixels or
rasterize annotations. Pen samples closer than 0.35 screen pixels are coalesced.

Marks stay live until the raster has caught up with them. `src/editor/preview.rs`
renders the raster one job at a time, off the UI thread; a result for an old
revision asks for another. Crop, resize and rotation run the same way, and land
with their raster.

Dragging a mark leaves the document unchanged until release. The raster stops below
the first moved mark, and that mark and the ones above it are drawn live. Release
commits one undo step.

Zoom and pan change only the view transform.

## Undo and memory

Undo states and render snapshots share the immutable source image through `Arc`, so
adding a mark does not copy the screenshot. Crop creates a new image and keeps the
old one for undo. History holds 30 states.

## Backdrops and animation

`src/editor/stage.rs` renders frames one at a time on the port's `Lane`, and the
newest request wins. A frame stays on screen until the next lands. Playback time
advances only while the window is visible and showing the frame it asked for, so
playback waits for a slow frame instead of skipping it. Backdrop frames and playing
entrances render at a 960 px edge, paused entrances at 1920 px, at 30 frames per
second of preview time.

`src/motion_shader.rs` runs the backdrop effects in Metal, with a matching CPU
evaluator if Metal fails. Periodic motion is driven by the sine and cosine of the
loop phase, so a loop's first and last frames meet.

## Export

GIF and MP4 export run off the UI thread and stream one frame at a time to the
encoder. GIF is 20 fps at a 960 px edge, with one 256-color palette learned across
the whole clip, which prevents shimmer. MP4 is 30 fps at a 1920 px edge, encoded by
an AVFoundation helper.

## Measure

These print timings for their own boundary. They are not input-to-display latency
or a frame rate.

```sh
cargo test --release --locked document_commit_benchmark -- --ignored --nocapture
cargo test --locked foreground_sampling_benchmark -- --ignored --nocapture
cargo test --locked motion_gallery_qa -- --ignored --nocapture
```

`document_commit_benchmark` commits a two-point mark to a 4K document.
`motion_gallery_qa` renders samples of the shader effects and measures their
compute-plus-readback cost.
