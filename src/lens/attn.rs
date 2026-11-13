//! Attn Lens reaches attn only through the `attn` CLI on this Mac. Its `--json`
//! output is the whole contract: `attn send sessions`, `attn send pair` and
//! `attn send <session>`, pinned by attn's own stack test.
use serde::Deserialize;
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// The name the user sees when Lens asks to pair, and in attn's Settings → Paired senders.
pub const SENDER_NAME: &str = "Attn Lens";
#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "dev.attn.lens.sender";

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Session {
    pub id: String,
    pub label: String,
    pub agent: String,
    pub state: String,
    /// The id of the outpost the session runs on; absent for this Mac.
    #[serde(default)]
    pub host: Option<String>,
    /// The outpost's name; attn repeats the id here when it cannot find the outpost.
    #[serde(default)]
    pub host_label: Option<String>,
    #[serde(default)]
    pub last_focused_at: Option<String>,
}

impl Session {
    /// Where the session runs, for the user: "this Mac", the outpost's name, or
    /// when attn has no name for it, "outpost" and the id's first eight characters.
    pub fn host_label(&self) -> String {
        match (&self.host_label, &self.host) {
            // attn sends the id as the name when it cannot find the outpost.
            (Some(name), host) if !name.is_empty() && Some(name) != host.as_ref() => name.clone(),
            (_, Some(id)) => format!("outpost {}", id.chars().take(8).collect::<String>()),
            (_, None) => "this Mac".into(),
        }
    }

