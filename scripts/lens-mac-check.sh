#!/bin/bash
# The Mac half of the foundation check, against the app `scripts/install.sh` installed.
# The driver posts real keys, so the caller says where: LENS_DRIVER_WINDOW=vm inside
# the VM (scripts/lens-vm.sh check), or =agreed in a window agreed with the user.
#   scripts/lens-mac-check.sh <session id to send to> <output dir>
# Writes driver.log and receipts.txt: the capture key (right ⌘) to the bar's window, then idle CPU and memory.
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET="${1:?usage: lens-mac-check.sh <session id> <output dir>}"
OUT="${2:?usage: lens-mac-check.sh <session id> <output dir>}"
APP="$HOME/Applications/Attn Lens.app"
LENS="$APP/Contents/MacOS/AttnLens"
mkdir -p "$OUT"

/usr/bin/swiftc -O tools/driver/driver.swift -o "$OUT/driver"

osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' \
    -e 'repeat while application id "dev.attn.lens" is running' -e 'delay 0.1' -e 'end repeat'
open "$APP" --args --automation
until "$LENS" lens '{"cmd":"state"}' >/dev/null 2>&1; do sleep 0.1; done
PID="$(pgrep -nx AttnLens)"

# Idle first: no capture has been open yet.
sleep 5
{
    echo "idle, 5 samples over 20 s, no capture open:"
    for _ in 1 2 3 4 5; do ps -o %cpu=,rss= -p "$PID"; sleep 4; done
} >"$OUT/receipts.txt"

"$LENS" lens '{"cmd":"sessions"}' >"$OUT/sessions.json"

cat >"$OUT/script.txt" <<SCRIPT
mark launched
expect "editor":false
dock Attn Lens
expect "editor":true
mark dock-opened-editor
wait 500
watch-window Attn Lens
tap rcmd
await-window
expect "picking":[
lens {"cmd":"pick"}
expect "capture":{
expect "bar":true
down fn
expect "input":"All"
up fn
expect "input":"None"
tap rcmd
expect "cause":"hotkey"
lens {"cmd":"state"}
lens {"cmd":"send","target":"$TARGET","message":"Attn Lens on Rune: foundation check."}
expect "state":"sent"
lens {"cmd":"state"}
SCRIPT
"$OUT/driver" --front "Attn Lens" "$OUT/script.txt" | tee "$OUT/driver.log"
grep 'window-visible' "$OUT/driver.log" >>"$OUT/receipts.txt"
cat "$OUT/receipts.txt"
