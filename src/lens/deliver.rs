//! What a capture becomes at Send: one message holding the flat timeline, a contact
//! sheet per six stills for order and layout, and every still at full resolution for
//! reading (D10 and D11 on the plan). Only `Document::export()` output is written.
//!
//! `plan_rows` and the sheet's sizing are adapted from `plan_contact_sheet_rows` and
//! `render_contact_sheet_for_duration` in Hex's `crates/capture/src/render.rs`
//! (<https://github.com/anomalyco/hex>, commit 54d1ebf), MIT License,
//! Copyright (c) 2026 Christopher Langton. See THIRD_PARTY_NOTICES.md.
use super::session::{Cause, HostTime, Segment};
use crate::document::{Document, Mark, Tool};
use image::{Rgba, RgbaImage, imageops};
use std::{
    ops::Range,
    path::{Path, PathBuf},
};

const STILLS_PER_SHEET: usize = 6;
const SHEET_MAX_EDGE: f32 = 1600.;
const SHEET_MAX_PIXELS: f32 = 1_560_000.;
const SHEET_PAD: u32 = 6;
const SHEET_GAP: u32 = 6;
const SHEET_BACKGROUND: Rgba<u8> = Rgba([28, 29, 34, 255]);
const CHIP_TEXT_SIZE: f32 = 18.;
const CHIP_PAD: u32 = 5;

/// One still as Send reads it, taken from the capture so the lock is not held
/// while images are encoded.
pub struct Frame {
    pub at: HostTime,
    pub cause: Cause,
    pub doc: Document,
}

pub struct Source {
    pub started: HostTime,
    pub frames: Vec<Frame>,
    pub segments: Vec<Segment>,
    /// What the user typed in the bar. It comes before the timeline.
    pub typed: String,
}

#[derive(Debug)]
pub struct Delivery {
    pub message: String,
    /// The sheets, then the frames in still order.
    pub files: Vec<PathBuf>,
}

/// Writes the PNGs into `dir` and composes the message.
pub fn build(source: &Source, dir: &Path) -> Result<Delivery, String> {
    let count = source.frames.len();
    if count == 0 {
        return Err("The capture has no still to send.".into());
    }
    // One still has no order to show, and its sheet would only repeat its frame.
    let sheets = if count == 1 {
        0
    } else {
        count.div_ceil(STILLS_PER_SHEET)
    };
    let mut sheet_files = vec![];
    let mut frame_files = vec![];
    for (sheet, frames) in source.frames.chunks(STILLS_PER_SHEET).enumerate() {
        let images: Vec<RgbaImage> = frames.iter().map(|f| f.doc.export()).collect();
        let first = sheet * STILLS_PER_SHEET;
        for (offset, image) in images.iter().enumerate() {
            let path = dir.join(frame_name(first + offset));
            save(image, &path)?;
            frame_files.push(path);
        }
        if sheets > 0 {
            let path = dir.join(sheet_name(sheet, sheets));
            save(&contact_sheet(&images, first), &path)?;
            sheet_files.push(path);
        }
    }
    sheet_files.append(&mut frame_files);
    Ok(Delivery {
        message: message(source, sheets),
        files: sheet_files,
    })
}

fn save(image: &RgbaImage, path: &Path) -> Result<(), String> {
    image
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))
}

fn frame_name(index: usize) -> String {
    format!("frame-{:02}.png", index + 1)
}

fn sheet_name(index: usize, sheets: usize) -> String {
    if sheets == 1 {
        "sheet.png".into()
    } else {
        format!("sheet-{}.png", index + 1)
    }
}

fn message(source: &Source, sheets: usize) -> String {
    let typed = source.typed.trim();
    if source.frames.len() == 1 && source.segments.is_empty() {
        return if typed.is_empty() {
            "Screen capture, 1 still.".into()
        } else {
            typed.into()
        };
    }
    let timeline = format!("{}\n{}", header(source, sheets), rows(source).join("\n"));
    if typed.is_empty() {
        timeline
    } else {
        format!("{typed}\n\n{timeline}")
    }
}

