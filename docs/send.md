# Send to an attn session

Attn Lens sends a marked-up screenshot to an agent running in
[attn](https://github.com/victorarias/attn), on this Mac or on an outpost. The
agent gets your message plus an `attn agent attachment … --out <path>` command
that saves the PNG where the agent runs, so a session on another machine works
the same as a local one.

## Capture and send

1. Press **⌘⌥2** anywhere. The display under the pointer freezes, outlined in
   orange, with a bar at the bottom.
2. Mark it up:
   - **drag**: arrow
   - **⇧ drag**: box
   - **⌥T**, then click: a label; type it and press **↩** (**Esc** cancels)
   - **⌘Z**: undo the last mark
3. Type your message. Typing always goes to the message field.
4. Press **↩**. The capture goes to the session on the chip at the left of the
   bar, which starts on the session you last focused in attn.

The overlay closes, focus returns to the app you were in, and the capture
shrinks into a tile in the corner. Click the tile to reopen it with its marks,
message and target, for example to send it to a second session. **Esc** closes
a capture without sending and keeps it in the tile too.

If attn refuses a send, the overlay stays open with your marks and message and
shows attn's error text as it printed it. Fix the cause, or press **Tab** to
pick another session, and press **↩** again.

## Other keys

| Key | Does |
| --- | --- |
| **Tab** | Opens the palette: your message on top, the chosen session first, other sessions grouped by machine, then Copy image, Save PNG and Edit. **↑ ↓** move, **↩** chooses, **Esc** goes back. |
| **⌘C** | Copies the marked-up image (when no message text is selected). |
| **⌘S** | Saves a PNG. |
| **⌘E** | Opens the capture in the editor for crop, pixelate, backdrops and GIF/MP4 export. In the editor, **⌘↩** (or **Send to attn** in the toolbar) brings the edited image back to the Send bar. |

## Pairing

Attn only accepts input as you from a program you paired. Until Lens is
paired, the bar says so and its button reads **Pair**. Press **↩** and attn asks
you to allow “Attn Lens”, showing a 4-digit code. The bar shows the same code.
Check they match and click **Allow** in attn. Lens then lists the sessions and
the chip shows where **↩** will send.

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

Lens needs Screen Recording access. When it is missing, Lens opens System
Settings at the list and floats a tile of the app beside it; drag the tile into
the list and enable it. macOS applies the grant only to a new process, so quit
and reopen Lens afterwards.

## Automation

Launched with `--automation`, Lens serves the same actions the keys and pointer
use, for tests and scripted proofs:

```sh
AttnLens lens capture                         # same as ⌘⌥2
AttnLens lens state                           # open capture, status, sessions
AttnLens lens overlay '{"type":"add_mark","tool":"arrow","points":[[900,300],[700,200]]}'
AttnLens lens overlay '{"type":"set_message","text":"This button does nothing"}'
AttnLens lens overlay '{"type":"choose_session","id":"<session id>"}'
AttnLens lens overlay '{"type":"send"}'
```

Other actions: `undo`, `toggle_palette`, `pair`, `copy`, `save`, `edit`,
`close`. Mark points are image pixels; a `text` mark takes `"text"`.
