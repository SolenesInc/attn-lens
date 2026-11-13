//! The capture key (right ⌘) and the voice key (right ⌥): one modifier, pressed
//! alone. A press released before [`HOLD`] with nothing else pressed is a tap; one
//! held past it is a hold. A press that meets another key, a click or another
//! modifier is a chord, and belongs to the app in front: right ⌘+C copies, right
//! ⌥+e types é, and neither opens anything.
//!
//! The state machine is Hex's `DictationHotkey` (`src/suppression.rs`,
//! <https://github.com/anomalyco/hex>, commit 54d1ebf), MIT License, Copyright (c)
//! 2026 Christopher Langton. See THIRD_PARTY_NOTICES.md. Kept: chord discard,
//! HID evidence of chords another app's shortcut hides, stale-key recovery after
//! a system shortcut swallows a release, and fn evidence from modifier events.
//! Cut: settings, Labs switches, mouse triggers, double tap, toggle locking and
//! the chord breaker. Hex locks a short tap and finishes it on the next press;
//! here a tap is reported as one, and Lens decides what it toggles, so the
//! machine holds no copy of whether voice runs.
//!
//! This file is free of any platform: [`tap`] feeds it macOS's events.
use crate::{
    lens::session::HostTime,
    platform_port::{Chord, KeyEvent, Press},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::OnceLock,
    time::Duration,
};

#[cfg(target_os = "macos")]
pub mod tap;

/// A press held this long is a hold, not a tap: Hex's `MINIMUM_HOLD_DURATION`.
pub const HOLD: Duration = Duration::from_millis(300);
/// How long the keyboard must read neutral before stale keys are forgotten.
const STALE_KEY_NEUTRAL: Duration = Duration::from_millis(100);
const PHYSICAL_KEY_DOWN_HISTORY: usize = 8;
const ESCAPE: u16 = 53;

pub const SHIFT: u64 = 1 << 17;
pub const CONTROL: u64 = 1 << 18;
pub const OPTION: u64 = 1 << 19;
pub const COMMAND: u64 = 1 << 20;
/// fn, which is also the draw key. `NSEventModifierFlagFunction`.
pub const FUNCTION: u64 = 1 << 23;
pub const MODIFIERS: u64 = SHIFT | CONTROL | OPTION | COMMAND | FUNCTION;
/// Device flags: which side of a modifier is down.
const LEFT_CONTROL: u64 = 0x0000_0001;
const LEFT_SHIFT: u64 = 0x0000_0002;
const RIGHT_SHIFT: u64 = 0x0000_0004;
const LEFT_COMMAND: u64 = 0x0000_0008;
const RIGHT_COMMAND: u64 = 0x0000_0010;
const LEFT_OPTION: u64 = 0x0000_0020;
const RIGHT_OPTION: u64 = 0x0000_0040;
const RIGHT_CONTROL: u64 = 0x0000_2000;
/// Each modifier as (its flag, left device flag, right device flag).
const SIDED: [(u64, u64, u64); 4] = [
    (CONTROL, LEFT_CONTROL, RIGHT_CONTROL),
    (OPTION, LEFT_OPTION, RIGHT_OPTION),
    (SHIFT, LEFT_SHIFT, RIGHT_SHIFT),
    (COMMAND, LEFT_COMMAND, RIGHT_COMMAND),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Both,
    /// The modifier's flag without device flags, as synthetic events may post it.
    Either,
}

/// Which side of `modifier` the flags hold, if any.
fn side(flags: u64, (modifier, left, right): (u64, u64, u64)) -> Option<Side> {
    match (flags & left != 0, flags & right != 0) {
        (true, false) => Some(Side::Left),
        (false, true) => Some(Side::Right),
        (true, true) => Some(Side::Both),
        (false, false) if flags & modifier != 0 => Some(Side::Either),
        _ => None,
    }
}

/// One right-hand modifier, pressed alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    sided: (u64, u64, u64),
}

impl Key {
    pub const RIGHT_COMMAND: Self = Self { sided: SIDED[3] };
    pub const RIGHT_OPTION: Self = Self { sided: SIDED[1] };

    fn flag(self) -> u64 {
        self.sided.0
    }

    /// The key is down, whatever else is (Hex's `required_modifiers_down`).
    fn down(self, flags: u64) -> bool {
        matches!(side(flags, self.sided), Some(Side::Right | Side::Both))
    }

