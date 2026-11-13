//! Explicit, local screen sampling. AppKit must be called on the main thread.
#![allow(unexpected_cfgs)] // objc 0.2 macros use a legacy cargo-clippy feature.
type SampleResult = Result<Option<[u8; 3]>, String>;
pub fn sample_screen_color() -> Result<async_channel::Receiver<SampleResult>, String> {
    let (sender, receiver) = async_channel::bounded(1);
    #[cfg(target_os = "macos")]
    unsafe {
        use objc::{class, msg_send, runtime::Object, sel, sel_impl};
        type Id = *mut Object;
        let sampler: Id = msg_send![class!(NSColorSampler), new];
        let handler = block::ConcreteBlock::new(move |color: Id| {
            let result = if color.is_null() {
                Ok(None)
            } else {
                let space: Id = msg_send![class!(NSColorSpace), sRGBColorSpace];
                let color: Id = msg_send![color, colorUsingColorSpace: space];
                if color.is_null() {
                    Err("The sampled color could not be converted to sRGB".into())
                } else {
                    let r: f64 = msg_send![color, redComponent];
                    let g: f64 = msg_send![color, greenComponent];
                    let b: f64 = msg_send![color, blueComponent];
                    Ok(Some(
                        [r, g, b].map(|v| (v.clamp(0., 1.) * 255.).round() as u8),
                    ))
                }
            };
            let _ = sender.try_send(result);
        })
        .copy();
        let _: () = msg_send![sampler, showSamplerWithSelectionHandler: &*handler];
        // AppKit retains the sampler and copies the handler until selection/cancel.
        let _: () = msg_send![sampler, release];
    }
    #[cfg(target_os = "linux")]
    {
        super::linux::permission()?;
        std::thread::spawn(move || {
            let result = std::process::Command::new("hyprpicker")
                .args(["--format", "hex", "--no-fancy"])
                .output()
                .map_err(|e| {
                    format!(
                        "Cannot run hyprpicker: {e}. Install hyprpicker for screen color sampling."
                    )
                })
                .and_then(|output| {
                    if !output.status.success() {
                        if output.stderr.is_empty() {
                            return Ok(None);
                        }
                        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
                    }
                    let text = String::from_utf8_lossy(&output.stdout);
                    if text.trim().is_empty() {
                        return Ok(None);
                    }
                    parse_hex(&text)
                        .map(Some)
                        .ok_or_else(|| "Screen sampler returned an invalid hex color".into())
                });
            let _ = sender.send_blocking(result);
        });
    }
    Ok(receiver)
}

/// Parses the helper's `#RGB` or `#RRGGBB` output.
#[cfg(target_os = "linux")]
fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let text = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if !text.is_ascii() || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match text.len() {
        3 => Some(std::array::from_fn(|i| {
            u8::from_str_radix(&text[i..i + 1], 16).unwrap() * 17
        })),
        6 => Some(std::array::from_fn(|i| {
            u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap()
        })),
        _ => None,
    }
}
