#!/bin/bash
# The capture key and the voice key against the installed Attn Lens, with TextEdit in
# front: right ⌘+C copies and opens nothing, right ⌥+e types é and starts no voice, a
# held right ⌘ opens nothing, a tap of right ⌘ opens the picker, and in a capture a
# held right ⌥ talks until release. Then, with Accessibility revoked, Lens opens the
# guide at launch and the keys do nothing, and once allowed it makes its taps. Right
# after `lens-vm.sh push` the first run can still find Lens trusted after the revoke
# (the VM's TCC keeps the fresh grant for a while); run it again. Posts real keys and edits TCC, so run it through
# `scripts/lens-vm.sh keys OUT`; never on a real Mac.
#   scripts/lens-keys.sh <output dir>
# Writes driver.log, lens.log (LENS_KEYS_LOG=1), states and guide.png. Exits non-zero
# on the first thing that does not hold.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:?usage: lens-keys.sh <output dir>}"
APP="$HOME/Applications/Attn Lens.app"
LENS="$APP/Contents/MacOS/AttnLens"
DB="/Library/Application Support/com.apple.TCC/TCC.db"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

/usr/bin/swiftc -O tools/driver/driver.swift -o "$OUT/driver"

quit_lens() {
    osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' \
        -e 'repeat while application id "dev.attn.lens" is running' -e 'delay 0.1' -e 'end repeat'
}
grant() {
    sudo sqlite3 "$DB" "INSERT OR REPLACE INTO access (service,client,client_type,auth_value,auth_reason,auth_version,flags) VALUES ('kTCCServiceAccessibility','dev.attn.lens',0,2,4,1,0);"
}
finish() {
    grant
    quit_lens || true
    osascript -e 'if application "TextEdit" is running then tell application "TextEdit" to quit saving no' || true
}
trap finish EXIT

quit_lens
open --env LENS_KEYS_LOG=1 --stderr "$OUT/lens.log" "$APP" --args --automation
until "$LENS" lens '{"cmd":"state"}' >/dev/null 2>&1; do sleep 0.1; done

printf 'copy me' >"$OUT/note.txt"
open -a TextEdit "$OUT/note.txt"
osascript -e 'tell application "TextEdit"' -e 'activate' \
    -e 'repeat until (count of windows) > 0' -e 'delay 0.1' -e 'end repeat' \
    -e 'set bounds of window 1 to {100, 100, 700, 500}' -e 'end tell'
pbcopy </dev/null

