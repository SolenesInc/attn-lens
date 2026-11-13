# Security policy

Attn Lens is in early development. Security fixes target the current `main` branch;
older development builds do not have a separate support policy.

## Report a vulnerability

Use GitHub's private **Report a vulnerability** form on the repository's
[Security page](https://github.com/SolenesInc/attn-lens/security/advisories)
when available. If private reporting is unavailable, open an issue asking for a
private reporting channel **without disclosing the vulnerability or sensitive
data**. A maintainer will arrange a channel before you share details.

Include the affected commit/build, macOS version, reproduction steps, expected
impact, and a minimal proof of concept using synthetic images. Do not attach
personal captures, credentials, or local signing material.

## Data and access boundaries

- Capture, annotation, transcription, OCR, clipboard copy, and file export run
  locally. Capture and media scratch files use private temporary directories;
  media helpers do not fetch remote URLs.
- A capture leaves the Mac only on Send, through the `attn` CLI, to the attn session
  you chose. That session may run on another machine.
- The first use of voice downloads the speech model from Hugging Face over HTTPS.
  Each file is pinned by URL, size and SHA-256.
- Attn accepts sends only from a paired program. Lens keeps its token in the login
  Keychain (service `dev.attn.lens.sender`) and passes it to `attn` on standard input.
- Pixelation is a visual effect. Crop out sensitive material when that is not enough.
- The development socket exists only with `--automation`. It lives in a directory
  only the local account can enter, but any process running as that account can
  use it to start captures, read their state, and send them.
- Local signing certificates and private keys live outside the checkout. The
  one-time trust script changes user-domain trust for code signing. It is not
  distribution signing or notarization.

## Dependency review baseline

An OSV review on 2026-10-03, including RustSec and GitHub advisories, matched
maintenance advisories for `paste` and `ttf-parser`, both still in `Cargo.lock`.
These flag unmaintained dependencies, not demonstrated machine compromise. Track
their upstream replacements rather than silently suppressing the warnings, and
rerun the scan when dependencies change.
