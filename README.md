# Attn Lens

Attn Lens captures part of your live screen, lets you mark it up in place, and
sends it to an [attn](https://github.com/victorarias/attn) agent session, on this
Mac or on an outpost. The screen stays live while you do it: clicks, scrolls and
typing keep reaching your apps.

It runs on macOS 15 or later. It is a fork of
[Glance](https://github.com/modem-dev/glance-desktop) (MIT).

## Install

From a checkout, with the tools listed in [BUILD.md](BUILD.md):

```sh
scripts/install.sh
```

The script builds the source, installs `~/Applications/Attn Lens.app`, and restarts
Lens if it is running. Run it again to update. Permissions and the attn pairing
carry over because every build is signed with the same identity: your Apple
Development certificate, or the one `GLANCE_CODESIGN_IDENTITY` names.

Lens is a Dock app. **⌘Q** quits, and a Dock click opens the editor.

## First capture

1. Press **⌘⌥X** and drag an area. A click takes the whole display, and **Esc**
   cancels.
2. Hold **fn** and drag to draw on the area.
3. Type a message in the bar. Click the page to give it the keyboard; **⌘⌥C** takes
   it back.
4. Press **↩** in the bar, or **⌘⌥↩** from anywhere.

The first send asks you to pair Lens with attn. The agent gets your message and
the stills Lens took along the way. [Capture and send](docs/send.md) covers stills,
pairing, the corner tile and what the agent receives.

## Voice

Press **⌘⌥S** during a capture to talk, and again to stop. Each phrase is
transcribed on this Mac and placed on the capture's timeline.

The first **⌘⌥S** downloads a 731 MB English speech model to
`~/Library/Application Support/dev.attn.lens/speech`, showing its progress in the
bar. An interrupted download resumes on the next **⌘⌥S**.

## Editor

The bar's **Edit** button or any thumbnail opens that still in the editor, for
pixelating a secret or adding a precise mark. A Dock click opens it for any PNG or
JPEG. It has crop, pixelate, callouts, backdrops, OCR, and GIF and MP4 export. See
[the editor's guide](docs/usage.md) and the [tool options](TOOL_OPTIONS.md).

## Permissions

- **Screen Recording**, to take stills. When it is missing, Lens opens System
  Settings at the list as it launches, and again on **⌘⌥X**; enable Attn Lens there, then quit and reopen it.
- **Microphone**, for voice. macOS asks on the first **⌘⌥S**.

## What stays on the Mac

Captures, marks, transcription and OCR all happen on this Mac. A capture leaves it
only when you send it, through `attn send`, to the session you chose. That session
may run on another machine. The only download Lens makes is the speech model, from
Hugging Face.

## More

- [Capture and send](docs/send.md) · [Editor](docs/usage.md) · [Tool options](TOOL_OPTIONS.md)
- [Build from source](BUILD.md) · [Contribute](CONTRIBUTING.md) · [Architecture](docs/architecture.md)
- [Report a bug](https://github.com/SolenesInc/attn-lens/issues) · [Security](SECURITY.md) · [Changelog](CHANGELOG.md)

## License

[MIT](LICENSE). See [third-party notices](THIRD_PARTY_NOTICES.md) for bundled code and assets.
