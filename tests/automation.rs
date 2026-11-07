//! The automation socket, with the fake port behind it and a stand-in `attn`.
use attn_lens::{
    lens::{Lens, automation},
    platform_port::fake::FakePort,
};
use serde_json::Value;
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

struct Harness {
    socket: PathBuf,
}

impl Harness {
    fn new(dir: &Path, token: Option<&str>) -> Self {
        let lens = Lens::new(Arc::new(FakePort::new(vec![], token)));
        let socket = dir.join(format!("lens-{}.sock", token.unwrap_or("unpaired")));
        automation::listen(lens, &socket).unwrap();
        Self { socket }
    }
    fn ask(&self, request: &str) -> Value {
        serde_json::from_str(&automation::request(&self.socket, request).unwrap()).unwrap()
    }
    fn state(&self) -> Value {
        self.ask(r#"{"cmd":"state"}"#)["state"].clone()
    }
}

/// A stand-in that records its arguments and the PNGs as they were when sent.
fn stand_in(dir: &Path) -> PathBuf {
    let bin = dir.join("attn");
    std::fs::write(
        &bin,
        format!(
            r#"#!/bin/sh
cat > /dev/null
if [ "$2" = "sessions" ]; then echo '[{{"id":"s1","label":"one","agent":"claude","state":"idle"}}]'; exit 0; fi
if [ "$2" = "closed" ]; then echo 'send: session closed is closed' >&2; exit 1; fi
echo "$@" > "{dir}/args"
for arg in "$@"; do case "$arg" in *.png) cp "$arg" "{dir}/";; esac; done
echo '{{"id":"cap"}}'
"#,
            dir = dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[test]
fn a_capture_is_started_read_and_sent_through_the_socket() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    // The only test in this binary, and no thread has started yet.
    unsafe { std::env::set_var("ATTN_BIN", stand_in(dir)) };
    let lens = Harness::new(dir, Some("tok"));

    assert_eq!(lens.state()["capture"], Value::Null);
    assert_eq!(lens.state()["input"], "None");

    assert_eq!(lens.ask(r#"{"cmd":"key","key":"capture"}"#)["ok"], true);
    assert_eq!(lens.state()["capture"]["area"]["rect"][2], 640.);
    assert_eq!(lens.ask(r#"{"cmd":"key","key":"draw-down"}"#)["ok"], true);
    assert_eq!(lens.state()["input"], "All");
    lens.ask(r#"{"cmd":"key","key":"draw-up"}"#);
    assert_eq!(lens.state()["input"], "None");

    assert_eq!(lens.ask(r#"{"cmd":"sessions"}"#)["sessions"][0]["id"], "s1");

    // A refused send keeps the capture and attn's words.
    assert_eq!(
        lens.ask(r#"{"cmd":"send","target":"closed","message":"hi"}"#)["ok"],
        true
    );
    let failed = lens.state();
    assert_eq!(failed["send"]["state"], "failed");
    assert_eq!(failed["send"]["error"], "send: session closed is closed");
    assert_eq!(failed["capture"]["stills"][0]["cause"], "send");

    // Sending again uses the stills it has, then closes the capture.
    lens.ask(r#"{"cmd":"key","key":"capture"}"#);
    assert_eq!(lens.state()["capture"]["stills"][1]["cause"], "hotkey");
    assert_eq!(
        lens.ask(r#"{"cmd":"send","target":"s1","message":"look here"}"#)["ok"],
        true
    );
    let sent = lens.state();
    assert_eq!(sent["send"]["state"], "sent");
    assert_eq!(sent["send"]["target"], "s1");
    assert_eq!(sent["capture"], Value::Null);
    let args = std::fs::read_to_string(dir.join("args")).unwrap();
    assert!(
        args.starts_with("send s1 --token-stdin --json --file "),
        "{args}"
    );
    assert!(args.ends_with("-m look here\n"), "{args}");
    for frame in ["frame-01.png", "frame-02.png"] {
        let image = image::open(dir.join(frame)).unwrap();
        assert_eq!((image.width(), image.height()), (1280, 800));
    }

    // Requests that cannot work say what is missing.
    let no_capture = lens.ask(r#"{"cmd":"send","target":"s1"}"#);
    assert_eq!(no_capture["ok"], false);
    assert!(no_capture["error"].as_str().unwrap().contains("⌘⌥X"));
    assert!(
        lens.ask(r#"{"cmd":"nope"}"#)["error"]
            .as_str()
            .unwrap()
            .contains("state, key, still")
    );
    assert!(
        lens.ask(r#"{"cmd":"key","key":"nope"}"#)["error"]
            .as_str()
            .unwrap()
            .contains("draw-down")
    );

    // Without a pairing, a send fails and says so.
    let unpaired = Harness::new(dir, None);
    unpaired.ask(r#"{"cmd":"key","key":"capture"}"#);
    unpaired.ask(r#"{"cmd":"send","target":"s1"}"#);
    assert_eq!(
        unpaired.state()["send"]["error"],
        "Attn Lens is not paired with attn."
    );
}
