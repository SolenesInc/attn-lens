//! What the user can ask the editor to do, independent of buttons, keys and menus.
use crate::{document::Tool, style::Style};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenImage,
    OpenPath(PathBuf),
    SaveImage,
    CopyImage,
    PasteImage,
    CopyOcr,
    Undo,
    Redo,
    Delete,
    DuplicateSelection,
    SelectAll,
    SelectTool(Tool),
    /// The colour of the selected marks, otherwise of the active tool's next mark. Each
    /// mark keeps its own opacity.
    SetRgb([u8; 3]),
    /// The opacity of the selected marks, otherwise of the active tool's next mark. Each
    /// mark keeps its own colour.
    SetOpacity(u8),
    SetStrokeWidth(f32),
    SetAppearance(Style),
    SetMagnifierZoom(f32),
    SetCounterNumber(u32),
    SetCropRatio(Option<f32>),
    AddLinePoint,
    StraightenLine,
    NudgeSelection {
        delta: (f32, f32),
        remember: bool,
    },
    Fit,
    ActualSize,
    Zoom(f32),
    /// In points from the canvas's top-left.
    ZoomAt {
        factor: f32,
        anchor: (f32, f32),
    },
    PanBy((f32, f32)),
    ToggleImageTools,
    ToggleColorPicker,
    SetResizeScale(f32),
    ToggleSmartResize,
    ApplyResize,
    Rotate,
    PickScreenColor,
    Cancel,
    Help,
    Quit,
}
