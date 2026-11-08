use image::{ImageReader, RgbaImage};
use std::path::PathBuf;
mod color_sampler;
pub use color_sampler::sample_screen_color;
pub fn load(path: &std::path::Path) -> Result<RgbaImage, String> {
    let reader = ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16000);
    limits.max_image_height = Some(16000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    let mut reader = reader;
    reader.limits(limits);
    reader
        .decode()
        .map(|i| i.to_rgba8())
        .map_err(|e| e.to_string())
}
/// Writes a PNG through a scratch file beside the destination, so a failed write leaves
/// any existing file as it was. A link is written through to its file, and an existing
/// file keeps its permissions; a new one gets the umask's, like any other new file.
pub fn save_png(image: &RgbaImage, path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let path = match std::fs::canonicalize(path) {
        Ok(target) => target,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(path).is_ok() {
                return Err(format!(
                    "{} links to a file that does not exist.",
                    path.display()
                ));
            }
            path.to_path_buf()
        }
        Err(error) => return Err(error.to_string()),
    };
    let folder = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    // Created 0666 less the umask, as `open` would create the file itself.
    let scratch = tempfile::Builder::new()
        .prefix(".attn-lens-")
        .suffix(".png")
        .permissions(std::fs::Permissions::from_mode(0o666))
        .tempfile_in(folder)
        .map_err(|e| e.to_string())?;
    if let Ok(existing) = std::fs::metadata(&path) {
        std::fs::set_permissions(scratch.path(), existing.permissions())
            .map_err(|e| e.to_string())?;
    }
    image
        .write_to(
            &mut std::io::BufWriter::new(scratch.as_file()),
            image::ImageFormat::Png,
        )
        .map_err(|e| e.to_string())?;
    scratch.persist(&path).map_err(|e| e.error.to_string())?;
    Ok(())
}
/// Where a Save dialog starts.
pub fn pictures_folder() -> Option<PathBuf> {
    home_folder("Pictures")
}

/// Where an MP4's Save dialog starts.
pub fn movies_folder() -> Option<PathBuf> {
    home_folder("Movies")
}

fn home_folder(name: &str) -> Option<PathBuf> {
    let folder = PathBuf::from(std::env::var_os("HOME")?).join(name);
    folder.is_dir().then_some(folder)
}
#[cfg(target_os = "macos")]
pub fn copy(image: RgbaImage) -> Result<(), String> {
    arboard::Clipboard::new()
        .map_err(|e| e.to_string())?
        .set_image(arboard::ImageData {
            width: image.width() as usize,
            height: image.height() as usize,
            bytes: std::borrow::Cow::Owned(image.into_raw()),
        })
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
pub fn clipboard_image() -> Result<RgbaImage, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let data = clipboard
        .get_image()
        .map_err(|_| "The clipboard doesn’t contain an image.".to_string())?;
    if data.width > 16000
        || data.height > 16000
        || data.width.saturating_mul(data.height) > 64_000_000
    {
        return Err("Clipboard image is too large.".into());
    }
    RgbaImage::from_raw(
        data.width as u32,
        data.height as u32,
        data.bytes.into_owned(),
    )
    .ok_or_else(|| "Invalid clipboard image.".into())
}

// Changing a save-dialog path can bypass its overwrite confirmation.
// An explicitly chosen filename may be replaced; an appended extension must
// never turn an unconfirmed choice into a replacement (including symlinks).
pub fn checked_export_path(mut path: PathBuf, extension: &str) -> Result<PathBuf, String> {
    if path.extension().is_none() {
        path.set_extension(extension);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => return Err("That filename already exists; choose the full filename in the save dialog to confirm replacement.".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.to_string()),
        }
    } else if path.extension().is_none_or(|ext| ext != extension) {
        return Err(format!("Choose a filename ending in .{extension}"));
    }
    Ok(path)
}

#[cfg(target_os = "macos")]
const SYSTEM_ANNOTATION_FONT: &str = "Arial";
#[cfg(target_os = "linux")]
const SYSTEM_ANNOTATION_FONT: &str = "DejaVu Sans";
#[cfg(target_os = "macos")]
pub const UI_FONT: &str = ".AppleSystemUIFont";
#[cfg(target_os = "linux")]
pub const UI_FONT: &str = "DejaVu Sans";

pub const FALLBACK_FONT_DATA: &[u8] = include_bytes!("../assets/fonts/Roboto-Regular.ttf");

struct AnnotationFont {
    family: &'static str,
    font: ab_glyph::FontArc,
}

