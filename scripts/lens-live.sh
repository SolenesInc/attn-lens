#!/usr/bin/env bash
# Drives a real Attn Lens on a real display through the orders that once broke
# it, using the automation socket and a stand-in `attn`:
#   reopen      capture, close, reopen from the tile
#   recapture   capture, close, capture again
#   retile      capture, mark, close, reopen, close (replaces the tile window)
#   covers      the overlay covers its whole display, menu bar included
#   twice       a second capture while one is being taken does nothing
#   pair-esc    pairing finished after Esc still stores the token
#   unlisted    after attn fails to list sessions, ↩ lists them again
#   send-esc    a send finished after Esc marks the tile sent
# Each sequence starts its own Lens and fails on a call that stops answering
# (HANG), a process that exits (DIED), a wrong state (WRONG) or a new crash
# report, which is copied into the output directory.
#
# Needs a signed bundle with Screen Recording access (./scripts/bundle.sh debug).
# The overlay covers the display under the pointer for a few seconds per step.
# The token lives in the Keychain under a throwaway account, removed at the end.
# Usage: scripts/lens-live.sh [out-dir] [app path]
set -uo pipefail
OUT=${1:-$(mktemp -d -t lens-live)}
APP=${2:-$(cd "$(dirname "$0")/.." && pwd)/target/Attn Lens.app}
LENS="$APP/Contents/MacOS/AttnLens"
INSTANCE=lens-live-$$
FAKE="$OUT/attn"
mkdir -p "$OUT" "$FAKE"
trap 'security delete-generic-password -s dev.attn.lens.sender -a "$INSTANCE" >/dev/null 2>&1' EXIT

# The stand-in answers the three calls Lens makes. Files in its folder steer it:
# list-fails makes `send sessions` fail, sessions.json is the list, and allow
# and deliver release a waiting `send pair` and `send <session>`.
cat >"$FAKE/attn" <<'EOF'
#!/bin/sh
dir=$(dirname "$0")
wait_for() { while [ ! -e "$dir/$1" ]; do sleep 0.1; done; }
case "$2" in
  sessions)
    read -r _
    if [ -e "$dir/list-fails" ]; then echo "send sessions: attn is not running" >&2; exit 1; fi
    cat "$dir/sessions.json" 2>/dev/null || echo '[]' ;;
  pair)
    echo '{"id":"p1","name":"Attn Lens","code":"4242","requested_at":"2026-11-03T00:00:00Z"}'
    wait_for allow
    echo '{"sender":{"id":"s","name":"Attn Lens","created_at":"2026-11-03T00:00:00Z"},"token":"tok-live"}' ;;
  *)
    read -r _
    wait_for deliver
    echo '{"id":"cap","attachments":[]}' ;;
esac
EOF
chmod +x "$FAKE/attn"

lens() { # one automation call, 10 s budget
  perl -e 'alarm 10; exec @ARGV' "$LENS" lens "$@" 2>&1
}

run() { # run <name> <step>...; a step is "label|args"
  local name=$1; shift
  local log="$OUT/$name.log" stamp="$OUT/.$name.start"
  touch "$stamp"
  env -i HOME="$HOME" USER="$USER" PATH=/usr/bin:/bin:/usr/sbin:/sbin TMPDIR="$TMPDIR" \
    RUST_BACKTRACE=1 ATTN_INSTANCE="$INSTANCE" ATTN_BIN="$FAKE/attn" \
    "$LENS" --automation >"$OUT/$name.stderr" 2>&1 &
  local pid=$!
  echo "pid $pid" >"$log"
  for _ in $(seq 1 20); do lens state >/dev/null && break; sleep 0.5; done
  local result=PASS reply="" code
  for step in "$@"; do
    local label=${step%%|*} args=${step#*|}
    case $label in
      wait) sleep "$args"; continue ;;
      sh) eval "$args"; echo "sh: $args" >>"$log"; continue ;;
      expect) # expect|<python expression over d, the last reply>
        if ! python3 -c 'import json,sys; d=json.loads(sys.argv[2]); sys.exit(0 if eval(sys.argv[1]) else 1)' "$args" "$reply"; then
          result="WRONG: expected $args"; break
        fi
        continue ;;
    esac
    reply=$(eval lens "$args"); code=$?
    echo "$label [$code] ${reply:0:300}" >>"$log"
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
SEND="overlay '{\"type\":\"send\"}'"
MARK="overlay '{\"type\":\"add_mark\",\"tool\":\"arrow\",\"points\":[[900,300],[700,200]]}'"
SESSIONS='[{"agent":"claude","id":"s1","label":"local","state":"idle"}]'
failed=0
run reopen "capture|capture" "wait|2" "close|$CLOSE" "wait|2" "reopen|reopen" "wait|2" "state|state" || failed=1
run recapture "capture|capture" "wait|2" "close|$CLOSE" "wait|2" "capture2|capture" "wait|2" "state|state" || failed=1
run retile "capture|capture" "wait|2" "mark|$MARK" "close|$CLOSE" "wait|3" "reopen|reopen" "wait|2" \
  "close2|$CLOSE" "wait|3" "state|state" || failed=1
run covers "capture|capture" "wait|2" "state|state" \
  "expect|d['overlay']['frame'] == [0, 0] + d['overlay']['display']" || failed=1
run twice "capture|capture" "capture2|capture" "expect|d['started'] is False" "wait|2" "state|state" \
  "expect|d['overlay'] is not None" || failed=1
run pair-esc "capture|capture" "wait|2" "send|$SEND" "wait|1" "state|state" \
  "expect|'4242' in d['overlay']['status']" "close|$CLOSE" "sh|touch $FAKE/allow" "wait|2" \
  "sh|security find-generic-password -s dev.attn.lens.sender -a $INSTANCE >/dev/null && echo stored >$FAKE/stored" \
  "capture2|capture" "wait|2" "state|state" "expect|d['overlay']['status'] == 'Ready'" || failed=1
[ -e "$FAKE/stored" ] || { echo "pair-esc: WRONG: the token was not in the Keychain"; failed=1; }
run unlisted "sh|touch $FAKE/list-fails" "capture|capture" "wait|2" "state|state" \
  "expect|d['overlay']['status'].startswith('Unlisted')" \
  "sh|rm $FAKE/list-fails; echo '$SESSIONS' >$FAKE/sessions.json" "send|$SEND" "wait|1" "state|state" \
  "expect|d['overlay']['status'] == 'Ready' and d['overlay']['target'] == 's1'" || failed=1
run send-esc "capture|capture" "wait|2" "send|$SEND" "state|state" \
  "expect|d['overlay']['status'] == 'Sending'" "close|$CLOSE" "wait|1" "state|state" \
  "expect|d['tile'] == {'sent_to': None}" "sh|touch $FAKE/deliver" "wait|2" "state|state" \
  "expect|d['overlay'] is None and d['tile'] == {'sent_to': 'local'}" || failed=1
echo "logs: $OUT"
exit $failed
