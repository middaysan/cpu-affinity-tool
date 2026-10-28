//! The Windows window, taskbar and tray share the bundled executable icon.

use eframe::egui::IconData;

pub fn load() -> Result<IconData, String> {
    let image = image::load_from_memory_with_format(
        include_bytes!("../assets/icon.ico"),
        image::ImageFormat::Ico,
    )
    .map_err(|error| format!("Failed to decode application icon: {error}"))?
    .to_rgba8();
    Ok(IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_ico_decodes_with_visible_alpha() {
        let icon = super::load().unwrap();
        assert_eq!((icon.width, icon.height), (64, 64));
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
    }
}
