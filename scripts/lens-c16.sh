#!/bin/bash
# C16's Mac half, against the installed Attn Lens: 60 s of continuous drawing with the
# driver, voice from a looped fixture through the real speech helper, and a still every
# 5 s. Posts real keys, so run it through `scripts/lens-vm.sh c16 OUT`; a number from the
# VM is a VM number. Writes frames.jsonl (every presented overlay frame), samples.txt
# (one line a second: cpu time and rss of Lens and the helper), state.json, summary.txt.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:?usage: lens-c16.sh <output dir>}"
APP="$HOME/Applications/Attn Lens.app"
LENS="$APP/Contents/MacOS/AttnLens"
READY_LIMIT_SECONDS=1200
mkdir -p "$OUT"
rm -f "$OUT/frames.jsonl"
state() { "$LENS" lens '{"cmd":"state"}'; }

# Looped speech, past the 60 s drawn.
WAV="$(cd "$OUT" && pwd)/talk.wav"
/usr/bin/python3 - speech/tests/fixtures/short-phrases.wav "$WAV" <<'PY'
import sys, wave
with wave.open(sys.argv[1]) as source:
    params = source.getparams()
    audio = source.readframes(params.nframes)
with wave.open(sys.argv[2], "wb") as out:
    out.setparams(params)
    while out.tell() < params.framerate * 75:
        out.writeframes(audio)
PY

/usr/bin/swiftc -O tools/driver/driver.swift -o "$OUT/driver"
quit() {
    osascript -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' \
        -e 'repeat while application id "dev.attn.lens" is running' -e 'delay 0.1' -e 'end repeat'
}
quit
open --env "RUNE_OVERLAY_FRAME_LOG=$OUT/frames.jsonl" "$APP" --args --automation
until state >/dev/null 2>&1; do sleep 0.1; done
trap 'quit || true' EXIT

cat >"$OUT/open.txt" <<SCRIPT
dock Attn Lens
expect "editor":true
wait 500
chord cmd+opt x
expect "picking":[
lens {"cmd":"pick"}
expect "capture":{
lens {"cmd":"voice-wav","path":"$WAV"}
chord cmd+opt s
SCRIPT
"$OUT/driver" --front "Attn Lens" "$OUT/open.txt" | tee "$OUT/driver.log"

deadline=$((SECONDS + READY_LIMIT_SECONDS))
until state | grep -q '"listening"'; do
    [ "$SECONDS" -lt "$deadline" ] || { echo "voice not listening after ${READY_LIMIT_SECONDS}s" >&2; exit 1; }
    sleep 0.5
done

{
    echo "down fn"
    echo "wait 100"
    # A drag takes about 3 s in the VM, where the driver checks the front window before
    # each step: 20 of them is the 60 s capture.
    for i in $(seq 0 19); do
        y=$((60 + (i * 37) % 560))
        echo "drag 100 $y 900 $((y + 40)) 55 16"
        echo "wait 90"
        if [ $((i % 2)) = 1 ]; then echo 'lens {"cmd":"still","cause":"hotkey"}'; fi
    done
    echo "up fn"
} >"$OUT/draw.txt"

PID="$(pgrep -nx AttnLens)"
HELPER="$(pgrep -P "$PID" | head -1)"
sample() {
    while :; do
        echo "$(date +%s) $(ps -o time=,rss= -p "$PID" | xargs) $(ps -o time=,rss= -p "$HELPER" | xargs)"
        sleep 1
    done >"$OUT/samples.txt"
}
sample &
SAMPLER=$!
begin=$(wc -l <"$OUT/frames.jsonl")
"$OUT/driver" --front "Attn Lens" "$OUT/draw.txt" | tee -a "$OUT/driver.log"
end=$(wc -l <"$OUT/frames.jsonl")
kill "$SAMPLER"
state >"$OUT/state.json"

sed -n "$((begin + 1)),${end}p" "$OUT/frames.jsonl" >"$OUT/drawing.jsonl"
/usr/bin/python3 - "$OUT" <<'PY' | tee "$OUT/summary.txt"
import json, sys
out = sys.argv[1]
rows = [json.loads(l) for l in open(f"{out}/drawing.jsonl")]
def report(label, xs):
    xs = sorted(xs)
    pick = lambda q: xs[min(len(xs) - 1, int(q * len(xs)))]
    print(f"{label}: n={len(xs)} p50={pick(.5):.2f} p99={pick(.99):.2f} longest={xs[-1]:.2f} ms")
report("draw_frame", [r["ms"] for r in rows])
times = [r["t_ms"] for r in rows]
report("present interval", [b - a for a, b in zip(times, times[1:])])
def seconds(text):
    m, s = text.split(":")
    return int(m) * 60 + float(s)
samples = [l.split() for l in open(f"{out}/samples.txt") if len(l.split()) == 5]
span = int(samples[-1][0]) - int(samples[0][0])
for name, t, r in (("lens", 1, 2), ("speech helper", 3, 4)):
    used = seconds(samples[-1][t]) - seconds(samples[0][t])
    print(f"{name}: cpu {100 * used / span:.0f}% of a core over {span}s, rss peak {max(int(s[r]) for s in samples) // 1024} MB")
state = json.load(open(f"{out}/state.json"))["state"]
capture = state["capture"]
print(f"stills={len(capture['stills'])} segments={len(capture['segments'])} voice={state['voice']}")
PY
