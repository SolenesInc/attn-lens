# Developing Attn Lens

[Build from source](../BUILD.md) · [Contributing](../CONTRIBUTING.md) · [Architecture](architecture.md)

## Workflow

Use `cargo run --locked` for UI iteration and the [bundled app](../BUILD.md#build-a-bundle)
for capture permissions and native helpers. Save or copy your image before quitting
and reopening a rebuilt app.

The [contributor checks](../CONTRIBUTING.md#make-a-pull-request) run formatting,
Clippy, and the regular tests. [QA](../QA.md) covers driven checks in a VM, desktop
acceptance and opt-in tests; [performance notes](../PERFORMANCE.md) cover benchmarks.

Launched with `--automation`, Lens serves a development socket:
`AttnLens lens '{"cmd":"state"}'`. The header of `src/lens/automation.rs` lists the
requests. Headless Rune runs, on macOS or Linux, are a development tool only.

### Optional pre-commit hook

```sh
./scripts/setup-dev.sh
```

The hook runs formatting and Clippy. Disable it with
`git config --local --unset core.hooksPath`. Both commands change this
repository's Git configuration, shared by its worktrees.

## CI

[CI](../.github/workflows/ci.yml) runs on pull requests and pushes to `main`.
On macOS it runs formatting, Clippy and tests for the app and the speech helper,
then an MP4 encode/PNG decode round trip and the OCR helper. On Linux it runs
formatting, Clippy, tests and the same video round trip, headless.

## Releases

Attn Lens installs from a checkout with `scripts/install.sh`.

1. Complete the [desktop acceptance checklist](../QA.md).
2. Move the relevant `Unreleased` changelog entries into a dated version section.
   Leave released sections immutable and start a fresh `Unreleased` section.
3. Align Cargo and bundle versions.
4. Publishing a release requires a user request. Use the released changelog section
   as the body, with `gh release create/edit --notes-file`.
