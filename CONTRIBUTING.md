# Contributing to Attn Lens

## Get started

Follow [BUILD.md](BUILD.md) for setup. Use
[the architecture guide](docs/architecture.md) to find the relevant modules.

## Report a problem or propose a change

- For bugs, include macOS version, architecture, reproduction steps, expected result,
  and what happened. Use synthetic screenshots and remove sensitive data from logs.
- For larger changes, open an issue describing the problem and approach first.
  Small fixes can go straight to a pull request.
- Report vulnerabilities through [SECURITY.md](SECURITY.md).

## Make a pull request

Keep each PR focused. Explain the resulting behavior and how you checked it.
Add a regression test for behavior changes, in the document model or as a Rune
script under `headless/`. Include a synthetic screenshot or recording for UI changes.

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

For native-helper or packaging changes, also run `./scripts/bundle.sh debug`.
Read opt-in tests before running them; some write media.
Use [QA.md](QA.md) for desktop acceptance and [development](docs/development.md)
for hooks, CI and release procedures.

Update usage docs for behavior or shortcut changes and add user-visible changes
to the existing `Unreleased` subsection in [CHANGELOG.md](CHANGELOG.md).
Use Conventional Commits, such as `fix: preserve selection after cropping`.

## Project conventions

- Use Rust, Cargo and the Rune UI toolkit.
- Dispatch toolbar, menu, shortcut and script commands through the shared typed
  actions in `Editor::dispatch`. Do not fabricate key events to invoke commands.
- Keep rendering and exports off the UI thread. Preserve editable annotations and
  undo, and reject stale worker results.
- Keep local editing offline; the only thing that leaves the Mac is a capture you send.
- Preserve third-party licenses. Keep signing keys, credentials and private captures outside Git.

See [AGENTS.md](AGENTS.md) for entry points and file and process safety.
Keep discussion respectful and focused on the change. Contributions use the
project's [MIT license](LICENSE).
