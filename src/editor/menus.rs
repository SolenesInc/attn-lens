//! Native menus. Each item dispatches the same action a key or button does. Key
//! equivalents go to the window first, so a focused text field keeps its own keys.
use super::{Editor, actions::Action};
use crate::document::Tool;
use rune::{App, MenuItem};

macro_rules! item {
    ($menu:literal, $title:literal, $action:expr) => {
        MenuItem::new($title, |s: &mut Editor, tx| s.dispatch_ui($action, tx)).menu($menu)
    };
    ($menu:literal, $title:literal, $action:expr, $key:literal) => {
        item!($menu, $title, $action)
            .key_equivalent($key.parse().expect("menu shortcut"))
            .keys_first()
    };
}

pub fn install(mut app: App<Editor>) -> App<Editor> {
    let items = [
        item!("File", "Open Image…", Action::OpenImage, "cmd+o"),
        item!("File", "Save PNG…", Action::SaveImage, "cmd+s"),
        item!("Edit", "Copy as OCR", Action::CopyOcr),
        item!("Edit", "Paste Image", Action::PasteImage),
        item!("Edit", "Delete", Action::Delete),
        item!("Edit", "Duplicate", Action::DuplicateSelection, "cmd+d"),
        item!("Draw", "Select Objects", Action::SelectTool(Tool::Select)),
        MenuItem::separator().menu("Draw"),
        item!("Draw", "Arrow", Action::SelectTool(Tool::Arrow)),
        item!("Draw", "Text", Action::SelectTool(Tool::Text)),
        item!("Draw", "Rectangle", Action::SelectTool(Tool::Rectangle)),
        item!("Draw", "Freehand Drawing", Action::SelectTool(Tool::Pen)),
        item!("Draw", "Highlighter", Action::SelectTool(Tool::Highlight)),
        item!("Draw", "Pixelate", Action::SelectTool(Tool::Pixelate)),
        item!("Draw", "Counter", Action::SelectTool(Tool::Counter)),
        item!("Draw", "Spotlight", Action::SelectTool(Tool::Spotlight)),
        item!("Draw", "Magnifier", Action::SelectTool(Tool::Magnifier)),
        MenuItem::separator().menu("Draw"),
        item!("Draw", "Crop", Action::SelectTool(Tool::Crop)),
        item!("Draw", "Resize & Rotate", Action::ToggleImageTools),
        item!("Zoom", "Fit to Window", Action::Fit, "cmd+1"),
        item!("Zoom", "Actual Size", Action::ActualSize, "cmd+0"),
        MenuItem::separator().menu("Zoom"),
        item!("Zoom", "Zoom In", Action::Zoom(1.25), "cmd+="),
        item!("Zoom", "Zoom Out", Action::Zoom(0.8), "cmd+-"),
        item!("Help", "Attn Lens Help", Action::Help),
    ];
    for item in items {
        app = app.menu_item(item);
    }
    // Undo, Redo, Copy, Paste and Select All come with the standard Edit menu, which
    // reaches `on_key` the way a typed key does.
    app.standard_menus()
}
