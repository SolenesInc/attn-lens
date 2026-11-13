#!/bin/bash
# Drawing across app switches, against the installed Attn Lens: draw while another
# app is in front, ⌘Tab to a second app and draw, ⌘Tab back and draw. Each draw must
# flip the ink layer to `All` and leave a mark on a still that Send delivers. Posts
# real keys, so run it through `scripts/lens-vm.sh switch OUT`; on a real Mac, only
# in a window agreed with the user (LENS_DRIVER_WINDOW=agreed).
#   scripts/lens-switch.sh <output dir>
# Writes driver.log, pages.log and state.json. Exits non-zero on the first thing
# that does not hold.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:?usage: lens-switch.sh <output dir>}"
APP="$HOME/Applications/Attn Lens.app"
LENS="$APP/Contents/MacOS/AttnLens"
mkdir -p "$OUT"

/usr/bin/swiftc -O tools/driver/driver.swift -o "$OUT/driver"
/usr/bin/swiftc -O tools/driver/underlay.swift -o "$OUT/underlay"

PAGES=()
finish() {
    for page in "${PAGES[@]}"; do kill "$page" 2>/dev/null || true; done
    osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' || true
}
trap finish EXIT

osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' \
    -e 'repeat while application id "dev.attn.lens" is running' -e 'delay 0.1' -e 'end repeat'
open "$APP" --args --automation
until "$LENS" lens '{"cmd":"state"}' >/dev/null 2>&1; do sleep 0.1; done

# Two apps under the area, as a terminal and a browser are. The second sits over the
# first but leaves a strip of it on the left to click.
: >"$OUT/pages.log"
"$OUT/underlay" 40 60 800 500 terminal >>"$OUT/pages.log" &
PAGES+=($!)
until grep -q ready "$OUT/pages.log"; do sleep 0.1; done
"$OUT/underlay" 160 120 800 500 browser >>"$OUT/pages.log" &
PAGES+=($!)
until [ "$(grep -c ready "$OUT/pages.log")" -eq 2 ]; do sleep 0.1; done

cat >"$OUT/script.txt" <<'SCRIPT'
mark start
# Order the apps: terminal in front, browser behind it, Lens behind both.
click 500 300
wait 300
click 100 300
wait 300

watch-window Attn Lens
tap rcmd
await-window
expect "picking":[
move 200 200
wait 100
drag 200 200 700 500 10 16
expect "bar":true
mark picked

down fn
expect "input":"All"
wait 300
drag 300 260 500 260 8 16
up fn
expect "input":"None"
mark drawn-terminal

chord cmd tab
wait 1000
mark switched-browser
down fn
expect "input":"All"
wait 300
drag 300 340 500 340 8 16
up fn
expect "input":"None"
mark drawn-browser

chord cmd tab
wait 1000
mark switched-terminal
down fn
expect "input":"All"
wait 300
drag 300 420 500 420 8 16
up fn
expect "input":"None"
mark drawn-terminal-again

wait 1000
lens {"cmd":"state"}
chord cmd+opt return
expect "state":"sent"
expect "capture":null
mark sent
SCRIPT

"$OUT/driver" --front "Attn Lens" --front underlay "$OUT/script.txt" | tee "$OUT/driver.log"

grep -F 'lens {"cmd":"state"} -> ' "$OUT/driver.log" | tail -1 | sed 's/.* -> //' >"$OUT/state.json"
# Each ⌘Tab brought the other page to the front before the next draw, and each draw
# is a still of its own with its mark.
/usr/bin/python3 -I - "$OUT/driver.log" "$OUT/pages.log" "$OUT/state.json" <<'PY'
import json, sys
driver, pages, state = sys.argv[1:]
marks = {}
for line in open(driver):
    t, _, text = line.partition(" ")
    if text.startswith("mark "):
        marks[text.split()[1]] = int(t)
active = []
for line in open(pages):
    t, _, text = line.partition(" ")
    if text.startswith("underlay active "):
        active.append((int(t), text.split()[2]))
def front(after, before):
    names = [name for t, name in active if marks[after] < t < marks[before]]
    return names[-1] if names else None
for after, before, want in [
    ("drawn-terminal", "drawn-browser", "browser"),
    ("drawn-browser", "drawn-terminal-again", "terminal"),
]:
    got = front(after, before)
    if got != want:
        sys.exit(f"after {after}, ⌘Tab should bring {want} to the front; it brought {got}")
stills = json.load(open(state))["state"]["capture"]["stills"]
print("stills:", [(s["cause"], s["marks"]) for s in stills])
drawn = [s for s in stills if s["cause"] == "draw" and s["marks"] == 1]
if len(drawn) != 3:
    sys.exit(f"expected 3 draw stills with one mark each, found {len(drawn)}")
PY
sent="$(ls -td "$HOME/lens-check/sent"/*/ | head -1)"
ls "$sent"
