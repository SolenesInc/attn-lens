//! The editor against the rows of `TOOL_OPTIONS.md` and `docs/usage.md`. Each script in
//! `headless/editor/` drives the fake port and holds its own tree assertions; a row
//! with no script fails `every_row_has_a_script`.
use image::GenericImageView;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};

const SCRIPTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/headless/editor");

/// Rows covered once Backdrop, Animation and video export come back, in their own change.
const LATER: [&str; 4] = [
    "options-backdrop",
    "options-animation",
    "export-gif",
    "export-mp4",
];

/// Scripts for behaviour the docs describe in prose, not in a table.
const PROSE: [&str; 14] = [
    "lens-follows-drag",
    "lens-tile-held",
    "raster-follows",
    "transform-together",
    "color-drag-undo",
    "lens-source",
    "long-stroke",
    "nav-zoom",
    "color-picker",
    "clipboard",
    "file-dialogs",
    "menus",
    "wide-image",
    "editor-keys",
];

fn slug(row: &str) -> String {
    row.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// First column of the markdown table under the header starting with `header`.
fn rows(doc: &str, header: &str) -> Vec<String> {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(doc)).unwrap();
    let mut lines = text.lines();
    assert!(
        lines.any(|line| line.starts_with(&format!("| {header} |"))),
        "{doc} has no table headed {header}"
    );
    lines
        .skip(1)
        .take_while(|line| line.starts_with('|'))
        .map(|line| line.split('|').nth(1).unwrap().trim().to_string())
        .collect()
}

fn documented() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for row in rows("TOOL_OPTIONS.md", "Tool") {
        names.insert(format!("options-{}", slug(&row)));
    }
    for row in rows("docs/usage.md", "Tool") {
        names.insert(format!("tool-{}", slug(&row)));
    }
    for row in rows("docs/usage.md", "Format") {
        names.insert(format!("export-{}", slug(&row)));
    }
    names
}

#[test]
fn every_row_has_a_script() {
    let names = documented();
    assert_eq!(names.len(), 27, "{names:?}");
    for name in &names {
        let later = LATER.contains(&name.as_str());
        let exists = Path::new(SCRIPTS).join(format!("{name}.json")).exists();
        assert!(
            later != exists,
            "row {name}: script exists={exists}, later={later}"
        );
    }
}

fn fixture(dir: &Path) -> Vec<(&'static str, String)> {
    let wide = dir.join("wide-input.png");
    if !wide.exists() {
        image::RgbaImage::from_pixel(8400, 66, image::Rgba([90, 100, 120, 255]))
            .save(&wide)
            .unwrap();
    }
    let small = dir.join("small-input.png");
    if !small.exists() {
        image::RgbaImage::from_pixel(64, 48, image::Rgba([20, 200, 90, 255]))
            .save(&small)
            .unwrap();
    }
    let large = dir.join("large-input.png");
    if !large.exists() {
        image::RgbaImage::from_pixel(1000, 700, image::Rgba([70, 90, 110, 255]))
            .save(&large)
            .unwrap();
    }
    std::fs::write(dir.join("not-an-image.txt"), "text").unwrap();
    vec![
        ("$FIXTURE", format!("{SCRIPTS}/fixture.png")),
        ("$LARGE", large.display().to_string()),
        ("$WIDE_SMALL", small.display().to_string()),
        ("$WIDE", wide.display().to_string()),
        ("$OUT", dir.display().to_string()),
    ]
}

/// Walks the accessibility tree.
fn nodes(node: &Value, out: &mut Vec<Value>) {
    out.push(node.clone());
    for child in node["children"].as_array().into_iter().flatten() {
        nodes(child, out);
    }
}

/// True when every field in `expected` matches `actual`; numbers within 0.01.
fn subset(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Number(a), Value::Number(b)) => {
            (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 0.01
        }
        (Value::Object(a), Value::Object(b)) => a
            .iter()
            .all(|(key, value)| b.get(key).is_some_and(|other| subset(value, other))),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| subset(x, y))
        }
        _ => expected == actual,
    }
}

fn near(a: [u8; 4], b: &Value) -> bool {
    a.iter()
        .zip(b.as_array().unwrap())
        .all(|(x, y)| (*x as i64 - y.as_i64().unwrap()).abs() <= 3)
}