fn header(source: &Source, sheets: usize) -> String {
    let count = source.frames.len();
    let length = clock(longest(source).since(source.started));
    let guide = match sheets {
        0 => format!("open {} to read text", frame_name(0)),
        1 => "sheet.png shows order and layout; open frame-NN.png, which is still N, to read text"
            .into(),
        _ => format!(
            "{} to {} show order and layout; open frame-NN.png, which is still N, to read text",
            sheet_name(0, sheets),
            sheet_name(sheets - 1, sheets),
        ),
    };
    let stills = if count == 1 { "still" } else { "stills" };
    format!("Screen capture, {length}, {count} {stills} ({guide}).")
}

/// The time of the capture's last event.
/// When a mark was drawn on the live capture.
fn span(mark: &Mark) -> Option<(HostTime, HostTime)> {
    mark.drawn
        .map(|(start, end)| (HostTime(start), HostTime(end)))
}

fn spans(frame: &Frame) -> impl Iterator<Item = (HostTime, HostTime)> + '_ {
    frame.doc.marks.iter().filter_map(span)
}

fn longest(source: &Source) -> HostTime {
    let frames = source
        .frames
        .iter()
        .flat_map(|frame| std::iter::once(frame.at).chain(spans(frame).map(|(_, end)| end)));
    let words = source.segments.iter().map(|segment| segment.end);
    frames.chain(words).max().unwrap_or(source.started)
}

/// Stills, marks and words as one list in time order. At the same instant a still
/// comes before its marks, and those before words.
fn rows(source: &Source) -> Vec<String> {
    let mut rows: Vec<(HostTime, u8, String)> = vec![];
    let since = |time: HostTime| clock(time.since(source.started));
    for (index, frame) in source.frames.iter().enumerate() {
        let still = index + 1;
        let at = frame.at;
        let cause = cause_name(frame.cause);
        rows.push((at, 0, format!("{} still {still} ({cause})", since(at))));
        // A mark added in the editor has no span, so it has no row.
        for mark in &frame.doc.marks {
            let Some((start, end)) = span(mark) else {
                continue;
            };
            let (from, to) = (since(start), since(end));
            let span = if from == to {
                from
            } else {
                format!("{from}–{to}")
            };
            let name = mark_name(mark);
            rows.push((start, 1, format!("{span} {name} on still {still}")));
        }
    }
    for segment in &source.segments {
        let row = format!("{} \"{}\"", since(segment.start), segment.text.trim());
        rows.push((segment.start, 2, row));
    }
    rows.sort_by_key(|&(at, rank, _)| (at, rank));
    rows.into_iter().map(|(_, _, row)| row).collect()
}

fn cause_name(cause: Cause) -> &'static str {
    match cause {
        Cause::Draw => "draw",
        Cause::Hotkey => "hotkey",
        Cause::Click => "click",
        Cause::WindowSwitch => "window switch",
        Cause::Send => "send",
    }
}

fn mark_name(mark: &Mark) -> String {
    let kind = super::ink::kind(mark);
    match mark.tool {
        Tool::Text => format!("{kind} \"{}\"", mark.text),
        Tool::Counter => format!("{kind} {}", mark.text),
        _ => kind.into(),
    }
}

