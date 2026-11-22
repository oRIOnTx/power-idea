//! App assets: our own icons (brand marks Lucide doesn't have) on top of gpui-kit's set.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

/// Icons bundled into the binary, served under `app-icons/`.
const ICONS: &[(&str, &[u8])] = &[(
    "app-icons/docker.svg",
    include_bytes!("../assets/icons/docker.svg"),
)];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match ICONS.iter().find(|(name, _)| *name == path) {
            Some((_, bytes)) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit::assets::AllAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut names = gpui_kit::assets::AllAssets.list(path)?;
        names.extend(
            ICONS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| (*name).into()),
        );
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::AssetSource as _;

    use super::Assets;

    #[test]
    fn serves_own_icons_and_falls_back_to_the_kit() {
        assert!(Assets.load("app-icons/docker.svg").unwrap().is_some());
        assert!(Assets.load("icons/container.svg").unwrap().is_some());
    }
}
