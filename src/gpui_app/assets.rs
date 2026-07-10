//! Embedded asset source for GPUI component icons
//!
//! Provides SVG icons required by gpui-component's TitleBar and other components.

use gpui::AssetSource;
use std::borrow::Cow;

/// Embedded asset source that provides SVG icons from compile-time included files.
pub struct EmbeddedAssets;

impl AssetSource for EmbeddedAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let data: Option<&'static [u8]> = match path {
            "icons/window-close.svg" => Some(include_bytes!("../../assets/icons/window-close.svg")),
            "icons/window-maximize.svg" => {
                Some(include_bytes!("../../assets/icons/window-maximize.svg"))
            }
            "icons/window-minimize.svg" => {
                Some(include_bytes!("../../assets/icons/window-minimize.svg"))
            }
            "icons/window-restore.svg" => {
                Some(include_bytes!("../../assets/icons/window-restore.svg"))
            }
            _ => None,
        };
        Ok(data.map(|d| Cow::Borrowed(d)))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        Ok(vec![])
    }
}
