//! The editor's rune-kit theme: one Kit per appearance.
use std::sync::OnceLock;

pub(crate) fn kit(dark: bool) -> &'static rune_kit::Kit {
    static DARK: OnceLock<rune_kit::Kit> = OnceLock::new();
    static LIGHT: OnceLock<rune_kit::Kit> = OnceLock::new();
    let once = if dark { &DARK } else { &LIGHT };
    once.get_or_init(|| rune_kit::Kit::neutral(dark))
}
