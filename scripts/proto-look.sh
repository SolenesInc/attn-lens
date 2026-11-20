#!/usr/bin/env bash
# PROTOTYPE (s-pkv5v1, branch proto/lens-capture-look, not for merging).
# Renders the capture surfaces in each look over a light and a dark synthetic desktop
# (tools/proto-look/desktop-*.png, drawn with ImageMagick, no real capture), crops each
# surface, and lays them out as one contact sheet per option:
#   OUT/a-system.png  option (a): the system appearance (light and dark) with the kit
#   OUT/b-hud.png     option (b): always the dark kit, as a HUD
# usage: scripts/proto-look.sh OUT
set -uo pipefail
[ $# -eq 1 ] || { echo "usage: $0 OUT" >&2; exit 2; }
repo=$(cd "$(dirname "$0")/.." && pwd)
out=$1
cargo build --quiet --locked --manifest-path "$repo/Cargo.toml" || exit 2
bin=${CARGO_TARGET_DIR:-$repo/target}/debug/attn-lens
# The capture scenes list sessions through the stand-in attn: never this machine's.
export ATTN_BIN=$repo/tools/scenes/attn
mkdir -p "$out/parts"
# Each render saves settings into a directory of its own, never this machine's prefs.
root=$(mktemp -d)
trap 'rm -r -- "$root"' EXIT
font=$(fc-match -f '%{file}' sans 2>/dev/null) || font=

# window scene crop(WxH+X+Y in points on the 1440x900 desktop) label
surfaces=(
    "bar open 800x110+180+398 bar"
    "bar palette 800x420+180+398 palette, 8 sessions"
    "bar voice-listening 800x110+180+398 voice listening"
    "tile sent 220x160+1220+736 tile"
    "pill recording 560x100+440+806 pill"
)
failed=0
render() { # look theme desktop: appends each surface's crop to `parts`
    local look=$1 theme=$2 desk=$3
    for s in "${surfaces[@]}"; do
        local window scene crop label prefs full part w h x y
        read -r window scene crop label <<<"$s"
        full=$out/parts/$look-$theme-$desk-$window-$scene.png
        part=$out/parts/crop-$look-$theme-$desk-$window-$scene.png
        prefs=$(mktemp -d -p "$root")
        if ! LENS_LOOK=$look RUNE_PREFS_DIR=$prefs "$bin" --window "$window" --scene "$scene" \
            --theme "$theme" --scale 2 --desktop "$repo/tools/proto-look/desktop-$desk.png" \
            --size 1440x900 --at 1 --snap "$full" >/dev/null 2>"$root/err"; then
            echo "FAIL $look $theme $desk $window $scene:" >&2
            tail -5 "$root/err" >&2
            failed=1
            continue
        fi
        # Rendered at --scale 2, so the crop is in device pixels.
        IFS='x+' read -r w h x y <<<"$crop"
        local shown="$label · $desk desktop · $theme appearance"
        [ "$look" = hud ] && shown="$label · $desk desktop · HUD"
        magick "$full" -crop "$((w * 2))x$((h * 2))+$((x * 2))+$((y * 2))" +repage \
            -set label "$shown" "$part"
        parts+=("$part")
    done
}
sheet() { # name title
    montage -label '%l' ${font:+-font "$font"} -pointsize 22 -geometry '+16+16' -tile 5x \
        -background '#7d7d85' -title "$2" "${parts[@]}" "$out/$1.png" && echo "$out/$1.png"
}
parts=()
for theme in light dark; do for desk in light dark; do render system "$theme" "$desk"; done; done
sheet a-system "(a) system appearance, attn kit"
parts=()
# The HUD ignores the appearance; it renders once per desktop.
for desk in light dark; do render hud light "$desk"; done
sheet b-hud "(b) always-dark HUD, attn kit"
# The pick, side by side: the bar with its palette and the pill, in each look, over each desktop.
for surface in bar-palette pill-recording; do
    parts=()
    for desk in light dark; do
        parts+=("$out/parts/crop-system-light-$desk-$surface.png" "$out/parts/crop-system-dark-$desk-$surface.png"
            "$out/parts/crop-hud-light-$desk-$surface.png")
    done
    montage -label '%l' ${font:+-font "$font"} -pointsize 22 -geometry '+16+16' -tile 3x \
        -background '#7d7d85' -title "(a) light appearance · (a) dark appearance · (b) HUD" \
        "${parts[@]}" "$out/compare-$surface.png" && echo "$out/compare-$surface.png"
done
exit $failed
