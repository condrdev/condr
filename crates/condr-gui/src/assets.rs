use gpui_kit::assets::Assets;
use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

pub(crate) const APP_LOGO: &str = "brand/condr.png";

/// Shared by the main and Settings windows. Wayland looks up condr.desktop;
/// X11 also accepts pixels directly. Windows uses the executable's icon resource.
pub(crate) fn window_options() -> gpui_kit::WindowOptions {
    gpui_kit::WindowOptions {
        app_id: Some("condr".into()),
        #[cfg(target_os = "linux")]
        icon: Some(std::sync::Arc::new(
            image::load_from_memory(include_bytes!("../../../assets/brand/condr.png"))
                .expect("bundled application icon is a valid PNG")
                .into_rgba8(),
        )),
        ..gpui_kit::component::TitleBar::window_options()
    }
}

/// Condr's own icons: status glyphs, the marks of the agent CLIs, kept for every agent
/// Condr may come to name (shared with the website from the repository's assets/agents/,
/// see its NOTICE), then the marks of the editors "Open in" launches
/// (assets/icons/NOTICE-EDITOR-ICONS). The file-type icons under `icons/jetbrains/` are
/// served from `app::file_icons` instead (assets/icons/NOTICE-FILE-ICONS).
const CONDR_ICON_PATHS: [&str; 28] = [
    "icons/circle.svg",
    "icons/circle-filled.svg",
    "icons/circle-alert.svg",
    "icons/server-plus.svg",
    "icons/folder-plus.svg",
    "icons/git-branch.svg",
    "icons/maximize-2.svg",
    "icons/minimize-2.svg",
    "icons/coffee.svg",
    "icons/claude.svg",
    "icons/codex.svg",
    "icons/opencode.svg",
    "icons/pi.svg",
    "icons/omp.svg",
    "icons/copilot.svg",
    "icons/kimi.svg",
    "icons/kilo.svg",
    "icons/qoder.svg",
    "icons/qwen.svg",
    "icons/cursor.svg",
    "icons/grok.svg",
    "icons/antigravity.svg",
    "icons/square-plus.svg",
    "icons/square-minus.svg",
    "icons/square-dot.svg",
    "icons/vscode.svg",
    "icons/zed.svg",
    "icons/intellij.svg",
];

pub(super) struct CondrAssets {
    pub(super) base: Assets,
}

impl CondrAssets {
    pub(super) fn new() -> Self {
        Self { base: Assets }
    }
}

impl AssetSource for CondrAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // The file-type icons (assets/icons/NOTICE-FILE-ICONS) come from their own table.
        if let Some(bytes) = crate::app::file_icons::icon_bytes(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        match path {
            APP_LOGO => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/brand/condr.png"
            )))),
            "icons/circle.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/circle.svg"
            )))),
            "icons/circle-filled.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/circle-filled.svg"
            )))),
            "icons/circle-alert.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/circle-alert.svg"
            )))),
            "icons/server-plus.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/server-plus.svg"
            )))),
            "icons/folder-plus.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/folder-plus.svg"
            )))),
            "icons/git-branch.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/git-branch.svg"
            )))),
            "icons/maximize-2.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/maximize-2.svg"
            )))),
            "icons/minimize-2.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/minimize-2.svg"
            )))),
            "icons/coffee.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/coffee.svg"
            )))),
            "icons/claude.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/claude.svg"
            )))),
            "icons/codex.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/codex.svg"
            )))),
            "icons/opencode.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/opencode.svg"
            )))),
            "icons/pi.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/pi.svg"
            )))),
            "icons/omp.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/omp.svg"
            )))),
            "icons/copilot.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/copilot.svg"
            )))),
            "icons/kimi.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/kimi.svg"
            )))),
            "icons/kilo.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/kilo.svg"
            )))),
            "icons/qoder.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/qoder.svg"
            )))),
            "icons/qwen.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/qwen.svg"
            )))),
            "icons/cursor.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/cursor.svg"
            )))),
            "icons/grok.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/grok.svg"
            )))),
            "icons/antigravity.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/agents/antigravity.svg"
            )))),
            "icons/square-plus.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/square-plus.svg"
            )))),
            "icons/square-minus.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/square-minus.svg"
            )))),
            "icons/square-dot.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/square-dot.svg"
            )))),
            "icons/vscode.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/vscode.svg"
            )))),
            "icons/zed.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/zed.svg"
            )))),
            "icons/intellij.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/intellij.svg"
            )))),
            _ => self.base.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = self.base.list(path)?;
        if APP_LOGO.starts_with(path) {
            assets.push(APP_LOGO.into());
        }
        assets.extend(
            CONDR_ICON_PATHS
                .into_iter()
                .filter(|asset| asset.starts_with(path))
                .map(SharedString::from),
        );
        assets.extend(
            crate::app::file_icons::icon_paths()
                .filter(|asset| asset.starts_with(path))
                .map(SharedString::from),
        );
        Ok(assets)
    }
}
