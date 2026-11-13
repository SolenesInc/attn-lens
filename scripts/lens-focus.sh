#!/bin/bash
# The keyboard after a capture ends, against the installed Attn Lens: type in TextEdit,
# capture, type in the bar, end the capture, then type again. The last text must land in
# TextEdit without a click, whichever way the capture ended. Posts real keys, so run it
# through `scripts/lens-vm.sh focus OUT`; on a real Mac, only in a window agreed with the user.
#   scripts/lens-focus.sh <output dir>
# Writes driver.log and one script per way out. Exits non-zero on the first thing that
# does not hold.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:?usage: lens-focus.sh <output dir>}"
APP="$HOME/Applications/Attn Lens.app"
LENS="$APP/Contents/MacOS/AttnLens"
mkdir -p "$OUT"

/usr/bin/swiftc -O tools/driver/driver.swift -o "$OUT/driver"

finish() {
    osascript -e 'tell application "TextEdit" to quit saving no' || true
    osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' || true
}
trap finish EXIT

osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' \
    -e 'repeat while application id "dev.attn.lens" is running' -e 'delay 0.1' -e 'end repeat'
open "$APP" --args --automation
until "$LENS" lens '{"cmd":"state"}' >/dev/null 2>&1; do sleep 0.1; done

# Plain text with no autocorrect, so what is typed is what is read back.
defaults write com.apple.TextEdit RichText -int 0
defaults write com.apple.TextEdit CorrectSpellingAutomatically -bool false
defaults write -g NSAutomaticTextCompletionEnabled -bool false
defaults write -g NSAutomaticSpellingCorrectionEnabled -bool false
defaults write -g NSAutomaticCapitalizationEnabled -bool false

# One TextEdit window under the area, in global top-left points.
osascript <<'APPLESCRIPT'
tell application "TextEdit"
    activate
    make new document
    set bounds of window 1 to {60, 60, 760, 560}
end tell
APPLESCRIPT
sleep 1

# A TextEdit that stops answering fails the check instead of hanging it.
text() { osascript -e 'with timeout of 15 seconds' -e 'tell application "TextEdit" to get text of document 1' -e 'end timeout'; }
clear_text() { osascript -e 'with timeout of 15 seconds' -e 'tell application "TextEdit" to set text of document 1 to ""' -e 'end timeout'; }

# The picker's Esc gives the keyboard back too.
picker_esc() {
    clear_text
    cat >"$OUT/picker-esc.txt" <<'SCRIPT'
mark start
click 300 300
wait 300
tap a
tap a
chord cmd+opt x
expect "picking":[
wait 500
tap esc
expect "picking":null
wait 1000
tap v
tap v
wait 500
SCRIPT
    "$OUT/driver" --front "Attn Lens" --front TextEdit "$OUT/picker-esc.txt" | tee -a "$OUT/driver.log"
    local got
    got="$(text)"
    echo "picker-esc: TextEdit holds \"$got\", wanted \"aavv\"" | tee -a "$OUT/results.txt"
    [ "$got" = "aavv" ] || { echo "FAIL picker-esc" | tee -a "$OUT/results.txt"; FAILED=1; }
}

# Types "aa" into TextEdit, captures over it, types "ii" into the bar, ends the capture
# with $2, then types "vv". "aaii" never reaches TextEdit; "aavv" is the pass.
way() {
    local name="$1" end="$2" before="${3:-}" want="${4:-aavv}"
    clear_text
    cat >"$OUT/$name.txt" <<SCRIPT
mark start
click 300 300
wait 300
tap a
tap a
chord cmd+opt x
expect "picking":[
move 200 200
wait 100
drag 200 200 700 500 10 16
expect "bar":true
wait 500
$before
tap i
tap i
$end
wait 1000
mark ended
tap v
tap v
wait 500
mark typed
SCRIPT
    "$OUT/driver" --front "Attn Lens" --front TextEdit "$OUT/$name.txt" | tee -a "$OUT/driver.log"
    local got
    got="$(text)"
    echo "$name: TextEdit holds \"$got\", wanted \"$want\"" | tee -a "$OUT/results.txt"
    [ "$got" = "$want" ] || { echo "FAIL $name" | tee -a "$OUT/results.txt"; FAILED=1; }
}

: >"$OUT/driver.log"
: >"$OUT/results.txt"
FAILED=0
picker_esc
way bar-return "tap return
expect \"state\":\"sent\"
expect \"capture\":null"
way hotkey-send "chord cmd+opt return
expect \"state\":\"sent\"
expect \"capture\":null"
way esc "tap esc
expect \"capture\":null"
# A click on the bar's message field, as the user gives it the keyboard with the mouse.
way bar-click "tap return
expect \"state\":\"sent\"
expect \"capture\":null" "click 600 534
wait 300"
# Ink first: the draw key puts the ink layer under the pointer, and Send follows it.
way draw-send "tap return
expect \"state\":\"sent\"
expect \"capture\":null" "down fn
expect \"input\":\"All\"
wait 300
drag 300 300 500 300 8 16
up fn
expect \"input\":\"None\"
wait 300"
# The editor is an ordinary window: it takes the app, so Send must give it back.
way editor-send "lens {\"cmd\":\"still\",\"cause\":\"hotkey\"}
lens {\"cmd\":\"edit\"}
wait 1500
chord cmd+opt return
expect \"state\":\"sent\"
expect \"capture\":null"
exit "$FAILED"