fn check_image(
    fail: &mut Vec<String>,
    image: &image::DynamicImage,
    expected: &Value,
    source: &image::DynamicImage,
) {
    let (width, height) = image.dimensions();
    if let Some(size) = expected["size"].as_array()
        && [width as u64, height as u64] != [size[0].as_u64().unwrap(), size[1].as_u64().unwrap()]
    {
        fail.push(format!("image is {width}×{height}, expected {size:?}"));
    }
    let xy = |p: &Value| (p[0].as_u64().unwrap() as u32, p[1].as_u64().unwrap() as u32);
    for pixel in expected["pixels"].as_array().into_iter().flatten() {
        let (x, y) = xy(pixel);
        let found = image.get_pixel(x, y).0;
        if !near(found, &pixel[2]) {
            fail.push(format!("pixel {x},{y} is {found:?}, expected {}", pixel[2]));
        }
    }
    for point in expected["changed"].as_array().into_iter().flatten() {
        let (x, y) = xy(point);
        if image.get_pixel(x, y) == source.get_pixel(x, y) {
            fail.push(format!("pixel {x},{y} did not change"));
        }
    }
    for point in expected["unchanged"].as_array().into_iter().flatten() {
        let (x, y) = xy(point);
        if image.get_pixel(x, y) != source.get_pixel(x, y) {
            fail.push(format!("pixel {x},{y} changed"));
        }
    }
}

/// Runs the script up to one mark and checks what it asserts there.
fn check(name: &str, script: &Path, dir: &Path, clipboard: &Path, expected: &Value) -> Vec<String> {
    let at = expected["at"].as_str().unwrap();
    let tree_path = dir.join(format!("{name}-{at}.json"));
    let run = Command::new(env!("CARGO_BIN_EXE_attn-lens"))
        .env("ATTN_LENS_FAKE_CLIPBOARD", clipboard)
        .args(["--script"])
        .arg(script)
        .args(["--window", "editor", "--scale", "1", "--at"])
        .arg(format!("mark:{at}"))
        .arg("--tree")
        .arg(&tree_path)
        .output()
        .unwrap();
    if !run.status.success() {
        return vec![format!(
            "{name} at {at}: {}",
            String::from_utf8_lossy(&run.stderr)
        )];
    }
    let tree: Value = serde_json::from_slice(&std::fs::read(&tree_path).unwrap()).unwrap();
    let elements = tree["elements"].as_array().unwrap();
    let element = |key: &str| elements.iter().find(|e| e["key"] == key);
    let mut a11y = vec![];
    nodes(&tree["accessibility"], &mut a11y);
    let node = |key: &str| a11y.iter().find(|n| n["key"] == key);
    let has_state = |key: &str, state: &str| {
        node(key).is_some_and(|n| {
            n["states"]
                .as_array()
                .is_some_and(|states| states.iter().any(|s| s == state))
        })
    };
    let mut fail = vec![];
    for key in expected["present"].as_array().into_iter().flatten() {
        if element(key.as_str().unwrap()).is_none() {
            fail.push(format!("{key} is missing"));
        }
    }
    for key in expected["absent"].as_array().into_iter().flatten() {
        if element(key.as_str().unwrap()).is_some() {
            fail.push(format!("{key} should be gone"));
        }
    }
    for key in expected["checked"].as_array().into_iter().flatten() {
        if !has_state(key.as_str().unwrap(), "checked") {
            fail.push(format!(
                "{key} should be checked: {:?}",
                node(key.as_str().unwrap())
            ));
        }
    }
    for key in expected["unchecked"].as_array().into_iter().flatten() {
        if !has_state(key.as_str().unwrap(), "unchecked") {
            fail.push(format!("{key} should be unchecked"));
        }
    }
    for (key, want) in expected["text"].as_object().into_iter().flatten() {
        let found = element(key).and_then(|e| e["text"].as_str());
        if found != want.as_str() && !(want == "" && found.is_none()) {
            fail.push(format!("{key} reads {found:?}, expected {want}"));
        }
    }
    for (key, want) in expected["value"].as_object().into_iter().flatten() {
        let found = node(key).and_then(|n| n["value"].as_str());
        if found != want.as_str() {
            fail.push(format!("{key} is {found:?}, expected {want}"));
        }
    }
    for (key, want) in expected["rect"].as_object().into_iter().flatten() {
        match element(key) {
            Some(found) if subset(want, &found["rect"]) => {}
            found => fail.push(format!(
                "{key} rect {:?}, expected {want}",
                found.map(|f| &f["rect"])
            )),
        }
    }
    if let Some(want) = expected["status_contains"].as_str() {
        let found = element("status")
            .and_then(|e| e["text"].as_str())
            .unwrap_or("");
        if !found.contains(want) {
            fail.push(format!(
                "status reads {found:?}, expected to contain {want:?}"
            ));
        }
    }
    if let Some(want) = expected["canvas"].as_str() {
        let found = node("canvas")
            .and_then(|n| n["name"].as_str())
            .unwrap_or("");
        if !found.contains(want) {
            fail.push(format!(
                "canvas reads {found:?}, expected to contain {want:?}"
            ));
        }
    }
    let state: Value = element("editor-state")
        .and_then(|e| serde_json::from_str(e["label"].as_str()?).ok())
        .unwrap_or(Value::Null);
    if !expected["state"].is_null() && !subset(&expected["state"], &state) {
        fail.push(format!("state is {state}, expected {}", expected["state"]));
    }
    if let Some(want) = expected["marks"].as_array() {
        let empty = vec![];
        let found = state["marks"].as_array().unwrap_or(&empty);
        if found.len() != want.len() {
            fail.push(format!(
                "{} marks, expected {}: {found:?}",
                found.len(),
                want.len()
            ));
        }
        for (index, (want, found)) in want.iter().zip(found).enumerate() {
            if !subset(want, found) {
                fail.push(format!("mark {index} is {found}, expected {want}"));
            }
        }
    }
    if let Some(want) = expected["points_over"].as_array() {
        let index = want[0].as_u64().unwrap();
        let least = want[1].as_u64().unwrap() as usize;
        let found = state["marks"][index as usize]["points"]
            .as_array()
            .map_or(0, Vec::len);
        if found <= least {
            fail.push(format!(
                "mark {index} has {found} points, expected over {least}"
            ));
        }
    }
    let source = image::open(format!("{SCRIPTS}/fixture.png")).unwrap();
    if !expected["png"].is_null() {
        let path = expected["png"]["path"].as_str().unwrap();
        match image::open(PathBuf::from(path)) {
            Ok(image) => check_image(&mut fail, &image, &expected["png"], &source),
            Err(error) => fail.push(format!("{path}: {error}")),
        }
    }
    if !expected["clipboard"].is_null() {
        match image::open(clipboard.join("image.png")) {
            Ok(image) => check_image(&mut fail, &image, &expected["clipboard"], &source),
            Err(error) => fail.push(format!("clipboard image: {error}")),
        }
    }
    if let Some(want) = expected["clipboard_text"].as_str() {
        let found = std::fs::read_to_string(clipboard.join("text.txt")).unwrap_or_default();
        if found != want {
            fail.push(format!("clipboard text is {found:?}, expected {want:?}"));
        }
    }
    fail.into_iter()
        .map(|message| format!("{name} at {at}: {message}"))
        .collect()
}