fn load_annotation_font(paths: &[&str]) -> AnnotationFont {
    if let Some(font) = paths
        .iter()
        .find_map(|path| ab_glyph::FontArc::try_from_vec(std::fs::read(path).ok()?).ok())
    {
        AnnotationFont {
            family: SYSTEM_ANNOTATION_FONT,
            font,
        }
    } else {
        AnnotationFont {
            family: "Roboto",
            font: ab_glyph::FontArc::try_from_slice(FALLBACK_FONT_DATA)
                .expect("The embedded annotation font must be valid"),
        }
    }
}

fn annotation_font_data() -> &'static AnnotationFont {
    static FONT: std::sync::OnceLock<AnnotationFont> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        #[cfg(target_os = "macos")]
        let paths = ["/System/Library/Fonts/Supplemental/Arial.ttf"];
        #[cfg(target_os = "linux")]
        let paths = [
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ];
        load_annotation_font(&paths)
    })
}

pub fn annotation_font() -> &'static ab_glyph::FontArc {
    &annotation_font_data().font
}

pub fn annotation_font_family() -> &'static str {
    annotation_font_data().family
}

pub fn shortcut_label(label: &str) -> String {
    if cfg!(target_os = "linux") {
        label
            .replace('⌘', "Ctrl+")
            .replace('⌥', "Alt+")
            .replace('⇧', "Shift+")
    } else {
        label.into()
    }
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{clipboard_image, copy};
#[cfg(test)]
mod tests {
    use super::checked_export_path;

    #[test]
    fn saving_keeps_the_destination_mode_and_writes_through_links() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;

        let existing = dir.path().join("existing.png");
        std::fs::write(&existing, "old").unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o644)).unwrap();
        super::save_png(&image, &existing).unwrap();
        assert_eq!(mode(&existing), 0o644);
        assert_eq!(image::open(&existing).unwrap().width(), 3);

        // A new file gets what any other new file in this folder gets.
        let reference = dir.path().join("reference");
        std::fs::write(&reference, "").unwrap();
        let new = dir.path().join("new.png");
        super::save_png(&image, &new).unwrap();
        assert_eq!(mode(&new), mode(&reference));

        let target = dir.path().join("target.png");
        std::fs::write(&target, "old").unwrap();
        let link = dir.path().join("link.png");
        symlink(&target, &link).unwrap();
        super::save_png(&image, &link).unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(image::open(&target).unwrap().width(), 3);
    }
    #[test]
    fn missing_or_invalid_system_fonts_render_with_the_embedded_fallback() {
        use ab_glyph::Font;
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.ttf");
        let invalid = dir.path().join("invalid.ttf");
        std::fs::write(&invalid, b"not a font").unwrap();
        for path in [&missing, &invalid] {
            let loaded = super::load_annotation_font(&[path.to_str().unwrap()]);
            assert_eq!(loaded.family, "Roboto");
            for label in ["1", "10", "999", "Glance"] {
                assert!(label.chars().all(|c| loaded.font.glyph_id(c).0 != 0));
                let mut image = image::GrayImage::new(160, 60);
                imageproc::drawing::draw_text_mut(
                    &mut image,
                    image::Luma([255]),
                    10,
                    10,
                    28.,
                    &loaded.font,
                    label,
                );
                assert!(image.pixels().any(|p| p[0] > 128), "Missing ink: {label}");
            }
        }
    }
    #[test]
    fn exported_text_uses_an_available_platform_font() {
        let mut document = crate::document::Document::new(image::RgbaImage::from_pixel(
            200,
            60,
            image::Rgba([255, 255, 255, 255]),
        ));
        document.commit(crate::document::Mark {
            style: Default::default(),
            tool: crate::document::Tool::Text,
            curve: None,
            points: vec![(10., 10.)],
            color: [0, 0, 0, 255],
            width: 4.,
            text: "Glance".into(),
            drawn: None,
        });
        assert!(
            document.render(None).pixels().any(|pixel| pixel[0] < 128),
            "Text must survive export"
        );
    }
    #[test]
    fn appended_export_extensions_cannot_bypass_overwrite_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        for extension in ["png", "gif", "mp4"] {
            let chosen = dir.path().join(format!("screenshot.{extension}"));
            std::fs::write(&chosen, b"original").unwrap();
            assert!(checked_export_path(dir.path().join("screenshot"), extension).is_err());
            assert_eq!(
                checked_export_path(chosen.clone(), extension).unwrap(),
                chosen
            );
            assert_eq!(std::fs::read(&chosen).unwrap(), b"original");
            assert!(checked_export_path(dir.path().join("screenshot.other"), extension).is_err());
            let new = dir.path().join(format!("new-{extension}"));
            assert_eq!(
                checked_export_path(new.clone(), extension).unwrap(),
                new.with_extension(extension)
            );
        }
        let link = dir.path().join("dangling.png");
        std::os::unix::fs::symlink(dir.path().join("missing"), &link).unwrap();
        assert!(checked_export_path(dir.path().join("dangling"), "png").is_err());
        assert!(link.is_symlink());
    }
}
