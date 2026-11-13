# Building Attn Lens

Attn Lens builds on macOS 15 or later with Rust and Cargo. Development is verified
on Apple Silicon.

## Requirements

- Xcode Command Line Tools (`xcode-select --install`), for `swiftc` and the macOS SDK
- Stable Rust from [rustup](https://rustup.rs/)
- CMake, for the speech helper

## Install from a checkout

```sh
git clone https://github.com/SolenesInc/attn-lens.git
cd attn-lens
scripts/install.sh
```

`scripts/install.sh` builds a release bundle, replaces
`~/Applications/Attn Lens.app`, quits the running copy and opens the new one. It
signs with this Mac's Apple Development identity unless `GLANCE_CODESIGN_IDENTITY`
names another, and refuses ad-hoc signing.

## Build a bundle

```sh
./scripts/bundle.sh          # release
./scripts/bundle.sh debug    # faster
open "target/Attn Lens.app"
```

The bundle is `Attn Lens.app`, its executable `AttnLens`, its bundle id
`dev.attn.lens`. It holds the video and OCR helpers from `native/` and the speech
helper from `speech/`.

`cargo run --locked` is useful for UI iteration; run a bundle build once first to
compile the native helpers. Capture permissions need the signed bundle.

## Signing

macOS ties Screen Recording and Microphone grants to the signing identity, so
every build should use the same one. With no identity named, the first build
creates a persistent local one in
`~/Library/Application Support/Glance/Signing` (`GLANCE_SIGNING_DIR` names another
location); keep that directory across rebuilds. If the build stops with a
certificate-trust message, review the setup script, run it once, and rebuild:

```sh
./scripts/trust-local-signing.sh
./scripts/bundle.sh
```

This adds user-Keychain trust for the development certificate, for code signing
only.

To use an existing certificate:

```sh
GLANCE_CODESIGN_IDENTITY="Your code-signing certificate name" ./scripts/bundle.sh
```

For an isolated packaging check, use a separate destination and ad-hoc signing:

```sh
GLANCE_BUNDLE_DEST="$(pwd)/target/packaging-check/Attn Lens.app" \
  GLANCE_CODESIGN_IDENTITY=- ./scripts/bundle.sh debug
```

Ad-hoc signing invalidates capture grants on rebuild. Do not replace an installed
app with an ad-hoc bundle.

## Permissions

On first capture, grant Attn Lens **Screen Recording** in **System Settings →
Privacy & Security** (called **Screen & System Audio Recording** on newer macOS),
then quit and reopen it. Use the same bundle path and signing identity for later
builds.

If capture fails despite an enabled grant, quit Lens, remove its entry with **−**,
add the app with **+**, enable access, and reopen it.

## Checks

Run the checks in [CONTRIBUTING.md](CONTRIBUTING.md#make-a-pull-request).
[Development](docs/development.md) covers hooks, CI and releases;
[QA](QA.md) covers desktop acceptance and opt-in tests.
