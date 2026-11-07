//! What the user can ask the editor to do, independent of buttons, keys and menus.
use crate::{
    animation::{AnimationControl, Entrance, Motion},
    backdrop::{Control, Format},
    document::Tool,
    style::Style,
};
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
    /// Opens or closes the Backdrop panel. Opening it frames an unframed image.
    ToggleBackdrop,
    ToggleAnimationPanel,
    /// Back to the active tool's options.
    ClosePanel,
    /// Turns framing off or on, keeping its settings.
    ToggleBackdropEnabled,
    SetBackdropFormat(Format),
    /// Solid or gradient, without motion.
    SetBackdropFill {
        gradient: bool,
    },
    SelectMotion(Motion),
    /// `None` picks a fresh variation.
    RandomizeMotion(Option<u32>),
    SetBackdropPreset(usize),
    SetBackdropColor {
        stop: usize,
        rgb: [u8; 3],
        remember: bool,
    },
    /// Opens the colour picker on one of the backdrop's colours.
    EditBackdropColor(usize),
    PickBackdropScreenColor(usize),
    /// `remember: false` joins the last undo step, as a slider drag does after its first move.
    SetBackdropControl {
        control: Control,
        value: u32,
        remember: bool,
    },
    TogglePlayback,
    ReplayAnimation,
    /// Seconds into the clip.
    SeekAnimation(f32),
    SelectEntrance(Entrance),
    /// Exit returns to the empty backdrop before repeating; otherwise the image holds.
    SetAnimationExit(bool),
    SetAnimationControl {
        control: AnimationControl,
        value: u32,
        remember: bool,
    },
    Export(AnimationFormat),
    CancelExport,
    Cancel,
    Help,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationFormat {
    Gif,
    Mp4,
}
