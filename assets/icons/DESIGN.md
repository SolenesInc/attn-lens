# Attn Lens app icon

`lens.png` is the production master: 1024 pixels square, a rounded macOS tile
with transparent margins. It is a close-up of attn's faceted smoked-obsidian
shorthair black cat with emerald-green eyes, framed inside neon viewfinder
corner brackets looking up at a glowing electric-cyan 6-blade optical aperture.
The left rim lighting, left brackets, and backdrop aura shift from attn's
magenta-violet to warm neon amber-gold so the two icons read as one family while
staying distinct at small Dock and Cmd+Tab sizes.

`examples/icon.rs` resizes the master into all ten standard macOS iconset PNGs
with premultiplied filtering to preserve alpha edges. `scripts/bundle.sh`
packages them into a hash-named Attn Lens ICNS resource.

Earlier artwork and its generation prompts are preserved in `archive/`.
