#!/bin/bash
# The editor's Mac checks, run in the VM by `scripts/lens-vm.sh editor OUT`: native
# menus, Copy as OCR, Pick from screen, scroll direction, ⌘-scroll zoom about the pointer,
# and live light and dark. The driver posts real input, so this runs only in the VM.
# It reads the editor through its accessibility tree (tools/driver/ax.swift) and writes
# receipts.txt plus screenshots to OUT. Any failed check stops it with the reason.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:?usage: lens-editor-check.sh OUT}"
APP="$HOME/Applications/Attn Lens.app"
mkdir -p "$OUT"
/usr/bin/swiftc -O tools/driver/driver.swift -o "$OUT/driver"
/usr/bin/swiftc -O tools/driver/ax.swift -o "$OUT/ax"
/usr/bin/swiftc -O tools/vm/editor-fixture.swift -o "$OUT/fixture"
RECEIPTS="$OUT/receipts.txt"
: >"$RECEIPTS"

say() { echo "$*" | tee -a "$RECEIPTS"; }
fail() {
    say "FAIL: $*"
    exit 1
}
drive() { # one driver script per step, from stdin
    cat >"$OUT/step.txt"
    "$OUT/driver" --front "${FRONT:-Attn Lens}" "$OUT/step.txt" >>"$OUT/driver.log" ||
        fail "driver: $(tail -1 "$OUT/driver.log")"
}
tree() { "$OUT/ax" tree "Attn Lens" >"$OUT/tree.jsonl" && cat "$OUT/tree.jsonl"; }
# field IDENTIFIER KEY: one attribute of the element with that identifier, from a fresh tree.
field() {
    tree | /usr/bin/python3 -I -c '
import json, sys
for line in sys.stdin:
    row = json.loads(line)
    if row.get("identifier") == sys.argv[1]:
        value = row.get(sys.argv[2])
        print(" ".join(map(str, value)) if isinstance(value, list) else value)
        break' "$1" "$2"
}
center() { field "$1" frame | awk '{ printf "%d %d", $1 + $3 / 2, $2 + $4 / 2 }'; }
# Accessibility starts when a client first asks, so the first reads can come back empty.
# 100 × 0.1 s is far past the one empty read the first run saw.
wait_for() { # IDENTIFIER KEY EXPECTED
    for _ in $(seq 1 100); do
        [ "$(field "$1" "$2")" = "$3" ] && return
        sleep 0.1
    done
    fail "$1 $2 is '$(field "$1" "$2")', expected '$3'"
}

osascript -e 'tell application "System Events" to tell appearance preferences to set dark mode to false'
# Changing the appearance can bring System Settings forward, over the editor.
osascript -e 'if application "System Settings" is running then tell application "System Settings" to quit'
osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' \
    -e 'repeat while application id "dev.attn.lens" is running' -e 'delay 0.1' -e 'end repeat'
open "$APP" --args --automation
until "$APP/Contents/MacOS/AttnLens" lens '{"cmd":"state"}' >/dev/null 2>&1; do sleep 0.1; done
# A Dock press in the first moments after launch was once lost, so wait for the menus,
# which Lens installs once its event loop runs. 100 × 0.1 s is past any launch seen.
for _ in $(seq 1 100); do
    "$OUT/ax" menus "Attn Lens" 2>/dev/null | grep -xF "File > Open Image…" >/dev/null && break
    sleep 0.1
done
drive <<<'dock Attn Lens'
wait_for editor role AXGroup
"$OUT/fixture" "$OUT/fixture.png" | tee -a "$RECEIPTS"
drive <<<'chord cmd v'
wait_for canvas title "Screenshot canvas, 800 by 400 pixels, 0 annotations"
fit="$(field zoom-label value)"
say "pasted: $(field canvas title), zoom $fit"

say "--- native menus"
"$OUT/ax" menus "Attn Lens" | tee "$OUT/menus.txt" | grep -v '^Apple' | tee -a "$RECEIPTS"
for item in "File > Open Image…" "File > Save PNG…" "Edit > Copy as OCR" "Edit > Paste Image" \
    "Edit > Undo" "Draw > Rectangle" "Zoom > Actual Size" "Help > Attn Lens Help"; do
    grep -qxF "$item" "$OUT/menus.txt" || fail "no menu item $item"
done
"$OUT/ax" menu "Attn Lens" Zoom "Actual Size"
wait_for zoom-label value "100%"
say "Zoom > Actual Size: zoom $(field zoom-label value)"
"$OUT/ax" menu "Attn Lens" Draw Rectangle
wait_for tool-rectangle value 1
say "Draw > Rectangle: tool-rectangle checked"
drive <<<'chord cmd 1'
wait_for zoom-label value "$fit"
say "⌘1 (Fit to Window): zoom $(field zoom-label value)"
for key in undo redo copy copy-ocr save open; do
    role="$(field "$key" role)"
    [ "$role" = AXButton ] || fail "toolbar $key is $role, expected AXButton"
done
say "toolbar actions are AXButton; tools are $(field tool-rectangle role)"

say "--- Copy as OCR"
pbcopy </dev/null
"$OUT/ax" menu "Attn Lens" Edit "Copy as OCR"
for _ in $(seq 1 100); do
    text="$(pbpaste)"
    [ -n "$text" ] && break
    sleep 0.1
done
say "clipboard: $text"
[ "$text" = "Attn Lens reads this line 42" ] || fail "OCR read '$text'"

