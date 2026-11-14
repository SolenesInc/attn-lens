# Attn Lens app icon

`lens.png` is the production master: 1024 pixels square, a rounded macOS tile
with transparent margins. It is attn's cat, a black cat with neon circuit lines
in its ears on a magenta, violet and cyan aurora, holding a magnifying glass to
one eye. attn's own app icon is the same cat without the glass, so the two read
as one family in the Dock.

The artwork was generated with Codex image generation from attn's chosen logo
and masked to a superellipse tile. The prompts, the rounds the user chose from
and the generator scripts are attached to attn's seed `s-7eyt9h`
(make-new-attn-logo).

`examples/icon.rs` resizes the master into all ten standard macOS iconset PNGs
with premultiplied filtering to preserve alpha edges. `scripts/bundle.sh`
packages them into a hash-named Attn Lens ICNS resource.

Earlier artwork and its generation prompts are preserved in `archive/`.
