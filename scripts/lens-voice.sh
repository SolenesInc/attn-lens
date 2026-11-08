#!/bin/bash
# The driven voice check, against the installed Attn Lens: ⌘⌥S plays a fixture WAV
# through the real speech helper in place of the microphone, and the socket shows
# each phrase on the capture clock beside a still taken during it. Posts real keys,
# so run it through `scripts/lens-vm.sh voice OUT`; on a real Mac, only in a window
# agreed with the user (LENS_DRIVER_WINDOW=agreed).
#   scripts/lens-voice.sh <output dir> [fixture name]
# Writes driver.log, voice-states.txt (what the bar showed while the model came
# ready), state.json and segments.txt. The first run downloads the model (731 MB).
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:?usage: lens-voice.sh <output dir> [fixture]}"
FIXTURE="${2:-short-phrases}"
WAV="$PWD/speech/tests/fixtures/$FIXTURE.wav"
TRUTH="$PWD/speech/tests/fixtures/$FIXTURE.json"
APP="$HOME/Applications/Attn Lens.app"
LENS="$APP/Contents/MacOS/AttnLens"
# Past the slowest download seen on a home line (731 MB at 2 MB/s is 6 min).
READY_LIMIT_SECONDS=1200
mkdir -p "$OUT"
state() { "$LENS" lens '{"cmd":"state"}'; }

/usr/bin/swiftc -O tools/driver/driver.swift -o "$OUT/driver"
osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' \
    -e 'repeat while application id "dev.attn.lens" is running' -e 'delay 0.1' -e 'end repeat'
open "$APP" --args --automation
until state >/dev/null 2>&1; do sleep 0.1; done
trap 'osascript -e "tell application id \"dev.attn.lens\" to quit" || true' EXIT

# The driver posts keys only while a Lens window is in front, so the editor opens first.
cat >"$OUT/open.txt" <<SCRIPT
dock Attn Lens
expect "editor":true
wait 500
watch-window Attn Lens
chord cmd+opt x
await-window
expect "picking":[
lens {"cmd":"pick"}
expect "capture":{
lens {"cmd":"voice-wav","path":"$WAV"}
mark voice-pressed
chord cmd+opt s
SCRIPT
"$OUT/driver" --front "Attn Lens" "$OUT/open.txt" | tee "$OUT/driver.log"

# The model downloads and loads before audio starts. Each change to Lens's state
# wakes the wait; note each voice state the bar showed until it listens.
: >"$OUT/voice-states.txt"
deadline=$((SECONDS + READY_LIMIT_SECONDS))
generation=0
began=
last=
while :; do
    left=$((deadline - SECONDS))
    [ "$left" -gt 0 ] || { echo "voice not listening after ${READY_LIMIT_SECONDS}s" >&2; exit 1; }
    reply="$("$LENS" lens "{\"cmd\":\"wait\",\"since\":$generation,\"timeout_ms\":$((left * 1000))}")"
    read -r generation failed now <<<"$(/usr/bin/python3 -c '
import json, sys
reply = json.loads(sys.argv[1])
if not reply["ok"]:
    sys.exit(reply["error"])
state = reply["state"]
print(reply["generation"], int(state["error"] is not None),
      json.dumps(state["voice"], separators=(",", ":")))' "$reply")"
    [ "$now" = "$last" ] || printf '%s %s\n' "$SECONDS" "$now" >>"$OUT/voice-states.txt"
    last="$now"
    case "$now" in
    *'"listening"'*) break ;;
    # ⌘⌥S may take a moment to arrive; off once voice has begun, or with an
    # error, means it failed.
    *'"off"'*) [ -z "$began" ] && [ "$failed" = 0 ] || { echo "voice stopped: $reply" >&2; exit 1; } ;;
    *) began=1 ;;
    esac
done
printf '%s listening\n' "$SECONDS" >>"$OUT/voice-states.txt"

AUDIO_MS="$(/usr/bin/python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["audio_ms"])' "$TRUTH")"
cat >"$OUT/talk.txt" <<SCRIPT
mark listening
wait $((AUDIO_MS / 2))
mark still
chord cmd+opt x
expect "cause":"hotkey"
wait $((AUDIO_MS / 2 + 500))
mark voice-stopped
chord cmd+opt s
expect "voice":{"state":"off"}
lens {"cmd":"state"}
SCRIPT
"$OUT/driver" --front "Attn Lens" "$OUT/talk.txt" | tee -a "$OUT/driver.log"
state >"$OUT/state.json"

# Phrases against the truth, on the capture clock: each start and end after the
# first phrase's start, beside the still's time on the same clock.
/usr/bin/python3 - "$OUT/state.json" "$TRUTH" <<'PY' | tee "$OUT/segments.txt"
import json, sys
capture = json.load(open(sys.argv[1]))["state"]["capture"]
phrases = json.load(open(sys.argv[2]))["phrases"]
segments = capture["segments"]
started = capture["started"]
print(f"capture started at {started} ns")
for still in capture["stills"]:
    print(f"still {still['id']} ({still['cause']}) at +{(still['at'] - started) / 1e6:.0f} ms")
assert len(segments) == len(phrases), f"{len(segments)} segments for {len(phrases)} phrases"
zero = segments[0]["start"] - phrases[0]["start_ms"] * 1_000_000
worst = 0
for segment, phrase in zip(segments, phrases):
    start = (segment["start"] - zero) / 1e6 - phrase["start_ms"]
    end = (segment["end"] - zero) / 1e6 - phrase["end_ms"]
    worst = max(worst, abs(start), abs(end))
    print(f"+{(segment['start'] - started) / 1e6:.0f}..+{(segment['end'] - started) / 1e6:.0f} ms "
          f"{segment['text']!r}: start {start:+.0f} ms, end {end:+.0f} ms against the WAV")
assert worst <= 300, f"a phrase is {worst:.0f} ms from its WAV time"
print(f"every phrase within {worst:.0f} ms of its place in the WAV")
PY
