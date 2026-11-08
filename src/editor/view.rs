//! The window: toolbar over canvas and sidebar.
use super::{CopyFeedback, Editor, actions::Action, icons};
use crate::document::Tool;
use rune::{Cursor, Cx, El, Icon, col, el, row, text, tok};

const TOOLS: [(Tool, &str, &Icon, &str); 11] = [
    (
        Tool::Select,
        "tool-select",
        &icons::MOUSE_POINTER_2,
        "Select (V)",
    ),
    (Tool::Pen, "tool-pen", &icons::PEN_LINE, "Draw (P)"),
    (
        Tool::Arrow,
        "tool-arrow",
        &icons::ARROW_UP_RIGHT,
        "Arrow (A)",
    ),
    (Tool::Rectangle, "tool-rectangle", &icons::SQUARE, "Box (R)"),
    (Tool::Text, "tool-text", &icons::TYPE, "Text (T)"),
    (
        Tool::Highlight,
        "tool-highlight",
        &icons::HIGHLIGHTER,
        "Highlight (H)",
    ),
    (
        Tool::Pixelate,
        "tool-pixelate",
        &icons::GRID_2X2,
        "Pixelate (B)",
    ),
    (Tool::Crop, "tool-crop", &icons::CROP, "Crop (X)"),
    (
        Tool::Counter,
        "tool-counter",
        &icons::LIST_ORDERED,
        "Step (N)",
    ),
    (
        Tool::Spotlight,
        "tool-spotlight",
        &icons::SCAN,
        "Spotlight (S)",
    ),
    (
        Tool::Magnifier,
        "tool-magnifier",
        &icons::SEARCH,
        "Magnifier (M)",
    ),
];

fn toolbar(s: &Editor, cx: &Cx<Editor>) -> El {
    let t = tok::theme(cx.system_dark());
    let button =
        |key: &str, label: &str, glyph: &'static Icon, toggle: Option<bool>, action: Action| {
            super::panels::controls::icon_button(cx, key, label.to_string(), glyph, toggle, action)
        };
    let mut tools = row()
        .gap(2.)
        .center_y()
        .child(
            text("attn lens")
                .font(tok::type_::SECTION)
                .color(t.text)
                .px(8.),
        )
        .child(button(
            "open",
            "Open image (⌘O)",
            &icons::FOLDER_OPEN,
            None,
            Action::OpenImage,
        ))
        .child(el().w(1.).h(22.).fill(t.separator));
    for (tool, key, glyph, label) in TOOLS {
        tools = tools.child(button(
            key,
            label,
            glyph,
            Some(s.tool == tool),
            Action::SelectTool(tool),
        ));
    }
    tools = tools
        .child(el().w(1.).h(22.).fill(t.separator))
        .child(button(
            "image-tools-button",
            "Image tools",
            &icons::MAXIMIZE,
            Some(s.panel == super::Panel::Image),
            Action::ToggleImageTools,
        ));
    let feedback = s.copy_feedback.map(|feedback| match feedback {
        CopyFeedback::Copied => "Copied!",
        CopyFeedback::ReadingText => "Reading text…",
        CopyFeedback::TextCopied => "Text copied!",
        CopyFeedback::NoText => "No text found",
    });
    let zoom = if s.doc.is_some() {
        format!("{:.0}%", (s.layout().scale * 100.).round())
    } else {
        "—".into()
    };
    let actions = row()
        .gap(2.)
        .center_y()
        .child(button(
            "undo",
            "Undo (⌘Z)",
            &icons::UNDO_2,
            None,
            Action::Undo,
        ))
        .child(button(
            "redo",
            "Redo (⇧⌘Z)",
            &icons::REDO_2,
            None,
            Action::Redo,
        ))
        .child(button(
            "copy",
            "Copy image (⌘C)",
            &icons::COPY,
            None,
            Action::CopyImage,
        ))
        .child(button(
            "copy-ocr",
            "Copy as OCR",
            &icons::SCAN_TEXT,
            None,
            Action::CopyOcr,
        ))
        .child(button(
            "save",
            "Save PNG (⌘S)",
            &icons::SAVE,
            None,
            Action::SaveImage,
        ))
        .child(button(
            "zoom-out",
            "Zoom out (⌘−)",
            &icons::ZOOM_OUT,
            None,
            Action::Zoom(0.8),
        ))
        .child(
            el().key("zoom-label")
                .role("button")
                .label("Fit to window (⌘1)")
                .a11y_value(zoom.clone())
                .px(4.)
                .center()
                .cursor(Cursor::Pointer)
                .on_press_tx(|s: &mut Editor, tx| s.dispatch_ui(Action::Fit, tx))
                .child(
                    text(zoom)
                        .font(tok::type_::LABEL)
                        .color(t.text_secondary)
                        .inert(),
                ),
        )
        .child(button(
            "zoom-in",
            "Zoom in (⌘=)",
            &icons::ZOOM_IN,
            None,
            Action::Zoom(1.25),
        ))
        .children(feedback.map(|feedback| {
            text(feedback)
                .key("copy-feedback")
                .font(tok::type_::LABEL)
                .color(t.text)
                .px(6.)
                .live_region(rune::LiveRegion::Polite)
        }));
    row()
        .key("toolbar")
        .w_full()
        .h(48.)
        .px(8.)
        .center_y()
        .justify_between()
        .fill(t.card)
        .child(tools)
        .child(actions)
}

pub fn view(s: &Editor, cx: &mut Cx<Editor>) -> El {
    let t = tok::theme(cx.system_dark());
    let root = col()
        .key("editor")
        .instant_changes()
        .w_full()
        .h_full()
        .fill(t.bg)
        .accepts_extensions(&["png", "jpg", "jpeg"])
        .on_drop_files(|s: &mut Editor, paths, tx| {
            if let Some(path) = paths.first() {
                s.dispatch_ui(Action::OpenPath(path.clone()), tx);
            }
        })
        .on_drop_rejected(|s: &mut Editor, _, _| {
            s.status = "Drop a PNG or JPEG image.".into();
        })
        .on_key_up(|s: &mut Editor, event, _| s.on_key_up(event))
        .children(s.state_probe())
        .child(toolbar(s, cx))
        .child(
            row()
                .w_full()
                .grow(1.)
                .min_h(0.)
                .child(s.canvas_view(cx))
                .child(s.sidebar(cx)),
        );
    // Always there, so a message never moves the canvas under the pointer.
    root.child(
        row()
            .key("status-bar")
            .w_full()
            .h(26.)
            .px(12.)
            .center_y()
            .fill(t.card)
            .child(
                text(s.status.clone())
                    .key("status")
                    .font(tok::type_::LABEL)
                    .color(t.text_secondary)
                    .truncate()
                    .live_region(rune::LiveRegion::Polite),
            ),
    )
}
