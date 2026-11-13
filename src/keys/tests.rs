//! Ported from Hex's `src/suppression.rs` tests at 54d1ebf (MIT, Christopher
//! Langton), for right-side keys and taps reported as taps.
use super::*;

const NO_FLAGS: u64 = 0;
const R_OPTION: u64 = OPTION | RIGHT_OPTION;
const R_COMMAND: u64 = COMMAND | RIGHT_COMMAND;
const L_OPTION: u64 = OPTION | LEFT_OPTION;
const L_SHIFT: u64 = SHIFT | LEFT_SHIFT;

fn start() -> HostTime {
    HostTime(60_000_000_000)
}

fn ms(millis: u64) -> HostTime {
    start().after(Duration::from_millis(millis))
}

fn key(code: u16, down: bool, flags: u64) -> InputEvent {
    InputEvent::Key { code, down, flags }
}

/// Feeds `trace` (milliseconds from [`start`], event) and returns what it made.
fn run(keys: &mut Keys, trace: &[(u64, InputEvent)]) -> Vec<KeyEvent> {
    trace
        .iter()
        .flat_map(|&(at, event)| keys.feed(event, ms(at)))
        .collect()
}

fn keys() -> Keys {
    Keys::new(Physical::default())
}

fn voice(press: Press) -> KeyEvent {
    KeyEvent::Voice(press)
}

const CAPTURE: KeyEvent = KeyEvent::Pressed(Chord::Capture);

fn recover_after_neutral_window(keys: &mut Keys, from: HostTime, flags: u64) -> HostTime {
    let repaired = from.after(STALE_KEY_NEUTRAL);
    keys.recover_stale_keys(from, || flags, |_| false);
    keys.recover_stale_keys(repaired, || flags, |_| false);
    repaired
}