fn run_script(name: &str) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let clipboard = dir.join("clipboard");
    std::fs::create_dir(&clipboard).unwrap();
    let mut body = std::fs::read_to_string(format!("{SCRIPTS}/{name}.json")).unwrap();
    for (from, to) in fixture(dir) {
        body = body.replace(from, &to);
    }
    let parsed: Value = serde_json::from_str(&body).unwrap();
    let script = dir.join("script.json");
    std::fs::write(&script, &body).unwrap();
    let asserts = parsed["asserts"].as_array().unwrap();
    assert!(!asserts.is_empty(), "{name} asserts nothing");
    asserts
        .iter()
        .flat_map(|expected| check(name, &script, dir, &clipboard, expected))
        .collect()
}

#[test]
fn every_script_holds() {
    let names: Vec<String> = documented()
        .into_iter()
        .filter(|name| !LATER.contains(&name.as_str()))
        .chain(PROSE.iter().map(|name| name.to_string()))
        .collect();
    let queue = std::sync::Mutex::new(names);
    let failures = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..std::thread::available_parallelism().map_or(2, |n| n.get().min(8)) {
            scope.spawn(|| {
                loop {
                    let Some(name) = queue.lock().unwrap().pop() else {
                        return;
                    };
                    let failed = run_script(&name);
                    failures.lock().unwrap().extend(failed);
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_script_exists_only_for_a_row_or_a_named_purpose() {
    let known = documented();
    for entry in std::fs::read_dir(SCRIPTS).unwrap() {
        let name = entry.unwrap().path();
        if name.extension().is_some_and(|ext| ext == "json") {
            let stem = name.file_stem().unwrap().to_str().unwrap();
            assert!(
                known.contains(stem) || PROSE.contains(&stem),
                "{stem} is not run by any test"
            );
        }
    }
}