fn clock(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A tile's rectangle on the sheet, in pixels: x, y, width, height.
type Tile = [u32; 4];

struct SheetLayout {
    width: u32,
    height: u32,
    tiles: Vec<Tile>,
}

fn contact_sheet(images: &[RgbaImage], first_still: usize) -> RgbaImage {
    let aspects: Vec<f32> = images
        .iter()
        .map(|image| image.width() as f32 / image.height() as f32)
        .collect();
    let native: Vec<u32> = images.iter().map(RgbaImage::width).collect();
    let layout = layout_sheet(&aspects, &native);
    let mut sheet = RgbaImage::from_pixel(layout.width, layout.height, SHEET_BACKGROUND);
    for (index, (image, tile)) in images.iter().zip(&layout.tiles).enumerate() {
        let [x, y, width, height] = *tile;
        let scaled = imageops::thumbnail(image, width, height);
        imageops::overlay(&mut sheet, &scaled, x.into(), y.into());
        draw_number(&mut sheet, x, y, first_still + index + 1);
    }
    sheet
}

/// The still's number in the tile's corner, the same N as `frame-NN.png`.
fn draw_number(sheet: &mut RgbaImage, x: u32, y: u32, still: usize) {
    use imageproc::{
        drawing::{draw_filled_rect_mut, draw_text_mut, text_size},
        rect::Rect,
    };
    let font = crate::platform::annotation_font();
    let text = still.to_string();
    let (width, _) = text_size(CHIP_TEXT_SIZE, font, &text);
    let side = CHIP_TEXT_SIZE as u32 + 2 * CHIP_PAD;
    let chip = Rect::at(x as i32, y as i32).of_size((width + 2 * CHIP_PAD).max(side), side);
    draw_filled_rect_mut(sheet, chip, Rgba([0, 0, 0, 255]));
    let text_x = x + (chip.width() - width) / 2;
    draw_text_mut(
        sheet,
        Rgba([255, 255, 255, 255]),
        text_x as i32,
        (y + CHIP_PAD) as i32,
        CHIP_TEXT_SIZE,
        font,
        &text,
    );
}

fn layout_sheet(aspects: &[f32], native_widths: &[u32]) -> SheetLayout {
    let rows = plan_rows(aspects);
    let row_aspects: Vec<f32> = rows
        .iter()
        .map(|row| aspects[row.clone()].iter().sum())
        .collect();
    let sheet_aspect = 1. / row_aspects.iter().map(|a| 1. / a).sum::<f32>().max(0.01);
    let (pad, gap) = (SHEET_PAD as f32, SHEET_GAP as f32);
    let widest = rows
        .iter()
        .map(|row| {
            native_widths[row.clone()].iter().sum::<u32>() as f32
                + gap * row.len().saturating_sub(1) as f32
        })
        .fold(0., f32::max);
    let longest_inner = SHEET_MAX_EDGE - 2. * pad;
    let mut inner = widest.max(960.).min(if sheet_aspect >= 1. {
        longest_inner
    } else {
        longest_inner * sheet_aspect
    });
    let height_at = |inner: f32| {
        let tiles: f32 = rows
            .iter()
            .zip(&row_aspects)
            .map(|(row, aspect)| (inner - gap * (row.len() - 1) as f32).max(32.) / aspect)
            .sum();
        tiles + gap * (rows.len() - 1) as f32
    };
    let (total_w, total_h) = (inner + 2. * pad, height_at(inner) + 2. * pad);
    let scale = (SHEET_MAX_EDGE / total_w.max(total_h))
        .min((SHEET_MAX_PIXELS / (total_w * total_h)).sqrt());
    if scale < 1. {
        inner = (total_w * scale - 2. * pad).floor().max(1.);
    }
    let inner = inner.round() as u32;
    let mut tiles = vec![[0; 4]; aspects.len()];
    let mut y = SHEET_PAD;
    for (row, row_aspect) in rows.iter().zip(&row_aspects) {
        let available = inner.saturating_sub(SHEET_GAP * (row.len() as u32 - 1));
        let height = ((available as f32 / row_aspect).round() as u32).max(1);
        let mut across = 0.;
        for (column, index) in row.clone().enumerate() {
            let left = (available as f32 * (across / row_aspect)).round() as u32;
            across += aspects[index];
            let right = if column + 1 == row.len() {
                available
            } else {
                (available as f32 * (across / row_aspect)).round() as u32
            };
            let x = SHEET_PAD + left + column as u32 * SHEET_GAP;
            tiles[index] = [x, y, right.saturating_sub(left).max(1), height];
        }
        y += height + SHEET_GAP;
    }
    SheetLayout {
        width: inner + 2 * SHEET_PAD,
        height: y - SHEET_GAP + SHEET_PAD,
        tiles,
    }
}

/// Splits the stills, in order, into rows so that each shows at a similar size and
/// the sheet's shape is near what vision models read best.
#[allow(clippy::single_range_in_vec_init)]
fn plan_rows(aspects: &[f32]) -> Vec<Range<usize>> {
    let n = aspects.len();
    if n == 1 {
        return vec![0..1];
    }
    if n == 2 {
        return if aspects[0] + aspects[1] > 3.6 {
            vec![0..1, 1..2]
        } else {
            vec![0..2]
        };
    }
    (0u32..1 << (n - 1))
        .map(|breaks| rows_from_breaks(breaks, n))
        .filter(|rows| {
            rows.iter()
                .all(|row| row.len() > 1 || aspects[row.start] >= 1.85)
        })
        .map(|rows| (layout_cost(&rows, aspects), rows))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or_else(|| vec![0..n], |(_, rows)| rows)
}

/// Bit `i` of `breaks` starts a new row after still `i`.
fn rows_from_breaks(breaks: u32, n: usize) -> Vec<Range<usize>> {
    let mut rows = vec![];
    let mut start = 0;
    for end in 1..=n {
        if end == n || breaks & (1 << (end - 1)) != 0 {
            rows.push(start..end);
            start = end;
        }
    }
    rows
}

fn layout_cost(rows: &[Range<usize>], aspects: &[f32]) -> f32 {
    let n = aspects.len();
    let total: f32 = aspects.iter().sum();
    let average = total / n as f32;
    let target_sheet = if average < 0.7 { 1.05 } else { 1.25 };
    let target_row = (total * target_sheet).sqrt().max(average);
    let row_aspects: Vec<f32> = rows
        .iter()
        .map(|row| aspects[row.clone()].iter().sum())
        .collect();
    let sheet_aspect = 1. / row_aspects.iter().map(|a| 1. / a).sum::<f32>();
    let sheet_penalty = (sheet_aspect / target_sheet).ln().powi(2) * 1.8;

    // A still in row r shows at a relative area of its aspect over the row's squared.
    let mut areas = vec![0f32; n];
    for (row, row_aspect) in rows.iter().zip(&row_aspects) {
        for index in row.clone() {
            areas[index] = aspects[index] / (row_aspect * row_aspect);
        }
    }
    let mean_log_area = areas.iter().map(|a| a.ln()).sum::<f32>() / n as f32;
    let area_variance = areas
        .iter()
        .map(|a| (a.ln() - mean_log_area).powi(2))
        .sum::<f32>()
        / n as f32;
    let mut sorted = areas.clone();
    sorted.sort_by(f32::total_cmp);
    let median = ((sorted[(n - 1) / 2] + sorted[n / 2]) * 0.5).max(1e-6);
    let uneven: f32 = areas
        .iter()
        .map(|area| {
            let ratio = area / median;
            let small = (0.58 - ratio).max(0.) / 0.58;
            let large = (ratio - 2.3).max(0.) / 2.3;
            small.powi(2) * 18. + large.powi(2) * 4.
        })
        .sum();

    let count = rows.len() as f32;
    let mean_log_row = row_aspects.iter().map(|a| a.ln()).sum::<f32>() / count;
    let mut row_variance = 0.;
    let mut row_penalty = 0.;
    let mut cramped = rows.len().saturating_sub(4) as f32 * 2.5;
    let mut tie = 0.;
    for (index, (row, row_aspect)) in rows.iter().zip(&row_aspects).enumerate() {
        row_variance += (row_aspect.ln() - mean_log_row).powi(2);
        row_penalty += (row_aspect / target_row).ln().powi(2);
        for aspect in &aspects[row.clone()] {
            let share = aspect / row_aspect * sheet_aspect.min(1.);
            cramped += ((0.16 - share).max(0.) / 0.16).powi(2) * 8.;
        }
        if average > 0.75 {
            cramped += row.len().saturating_sub(4) as f32 * 2.;
        }
        if average > 1.15 {
            cramped += row.len().saturating_sub(3) as f32 * 2.;
        }
        tie += index as f32 * 1e-4 * row.len() as f32;
    }
    sheet_penalty
        + area_variance * 8.
        + uneven
        + row_variance / count * 0.6
        + row_penalty / count * 0.4
        + cramped
        + tie
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLORS: [[u8; 4]; 3] = [[200, 40, 40, 255], [40, 200, 40, 255], [40, 40, 200, 255]];

    fn at(seconds: f64) -> HostTime {
        HostTime::from_secs(100. + seconds)
    }

    fn mark(tool: Tool, text: &str) -> Mark {
        Mark {
            drawn: None,
            style: Default::default(),
            tool,
            curve: None,
            points: vec![(10., 10.), (400., 300.)],
            color: [255, 255, 0, 255],
            width: 8.,
            text: text.into(),
        }
    }

    fn doc(color: [u8; 4], marks: Vec<Mark>) -> Document {
        let mut doc = Document::new(RgbaImage::from_pixel(1000, 650, Rgba(color)));
        for mark in marks {
            doc.commit(mark);
        }
        doc
    }

    fn drawn(mark: Mark, start: f64, end: f64) -> Mark {
        Mark {
            drawn: Some((at(start).0, at(end).0)),
            ..mark
        }
    }

    fn word(start: f64, end: f64, text: &str) -> Segment {
        Segment {
            start: at(start),
            end: at(end),
            text: text.into(),
        }
    }

    /// Three stills, two marks, and words that begin during and between the marks.
    fn fixture() -> Source {
        Source {
            started: at(0.),
            typed: String::new(),
            frames: vec![
                Frame {
                    at: at(0.2),
                    cause: Cause::Hotkey,
                    doc: doc(COLORS[0], vec![drawn(mark(Tool::Arrow, ""), 5.1, 7.4)]),
                },
                Frame {
                    at: at(9.),
                    cause: Cause::Click,
                    doc: doc(COLORS[1], vec![drawn(mark(Tool::Rectangle, ""), 12., 15.)]),
                },
                Frame {
                    at: at(40.),
                    cause: Cause::Send,
                    doc: doc(COLORS[2], vec![]),
                },
            ],
            segments: vec![
                word(3., 6., "this button does nothing"),
                word(6.5, 8., "when I click it"),
                word(14., 42., "and then this error shows up"),
            ],
        }
    }

    fn written(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn file_names(delivery: &Delivery) -> Vec<&str> {
        delivery
            .files
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap())
            .collect()
    }

    #[test]
    fn the_timeline_interleaves_stills_marks_and_words_in_time_order() {
        let dir = tempfile::tempdir().unwrap();
        let delivery = build(&fixture(), dir.path()).unwrap();
        assert_eq!(
            delivery.message,
            "Screen capture, 0:42, 3 stills (sheet.png shows order and layout; open frame-NN.png, which is still N, to read text).\n\
             0:00 still 1 (hotkey)\n\
             0:03 \"this button does nothing\"\n\
             0:05–0:07 arrow on still 1\n\
             0:06 \"when I click it\"\n\
             0:09 still 2 (click)\n\
             0:12–0:15 box on still 2\n\
             0:14 \"and then this error shows up\"\n\
             0:40 still 3 (send)"
        );
    }

    #[test]
    fn typed_words_come_before_the_timeline() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = fixture();
        source.typed = "  look at this \n".into();
        let message = build(&source, dir.path()).unwrap().message;
        assert!(
            message.starts_with("look at this\n\nScreen capture, 0:42"),
            "{message}"
        );
    }

    #[test]
    fn only_flattened_pngs_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let delivery = build(&fixture(), dir.path()).unwrap();
        assert_eq!(
            file_names(&delivery),
            ["sheet.png", "frame-01.png", "frame-02.png", "frame-03.png"]
        );
        assert_eq!(
            written(dir.path()),
            ["frame-01.png", "frame-02.png", "frame-03.png", "sheet.png"]
        );
        for path in &delivery.files {
            let format = image::ImageReader::open(path)
                .unwrap()
                .with_guessed_format()
                .unwrap()
                .format();
            assert_eq!(format, Some(image::ImageFormat::Png), "{path:?}");
        }
        // Full resolution, with the mark in the pixels.
        let source = fixture();
        for (index, frame) in source.frames.iter().enumerate() {
            let on_disk = image::open(dir.path().join(frame_name(index)))
                .unwrap()
                .into_rgba8();
            assert_eq!(on_disk.dimensions(), (1000, 650));
            assert_eq!(on_disk, frame.doc.export());
        }
        let marked = image::open(dir.path().join("frame-01.png"))
            .unwrap()
            .into_rgba8();
        assert_ne!(marked, RgbaImage::from_pixel(1000, 650, Rgba(COLORS[0])));
    }

    #[test]
    fn the_sheet_shows_every_still_in_its_tile() {
        let dir = tempfile::tempdir().unwrap();
        let source = fixture();
        build(&source, dir.path()).unwrap();
        let sheet = image::open(dir.path().join("sheet.png"))
            .unwrap()
            .into_rgba8();
        let layout = layout_sheet(&[1000. / 650.; 3], &[1000; 3]);
        assert_eq!(sheet.dimensions(), (layout.width, layout.height));
        assert_eq!(sheet.get_pixel(0, 0), &SHEET_BACKGROUND);
        for (tile, color) in layout.tiles.iter().zip(COLORS) {
            let [x, y, width, height] = *tile;
            // The bottom right, away from the number and the first still's arrow.
            let pixel = sheet.get_pixel(x + width - 8, y + height - 8);
            assert_eq!(pixel, &Rgba(color), "{tile:?}");
        }
    }

    #[test]
    fn seven_stills_take_two_sheets_numbered_through() {
        let dir = tempfile::tempdir().unwrap();
        let frame = |index: usize| Frame {
            at: at(index as f64),
            cause: Cause::Click,
            doc: doc(COLORS[index % 3], vec![]),
        };
        let source = Source {
            started: at(0.),
            typed: String::new(),
            frames: (0..7).map(frame).collect(),
            segments: vec![],
        };
        let delivery = build(&source, dir.path()).unwrap();
        assert_eq!(delivery.files.len(), 9);
        assert_eq!(file_names(&delivery)[..2], ["sheet-1.png", "sheet-2.png"]);
        assert!(
            delivery
                .message
                .contains("sheet-1.png to sheet-2.png show order")
        );
        assert!(dir.path().join("frame-07.png").exists());
    }

    #[test]
    fn one_silent_still_sends_a_short_message_and_one_png() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = fixture();
        source.frames.truncate(1);
        source.segments.clear();
        let delivery = build(&source, dir.path()).unwrap();
        assert_eq!(delivery.message, "Screen capture, 1 still.");
        assert_eq!(file_names(&delivery), ["frame-01.png"]);
        assert_eq!(written(dir.path()), ["frame-01.png"]);

        source.typed = "this one".into();
        assert_eq!(build(&source, dir.path()).unwrap().message, "this one");
    }

    #[test]
    fn one_still_with_words_keeps_the_timeline_without_a_sheet() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = fixture();
        source.frames.truncate(1);
        source.segments.truncate(1);
        let delivery = build(&source, dir.path()).unwrap();
        assert_eq!(file_names(&delivery), ["frame-01.png"]);
        assert_eq!(
            delivery.message,
            "Screen capture, 0:07, 1 still (open frame-01.png to read text).\n\
             0:00 still 1 (hotkey)\n\
             0:03 \"this button does nothing\"\n\
             0:05–0:07 arrow on still 1"
        );
    }

    #[test]
    fn every_sheet_stays_inside_its_budget_with_tiles_on_it_and_apart() {
        let shapes: [f32; 6] = [1.54, 0.46, 2.4, 1.0, 3.2, 0.012];
        for count in 1..=STILLS_PER_SHEET {
            for first in 0..shapes.len() {
                for &native in &[640u32, 3420] {
                    let aspects: Vec<f32> = (0..count)
                        .map(|i| shapes[(first + i * (first + 1)) % shapes.len()])
                        .collect();
                    let layout = layout_sheet(&aspects, &vec![native; count]);
                    let case = format!("{aspects:?} native {native}");
                    assert!(layout.width.max(layout.height) <= 1600, "{case}");
                    assert!(
                        (layout.width * layout.height) as f32 <= SHEET_MAX_PIXELS * 1.03,
                        "{}x{} {case}",
                        layout.width,
                        layout.height
                    );
                    for (i, a) in layout.tiles.iter().enumerate() {
                        assert!(
                            a[0] + a[2] <= layout.width && a[1] + a[3] <= layout.height,
                            "{case}"
                        );
                        for b in &layout.tiles[i + 1..] {
                            let apart = a[0] + a[2] <= b[0]
                                || b[0] + b[2] <= a[0]
                                || a[1] + a[3] <= b[1]
                                || b[1] + b[3] <= a[1];
                            assert!(apart, "{a:?} {b:?} {case}");
                        }
                    }
                }
            }
        }
    }
}
