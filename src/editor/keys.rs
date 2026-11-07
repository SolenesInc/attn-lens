//! Window keys: tools, shortcuts, nudging, and the Space and Z holds.
use super::{Editor, actions::Action};
use crate::document::Tool;
use rune::{Key, KeyEvent, Tx};

impl Editor {
    /// Keys nothing focused took. True consumes the key.
    pub(crate) fn on_key(&mut self, event: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = event.mods;
        // Typing into a field: only commands past this point.
        let in_field = tx.focused().is_some();
        if let Key::Command(key) = event.key {
            let action = match key {
                'o' => Action::OpenImage,
                's' => Action::SaveImage,
                'c' => Action::CopyImage,
                'v' => Action::PasteImage,
                'a' => Action::SelectAll,
                'd' => Action::DuplicateSelection,
                'z' if mods.shift => Action::Redo,
                'z' => Action::Undo,
                '1' => Action::Fit,
                '0' => Action::ActualSize,
                '+' | '=' => Action::Zoom(1.25),
                '-' => Action::Zoom(0.8),
                'q' => Action::Quit,
                _ => return false,
            };
            self.dispatch_ui(action, tx);
            return true;
        }
        if in_field {
            return false;
        }
        if mods.alt || mods.ctrl {
            return false;
        }
        match &event.key {
            Key::Text(text) => {
                let action = match text.to_lowercase().as_str() {
                    " " => {
                        self.space_down = true;
                        return true;
                    }
                    "z" => {
                        self.zoom_down = true;
                        return true;
                    }
                    "v" => Action::SelectTool(Tool::Select),
                    "p" => Action::SelectTool(Tool::Pen),
                    "a" => Action::SelectTool(Tool::Arrow),
                    "r" => Action::SelectTool(Tool::Rectangle),
                    "t" => Action::SelectTool(Tool::Text),
                    "h" => Action::SelectTool(Tool::Highlight),
                    "b" => Action::SelectTool(Tool::Pixelate),
                    "x" => Action::SelectTool(Tool::Crop),
                    "n" => Action::SelectTool(Tool::Counter),
                    "s" => Action::SelectTool(Tool::Spotlight),
                    "m" => Action::SelectTool(Tool::Magnifier),
                    _ => return false,
                };
                self.dispatch_ui(action, tx);
                true
            }
            Key::Left | Key::Right | Key::Up | Key::Down if !self.selected.is_empty() => {
                let step = if mods.shift { 10. } else { 1. };
                let delta = match event.key {
                    Key::Left => (-step, 0.),
                    Key::Right => (step, 0.),
                    Key::Up => (0., -step),
                    _ => (0., step),
                };
                // Held repeats form one undo step.
                let remember = !tx.key_repeat();
                self.dispatch_ui(Action::NudgeSelection { delta, remember }, tx);
                true
            }
            Key::Backspace | Key::Delete => {
                self.dispatch_ui(Action::Delete, tx);
                true
            }
            Key::Escape => {
                self.dispatch_ui(Action::Cancel, tx);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn on_key_up(&mut self, event: &KeyEvent) -> bool {
        match &event.key {
            Key::Text(text) if text == " " => {
                self.space_down = false;
                self.end_pan();
                true
            }
            Key::Text(text) if text.eq_ignore_ascii_case("z") => {
                self.zoom_down = false;
                true
            }
            _ => false,
        }
    }

    /// A key window that loses focus never hears the release.
    pub(crate) fn on_window_focus(&mut self, focused: bool) {
        if !focused {
            self.space_down = false;
            self.zoom_down = false;
            self.end_pan();
        }
    }
}
