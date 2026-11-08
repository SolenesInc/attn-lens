//! Reproducible CPU preparation benchmark; not an end-to-end display latency test.
use crate::document::{Document, Mark, Tool};
use image::{Rgba, RgbaImage};
use std::time::Instant;
fn samples(mut run: impl FnMut(), count: usize) -> (f64, f64) {
    for _ in 0..10 {
        run();
    }
    let mut times: Vec<f64> = (0..count)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed().as_secs_f64() * 1000.
        })
        .collect();
    times.sort_by(f64::total_cmp);
    (times[count / 2], times[count * 95 / 100])
}
#[test]
#[ignore = "run explicitly in release mode for performance measurements"]
fn document_commit_benchmark() {
    let mut document = Document::new(RgbaImage::from_pixel(3840, 2160, Rgba([0, 0, 0, 255])));
    let mark = Mark {
        style: Default::default(),
        tool: Tool::Pen,
        curve: None,
        points: vec![(10., 10.), (100., 100.)],
        color: [255, 0, 0, 255],
        width: 5.,
        text: String::new(),
        drawn: None,
    };
    let commit = samples(
        || {
            document.commit(mark.clone());
        },
        100,
    );
    println!("4K history commit p50 {:.3}, p95 {:.3}", commit.0, commit.1);
}