# Voice plays a fixture, so no Microphone prompt covers the screen.
WAV="$PWD/speech/tests/fixtures/short-phrases.wav"
cat >"$OUT/script.txt" <<SCRIPT
mark start
click 400 300
wait 300
# Right ⌘+C copies the text and opens nothing.
chord cmd a
wait 200
chord rcmd c
wait 800
lens {"cmd":"state"}
mark copied
# Right ⌥+e then e types é and starts no voice.
tap right
chord ropt e
tap e
wait 800
lens {"cmd":"state"}
mark accented
# A held right ⌘ opens nothing.
tap rcmd 700
wait 800
lens {"cmd":"state"}
mark held-command
# A tap of right ⌘ opens the picker.
watch-window Attn Lens
tap rcmd
await-window
expect "picking":[
mark picker
move 200 200
wait 100
drag 200 200 600 450 10 16
expect "bar":true
mark capture
lens {"cmd":"voice-wav","path":"$WAV"}
# A held right ⌥ talks until release.
down ropt
wait 1200
lens {"cmd":"state"}
mark holding
up ropt
wait 1500
lens {"cmd":"state"}
mark released
tap esc
SCRIPT

"$OUT/driver" --front "Attn Lens" --front TextEdit "$OUT/script.txt" | tee "$OUT/driver.log"
pbpaste >"$OUT/clipboard.txt"
osascript -e 'tell application "TextEdit" to get text of document 1' >"$OUT/textedit.txt"

/usr/bin/python3 -I - "$OUT" <<'PY'
import json, pathlib, sys
out = pathlib.Path(sys.argv[1])
states = {}
pending = None
for line in (out / "driver.log").read_text().splitlines():
    _, _, text = line.partition(" ")
    if text.startswith('lens {"cmd":"state"} -> '):
        pending = json.loads(text.split(" -> ", 1)[1])["state"]
    elif text.startswith("mark ") and pending is not None:
        states[text.split()[1]] = pending
        pending = None
(out / "states.json").write_text(json.dumps(states, indent=1))
def check(ok, what):
    print(("ok   " if ok else "FAIL ") + what)
    if not ok:
        sys.exit(1)
check((out / "clipboard.txt").read_text().strip() == "copy me", "right ⌘+C copied the text")
for mark in ["copied", "accented", "held-command"]:
    s = states[mark]
    check(not s["picking"] and s["capture"] is None, f"{mark}: no picker, no capture")
    check(s["voice"] == {"state": "off"}, f"{mark}: no voice")
check("é" in (out / "textedit.txt").read_text(), "right ⌥+e then e typed é")
check(states["holding"]["voice"]["state"] != "off", "a held right ⌥ runs voice: %s" % states["holding"]["voice"])
check(states["released"]["voice"]["state"] in ("off", "finishing"), "its release stops voice: %s" % states["released"]["voice"])
PY

# Without Accessibility the keys cannot be read, and the guide says so at launch. A
# grant then reaches the running app: the next tap opens the picker.
quit_lens
osascript -e 'if application "System Settings" is running then tell application "System Settings" to quit' || true
sudo sqlite3 "$DB" "DELETE FROM access WHERE service='kTCCServiceAccessibility' AND client='dev.attn.lens';"
sudo killall tccd
sleep 2
open --stderr "$OUT/lens-no-accessibility.log" "$APP" --args --automation
until "$LENS" lens '{"cmd":"state"}' >/dev/null 2>&1; do sleep 0.1; done
# The tap thread says so at once; the guide takes a moment to float beside Settings.
for _ in $(seq 100); do
    grep -q "allow Attn Lens in Accessibility" "$OUT/lens-no-accessibility.log" && break
    sleep 0.1
done
sleep 3
screencapture -x "$OUT/guide.png"
if ! grep -q "allow Attn Lens in Accessibility" "$OUT/lens-no-accessibility.log"; then
    echo "FAIL Lens did not say the keys cannot be read; tccd said:"
    log show --last 1m --predicate 'process == "tccd"' | grep -i 'attn lens\|dev.attn.lens' | tail -5
    exit 1
fi
echo "ok   without Accessibility, Lens says the keys cannot be read; see guide.png"
cat >"$OUT/grant.txt" <<'SCRIPT'
tap rcmd
wait 1500
lens {"cmd":"state"}
mark untrusted
SCRIPT
"$OUT/driver" --front "Attn Lens" --front "System Settings" "$OUT/grant.txt" | tee "$OUT/driver-untrusted.log"
grep -F '"picking":null' "$OUT/driver-untrusted.log" >/dev/null
echo "ok   without Accessibility, right ⌘ opens nothing"
grant
sudo killall tccd
# A switch flipped in System Settings tells running apps that trust changed, and
# AXIsProcessTrusted answers afresh; a database write alone does not, so post it.
notifyutil -p com.apple.accessibility.api
/usr/bin/swift -e 'import Foundation; DistributedNotificationCenter.default().postNotificationName(NSNotification.Name("com.apple.accessibility.api"), object: nil, userInfo: nil, deliverImmediately: true)'
# The tap thread checks for the grant every 5 s and then makes its taps. Whether
# they get events at once is for the real switch on a real Mac: here WindowServer
# keeps its answer for the running process, since the grant is a database write.
for _ in $(seq 100); do
    grep -q "the keys are read again" "$OUT/lens-no-accessibility.log" && break
    sleep 0.1
done
grep -q "the keys are read again" "$OUT/lens-no-accessibility.log"
echo "ok   once allowed, with no relaunch, Lens makes its taps"
# A tap made while untrusted makes macOS record an Input Monitoring denial, which
# then blocks the keys even with Accessibility. Lens makes none, so none is recorded.
[ -z "$(sudo sqlite3 "$DB" "SELECT 1 FROM access WHERE service='kTCCServiceListenEvent' AND client='dev.attn.lens' AND auth_value=0;")" ]
echo "ok   no Input Monitoring denial was recorded"
