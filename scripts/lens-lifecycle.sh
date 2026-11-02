#!/usr/bin/env bash
# Opens and closes Lens windows in the orders that once hung or crashed it, on a
# real display, through the automation socket:
#   reopen    capture, close, reopen from the tile
#   recapture capture, close, capture again
#   retile    capture, mark, close, reopen, close (replaces the tile window)
# Each sequence starts its own Lens and fails on a call that stops answering
# (HANG) or a process that exits (DIED). Crash reports written during a run are
# copied into the output directory.
#
# Needs a signed bundle with Screen Recording access (./scripts/bundle.sh debug).
# The overlay covers the display under the pointer for a few seconds per step.
# Usage: scripts/lens-lifecycle.sh [out-dir] [app path]
set -uo pipefail
OUT=${1:-$(mktemp -d -t lens-lifecycle)}
APP=${2:-$(cd "$(dirname "$0")/.." && pwd)/target/Attn Lens.app}
LENS="$APP/Contents/MacOS/AttnLens"
mkdir -p "$OUT"

lens() { # one automation call, 10 s budget
  perl -e 'alarm 10; exec @ARGV' "$LENS" lens "$@" 2>&1
}

run() { # run <name> <step>...; a step is "label|args"
  local name=$1; shift
  local log="$OUT/$name.log" stamp="$OUT/.$name.start"
  touch "$stamp"
  env -i HOME="$HOME" USER="$USER" PATH=/usr/bin:/bin:/usr/sbin:/sbin TMPDIR="$TMPDIR" \
    RUST_BACKTRACE=1 "$LENS" --automation >"$OUT/$name.stderr" 2>&1 &
  local pid=$!
  echo "pid $pid" >"$log"
  for _ in $(seq 1 20); do lens state >/dev/null && break; sleep 0.5; done
  local result=PASS
  for step in "$@"; do
    local label=${step%%|*} args=${step#*|}
    if [ "$label" = wait ]; then sleep "$args"; continue; fi
    local reply code
    reply=$(eval lens "$args"); code=$?
    echo "$label [$code] ${reply:0:200}" >>"$log"
    if [ $code -eq 142 ] || [[ "$reply" == *"temporarily unavailable"* ]]; then result="HANG at $label"; break; fi
    if ! kill -0 $pid 2>/dev/null; then result="DIED at $label"; break; fi
  done
  kill -0 $pid 2>/dev/null && kill -9 $pid # the Lens this run started, and only it
  wait $pid 2>/dev/null
  sleep 3 # ReportCrash writes the report shortly after the process ends
  local reports
  reports=$(find ~/Library/Logs/DiagnosticReports -name "AttnLens-*.ips" -newer "$stamp")
  if [ -n "$reports" ]; then
    echo "$reports" | while read -r r; do cp "$r" "$OUT/$name-$(basename "$r")"; done
    [ "$result" = PASS ] && result="CRASHED"
    result="$result (crash report in $OUT)"
  fi
  echo "$name: $result"
  [ "$result" = PASS ]
}

CLOSE="overlay '{\"type\":\"close\"}'"
MARK="overlay '{\"type\":\"add_mark\",\"tool\":\"arrow\",\"points\":[[900,300],[700,200]]}'"
failed=0
run reopen "capture|capture" "wait|2" "close|$CLOSE" "wait|2" "reopen|reopen" "wait|2" "state|state" || failed=1
run recapture "capture|capture" "wait|2" "close|$CLOSE" "wait|2" "capture2|capture" "wait|2" "state|state" || failed=1
run retile "capture|capture" "wait|2" "mark|$MARK" "close|$CLOSE" "wait|3" "reopen|reopen" "wait|2" \
  "close2|$CLOSE" "wait|3" "state|state" || failed=1
echo "logs: $OUT"
exit $failed
