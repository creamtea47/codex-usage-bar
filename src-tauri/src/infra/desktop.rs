use tauri::AppHandle;
#[cfg(target_os = "macos")]
use tauri::Manager;
pub(crate) const MAIN_WINDOW_LABEL: &str = "main";
pub(crate) const SETTINGS_WINDOW_LABEL: &str = "settings";
/// macOS 只有在两个应用窗口都隐藏时才移除 Dock 图标，避免设置窗口仍可见却失去应用入口。
pub fn sync_platform_visibility(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let any_visible = ["main", "settings"].iter().any(|label| {
            app.get_webview_window(label)
                .and_then(|window| window.is_visible().ok())
                .unwrap_or(false)
        });
        apply_macos_visibility_policy(app, any_visible);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

#[cfg(target_os = "macos")]
pub(crate) fn prepare_macos_application_for_window_activation(app: &AppHandle) {
    if app.show().is_err() {
        log::warn!("无法从托盘恢复应用可见性。");
    }
    apply_macos_visibility_policy(app, true);
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn prepare_macos_application_for_window_activation(_app: &AppHandle) {}

#[cfg(target_os = "macos")]
fn apply_macos_visibility_policy(app: &AppHandle, visible: bool) {
    use tauri::ActivationPolicy;
    let _ = app.set_dock_visibility(visible);
    let policy = if visible {
        ActivationPolicy::Regular
    } else {
        ActivationPolicy::Accessory
    };
    let _ = app.set_activation_policy(policy);
}