    /// The session's state in words; a state Lens does not know shows without underscores.
    pub fn state_label(&self) -> String {
        match self.state.as_str() {
            "waiting_input" => "waiting for you".into(),
            "pending_approval" => "needs approval".into(),
            "launching" => "starting".into(),
            other => other.replace('_', " "),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PairRequest {
    pub code: String,
}

/// attn's saved capture; Lens needs only to know it parsed.
#[derive(Clone, Debug, Deserialize)]
pub struct Sent {}

/// Why a call to attn did not succeed.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// Lens holds no token, or attn no longer knows it: the user must pair (again).
    NotPaired(String),
    /// Anything else, with attn's error text verbatim.
    Error(String),
}

/// Where Lens finds the CLI. A GUI app does not inherit the shell's PATH, so it
/// looks where attn installs itself; `ATTN_BIN` overrides.
pub fn locate() -> Result<PathBuf, Failure> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut candidates: Vec<PathBuf> = std::env::var_os("ATTN_BIN")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    candidates.extend([
        PathBuf::from("/Applications/attn.app/Contents/MacOS/attn"),
        home.join("Applications/attn.app/Contents/MacOS/attn"),
    ]);
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join("attn")));
    }
    candidates.extend(["/opt/homebrew/bin/attn", "/usr/local/bin/attn"].map(PathBuf::from));
    candidates.push(home.join(".local/bin/attn"));
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or_else(|| {
            Failure::Error(format!(
                "Attn Lens could not find the attn CLI. Looked in: {}. Install attn, or set ATTN_BIN to its path.",
                candidates
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

#[cfg(target_os = "macos")]
pub fn token() -> Option<String> {
    let bytes =
        security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, account()).ok()?;
    String::from_utf8(bytes)
        .ok()
        .filter(|t| !t.trim().is_empty())
}

#[cfg(target_os = "macos")]
pub fn store_token(token: &str) -> Result<(), Failure> {
    security_framework::passwords::set_generic_password(
        KEYCHAIN_SERVICE,
        account(),
        token.as_bytes(),
    )
    .map_err(|e| Failure::Error(format!("Paired, but the Keychain refused the token: {e}")))
}

#[cfg(target_os = "macos")]
pub fn forget_token() {
    let _ = security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, account());
}

#[cfg(target_os = "macos")]
/// One token per attn instance, so a test instance never overwrites the real pairing.
fn account() -> &'static str {
    static ACCOUNT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ACCOUNT.get_or_init(|| {
        std::env::var("ATTN_INSTANCE")
            .ok()
            .filter(|i| !i.is_empty())
            .unwrap_or_else(|| "default".into())
    })
}

/// Lists the sessions the paired sender can send to.
pub fn sessions(bin: &Path, token: &str) -> Result<Vec<Session>, Failure> {
    let out = run(bin, &["send", "sessions", "--token-stdin", "--json"], token)?;
    serde_json::from_str(out.trim()).map_err(|e| {
        Failure::Error(format!(
            "attn send sessions printed JSON Lens cannot read: {e}"
        ))
    })
}

/// Sends `files` and `message` to `session`.
pub fn send(
    bin: &Path,
    token: &str,
    session: &str,
    files: &[PathBuf],
    message: &str,
) -> Result<Sent, Failure> {
    let files: Vec<_> = files.iter().map(|f| f.to_string_lossy()).collect();
    let mut args = vec!["send", session, "--token-stdin", "--json"];
    for file in &files {
        args.extend(["--file", file]);
    }
    if !message.trim().is_empty() {
        args.extend(["-m", message]);
    }
    let out = run(bin, &args, token)?;
    serde_json::from_str(out.trim())
        .map_err(|e| Failure::Error(format!("attn send printed JSON Lens cannot read: {e}")))
}

/// Asks the user to pair Lens. `shown` gets the code attn shows beside Allow;
/// returns the token once the user allows, or attn's refusal.
pub fn pair(bin: &Path, shown: impl FnOnce(PairRequest)) -> Result<String, Failure> {
    let mut child = Command::new(bin)
        .args(["send", "pair", "--name", SENDER_NAME, "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure::Error(format!("Could not run {}: {e}", bin.display())))?;
    let mut lines = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
    let mut shown = Some(shown);
    let mut token = None;
    for line in lines.by_ref() {
        let Ok(line) = line else { break };
        if let Some(show) = shown.take() {
            match serde_json::from_str::<PairRequest>(&line) {
                Ok(request) => show(request),
                Err(e) => {
                    let _ = child.kill();
                    return Err(Failure::Error(format!(
                        "attn send pair printed a request Lens cannot read: {e}"
                    )));
                }
            }
        } else {
            #[derive(Deserialize)]
            struct Paired {
                token: String,
            }
            token = serde_json::from_str::<Paired>(&line).ok().map(|p| p.token);
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|e| Failure::Error(e.to_string()))?;
    match token {
        Some(token) if output.status.success() => Ok(token),
        _ => Err(Failure::Error(error_text(
            &output.stderr,
            output.status.code(),
        ))),
    }
}

fn run(bin: &Path, args: &[&str], token: &str) -> Result<String, Failure> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure::Error(format!("Could not run {}: {e}", bin.display())))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{token}");
    }
    let output = child
        .wait_with_output()
        .map_err(|e| Failure::Error(e.to_string()))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let text = error_text(&output.stderr, output.status.code());
    // attn's refusal of a token it does not know names the way back: pairing.
    if text.contains("attn send pair") {
        return Err(Failure::NotPaired(text));
    }
    Err(Failure::Error(text))
}

fn error_text(stderr: &[u8], code: Option<i32>) -> String {
    let text = String::from_utf8_lossy(stderr).trim().to_string();
    if text.is_empty() {
        format!(
            "attn exited with status {}",
            code.map_or("?".into(), |c| c.to_string())
        )
    } else {
        text
    }
}

/// The session Send starts on: the one the user last focused in attn, else the first.
pub fn default_session(sessions: &[Session]) -> Option<usize> {
    sessions
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.last_focused_at.as_deref().map(|at| (i, at)))
        .max_by(|a, b| a.1.cmp(b.1))
        .map(|(i, _)| i)
        .or((!sessions.is_empty()).then_some(0))
}

