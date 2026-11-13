#!/bin/bash
# The driven half of the round trip, against the installed Attn Lens: hovering the
# tile holds it past its 5 s, a real click on it reopens the capture, and coming
# Back from the editor is not counted as a change on screen. Posts real pointer
# events, so run it through `scripts/lens-vm.sh tile OUT`.
#   scripts/lens-tile.sh <output dir>
# Writes driver.log, watch.log and receipts.txt. Exits non-zero on the first thing
# that does not hold.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:?usage: lens-tile.sh <output dir>}"
APP="$HOME/Applications/Attn Lens.app"
LENS="$APP/Contents/MacOS/AttnLens"
mkdir -p "$OUT"

/usr/bin/swiftc -O tools/driver/driver.swift -o "$OUT/driver"
/usr/bin/swiftc -O tools/driver/underlay.swift -o "$OUT/underlay"

UNDERLAYS=()
finish() {
    for page in "${UNDERLAYS[@]}"; do kill "$page" 2>/dev/null || true; done
    osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' || true
}
trap finish EXIT

page() {
    local log="$OUT/underlay-${#UNDERLAYS[@]}.log"
    "$OUT/underlay" "$@" >"$log" &
    UNDERLAYS+=($!)
    until grep -q ready "$log"; do sleep 0.1; done
}
drive() {
    "$OUT/driver" --front "Attn Lens" --front underlay "$1" | tee -a "$OUT/driver.log"
}
watch_lines() { wc -l <"$OUT/watch.log" | tr -d ' '; }

osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' \
    -e 'repeat while application id "dev.attn.lens" is running' -e 'delay 0.1' -e 'end repeat'
: >"$OUT/driver.log"
: >"$OUT/watch.log"
open "$APP" --env LENS_WATCH_LOG=1 --stderr "$OUT/watch.log" --args --automation
until "$LENS" lens '{"cmd":"state"}' >/dev/null 2>&1; do sleep 0.1; done
page 60 60 800 500

# tile::placement, in global top-left points: the tile's middle, clear of its ×.
read -r WIDTH HEIGHT < <(osascript -l JavaScript -e \
    'ObjC.import("AppKit"); var s = $.NSScreen.screens.objectAtIndex(0).frame.size; s.width + " " + s.height')
TILE_X=$((${WIDTH%.*} - 16 - 88))
TILE_Y=$((${HEIGHT%.*} - 16 - 64))

cat >"$OUT/tile.txt" <<SCRIPT
mark start
# Off the corner: a tile that appears under a resting pointer sees no hover until it moves.
move 300 300
wait 100
lens {"cmd":"pick","display":1,"rect":[100,150,500,300]}
expect "bar":true
tap rcmd
expect "cause":"hotkey"
chord cmd+opt return
expect "state":"sent"
expect "leaving":true
mark sent
# The tile has to draw before it has a region to hover; Rune sees hover only on movement.
wait 500
move $TILE_X $TILE_Y
expect "leaving":false
mark hovering
# Past the tile's 5 s, with the pointer resting on it.
wait 7000
expect "leaving":false
expect "sent":true
mark still-held
move 300 300
expect "leaving":true
mark left
wait 5500
expect "tile":null
mark gone

# A real click on the tile opens the capture again, its still with it.
lens {"cmd":"pick-last"}
expect "bar":true
tap rcmd
expect "cause":"hotkey"
chord cmd+opt return
expect "state":"sent"
expect "leaving":true
wait 500
# A person points before pressing; the tile takes the pointer once Regions has sampled it there.
move $TILE_X $TILE_Y
expect "leaving":false
wait 300
click $TILE_X $TILE_Y
expect "tile":null
expect "cause":"hotkey"
mark reopened
lens {"cmd":"state"}
chord cmd+opt return
expect "state":"sent"
lens {"cmd":"discard"}
expect "tile":null
SCRIPT
drive "$OUT/tile.txt"

# The whole display is the area, so the editor is on the watched screen.
cat >"$OUT/edit.txt" <<'SCRIPT'
lens {"cmd":"pick"}
expect "bar":true
wait 500
lens {"cmd":"still","cause":"hotkey"}
expect "cause":"hotkey"
lens {"cmd":"edit"}
expect "editing":
expect "editor":true
mark editing
wait 1000
SCRIPT
cat >"$OUT/back.txt" <<'SCRIPT'
lens {"cmd":"back"}
expect "editing":null
mark back
# Long past the 250 ms sampling and the 66 ms window poll.
wait 3000
lens {"cmd":"state"}
SCRIPT
drive "$OUT/edit.txt"
before="$(watch_lines)"
drive "$OUT/back.txt"
after_back="$(watch_lines)"
sed -n "$((before + 1)),${after_back}p" "$OUT/watch.log" >"$OUT/after-back.log"
# The sampler reports every frame; only CHANGE_FRACTION and up counts as a new screen.
if grep -Eq 'WindowSwitch|Click' "$OUT/after-back.log" ||
    sed -nE 's/^watch Change\(([0-9.e-]+)\)$/\1/p' "$OUT/after-back.log" | awk -v limit=0.6 '$1 >= limit { found = 1 } END { exit !found }'; then
    echo "Coming Back counted as a change on screen:" >&2
    cat "$OUT/after-back.log" >&2
    exit 1
fi

# A control: the same watch does see a window that really arrives.
page 200 200 600 400
seen() { sed -n "$((after_back + 1)),\$p" "$OUT/watch.log" | grep -q WindowSwitch; }
for _ in $(seq 1 200); do
    seen && break
    sleep 0.025
done
seen || {
    echo "After Back the watch saw no new window; its silence proves nothing." >&2
    exit 1
}
sed -n "$((after_back + 1)),\$p" "$OUT/watch.log" | tee "$OUT/control.log"

{
    grep ' mark ' "$OUT/driver.log"
    echo "largest change after Back: $(sed -nE 's/^watch Change\(([0-9.e-]+)\)$/\1/p' "$OUT/after-back.log" | sort -g | tail -1)"
    echo "window switches after the control window: $(grep -c WindowSwitch "$OUT/control.log")"
} >"$OUT/receipts.txt"
cat "$OUT/receipts.txt"