    /// The key is down and no other modifier is, its left twin included (Hex's
    /// `exact_modifiers`).
    fn alone(self, flags: u64) -> bool {
        flags & FUNCTION == 0
            && SIDED.iter().all(|&sided| {
                let held = side(flags, sided);
                if sided == self.sided {
                    held == Some(Side::Right)
                } else {
                    held.is_none()
                }
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// A modifier changed: `kCGEventFlagsChanged`.
    Flags(u64),
    Key {
        code: u16,
        down: bool,
        flags: u64,
    },
    /// A key press seen at the HID level, where another app's global shortcut
    /// cannot hide it. It only signals a chord: the session `Key` events remain
    /// the source of key state.
    PhysicalKeyDown {
        code: u16,
    },
    MouseDown,
    /// macOS disabled a tap, so input went unseen for a while.
    TapDisabled,
}

/// What ended a press as a chord.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Discard {
    Key,
    HidKey,
    Mouse,
    ExtraModifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// The key is down alone. `held` once it passed [`HOLD`] and reported a hold.
    Pressed {
        at: HostTime,
        held: bool,
    },
    /// A chord, or keys of unknown state: nothing counts until all is released.
    Dirty,
}

/// Modifier changes seen since the keyboard last reported no modifiers.
#[derive(Clone, Copy)]
struct ModifierHistory {
    flags: u64,
    flags_at: HostTime,
    since_neutral: u64,
    neutral_at: Option<HostTime>,
}

impl ModifierHistory {
    /// No evidence, and none from events stamped before `at`.
    const fn reset(at: HostTime) -> Self {
        Self {
            flags: 0,
            flags_at: at,
            since_neutral: 0,
            neutral_at: None,
        }
    }
}

/// The live keyboard, read only by the stale-press check. The default reads as
/// everything up, which is what unit tests and the fake see.
#[derive(Clone, Copy)]
pub struct Physical {
    pub flags: fn() -> u64,
    pub key_down: fn(u16) -> bool,
}

impl Default for Physical {
    fn default() -> Self {
        Self {
            flags: || 0,
            key_down: |_| false,
        }
    }
}

/// `LENS_KEYS_LOG=1` prints each decision, for driven checks.
fn log(message: std::fmt::Arguments) {
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("LENS_KEYS_LOG").is_some_and(|v| v == "1")) {
        eprintln!("Attn Lens keys: {message}");
    }
}

/// One key's tap-or-hold machine.
pub struct Trigger {
    key: Key,
    /// Whether a hold is reported. The capture key reports only taps.
    holds: bool,
    state: State,
    pressed_keys: HashSet<u16>,
    stale_keys_neutral_since: Option<HostTime>,
    recovery_ignore_through: Option<HostTime>,
    recovery_updated_keys: HashSet<u16>,
    // Fn evidence comes only from modifier events; None marks a blind period.
    // The timestamp keeps delayed events from restoring invalidated evidence.
    function_modifier: (HostTime, Option<bool>),
    // Recent HID key presses, which another app's shortcut may have hidden.
    physical_key_downs: VecDeque<(HostTime, u16)>,
    modifier_history: ModifierHistory,
    // When each tracked key last went down.
    key_down_at: HashMap<u16, HostTime>,
    physical: Physical,
}

impl Trigger {
    pub fn new(key: Key, holds: bool, physical: Physical) -> Self {
        Self {
            key,
            holds,
            state: State::Idle,
            pressed_keys: HashSet::new(),
            stale_keys_neutral_since: None,
            recovery_ignore_through: None,
            recovery_updated_keys: HashSet::new(),
            function_modifier: (HostTime(0), None),
            physical_key_downs: VecDeque::new(),
            modifier_history: ModifierHistory::reset(HostTime(0)),
            key_down_at: HashMap::new(),
            physical,
        }
    }

    fn held(&self) -> bool {
        matches!(self.state, State::Pressed { held: true, .. })
    }

    /// When a press now down becomes a hold, if it can.
    pub fn hold_due(&self) -> Option<HostTime> {
        match self.state {
            State::Pressed { at, held: false } if self.holds => Some(at.after(HOLD)),
            _ => None,
        }
    }

    /// Reports the hold of a press down since [`HOLD`] before `now`. Call it
    /// before each event and when [`Self::hold_due`] passes.
    pub fn tick(&mut self, now: HostTime) -> Option<Press> {
        let due = self.hold_due()?;
        if now < due {
            return None;
        }
        if let State::Pressed { held, .. } = &mut self.state {
            *held = true;
        }
        Some(Press::HoldStart)
    }

    /// Whether [`Self::recover_stale_keys`] has anything to repair.
    pub fn recoverable(&self) -> bool {
        match self.state {
            State::Dirty => true,
            State::Idle => !self.pressed_keys.is_empty(),
            State::Pressed { .. } => false,
        }
    }

