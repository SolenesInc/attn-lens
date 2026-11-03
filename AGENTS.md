# Working on Attn Lens

Attn Lens captures the screen, marks it up in place, and sends it to an attn
agent session through the `attn` CLI. It is a fork of Glance, whose editor it
keeps; the rest of this file describes Glance's rules, which still apply.

## Lens entry points

- `src/lens/mod.rs`: the ⌘⌥2 global shortcut, overlay and corner-tile lifecycle.
- `src/lens/overlay.rs`: the in-place Send surface. Pointer, keys and automation
  all dispatch `overlay::Action`; add new behavior there, not in input handlers.
- `src/lens/attn.rs`: the only contract with attn: `attn send sessions`, `attn
  send pair` and `attn send <session>` JSON, plus the Keychain token. Its tests
  use a stand-in `attn`; attn's `cmd/attn/send_stack_test.go` pins the real JSON.
- `src/lens/capture.rs`: stills through Hex's capture-core (ScreenCaptureKit).
- `src/permission_guide.rs`, `native/permission_guide.m`: from Hex (MIT).
- Lens automation: launch with `--automation`, then `AttnLens lens capture`,
  `lens state`, `lens overlay '<action JSON>'`. The overlay is not an MCP surface;
  agents send to sessions with `attn send` directly.
- Sign dev bundles with a stable identity (`GLANCE_CODESIGN_IDENTITY`): an
  ad-hoc rebuild silently invalidates the Screen Recording grant.
- `vendor/gpui` is GPUI 0.2.2 with three macOS window fixes, each marked
  `Attn Lens:` in `src/platform/mac/window.rs`: the key-status handler releases
  its state lock before `resignKeyWindow` re-enters it (without this, Lens hangs
  on reopening a capture); `PopUp` windows opened with `focus: false` use a
  panel class that never becomes key (the corner tile); and pop-ups keep their
  frame instead of being pushed below the menu bar (the overlay covers the whole
  display). Drop the vendored copy once upstream GPUI has all three.
- `scripts/lens-live.sh` drives a signed Lens on the real display through the
  window and send orders that once broke it, against a stand-in `attn`. Run it
  after changing window lifecycle, pairing, sending or GPUI.

# Glance

Glance is a native macOS and experimental Omarchy screenshot editor in Rust/GPUI. It edits one document,
exports PNG/GIF/MP4, shares images through glance.sh, and offers opt-in local MCP
control. Use Cargo and the existing Rust tooling; do not add a JavaScript toolchain.

## Entry points and architecture

- `src/main.rs`: startup and window creation.
- `src/editor/actions.rs` and `dispatch.rs`: common typed application actions;
  toolbar, menus, shortcuts, gestures, and MCP use this path.
- `src/document.rs` and `document/actions.rs`: image model, validated edits, undo.
- `src/editor/input.rs`, `view.rs`, `canvas.rs`, `panels/`: input and native UI.
- `src/editor/jobs.rs`: workers, operation IDs, stale-result rejection.
- `src/animation/preview.rs`, `preview/quality.rs`, `src/shaders/motion.metal`:
  worker-rendered shader previews, adaptive quality, periodic motion effects.
- `src/platform.rs`: platform capture, dialogs, fonts, and clipboard; Linux lives in `src/platform/linux.rs`.
- `src/glance.rs`: encrypted remote-sharing protocol.
- `src/mcp.rs`, `src/automation.rs`, `src/editor/automation.rs`: stdio MCP and IPC.
- `native/`: AVFoundation/FFmpeg helpers. `scripts/bundle.sh`: macOS packaging;
  `scripts/package-linux.sh`: Linux archive and Arch package recipe.

See `docs/architecture.md` for details and `docs/usage.md` for current behavior.
`PLAN.md` is historical; do not use its early scope as the current feature list.

## Working rules

- Keep rendering/export work off the UI thread. Preserve undo and editable
  annotations; guard asynchronous results with operation/revision checks.
- Dispatch typed actions from new UI or automation triggers. Do not fabricate
  keyboard events to invoke commands.
- Keep local editing offline. Do not upload captures, expose the automation
  bridge, or change certificate trust as an incidental validation step.
- Use synthetic samples for docs and tests. Preserve third-party licenses and
  provenance. Keep signing material and credentials outside Git.
- Keep PRs focused and update usage docs when behavior or shortcuts change.

## File and process safety

- Use private, randomized temporary directories for captures and helper outputs.
  Keep export scratch files beside the destination for atomic rename. Never use
  predictable PID/counter paths or truncate an unreserved temporary file.
- Preserve the exact filename confirmed by a save dialog. An appended extension
  must not silently replace an existing file or symlink. MCP exports create new
  destinations and must keep rejecting existing files.
- Treat automation media inputs as local files: reject URLs and special files,
  bound actual reads, and disable network protocols in media helpers. Pass
  arguments directly to processes; never interpolate them into shell scripts.
- Check dependency advisories during security reviews. Distinguish affected
  versions from reachable vulnerable APIs; record evidence and upstream limits.

## MCP parity

- Every user-facing command or editable setting needs a semantic MCP path through
  the shared application/document actions, plus state read-back where relevant.
  Implement UI and MCP support in the same change.
- Update `src/mcp.rs` discovery/schema and `mcp/README.md` with action, parameter,
  annotation, or export changes. A serializable Rust action alone is not enough:
  the published MCP schema must accept and describe it.
- Keep `src/mcp/contract_tests.rs` passing. It compares Serde's action inventory
  with the published schema and validates a complete round-trip payload for every
  exposed action. Add parameterized fixtures for new actions and behavior tests
  through the bridge/shared dispatcher for changed semantics, including state,
  undo and revision conflicts where applicable.
- Internal prepared results and pointer gestures may stay unexposed; record a
  reason and semantic alternative in the contract test's explicit exclusions.
  Prefer revision-scoped document tools over raw annotation indices. Never expose
  worker completion actions or require fabricated input events for parity.

## Validation

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

For native-helper or packaging changes, build with `./scripts/bundle.sh debug`.
For CI/isolated packaging checks, set `GLANCE_CODESIGN_IDENTITY=-` to avoid
certificate trust setup. Ad-hoc bundles have unstable capture permissions;
do not replace a user's installed app with one as part of validation.

Tests use GPUI's virtual platform. Check `QA.md` for manual desktop verification.
Ignored tests are opt-in and may write media, use native helpers, benchmark, or
upload to production; read the test before explicitly running it.

## Changelog and releases

Maintain top-level `CHANGELOG.md` for user-visible changes. Add entries to the
existing `Added`, `Changed`, or `Fixed` subsection of `Unreleased`; do not create
duplicate headings. Omit internal-only refactors unless behavior changes.

On release, move the relevant entries into a dated version section, leave that
released section immutable, and start a fresh `Unreleased` section. Align Cargo
and bundle versions. Use the released section as the GitHub release body and
verify any generated notes. Use `gh release create/edit --notes-file` for
multiline notes. Publishing releases or changing visibility requires a user
request; preparing the repository does not make it public.

Commit titles use Conventional Commits: `<type>[scope]: <description>`.
