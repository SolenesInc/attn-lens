#!/bin/bash
# The driven half of the live capture check, against the installed Attn Lens: the
# picker, pass-through to a page, the keyboard's owner, drawing with the draw key,
# corner handles and Send. Posts real keys, so run it through
# `scripts/lens-vm.sh live OUT`; on a real Mac, only in a window agreed with the user
# (LENS_DRIVER_WINDOW=agreed).
#   scripts/lens-live.sh <output dir>
# Writes driver.log, underlay.log, receipts.txt and capture.mov. Sent frames land where
# the `attn` on PATH puts them. Exits non-zero on the first thing that does not hold.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:?usage: lens-live.sh <output dir>}"
APP="$HOME/Applications/Attn Lens.app"
LENS="$APP/Contents/MacOS/AttnLens"
mkdir -p "$OUT"

/usr/bin/swiftc -O tools/driver/driver.swift -o "$OUT/driver"
/usr/bin/swiftc -O tools/driver/underlay.swift -o "$OUT/underlay"

UNDERLAY=
RECORDER=
finish() {
    [ -z "$RECORDER" ] || kill -INT "$RECORDER" 2>/dev/null || true
    [ -z "$UNDERLAY" ] || kill "$UNDERLAY" 2>/dev/null || true
    osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' || true
}
trap finish EXIT

osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' \
    -e 'repeat while application id "dev.attn.lens" is running' -e 'delay 0.1' -e 'end repeat'
open "$APP" --args --automation
until "$LENS" lens '{"cmd":"state"}' >/dev/null 2>&1; do sleep 0.1; done

# The page sits under the whole area, in global top-left points.
"$OUT/underlay" 60 60 800 500 >"$OUT/underlay.log" &
UNDERLAY=$!
until grep -q ready "$OUT/underlay.log"; do sleep 0.1; done

screencapture -v -V 60 "$OUT/capture.mov" &
RECORDER=$!

cat >"$OUT/script.txt" <<'SCRIPT'
mark start
watch-window Attn Lens
chord cmd+opt x
await-window
expect "picking":[
tap esc
expect "picking":null
chord cmd+opt x
expect "picking":[
drag 100 150 600 450 10 16
expect "rect":[100.0,150.0,500.0,300.0]
expect "bar":true
expect "ink":true
expect "keys":"Bar"
mark pick-done

click 200 250
wait 200
scroll -40
wait 200
drag 150 250 450 250 10 16
wait 200
expect "input":"None"
mark passed-through

chord cmd+opt c
expect "keys":"Bar"
tap h
tap i
expect "message":"hi"
mark typed

down fn
expect "input":"All"
wait 30
drag 200 200 400 200 8 16
expect "marks":1
wait 30
down shift
drag 200 300 300 380 8 16
expect "marks":2
up shift
up fn
expect "input":"None"
mark drawn

chord cmd+opt x
expect "cause":"hotkey"
mark still

drag 592 442 642 482 8 16
expect "rect":[100.0,150.0,550.0,340.0]
mark handle-moved

lens {"cmd":"state"}
chord cmd+opt return
expect "state":"sent"
expect "capture":null
mark sent
lens {"cmd":"state"}
SCRIPT

"$OUT/driver" --front "Attn Lens" --front underlay "$OUT/script.txt" | tee "$OUT/driver.log"

# Each line the page must have heard while the draw key was up.
for heard in click scroll selected; do
    grep -q "underlay $heard" "$OUT/underlay.log" || {
        echo "The page never heard: $heard" >&2
        exit 1
    }
done
# Drawing and typing in the bar must not reach the page.
if grep -q "underlay typed" "$OUT/underlay.log"; then
    echo "Typing meant for the bar reached the page." >&2
    exit 1
fi

{
    grep 'window-visible' "$OUT/driver.log"
    grep ' mark ' "$OUT/driver.log"
} >"$OUT/receipts.txt"
cat "$OUT/receipts.txt"
