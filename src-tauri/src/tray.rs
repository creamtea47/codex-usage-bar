use crate::models::Language;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

pub const TRAY_ID: &str = "codex-usage-bar";
const SHOW_MAIN_ID: &str = "show-main";
const OPEN_SETTINGS_ID: &str = "open-settings";
const QUIT_ID: &str = "quit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrayTexts {
    show_main: &'static str,
    open_settings: &'static str,
    quit: &'static str,
}

impl TrayTexts {
    fn from_language(language: Language) -> Self {
        match language {
            Language::ZhCn | Language::System => Self {
                show_main: "显示主窗口",
                open_settings: "打开设置",
                quit: "退出",
            },
            Language::En => Self {
                show_main: "Show main window",
                open_settings: "Open settings",
                quit: "Quit",
            },
        }
    }
}

fn create_menu(app: &AppHandle, language: Language) -> tauri::Result<Menu<tauri::Wry>> {
    let texts = TrayTexts::from_language(language);
    let show_main = MenuItem::with_id(app, SHOW_MAIN_ID, texts.show_main, true, None::<&str>)?;
    let open_settings = MenuItem::with_id(
        app,
        OPEN_SETTINGS_ID,
        texts.open_settings,
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, QUIT_ID, texts.quit, true, None::<&str>)?;
    Menu::with_items(app, &[&show_main, &open_settings, &separator, &quit])
}

/// 创建始终可用的托盘入口。显式“退出”是隐藏主窗口后关闭后台进程的唯一菜单动作。
pub fn create(app: &AppHandle, language: Language) -> tauri::Result<()> {
    let menu = create_menu(app, language)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("CodexUsageBar")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW_MAIN_ID => show_main_window(app),
            OPEN_SETTINGS_ID => show_settings_window(app),
            QUIT_ID => {
                log::info!("用户通过托盘菜单退出应用。");
                app.exit(0);
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    } else {
        log::warn!("无法取得默认窗口图标，托盘将使用系统回退图标。");
    }
    builder.build(app)?;
    Ok(())
}

pub fn update_menu(app: &AppHandle, language: Language) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    match create_menu(app, language).and_then(|menu| tray.set_menu(Some(menu))) {
        Ok(()) => {}
        Err(_) => log::warn!("无法更新托盘菜单语言。"),
    }
}

pub fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        log::warn!("无法从托盘定位主窗口。");
        return;
    };
    #[cfg(target_os = "windows")]
    let _ = window.set_skip_taskbar(false);
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    apply_macos_visibility_policy(app, true);
}

fn show_settings_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("settings") else {
        log::warn!("无法从托盘定位设置窗口。");
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    apply_macos_visibility_policy(app, true);
}

pub fn hide_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.hide().is_err() {
        log::warn!("无法隐藏主窗口到系统托盘。");
        return;
    }
    #[cfg(target_os = "windows")]
    let _ = window.set_skip_taskbar(true);
    sync_platform_visibility(app);
}

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

#[cfg(not(target_os = "macos"))]
fn apply_macos_visibility_policy(_app: &AppHandle, _visible: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_menu_copy_is_complete_in_both_supported_languages() {
        assert_eq!(
            TrayTexts::from_language(Language::ZhCn),
            TrayTexts {
                show_main: "显示主窗口",
                open_settings: "打开设置",
                quit: "退出",
            }
        );
        assert_eq!(
            TrayTexts::from_language(Language::En),
            TrayTexts {
                show_main: "Show main window",
                open_settings: "Open settings",
                quit: "Quit",
            }
        );
    }
}
