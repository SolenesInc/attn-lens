#!/bin/sh
# Build this checkout, install it as ~/Applications/Attn Lens.app and relaunch it.
set -eu
cd "$(dirname "$0")/.."
DEST="$HOME/Applications/Attn Lens.app"

if [ -z "${GLANCE_CODESIGN_IDENTITY:-}" ]; then
    GLANCE_CODESIGN_IDENTITY="$(security find-identity -v -p codesigning \
        | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -n 1)"
fi
if [ "$GLANCE_CODESIGN_IDENTITY" = '-' ]; then
    printf 'Refusing ad-hoc signing: every reinstall would lose Screen Recording permission and the attn pairing.\n' >&2
    exit 1
fi
export GLANCE_CODESIGN_IDENTITY

GLANCE_BUNDLE_DEST="$DEST" ./scripts/bundle.sh release
osascript \
    -e 'if application id "dev.attn.lens" is running then tell application id "dev.attn.lens" to quit' \
    -e 'repeat while application id "dev.attn.lens" is running' -e 'delay 0.1' -e 'end repeat'
open "$DEST"
printf 'Installed and opened %s\n' "$DEST"
