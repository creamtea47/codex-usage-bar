use crate::domain::settings::Language;
use crate::infra::desktop::sync_platform_visibility;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
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
    let menu = Menu::with_items(app, &[&show_main, &open_settings])?;
    if let Some(text) = crate::app::lightweight::summary(app, language) {
        menu.append(&MenuItem::with_id(
            app,
            "memory-result",
            text,
            false,
            None::<&str>,
        )?)?;
    }
    menu.append(&separator)?;
    menu.append(&quit)?;
    Ok(menu)
}

fn should_show_main_window(event: &TrayIconEvent) -> bool {
    // Tauri 2.11.5 的 Up/Down 文案注释相反；官方示例与锁定的 tray-icon 0.24.2
    // 原生映射均以 Up 表示按钮释放。
    matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    )
}

/// 创建始终可用的托盘入口。显式“退出”是隐藏主窗口后关闭后台进程的唯一菜单动作。
pub fn create(app: &AppHandle, language: Language) -> tauri::Result<()> {
    let menu = create_menu(app, language)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("CodexUsageBar")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if should_show_main_window(&event) {
                show_main_window(tray.app_handle());
            }
        })
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
    show_window(app, "main");
}
fn show_settings_window(app: &AppHandle) {
    show_window(app, "settings");
}
fn show_window(app: &AppHandle, label: &'static str) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::app::lightweight::show(&app, label, None).await {
            log::warn!("Window activation failed: {label}, {error}");
        }
    });
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
    crate::app::lightweight::schedule(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::{tray::TrayIconId, PhysicalPosition, Rect};

    fn click_event(button: MouseButton, button_state: MouseButtonState) -> TrayIconEvent {
        TrayIconEvent::Click {
            id: TrayIconId::new(TRAY_ID),
            position: PhysicalPosition::new(0.0, 0.0),
            rect: Rect::default(),
            button,
            button_state,
        }
    }

    fn double_click_event(button: MouseButton) -> TrayIconEvent {
        TrayIconEvent::DoubleClick {
            id: TrayIconId::new(TRAY_ID),
            position: PhysicalPosition::new(0.0, 0.0),
            rect: Rect::default(),
            button,
        }
    }

    fn pointer_event(kind: &str) -> TrayIconEvent {
        let id = TrayIconId::new(TRAY_ID);
        let position = PhysicalPosition::new(0.0, 0.0);
        let rect = Rect::default();
        match kind {
            "enter" => TrayIconEvent::Enter { id, position, rect },
            "move" => TrayIconEvent::Move { id, position, rect },
            "leave" => TrayIconEvent::Leave { id, position, rect },
            _ => unreachable!("unsupported pointer event"),
        }
    }

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

    #[test]
    fn only_left_button_release_routes_to_main_window_activation() {
        let cases = [
            (
                "left release",
                click_event(MouseButton::Left, MouseButtonState::Up),
                true,
            ),
            (
                "left press",
                click_event(MouseButton::Left, MouseButtonState::Down),
                false,
            ),
            (
                "right release",
                click_event(MouseButton::Right, MouseButtonState::Up),
                false,
            ),
            (
                "right press",
                click_event(MouseButton::Right, MouseButtonState::Down),
                false,
            ),
            (
                "middle release",
                click_event(MouseButton::Middle, MouseButtonState::Up),
                false,
            ),
            (
                "middle press",
                click_event(MouseButton::Middle, MouseButtonState::Down),
                false,
            ),
            (
                "left double click",
                double_click_event(MouseButton::Left),
                false,
            ),
            (
                "right double click",
                double_click_event(MouseButton::Right),
                false,
            ),
            (
                "middle double click",
                double_click_event(MouseButton::Middle),
                false,
            ),
            ("pointer enter", pointer_event("enter"), false),
            ("pointer move", pointer_event("move"), false),
            ("pointer leave", pointer_event("leave"), false),
        ];

        for (case, event, expected) in cases {
            assert_eq!(should_show_main_window(&event), expected, "{case}");
        }
    }
}