    /// Forgets keys whose release was never seen, once the keyboard has read
    /// neutral for [`STALE_KEY_NEUTRAL`]. Call it only with no input queued.
    /// It repairs bookkeeping and never ends a press.
    pub fn recover_stale_keys(
        &mut self,
        now: HostTime,
        mut flags: impl FnMut() -> u64,
        mut key_down: impl FnMut(u16) -> bool,
    ) {
        if !self.recoverable() {
            self.stale_keys_neutral_since = None;
            return;
        }
        // Arrow key metadata can leave fn set after release. Ignore it only when
        // modifier events independently say fn is not held.
        let modifiers = if self.function_modifier.1 == Some(false) {
            MODIFIERS & !FUNCTION
        } else {
            MODIFIERS
        };
        if flags() & modifiers != 0
            || (!self.pressed_keys.is_empty()
                && self.pressed_keys.iter().all(|&code| key_down(code)))
            || (0..128).any(&mut key_down)
            || flags() & modifiers != 0
        {
            self.stale_keys_neutral_since = None;
            return;
        }
        let Some(neutral_since) = self.stale_keys_neutral_since else {
            self.stale_keys_neutral_since = Some(now);
            return;
        };
        if now.since(neutral_since) < STALE_KEY_NEUTRAL {
            return;
        }
        // A system shortcut such as ⌘Tab swallows its key's release.
        log(format_args!(
            "{:?}: forgot stale keys {:?} after a neutral keyboard",
            self.key, self.pressed_keys
        ));
        self.pressed_keys.clear();
        self.key_down_at.clear();
        self.state = State::Idle;
        self.stale_keys_neutral_since = None;
        self.recovery_ignore_through = Some(now);
        self.recovery_updated_keys.clear();
    }

    /// Follows key state. Returns whether a key went down that was not down,
    /// or `None` for an edge older than recovery that must not count.
    fn track_key_state(&mut self, event: InputEvent, at: HostTime) -> Option<bool> {
        self.stale_keys_neutral_since = None;
        let fenced = self
            .recovery_ignore_through
            .is_some_and(|fence| at <= fence);
        if let InputEvent::Flags(flags) = event
            && at >= self.function_modifier.0
        {
            self.function_modifier = (at, Some(flags & FUNCTION != 0));
        } else if matches!(event, InputEvent::TapDisabled) {
            self.function_modifier = (at, None);
            self.modifier_history = ModifierHistory::reset(at);
        }
        // Only modifier changes: key events can carry fn as navigation metadata.
        // Fenced edges predate the evidence that replaced them.
        if let InputEvent::Flags(flags) = event
            && !fenced
            && at >= self.modifier_history.flags_at
        {
            let history = &mut self.modifier_history;
            history.flags = flags;
            history.flags_at = at;
            if flags & MODIFIERS == 0 {
                history.since_neutral = 0;
                history.neutral_at = Some(at);
            } else {
                history.since_neutral |= flags & MODIFIERS;
            }
        }
        if let InputEvent::Key { code, .. } = event
            && let Some(fence) = self.recovery_ignore_through
        {
            // A delayed pre-recovery release must not erase a newer press of that key.
            if at <= fence && self.recovery_updated_keys.contains(&code) {
                return None;
            }
            if at > fence {
                self.recovery_updated_keys.insert(code);
            }
        }
        Some(match event {
            InputEvent::Key { code, down, .. } => {
                if down {
                    let down_at = self.key_down_at.entry(code).or_insert(at);
                    *down_at = (*down_at).max(at);
                    self.pressed_keys.insert(code)
                } else {
                    self.pressed_keys.remove(&code);
                    self.key_down_at.remove(&code);
                    false
                }
            }
            InputEvent::Flags(_)
            | InputEvent::PhysicalKeyDown { .. }
            | InputEvent::MouseDown
            | InputEvent::TapDisabled => false,
        })
    }

