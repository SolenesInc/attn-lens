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
const KEYCHAIN_SERVICE: &str = "dev.attn.lens.sender";

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Session {
    pub id: String,
    pub label: String,
    pub agent: String,
    pub state: String,
    /// The outpost the session runs on; absent for this Mac.
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub last_focused_at: Option<String>,
}

impl Session {
    pub fn host_label(&self) -> &str {
        self.host.as_deref().unwrap_or("this Mac")
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

pub fn token() -> Option<String> {
    let bytes =
        security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, account()).ok()?;
    String::from_utf8(bytes)
        .ok()
        .filter(|t| !t.trim().is_empty())
}

pub fn store_token(token: &str) -> Result<(), Failure> {
    security_framework::passwords::set_generic_password(
        KEYCHAIN_SERVICE,
        account(),
        token.as_bytes(),
    )
    .map_err(|e| Failure::Error(format!("Paired, but the Keychain refused the token: {e}")))
}

pub fn forget_token() {
    let _ = security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, account());
}

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

/// Sends `file` and `message` to `session`.
pub fn send(
    bin: &Path,
    token: &str,
    session: &str,
    file: &Path,
    message: &str,
) -> Result<Sent, Failure> {
    let file = file.to_string_lossy();
    let mut args = vec!["send", session, "--token-stdin", "--file", &file, "--json"];
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
echo '[{"agent":"claude","id":"s1","label":"local","last_focused_at":"2026-11-02T10:00:00Z","state":"idle"},{"agent":"claude","host":"ai-sandbox","id":"s2","label":"remote","state":"working"}]'"#,
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
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].host_label(), "this Mac");
        assert_eq!(sessions[1].host.as_deref(), Some("ai-sandbox"));
        assert_eq!(default_session(&sessions), Some(0));
    }

    #[test]
    fn send_passes_the_file_and_message_and_keeps_a_refusal_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("screenshot.png");
        std::fs::write(&file, b"png").unwrap();
        let ok = fake(
            dir.path(),
            r#"echo '{"id":"cap","attachments":[{"id":"a","name":"screenshot.png","media_type":"image/png","bytes":3}]}'"#,
        );
        send(&ok, "tok", "s1", &file, "look here").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("args"))
                .unwrap()
                .trim(),
            format!(
                "send s1 --token-stdin --file {} --json -m look here",
                file.display()
            )
        );
        let closed = fake(dir.path(), "echo 'send: session s1 is closed' >&2; exit 1");
        match send(&closed, "tok", "s1", &file, "") {
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