#[test]
fn a_short_tap_is_a_tap_and_a_long_hold_starts_and_ends_on_release() {
    let mut k = keys();
    let tap = run(
        &mut k,
        &[
            (0, InputEvent::Flags(R_OPTION)),
            (299, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(tap, [voice(Press::Tap)]);

    // The hold starts once the threshold passes, with no event to carry it.
    let mut k = keys();
    assert_eq!(run(&mut k, &[(1_000, InputEvent::Flags(R_OPTION))]), []);
    assert_eq!(k.hold_due(), Some(ms(1_300)));
    assert_eq!(k.tick(ms(1_299)), []);
    assert_eq!(k.tick(ms(1_300)), [voice(Press::HoldStart)]);
    assert_eq!(k.hold_due(), None);
    assert_eq!(
        run(&mut k, &[(4_000, InputEvent::Flags(NO_FLAGS))]),
        [voice(Press::HoldEnd)]
    );

    // A release that arrives before the tick still reads as a hold.
    let mut k = keys();
    let late = run(
        &mut k,
        &[
            (0, InputEvent::Flags(R_OPTION)),
            (300, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(late, [voice(Press::HoldStart), voice(Press::HoldEnd)]);

    // The next press starts afresh.
    assert_eq!(
        run(
            &mut k,
            &[
                (2_000, InputEvent::Flags(R_OPTION)),
                (2_100, InputEvent::Flags(NO_FLAGS))
            ]
        ),
        [voice(Press::Tap)]
    );
}

#[test]
fn the_capture_key_reports_taps_and_a_hold_does_nothing() {
    let mut k = keys();
    assert_eq!(
        run(
            &mut k,
            &[
                (0, InputEvent::Flags(R_COMMAND)),
                (120, InputEvent::Flags(NO_FLAGS))
            ]
        ),
        [CAPTURE]
    );
    assert_eq!(k.hold_due(), None);
    assert_eq!(
        run(
            &mut k,
            &[
                (1_000, InputEvent::Flags(R_COMMAND)),
                (2_000, InputEvent::Flags(NO_FLAGS))
            ]
        ),
        []
    );
}

#[test]
fn a_chord_is_discarded_and_belongs_to_the_app() {
    // Right ⌥+e for an accent: no voice, before or after the threshold.
    let mut k = keys();
    let accent = run(
        &mut k,
        &[
            (0, InputEvent::Flags(R_OPTION)),
            (100, key(14, true, R_OPTION)),
            (160, key(14, false, R_OPTION)),
            (200, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(accent, []);
    let late = run(
        &mut k,
        &[
            (1_000, InputEvent::Flags(R_OPTION)),
            (1_400, key(14, true, R_OPTION)),
            (1_460, key(14, false, R_OPTION)),
            (1_500, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(late, [voice(Press::HoldStart), voice(Press::HoldEnd)]);

    // Right ⌘+C copies and opens nothing.
    let copy = run(
        &mut k,
        &[
            (3_000, InputEvent::Flags(R_COMMAND)),
            (3_050, key(8, true, R_COMMAND)),
            (3_090, key(8, false, R_COMMAND)),
            (3_120, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(copy, []);

    // A click with the key down is ⌘-click.
    let click = run(
        &mut k,
        &[
            (4_000, InputEvent::Flags(R_COMMAND)),
            (4_050, InputEvent::MouseDown),
            (4_100, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(click, []);

    // Both keys together are neither.
    let both = run(
        &mut k,
        &[
            (5_000, InputEvent::Flags(R_COMMAND)),
            (5_040, InputEvent::Flags(R_COMMAND | R_OPTION)),
            (5_080, InputEvent::Flags(R_OPTION)),
            (5_120, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(both, []);
    // Nor is one left alone after another modifier comes up.
    let left_alone = run(
        &mut k,
        &[
            (5_500, InputEvent::Flags(L_SHIFT)),
            (5_520, InputEvent::Flags(L_SHIFT | R_OPTION)),
            (5_540, InputEvent::Flags(R_OPTION)),
            (5_580, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(left_alone, []);

    // After a chord the next clean tap counts again.
    assert_eq!(
        run(
            &mut k,
            &[
                (6_000, InputEvent::Flags(R_COMMAND)),
                (6_100, InputEvent::Flags(NO_FLAGS))
            ]
        ),
        [CAPTURE]
    );
}

#[test]
fn an_extra_modifier_discards_only_before_the_threshold() {
    let mut k = keys();
    let early = run(
        &mut k,
        &[
            (0, InputEvent::Flags(R_OPTION)),
            (100, InputEvent::Flags(R_OPTION | L_SHIFT)),
            (150, InputEvent::Flags(R_OPTION)),
            (200, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(early, []);
    let late = run(
        &mut k,
        &[
            (1_000, InputEvent::Flags(R_OPTION)),
            (1_400, InputEvent::Flags(R_OPTION | L_SHIFT)),
            (1_500, InputEvent::Flags(R_OPTION)),
            (1_600, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(late, [voice(Press::HoldStart), voice(Press::HoldEnd)]);
}

#[test]
fn only_the_right_side_counts() {
    let mut k = keys();
    let left = run(
        &mut k,
        &[
            (0, InputEvent::Flags(L_OPTION)),
            (100, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(left, []);
    let both_sides = run(
        &mut k,
        &[
            (1_000, InputEvent::Flags(R_OPTION)),
            (1_050, InputEvent::Flags(R_OPTION | L_OPTION)),
            (1_100, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(both_sides, []);
    // Without device flags, as some synthetic events post them, no side is known.
    let sideless = run(
        &mut k,
        &[
            (2_000, InputEvent::Flags(OPTION)),
            (2_100, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(sideless, []);
}

#[test]
fn escape_is_never_a_chord() {
    let mut k = keys();
    let trace = run(
        &mut k,
        &[
            (0, InputEvent::Flags(R_OPTION)),
            (50, key(ESCAPE, true, R_OPTION)),
            (80, key(ESCAPE, false, R_OPTION)),
            (120, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(trace, [voice(Press::Tap)]);
}

#[test]
fn hid_evidence_discards_a_chord_hidden_by_a_global_shortcut() {
    // Another app's shortcut (right ⌘+Space) takes the key before the session sees it.
    let mut k = keys();
    let hidden = run(
        &mut k,
        &[
            (0, InputEvent::Flags(R_COMMAND)),
            (60, InputEvent::PhysicalKeyDown { code: 49 }),
            (140, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(hidden, []);
    // HID evidence stamped after the press can arrive before it.
    let mut k = keys();
    let overtaken = [
        (InputEvent::PhysicalKeyDown { code: 49 }, ms(1_040)),
        (InputEvent::Flags(R_COMMAND), ms(1_000)),
        (InputEvent::Flags(NO_FLAGS), ms(1_100)),
    ]
    .into_iter()
    .flat_map(|(event, at)| k.feed(event, at))
    .collect::<Vec<_>>();
    assert_eq!(overtaken, []);
    // Delayed evidence of a key before the press is no chord.
    let mut k = keys();
    let before = [
        (InputEvent::Flags(R_COMMAND), ms(2_000)),
        (InputEvent::PhysicalKeyDown { code: 0 }, ms(1_990)),
        (InputEvent::Flags(NO_FLAGS), ms(2_100)),
    ]
    .into_iter()
    .flat_map(|(event, at)| k.feed(event, at))
    .collect::<Vec<_>>();
    assert_eq!(before, [CAPTURE]);
}

#[test]
fn a_disabled_tap_ends_a_hold_and_waits_for_a_neutral_keyboard() {
    let mut k = keys();
    let trace = run(
        &mut k,
        &[
            (0, InputEvent::Flags(R_OPTION)),
            (500, InputEvent::TapDisabled),
            (600, InputEvent::Flags(NO_FLAGS)),
            (1_000, InputEvent::Flags(R_OPTION)),
            (1_100, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(
        trace,
        [
            voice(Press::HoldStart),
            voice(Press::HoldEnd),
            voice(Press::Tap)
        ]
    );
}

#[test]
fn stale_key_recovery_restores_a_fresh_hold_without_changing_its_boundaries() {
    let mut k = keys();
    // The ordinary key's release never reached the machine.
    assert_eq!(
        run(
            &mut k,
            &[
                (0, key(0, true, 0)),
                (0, InputEvent::Flags(R_OPTION)),
                (200, InputEvent::Flags(NO_FLAGS))
            ]
        ),
        []
    );
    let first = ms(1_000);
    k.recover_stale_keys(first, || 0, |_| false);
    k.recover_stale_keys(first.after(Duration::from_millis(99)), || 0, |_| false);
    assert!(k.voice.pressed_keys.contains(&0));
    let repaired = first.after(STALE_KEY_NEUTRAL);
    k.recover_stale_keys(repaired, || 0, |_| false);
    assert!(k.voice.pressed_keys.is_empty());
    let press = repaired.after(Duration::from_millis(1));
    assert_eq!(k.feed(InputEvent::Flags(R_OPTION), press), []);
    assert_eq!(k.hold_due(), Some(press.after(HOLD)));
    assert_eq!(
        k.feed(
            InputEvent::Flags(NO_FLAGS),
            press.after(Duration::from_millis(500))
        ),
        [voice(Press::HoldStart), voice(Press::HoldEnd)]
    );
}

/// ⌘Tab whose Tab release the app switcher swallows.
fn command_tab(k: &mut Keys, at: u64) -> u64 {
    let trace = run(
        k,
        &[
            (at, InputEvent::Flags(COMMAND | LEFT_COMMAND)),
            (at + 40, key(48, true, COMMAND | LEFT_COMMAND)),
            (at + 120, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(trace, []);
    assert!(k.voice.pressed_keys.contains(&48));
    at + 120
}

#[test]
fn a_press_soon_after_command_tab_counts() {
    let mut k = keys();
    let released = command_tab(&mut k, 0);
    // Well inside the 100 ms recovery needs.
    let press = released + 30;
    assert_eq!(run(&mut k, &[(press, InputEvent::Flags(R_OPTION))]), []);
    assert!(k.voice.pressed_keys.is_empty());
    // An older queued edge cannot end the fresh press.
    assert_eq!(run(&mut k, &[(released, InputEvent::Flags(NO_FLAGS))]), []);
    assert_eq!(
        run(&mut k, &[(press + 100, InputEvent::Flags(NO_FLAGS))]),
        [voice(Press::Tap)]
    );
    // The capture key resynchronizes the same way.
    let released = command_tab(&mut k, 2_000);
    assert_eq!(
        run(
            &mut k,
            &[
                (released + 30, InputEvent::Flags(R_COMMAND)),
                (released + 100, InputEvent::Flags(NO_FLAGS))
            ]
        ),
        [CAPTURE]
    );
}

#[test]
fn a_stale_press_waits_while_another_modifier_went_down() {
    let mut k = keys();
    let released = command_tab(&mut k, 0);
    let trace = run(
        &mut k,
        &[
            (released + 10, InputEvent::Flags(L_SHIFT)),
            (released + 20, InputEvent::Flags(L_SHIFT | R_OPTION)),
            (released + 30, InputEvent::Flags(R_OPTION)),
            (released + 60, InputEvent::Flags(NO_FLAGS)),
        ],
    );
    assert_eq!(trace, []);
    // Recovery, once the keyboard is neutral, lets the next press through.
    let repaired = recover_after_neutral_window(&mut k, ms(released + 200), 0);
    let after = repaired.since(start()).as_millis() as u64 + 1;
    assert_eq!(
        run(
            &mut k,
            &[
                (after, InputEvent::Flags(R_OPTION)),
                (after + 50, InputEvent::Flags(NO_FLAGS))
            ]
        ),
        [voice(Press::Tap)]
    );
}

#[test]
fn arrow_function_metadata_does_not_block_the_next_option_hold() {
    // Observed after an arrow key-up with no physical keys held: numeric-pad,
    // secondary-fn and noncoalesced flags remain in native session state.
    let arrow_flags = 0x00a0_0100;
    let mut k = keys();
    let trace = run(
        &mut k,
        &[
            (0, InputEvent::Flags(R_OPTION)),
            (15, InputEvent::Flags(R_OPTION | L_SHIFT)),
            (30, key(123, true, arrow_flags | R_OPTION | L_SHIFT)),
            (150, InputEvent::Flags(R_OPTION)),
            (155, InputEvent::Flags(0x100)),
            (180, key(123, false, arrow_flags)),
        ],
    );
    assert_eq!(trace, []);
    assert_eq!(k.voice.state, State::Dirty);
    assert!(k.voice.pressed_keys.is_empty());

    recover_after_neutral_window(&mut k, ms(1_000), arrow_flags);
    assert_eq!(k.voice.state, State::Idle);
    let press = run(
        &mut k,
        &[
            (5_000, InputEvent::Flags(R_OPTION)),
            (6_000, InputEvent::Flags(0x100)),
        ],
    );
    assert_eq!(press, [voice(Press::HoldStart), voice(Press::HoldEnd)]);
    // Arrow metadata never moves the draw key.
    assert!(!k.draw_held());
}

#[test]
fn function_recovery_waits_out_an_unknown_or_held_fn() {
    // fn read from the keyboard with no modifier event: it may be held.
    let mut k = keys();
    k.voice.state = State::Dirty;
    for offset in [0, 100, 1_000] {
        k.recover_stale_keys(ms(offset), || FUNCTION, |_| false);
        assert_eq!(k.voice.state, State::Dirty);
    }
    recover_after_neutral_window(&mut k, ms(2_000), 0);
    assert_eq!(k.voice.state, State::Idle);
    // A modifier event that says fn is up lets the leftover metadata be ignored.
    k.voice.state = State::Dirty;
    k.feed(InputEvent::Flags(NO_FLAGS), ms(3_000));
    k.voice.state = State::Dirty;
    recover_after_neutral_window(&mut k, ms(3_001), FUNCTION);
    assert_eq!(k.voice.state, State::Idle);
}

#[test]
fn a_blind_period_invalidates_fn_evidence() {
    let mut k = keys();
    k.feed(InputEvent::Flags(NO_FLAGS), ms(0));
    k.feed(InputEvent::TapDisabled, ms(10));
    // An older modifier event cannot restore the evidence.
    k.feed(InputEvent::Flags(NO_FLAGS), ms(9));
    k.voice.state = State::Dirty;
    recover_after_neutral_window(&mut k, ms(20), FUNCTION);
    assert_eq!(k.voice.state, State::Dirty);
}

#[test]
fn fn_is_the_draw_key_from_modifier_events_only() {
    let mut k = keys();
    let trace = run(
        &mut k,
        &[
            (0, key(124, true, FUNCTION | 0x0020_0000)),
            (40, key(124, false, FUNCTION | 0x0020_0000)),
            (100, InputEvent::Flags(FUNCTION)),
            (150, InputEvent::Flags(FUNCTION | L_SHIFT)),
            (200, InputEvent::Flags(L_SHIFT)),
            (250, InputEvent::Flags(NO_FLAGS)),
            (300, InputEvent::Flags(FUNCTION)),
            (350, InputEvent::TapDisabled),
        ],
    );
    assert_eq!(
        trace,
        [
            KeyEvent::DrawDown,
            KeyEvent::DrawUp,
            KeyEvent::DrawDown,
            KeyEvent::DrawUp
        ]
    );
}
