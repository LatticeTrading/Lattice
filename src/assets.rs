//! App-owned SVGs composed with Kit's bundled icons; no runtime file reads.

use std::borrow::Cow;

use gpui_kit::{AssetSource, SharedString, assets::Assets};

pub const CONNECTION_STATUS: &str = "lattice/connection-status.svg";

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if path == CONNECTION_STATUS {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/connection-status.svg"
            ))));
        }
        if path.starts_with("lattice/") {
            return Ok(None);
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        if CONNECTION_STATUS.starts_with(path) {
            paths.push(CONNECTION_STATUS.into());
        }
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_icons_are_available_without_filesystem_or_network() {
        for path in [
            CONNECTION_STATUS,
            "icons/search.svg",
            "icons/plus.svg",
            "icons/settings.svg",
            "icons/window-close.svg",
            "icons/window-minimize.svg",
            "icons/window-maximize.svg",
            "icons/window-restore.svg",
        ] {
            assert!(AppAssets.load(path).unwrap().is_some(), "{path}");
        }
        assert!(AppAssets.load("lattice/missing.svg").unwrap().is_none());
        assert_eq!(
            AppAssets.list("lattice/").unwrap(),
            vec![SharedString::from(CONNECTION_STATUS)]
        );
    }
}