    pub fn process(&mut self, event: InputEvent, now: HostTime) -> Option<Press> {
        self.stale_keys_neutral_since = None;
        if matches!(event, InputEvent::TapDisabled) {
            let held = self.held();
            self.track_key_state(event, now);
            self.state = State::Dirty;
            return held.then_some(Press::HoldEnd);
        }
        let prior = self.modifier_history;
        self.track_key_state(event, now)?;
        let key_press = self.key_press(event, prior.flags);

        if self
            .recovery_ignore_through
            .is_some_and(|sampled_through| now <= sampled_through)
        {
            if key_press {
                log(format_args!(
                    "{:?}: press ignored inside a stale-key recovery window",
                    self.key
                ));
            }
            // A key can go down during the non-atomic scan. Keep its edge but
            // require a later neutral event before accepting another press.
            let neutral = matches!(
                event,
                InputEvent::Flags(flags) | InputEvent::Key { flags, .. }
                    if flags & MODIFIERS == 0 && self.pressed_keys.is_empty()
            );
            if !neutral && matches!(self.state, State::Idle | State::Dirty) {
                self.state = State::Dirty;
            }
            return None;
        }

        let ignored_because = key_press.then(|| self.resync_stale_press(now)).flatten();
        // Hex also takes the key left alone by another modifier's release (right ⌘
        // up while right ⌥ stays down); here only the key's own down edge counts.
        let pressed = key_press && self.pressed_keys.is_empty();
        if let InputEvent::PhysicalKeyDown { code } = event {
            if self.physical_key_downs.len() == PHYSICAL_KEY_DOWN_HISTORY {
                self.physical_key_downs.pop_front();
            }
            self.physical_key_downs.push_back((now, code));
        }
        // HID evidence can overtake the session event of the press it follows.
        if pressed
            && self.state == State::Idle
            && let Some(&(_, code)) = self
                .physical_key_downs
                .iter()
                .find(|(at, _)| *at > now && at.since(now) < HOLD)
        {
            log(format_args!(
                "{:?}: press dropped: HID key {code} overtook it",
                self.key
            ));
            self.state = State::Dirty;
            return None;
        }
        let flags = match event {
            InputEvent::Flags(flags) | InputEvent::Key { flags, .. } => Some(flags),
            InputEvent::PhysicalKeyDown { .. }
            | InputEvent::MouseDown
            | InputEvent::TapDisabled => None,
        };
        let released = flags.is_some_and(|flags| !self.key.down(flags));
        let other_key_down = matches!(
            event,
            InputEvent::Key { down: true, .. } | InputEvent::PhysicalKeyDown { .. }
        );
        let extra_modifiers =
            flags.is_some_and(|flags| self.key.down(flags) && !self.key.alone(flags));
        // Escape is never a chord (Hex #786).
        let chord_key_down = other_key_down
            && !matches!(
                event,
                InputEvent::Key { code: ESCAPE, .. } | InputEvent::PhysicalKeyDown { code: ESCAPE }
            );

        let press = match self.state {
            State::Idle if pressed => {
                self.state = State::Pressed {
                    at: now,
                    held: false,
                };
                None
            }
            State::Pressed { at, held } if released => {
                self.state = State::Idle;
                if held {
                    Some(Press::HoldEnd)
                } else if now.since(at) < HOLD {
                    Some(Press::Tap)
                } else {
                    // A hold of a key that reports none.
                    None
                }
            }
            State::Pressed { at, held }
                // Delayed HID evidence from before the press is not a chord.
                if !(matches!(event, InputEvent::PhysicalKeyDown { .. }) && now < at)
                    // Any key with the modifier is a chord, held or not; a click or
                    // another modifier only before the press is a hold.
                    && (chord_key_down
                        || (now.since(at) < HOLD
                            && (matches!(event, InputEvent::MouseDown) || extra_modifiers))) =>
            {
                self.state = State::Dirty;
                let cause = match event {
                    InputEvent::PhysicalKeyDown { .. } => Discard::HidKey,
                    InputEvent::Key { .. } if chord_key_down => Discard::Key,
                    InputEvent::MouseDown => Discard::Mouse,
                    _ => Discard::ExtraModifier,
                };
                log(format_args!(
                    "{:?}: chord ({cause:?}) {} ms after the press",
                    self.key,
                    now.since(at).as_millis()
                ));
                held.then_some(Press::HoldEnd)
            }
            State::Dirty
                if flags.is_some_and(|flags| flags & MODIFIERS == 0)
                    && self.pressed_keys.is_empty() =>
            {
                self.state = State::Idle;
                None
            }
            _ => None,
        };
        if press.is_none()
            && let Some(reason) = ignored_because
        {
            log(format_args!("{:?}: press ignored: {reason}", self.key));
        }
        press
    }

    /// A down edge of the key, whether or not tracked keys block it.
    fn key_press(&self, event: InputEvent, prior_flags: u64) -> bool {
        matches!(event, InputEvent::Flags(flags) if self.key.alone(flags))
            && !self.key.down(prior_flags)
    }

