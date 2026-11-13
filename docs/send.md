# Capture and send to an attn session

Attn Lens captures part of your live screen, lets you mark it up in place, and
sends it to an agent running in [attn](https://github.com/victorarias/attn), on
this Mac or on an outpost. The screen stays live for the whole capture: clicks,
scrolls and typing keep reaching your apps.

## Capture and send

1. Press **⌘⌥X**. Drag for an area, click for the whole display, press **↩** for
   the last area, or **Esc** to cancel. The area stays outlined; drag a corner to
   adjust it.
2. Hold **fn** and drag to draw. A straight drag is an arrow, any other stays
   freehand, and **⇧** makes a box. The bar's Highlight, Label and Counter buttons
   set the next stroke.
3. Type your message in the bar. Click the page to give it the keyboard; **⌘⌥C**
   takes it back.
4. Press **⌘⌥S** to talk: what you say is transcribed on this Mac, phrase by
   phrase. **⌘⌥S** again stops. The first use downloads a 731 MB speech model, and
   macOS asks for Microphone access.
5. Press **↩** in the bar, or **⌘⌥↩** from anywhere. The capture goes to the
   session on the chip at the left of the bar.

Lens takes stills as you go: when you start drawing on a new view, on **⌘⌥X**, on
each click in the area, on a window switch, and at Send if the screen moved after
the last one. Ink fades once its still is saved and the screen changes. Each
thumbnail in the bar has an **×** to drop that still.

## Edit a still

The bar's **Edit** opens the newest still in the editor; a thumbnail opens that
still. Use any tool, for example **Pixelate** on a secret. See
[the editor's guide](usage.md).

- **‹ Back** (or closing the window, or **⌘⌥X**) returns to the same live capture.
  The still keeps its marks and undo history, so you can open it again and go on.
- **Send ↩** in the editor's toolbar, or **⌘⌥↩**, sends the capture from there,
  with the stills you have. It adds no fresh still of the screen.

While a still is open the capture waits: its outline and bar hide, and no stills
are taken. Voice keeps running. Lens never sends a raw copy of an edited still, but
each still is its own image: a secret visible in two stills needs pixelating in
both, or **×** on one of them.

## After Send: the tile

The capture shrinks to a tile in the corner of the display. It leaves after 5
seconds, and stays while the pointer is on it. Click it to reopen the capture as
it was, for example to send it to a second session.

**Esc** closes a capture without sending. If it holds a still, it keeps a tile
that stays until you click it, press its **×**, or start another capture, which
takes the tile's place.

If attn refuses a send, the capture stays open with your stills and message and
shows attn's error as it printed it. Fix the cause, or press **Tab** to pick
another session, and send again. If Lens cannot list attn's sessions, the bar shows
attn's error instead; once attn is back, **↩** or **Tab** lists them again.

## What the agent receives

One `attn send`, holding:

- **A message.** What you typed comes first. A timeline follows, in time order, of
  the stills, your marks and each spoken phrase. A capture with one still and no
  voice has no timeline.
- **Contact sheets.** With two or more stills, one numbered sheet per six stills
  shows their order and layout.
- **Every still** as a full-resolution PNG, `frame-NN.png`, rendered from its edited
  document.

The agent gets a command that saves each PNG where it runs, so a session on another
machine works like a local one.

## Keys

| Key | Does |
| --- | --- |
| **⌘⌥X** | Starts a capture. During one, takes a still; in the editor, goes Back. |
| **fn** held | Drags draw. |
| **⌘⌥S** | Starts or stops voice, during a capture. |
| **⌘⌥C** | Gives the keyboard back to the message. |
| **⌘⌥↩** | Sends, from anywhere, including the editor. |
| **Tab** | Opens the palette of sessions. |
| **Esc** | Closes the capture, keeping its tile if it holds a still. |

Lens is a Dock app: **⌘Q** quits, and a Dock click opens the editor, empty and
ready for an image.

## Pairing

Attn only accepts input as you from a program you paired. Until Lens is
paired, the bar says so and its button reads **Pair**. Press **↩** and attn asks
you to allow “Attn Lens”, showing a 4-digit code. The bar shows the same code.
Check they match and click **Allow** in attn. Lens then lists the sessions and
the chip shows where **↩** will send. Closing the capture while attn waits for
Allow does not cancel the pairing: once you allow it, the next capture is paired.

Lens keeps the token in your login Keychain (service `dev.attn.lens.sender`) and
stays paired until you remove it in attn under **Settings → Paired senders**.
If you do, the next send says the token is no longer known; press **↩** to pair
again.

A paired sender belongs to the attn profile you had selected when you clicked
Allow, and lists and sends only to that profile's sessions.

## Finding attn

Lens runs the `attn` CLI. It looks for `ATTN_BIN`, then
`/Applications/attn.app` and `~/Applications/attn.app`, then `PATH`,
`/opt/homebrew/bin`, `/usr/local/bin` and `~/.local/bin`. If none exists, the
bar names every place it looked.

## Screen Recording

Lens needs two accesses: Screen Recording, to take stills, and Microphone, for
voice (macOS asks on the first **⌘⌥S**). When Screen Recording is missing, Lens
opens System Settings at the list, at launch and again on **⌘⌥X**, and floats a
guide beside it. The guide says Lens is being set up, lists both accesses with
why each is needed, and gives the steps: drag the tile of the app into the list
and switch it on, then quit and reopen Lens, because macOS applies the grant only
to a new process.

## Automation

Launched with `--automation`, Lens serves a development socket for scripted checks:

```sh
AttnLens lens '{"cmd":"state"}'
```

The header of `src/lens/automation.rs` lists the requests.
