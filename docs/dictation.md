# Dictation

With no capture open, the voice key, **right ⌥**, dictates into the app in front:
a browser field, a terminal, a text editor. Tap it to start and tap again to
stop, or hold it and talk until you let go.

## What you see

A pill sits at the bottom of the display with the pointer, above the Dock. It
never takes the keyboard or a click, so the app in front keeps its cursor.

1. While you talk, the pill shows the words as each phrase is transcribed.
2. When you stop, attn tidies the text (filler words, punctuation, casing) under
   your profile. The pill shows the raw text and **Refining… · ↩ paste as is ·
   esc cancel**.
3. The text pastes once at the cursor, and the pill says how it went for 3 s.

While the pill says Refining:

- **↩** pastes the raw text at once. attn's answer, if it comes later, is dropped.
- **Esc** pastes nothing.

Lens takes ↩ and Esc from the app in front only then; at any other time they
reach the app as usual.

## When the text is not refined

The raw text pastes, and the pill says why, when:

- attn's refinement fails, or attn refuses Lens's token. A refused token also
  unpairs Lens, as a send does; pair again from the bar.
- attn is not installed, or cannot be run.
- attn does not answer within its own limit (15 s plus a quarter of the
  recording) and 2 s more. Lens then stops it.

If nothing was heard, nothing pastes and the pill says **Didn't catch anything**.

Without Accessibility, Lens cannot post ⌘V: the text goes to the clipboard and the
pill says **Copied — allow Accessibility to paste**.

## The clipboard

The paste goes through the clipboard: Lens saves what it holds, writes the text
marked so clipboard managers skip it, posts ⌘V, and puts the saved clipboard back
half a second later, unless something else wrote it meanwhile. In Chromium 155 and
156, which drop a ⌘V that races their clipboard, Lens types the text instead.

## Copy Last Dictation

The Dock menu's **Copy Last Dictation** puts the last dictation's text on the
clipboard, refined or raw, cancelled ones included, in case a paste landed nowhere.
It lasts until Lens quits.

## From dictation to a capture

Press the capture key, **right ⌘**, while dictating, and pick an area. The capture
takes the dictation over: the words so far are its first phrases, its timeline
starts where you began talking, voice keeps running, and nothing pastes. If you
were already waiting for attn, its refinement stops and the raw phrases go to the
capture. Cancel the pick and the dictation carries on.

## Checking it

`LENS_DICTATION_LOG=1` prints each step with the time since launch: the voice key,
the pill showing, voice reporting, attn asked and answering, the paste.