#[cfg(test)]
mod tests {
    //! The CLI contract, against a stand-in `attn` that prints what attn's
    //! stack test pins (cmd/attn/send_stack_test.go) and records its input.
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake(dir: &Path, body: &str) -> PathBuf {
        let bin = dir.join("attn");
        std::fs::write(
            &bin,
            format!("#!/bin/sh\nprintf '%s\\n' \"$*\" > \"$(dirname \"$0\")/args\"\n{body}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    #[test]
    fn sessions_read_the_token_from_stdin_and_parse_local_and_outpost_rows() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake(
            dir.path(),
            r#"read token; printf '%s' "$token" > "$(dirname "$0")/token"
echo '[{"agent":"claude","id":"s1","label":"local","last_focused_at":"2026-11-02T10:00:00Z","state":"idle"},{"agent":"claude","host":"7a35fcaf-012f-4a33-8239-1da4d324514f","id":"s2","label":"remote","state":"waiting_input"},{"agent":"claude","host":"7a35fcaf-012f-4a33-8239-1da4d324514f","host_label":"ai-sandbox","id":"s3","label":"named","state":"pending_approval"},{"agent":"claude","host":"0b1c2d3e-4f50-6172-8394-a5b6c7d8e9f0","host_label":"0b1c2d3e-4f50-6172-8394-a5b6c7d8e9f0","id":"s4","label":"gone","state":"idle"}]'"#,
        );
        let sessions = sessions(&bin, "tok-1").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("token")).unwrap(),
            "tok-1"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("args"))
                .unwrap()
                .trim(),
            "send sessions --token-stdin --json"
        );
        assert_eq!(sessions.len(), 4);
        // attn gives an outpost's id as `host`; the user sees its name, or a short form until attn sends one.
        let shown = |s: &Session| (s.host_label(), s.state_label());
        assert_eq!(shown(&sessions[0]), ("this Mac".into(), "idle".into()));
        assert_eq!(
            shown(&sessions[1]),
            ("outpost 7a35fcaf".into(), "waiting for you".into())
        );
        assert_eq!(
            shown(&sessions[2]),
            ("ai-sandbox".into(), "needs approval".into())
        );
        // An outpost attn cannot find comes with its id as the name.
        assert_eq!(
            shown(&sessions[3]),
            ("outpost 0b1c2d3e".into(), "idle".into())
        );
        assert_eq!(default_session(&sessions), Some(0));
    }

    #[test]
    fn send_passes_every_file_and_the_message_and_keeps_a_refusal_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("frame-01.png");
        let sheet = dir.path().join("sheet.png");
        std::fs::write(&file, b"png").unwrap();
        std::fs::write(&sheet, b"png").unwrap();
        let ok = fake(
            dir.path(),
            r#"echo '{"id":"cap","attachments":[{"id":"a","name":"frame-01.png","media_type":"image/png","bytes":3}]}'"#,
        );
        send(
            &ok,
            "tok",
            "s1",
            &[sheet.clone(), file.clone()],
            "look here",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("args"))
                .unwrap()
                .trim(),
            format!(
                "send s1 --token-stdin --json --file {} --file {} -m look here",
                sheet.display(),
                file.display()
            )
        );
        let closed = fake(dir.path(), "echo 'send: session s1 is closed' >&2; exit 1");
        match send(&closed, "tok", "s1", &[file], "") {
            Err(Failure::Error(text)) => assert_eq!(text, "send: session s1 is closed"),
            other => panic!("expected the CLI's error verbatim, got {other:?}"),
        }
    }

    #[test]
    fn a_token_attn_no_longer_knows_asks_to_pair_again() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake(
            dir.path(),
            "echo 'send sessions: the sender token matches no paired sender; it was unpaired or never allowed. Pair again with attn send pair --name <name>' >&2; exit 1",
        );
        assert!(matches!(sessions(&bin, "gone"), Err(Failure::NotPaired(_))));
    }

    #[test]
    fn pairing_shows_the_code_then_returns_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake(
            dir.path(),
            r#"echo '{"id":"p1","name":"Attn Lens","code":"0420","requested_at":"2026-11-02T10:00:00Z"}'
echo '{"sender":{"id":"x","name":"Attn Lens","created_at":"2026-11-02T10:00:00Z"},"token":"tok-new"}'"#,
        );
        let mut code = None;
        assert_eq!(pair(&bin, |r| code = Some(r.code)).unwrap(), "tok-new");
        assert_eq!(code.as_deref(), Some("0420"));
        let denied = fake(
            dir.path(),
            r#"echo '{"id":"p1","name":"Attn Lens","code":"0420","requested_at":"x"}'; echo 'send pair: the user denied Attn Lens' >&2; exit 1"#,
        );
        assert_eq!(
            pair(&denied, |_| {}),
            Err(Failure::Error(
                "send pair: the user denied Attn Lens".into()
            ))
        );
    }

    #[test]
    fn default_session_is_the_last_focused() {
        let row = |id: &str, at: Option<&str>| Session {
            id: id.into(),
            label: id.into(),
            agent: "claude".into(),
            state: "idle".into(),
            host: None,
            host_label: None,
            last_focused_at: at.map(Into::into),
        };
        assert_eq!(default_session(&[]), None);
        assert_eq!(default_session(&[row("a", None), row("b", None)]), Some(0));
        let rows = [
            row("a", Some("2026-11-02T09:00:00Z")),
            row("b", None),
            row("c", Some("2026-11-02T10:00:00Z")),
        ];
        assert_eq!(default_session(&rows), Some(2));
    }
}