    /// A system shortcut such as ⌘Tab swallows its key's release, and the stale
    /// key blocks the next press until [`Self::recover_stale_keys`] has sampled
    /// a neutral keyboard. A press of the key itself resynchronizes at once, as
    /// a fresh press, when the keyboard reported no modifiers after every
    /// tracked key went down, only this key's modifier went down since, and both
    /// the tracked keys and every other modifier read up now. Returns why the
    /// press stays ignored.
    fn resync_stale_press(&mut self, now: HostTime) -> Option<&'static str> {
        let has_stale = !self.pressed_keys.is_empty();
        match self.state {
            State::Dirty if has_stale => {}
            State::Dirty => return Some("waiting for a neutral keyboard"),
            // Tracked keys block a modifier-only press outside Dirty.
            State::Idle if has_stale => {}
            _ => return None,
        }
        let history = self.modifier_history;
        // Typing after the switch is fine; only the stale keys' own presses count.
        let neutral_since_down = |code: &u16| {
            history
                .neutral_at
                .zip(self.key_down_at.get(code).copied())
                .is_some_and(|(neutral, down)| neutral > down)
        };
        if !self.pressed_keys.iter().all(neutral_since_down) {
            return Some("no neutral keyboard since the tracked keys went down");
        }
        if history.since_neutral & !self.key.flag() != 0 {
            return Some("another modifier went down since the keyboard was neutral");
        }
        if self
            .pressed_keys
            .iter()
            .any(|&code| (self.physical.key_down)(code))
        {
            return Some("a tracked key is physically down");
        }
        // As in recovery, arrow keys can leave fn set after release.
        let mut others = MODIFIERS & !self.key.flag();
        if self.function_modifier.1 == Some(false) {
            others &= !FUNCTION;
        }
        if (self.physical.flags)() & others != 0 {
            return Some("another modifier is physically down");
        }
        log(format_args!(
            "{:?}: forgot stale keys {:?} on a press",
            self.key, self.pressed_keys
        ));
        self.pressed_keys.clear();
        self.key_down_at.clear();
        self.state = State::Idle;
        self.stale_keys_neutral_since = None;
        // Edges stamped before the press must not finish or restart it. Edges at
        // the press itself still count, so a same-tick chord discards it.
        self.recovery_ignore_through = now.0.checked_sub(1).map(HostTime);
        self.recovery_updated_keys.clear();
        None
    }
}

/// Both keys and the draw key, fed every event.
pub struct Keys {
    capture: Trigger,
    voice: Trigger,
    /// Whether fn is down, from modifier events only: key events carry fn as
    /// navigation metadata on arrows. `false` after a blind period.
    draw: bool,
}

impl Keys {
    pub fn new(physical: Physical) -> Self {
        Self {
            capture: Trigger::new(Key::RIGHT_COMMAND, false, physical),
            voice: Trigger::new(Key::RIGHT_OPTION, true, physical),
            draw: false,
        }
    }

    /// Whether fn is down now, as the events have told it.
    pub fn draw_held(&self) -> bool {
        self.draw
    }

    /// The events `event` at `at` makes, in order.
    pub fn feed(&mut self, event: InputEvent, at: HostTime) -> Vec<KeyEvent> {
        let mut out = self.tick(at);
        if let Some(press) = self.capture.process(event, at)
            && press == Press::Tap
        {
            out.push(KeyEvent::Pressed(Chord::Capture));
        }
        if let Some(press) = self.voice.process(event, at) {
            out.push(KeyEvent::Voice(press));
        }
        let draw = match event {
            InputEvent::Flags(flags) => flags & FUNCTION != 0,
            InputEvent::TapDisabled => false,
            _ => self.draw,
        };
        if draw != self.draw {
            self.draw = draw;
            out.push(if draw {
                KeyEvent::DrawDown
            } else {
                KeyEvent::DrawUp
            });
        }
        out
    }

    /// The holds due by `now`.
    pub fn tick(&mut self, now: HostTime) -> Vec<KeyEvent> {
        self.voice
            .tick(now)
            .map(KeyEvent::Voice)
            .into_iter()
            .collect()
    }

    /// When the next hold is due, if a press is down.
    pub fn hold_due(&self) -> Option<HostTime> {
        self.voice.hold_due()
    }

    pub fn recoverable(&self) -> bool {
        self.capture.recoverable() || self.voice.recoverable()
    }

    pub fn recover_stale_keys(
        &mut self,
        now: HostTime,
        mut flags: impl FnMut() -> u64,
        mut key_down: impl FnMut(u16) -> bool,
    ) {
        self.capture
            .recover_stale_keys(now, &mut flags, &mut key_down);
        self.voice
            .recover_stale_keys(now, &mut flags, &mut key_down);
    }
}

#[cfg(test)]
mod tests;