say "--- Pick from screen"
"$OUT/ax" menu "Attn Lens" Draw "Freehand Drawing"
wait_for tool-pen value 1
read -r x y <<<"$(center opacity-dec)"
drive <<<"click $x $y"
wait_for opacity-field value 90
say "before: color $(field custom-color value), opacity $(field opacity-field value)"
read -r x y <<<"$(center pick-screen)"
drive <<<"click $x $y"
wait_for status value "Pick a screen color • Escape to cancel"
# The block fills the image's lower 250 of 400 rows; aim inside it.
read -r ix iy iw ih <<<"$(field image frame)"
px=$(awk "BEGIN { printf \"%d\", $ix + $iw / 2 }")
py=$(awk "BEGIN { printf \"%d\", $iy + $ih * 0.8 }")
drive <<<"move $px $py
wait 300
click $px $py"
wait_for custom-color value "#1F7A4D"
wait_for opacity-field value 90
say "after picking the block: color $(field custom-color value), opacity $(field opacity-field value)"

say "--- scroll and zoom"
read -r cx cy <<<"$(center canvas)"
before="$(field image frame)"
drive <<<"move $cx $cy
wait 200
scroll 30 1
scroll 30 2
scroll 30 2
scroll 0 4"
sleep 0.5
after="$(field image frame)"
say "wheel +90 px (AppKit: positive reveals what is above): image $before -> $after"
/usr/bin/python3 -I -c '
import sys
b, a = [list(map(float, s.split())) for s in sys.argv[1:3]]
sys.exit(0 if a[0] == b[0] and a[1] - b[1] == 90 and a[2:] == b[2:] else 1)' "$before" "$after" ||
    fail "a +90 wheel should move the image down 90 points"
before="$after"
zoom_before="$(field zoom-label value)"
drive <<<"move $cx $cy
wait 200
down cmd
scroll 25 1
scroll 25 2
scroll 0 4
up cmd"
sleep 0.5
after="$(field image frame)"
say "⌘-wheel +50 at ($cx,$cy): zoom $zoom_before -> $(field zoom-label value), image $before -> $after"
/usr/bin/python3 -I -c '
import math, sys
x, y = map(float, sys.argv[3:5])
b, a = [list(map(float, s.split())) for s in sys.argv[1:3]]
factor = a[2] / b[2]
held = all(abs((p - a[i]) / a[i + 2] - (p - b[i]) / b[i + 2]) < 1e-3 for i, p in ((0, x), (1, y)))
print(f"factor {factor:.4f}, expected {math.exp(50 * 0.008):.4f}; point under the pointer held: {held}")
sys.exit(0 if held and abs(factor - math.exp(50 * 0.008)) < 1e-3 else 1)' "$before" "$after" "$cx" "$cy" |
    tee -a "$RECEIPTS" || fail "⌘-wheel should zoom 1.49x about the pointer"

say "--- reference: the same wheel event in TextEdit"
seq 1 2000 >"$OUT/long.txt"
open -a TextEdit "$OUT/long.txt"
for _ in $(seq 1 100); do # TextEdit showed its window within 3 s in the first run.
    "$OUT/ax" tree TextEdit 2>/dev/null | grep AXScrollBar >/dev/null && break
    sleep 0.2
done
bar() { "$OUT/ax" tree TextEdit | /usr/bin/python3 -I -c '
import json, sys
bars = [json.loads(l) for l in sys.stdin if "AXScrollBar" in l]
print(max(bars, key=lambda b: b["frame"][3])["value"])'; }
read -r tx ty tw th <<<"$("$OUT/ax" tree TextEdit | head -1 | /usr/bin/python3 -I -c 'import json,sys; print(*json.load(sys.stdin)["frame"])')"
top="$(bar)"
FRONT=TextEdit drive <<<"move $((${tx%.*} + ${tw%.*} / 2)) $((${ty%.*} + ${th%.*} / 2))
wait 200
scroll -30 1
scroll -30 2
scroll -30 2
scroll 0 4"
sleep 0.5
moved="$(bar)"
say "TextEdit, wheel -90: vertical scroller $top -> $moved (it moved on, so its text moved up)"
/usr/bin/python3 -I -c 'import sys; sys.exit(0 if float(sys.argv[2]) > float(sys.argv[1]) else 1)' "$top" "$moved" ||
    fail "TextEdit did not scroll on, so the reference for the wheel's sign is missing"
osascript -e 'tell application "TextEdit" to quit saving no'

say "--- light and dark"
read -r sx sy sw sh <<<"$(field sidebar frame)"
region="${sx%.*},$((${sy%.*} + 450)),${sw%.*},100"
screencapture -x "$OUT/light.png"
screencapture -x -R "$region" "$OUT/light-sidebar.png"
osascript -e 'tell application "System Events" to tell appearance preferences to set dark mode to true'
sleep 1
screencapture -x "$OUT/dark.png"
screencapture -x -R "$region" "$OUT/dark-sidebar.png"
osascript -e 'tell application "System Events" to tell appearance preferences to set dark mode to false'
light="$(/sbin/md5 -q "$OUT/light-sidebar.png")"
dark="$(/sbin/md5 -q "$OUT/dark-sidebar.png")"
say "empty sidebar strip $region: light $light, dark $dark"
[ "$light" != "$dark" ] || fail "the sidebar did not change with the system appearance"
say "PASS"
