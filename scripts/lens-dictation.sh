#!/bin/bash
# Dictation against the installed Attn Lens, with TextEdit in front the whole time and
# tools/vm/attn as attn: right ⌥ starts a dictation that plays a speech fixture, a key
# typed while the pill shows lands in TextEdit, right ⌥ stops it, and attn's refinement
# pastes at the cursor; then a second dictation where ↩ pastes the raw text without
# reaching TextEdit. The clipboard holds its earlier text after both. Posts real keys
# and writes the Keychain, so run it through `scripts/lens-vm.sh dictation OUT`; never
# on a real Mac.
#   scripts/lens-dictation.sh <output dir>
# Writes driver.log, lens.log (LENS_DICTATION_LOG=1), appeared.log, timings.json and
# dictation.mov. Exits non-zero on the first thing that does not hold.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:?usage: lens-dictation.sh <output dir>}"
APP="$HOME/Applications/Attn Lens.app"
LENS="$APP/Contents/MacOS/AttnLens"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
WAV="$PWD/speech/tests/fixtures/short-phrases.wav"

/usr/bin/swiftc -O tools/driver/driver.swift -o "$OUT/driver"

RECORDER=
WATCHER=
quit_lens() {
    osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' \
        -e 'repeat while application id "dev.attn.lens" is running' -e 'delay 0.1' -e 'end repeat'
}
finish() {
    [ -z "$RECORDER" ] || kill -KILL "$RECORDER" 2>/dev/null || true
    [ -z "$WATCHER" ] || kill "$WATCHER" 2>/dev/null || true
    quit_lens || true
    osascript -e 'if application "TextEdit" is running then tell application "TextEdit" to quit saving no' || true
}
trap finish EXIT

# Lens reads the token tools/vm/attn pairs with from its Keychain item.
security add-generic-password -U -A -s dev.attn.lens.sender -a default -w vm-stand-in-token

quit_lens
open --env LENS_DICTATION_LOG=1 --stderr "$OUT/lens.log" "$APP" --args --automation
until "$LENS" lens '{"cmd":"state"}' >/dev/null 2>&1; do sleep 0.1; done
"$LENS" lens "{\"cmd\":\"voice-wav\",\"path\":\"$WAV\"}" >/dev/null

: >"$OUT/note.txt"
open -a TextEdit "$OUT/note.txt"
osascript -e 'tell application "TextEdit"' -e 'activate' \
    -e 'repeat until (count of windows) > 0' -e 'delay 0.1' -e 'end repeat' \
    -e 'set bounds of window 1 to {100, 100, 900, 500}' -e 'end tell'
printf 'earlier clipboard' | pbcopy

# When TextEdit's text changes, in wall-clock milliseconds, to line up with lens.log.
/usr/bin/python3 -I - "$OUT/appeared.log" <<'PY' &
import subprocess, sys, time
last = None
with open(sys.argv[1], "w", buffering=1) as log:
    while True:
        text = subprocess.run(["osascript", "-e", 'tell application "TextEdit" to get text of document 1'],
                              capture_output=True, text=True).stdout.strip()
        if text != last:
            log.write(f"{int(time.time() * 1000)} {text!r}\n")
            last = text
        time.sleep(0.005)
PY
WATCHER=$!

# It must outlast the driven run.
screencapture -v -V 40 "$OUT/dictation.mov" &
RECORDER=$!

cat >"$OUT/script.txt" <<'SCRIPT'
mark start
click 500 300
wait 500
# Cold: the first dictation since launch starts the speech helper.
tap ropt
wait 1500
lens {"cmd":"state"}
mark recording
# The pill never takes the keyboard: this lands in TextEdit.
tap e
wait 7000
tap ropt
wait 400
lens {"cmd":"state"}
mark refining
wait 3000
lens {"cmd":"state"}
mark pasted
# Warm: ↩ while attn refines pastes the raw text, and TextEdit never sees the ↩.
tap ropt
wait 7500
tap ropt
wait 400
tap return
wait 1500
lens {"cmd":"state"}
mark entered
wait 3500
lens {"cmd":"state"}
mark done
SCRIPT

"$OUT/driver" --front TextEdit "$OUT/script.txt" | tee "$OUT/driver.log"
osascript -e 'tell application "TextEdit" to get text of document 1' >"$OUT/textedit.txt"
osascript -e 'tell application "System Events" to get name of first process whose frontmost is true' >"$OUT/front.txt"
pbpaste >"$OUT/clipboard.txt"
wait "$RECORDER" || true
RECORDER=

/usr/bin/python3 -I - "$OUT" <<'PY'
import json, pathlib, re, sys
out = pathlib.Path(sys.argv[1])
states, pending = {}, None
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
d = lambda mark: states[mark]["dictation"] or {}
check(d("recording").get("state") == "recording", "right ⌥ starts a dictation: %s" % d("recording"))
check(d("refining").get("state") == "refining", "right ⌥ again asks attn: %s" % d("refining"))
check(d("pasted").get("note") == "Pasted", "attn's text pasted: %s" % d("pasted"))
check(d("entered").get("note") == "Pasted as is", "↩ pasted the raw text: %s" % d("entered"))
text = (out / "textedit.txt").read_text().strip()
raw = d("entered")["text"]
check(text.startswith("eRefined: ") and text.endswith(raw) and "\n" not in text,
      "TextEdit holds e, the refined text, then the raw text, and no ↩: %r" % text)
check((out / "front.txt").read_text().strip() == "TextEdit", "TextEdit is still in front")
check((out / "clipboard.txt").read_text() == "earlier clipboard", "the clipboard holds its earlier text")

# Timings, from lens.log and appeared.log on one wall clock.
log = []
for line in (out / "lens.log").read_text().splitlines():
    m = re.match(r"Attn Lens dictation +[\d.]+ ms \(unix (\d+) ms\): (.*)", line)
    if m:
        log.append((int(m.group(1)), m.group(2)))
appeared = [(int(t), text) for t, text in (l.split(" ", 1) for l in (out / "appeared.log").read_text().splitlines())]
def at(pattern, after=0):
    return next(t for t, m in log if t >= after and re.search(pattern, m))
timings = {}
for name, start in [("cold", 0), ("warm", at("pasted") + 1)]:
    key = at(r"voice key: dictation \d+ starts", start)
    timings[name] = {
        "voice key to pill shown ms": at("pill shows", key) - key,
        "voice key to voice reporting ms": at("voice reports", key) - key,
    }
answered = at("attn answered")
landed = next(t for t, text in appeared if "Refined" in text)
timings["cold"]["attn answered to text in TextEdit ms"] = landed - answered
entered = at(r"settled, Pasted as is")
landed = next(t for t, text in appeared if raw in text and t >= entered)
timings["warm"]["↩ to text in TextEdit ms"] = landed - entered
(out / "timings.json").write_text(json.dumps(timings, indent=1))
print(json.dumps(timings, indent=1))
PY
