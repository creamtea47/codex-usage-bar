use crate::app::account_hub::AccountHub;
use crate::app::continuation_scheduler::start_quota_auto_continue_loop;
use crate::app::lightweight;
use crate::app::refresh::start_refresh_loop;
use crate::app::state::resolved_language;
use crate::app::state::AppState;
use crate::app::state::SETTINGS_FILE_NAME;
use crate::app::tray;
use crate::app::updates::start_update_check_loop;
use crate::app::windows::restore_main_window;
use crate::app::windows::restore_settings_window;
use crate::app::windows::schedule_geometry_save;
use crate::app::windows::settings_window_title;
use crate::app::windows::MAIN_COMPACT_HEIGHT;
use crate::infra::desktop::MAIN_WINDOW_LABEL;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::infra::network;
use crate::infra::settings::apply_compact_layout_migration;
use crate::infra::settings::cleanup_logs;
use crate::infra::settings::load_settings;
use crate::infra::settings::save_settings as persist_settings;
use crate::infra::usage::UsageClient;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;
use tauri::WindowEvent;
use tauri_plugin_log::Target;
use tauri_plugin_log::TargetKind;

pub(crate) const LOG_RETENTION: Duration = Duration::from_secs(14 * 24 * 60 * 60);

pub fn run() {
    let mut builder = tauri::Builder::default();

    // 必须在会创建窗口、托盘或后台任务的插件之前注册，确保重复启动只唤醒首个实例。
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            handle_second_instance(args.len(), || tray::show_main_window(app));
        }));
    }

    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // 仅写入应用日志目录；关闭默认 stdout/Trace，避免第三方依赖输出无关运行细节。
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([Target::new(TargetKind::LogDir {
                    file_name: Some("codex-usage-bar".into()),
                })])
                .level(log::LevelFilter::Info)
                // Updater 依赖可能记录远端 URL 或响应解析细节；本应用已自行写入脱敏的结果类别，
                // 因此不把该依赖的原始日志落盘，保证日志边界不随第三方实现变化。
                .filter(|metadata| metadata.target().starts_with("codex_usage_bar"))
                .build(),
        )
        .setup(|app| {
            let config_directory = app
                .path()
                .app_config_dir()
                .map_err(|_| io::Error::other("无法定位应用配置目录。"))?;
            let settings_path = config_directory.join(SETTINGS_FILE_NAME);
            let mut stored_settings = load_settings(&settings_path);
            if apply_compact_layout_migration(&mut stored_settings, MAIN_COMPACT_HEIGHT) {
                // 迁移只写本应用 settings.json，不读取或改写 Codex 的 auth.json。
                persist_settings(&settings_path, &stored_settings)
                    .map_err(|_| io::Error::other("无法保存紧凑布局迁移。"))?;
                log::info!("已完成一次性紧凑布局迁移。");
            }

            let usage_client =
                UsageClient::new().map_err(|error| io::Error::other(error.to_string()))?;
            let state = Arc::new(
                AppState::new(usage_client, stored_settings.clone(), settings_path)
                    .map_err(|_| io::Error::other("无法初始化额度自动接续运行时。"))?,
            );
            app.manage(state.clone());
            app.manage(lightweight::Runtime::default());

            if let Ok(log_directory) = app.path().app_log_dir() {
                cleanup_logs(&log_directory, LOG_RETENTION);
            }

            let main_window = app
                .get_webview_window(MAIN_WINDOW_LABEL)
                .ok_or_else(|| io::Error::other("无法创建主窗口。"))?;
            let settings_window = app
                .get_webview_window(SETTINGS_WINDOW_LABEL)
                .ok_or_else(|| io::Error::other("无法创建设置窗口。"))?;
            let _ = settings_window
                .set_title(settings_window_title(stored_settings.preferences.language));
            restore_main_window(&main_window, &stored_settings, &state);
            restore_settings_window(&settings_window, &stored_settings);
            tray::create(
                app.handle(),
                resolved_language(stored_settings.preferences.language),
            )?;
            // 静态窗口在配置中默认隐藏；显式隐藏保证升级时不会与主卡同时出现。
            let _ = settings_window.hide();
            main_window.show()?;
            let hub = AccountHub::new(state.clone()).map_err(io::Error::other)?;
            app.manage(hub.clone());
            app.manage(crate::infra::login::BrowserLogin::default());
            app.manage(network::Connectivity::default());
            #[cfg(target_os = "macos")]
            crate::app::network::watch(app.handle().clone());
            for account in hub.all() {
                start_refresh_loop(app.handle().clone(), account.clone());
                start_quota_auto_continue_loop(app.handle().clone(), account);
            }
            start_update_check_loop(app.handle().clone(), state);
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                WindowEvent::Moved(_) => schedule_geometry_save(window.clone()),
                WindowEvent::Resized(_) => schedule_geometry_save(window.clone()),
                WindowEvent::CloseRequested { api, .. } => {
                    let app = window.app_handle().clone();
                    let state = app.state::<Arc<AppState>>().inner().clone();
                    match window.label() {
                        SETTINGS_WINDOW_LABEL => {
                            // 关闭设置仅隐藏其独立窗口，主悬浮卡继续运行。
                            api.prevent_close();
                            state.save_window_placement_for_label(&app, SETTINGS_WINDOW_LABEL);
                            if window.hide().is_err() {
                                log::warn!("无法隐藏设置窗口。");
                            }
                            crate::infra::desktop::sync_platform_visibility(&app);
                            lightweight::schedule(&app);
                        }
                        MAIN_WINDOW_LABEL => {
                            // 关闭主卡前同步保存几何；是否退出由用户显式设置决定。
                            api.prevent_close();
                            state.save_window_placement_for_label(&app, MAIN_WINDOW_LABEL);
                            if state.current_settings().minimize_to_tray_on_close {
                                tray::hide_main_window(&app);
                                log::info!("主悬浮卡已隐藏到系统托盘。");
                            } else {
                                log::info!("主悬浮卡已关闭，应用即将退出。");
                                app.exit(0);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            crate::ipc::accounts::get_accounts,
            crate::ipc::accounts::select_account,
            crate::ipc::windows::enter_lightweight,
            crate::ipc::windows::get_lightweight_measurement,
            crate::ipc::accounts::import_accounts,
            crate::ipc::login::login_and_import,
            crate::ipc::login::cancel_account_login,
            crate::ipc::login::account_login_pending,
            crate::ipc::accounts::update_account,
            crate::ipc::accounts::remove_account,
            crate::ipc::accounts::apply_codex_account,
            crate::ipc::accounts::restore_codex_account,
            crate::ipc::continuation::get_account_models,
            crate::ipc::usage::get_reset_credit_details,
            crate::ipc::history::get_usage_analytics,
            crate::ipc::usage::get_dashboard,
            crate::ipc::usage::refresh_dashboard,
            crate::ipc::settings::get_settings,
            crate::ipc::settings::save_settings,
            crate::ipc::continuation::get_quota_auto_continue_status,
            crate::ipc::continuation::set_quota_auto_continue_enabled,
            crate::ipc::continuation::test_quota_auto_continue,
            crate::ipc::notifications::set_notifications_enabled,
            crate::ipc::notifications::send_test_notification,
            crate::ipc::history::get_usage_history,
            crate::ipc::history::set_history_enabled,
            crate::ipc::history::clear_usage_history,
            crate::ipc::diagnostics::report_settings_ui_fault,
            crate::ipc::diagnostics::get_diagnostics,
            crate::ipc::windows::open_settings_window,
            crate::ipc::windows::mark_main_size_manual,
            crate::ipc::settings::get_autostart,
            crate::ipc::settings::set_autostart,
            crate::ipc::updates::get_app_update_info,
            crate::ipc::updates::check_app_update,
            crate::ipc::updates::install_app_update
        ])
        .build(tauri::generate_context!())
        .expect("启动 CodexUsageBar 失败")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                if app.webview_windows().is_empty() {
                    api.prevent_exit();
                }
            }
        });
}

/// 处理重复启动通知，只记录脱敏上下文并委托既有窗口激活入口。
///
/// 参数内容可能包含本机路径或协议参数，因此日志只保留数量；`show_main_window`
/// 仅定位 Tauri 已创建的主窗口，不会创建第二个窗口或托盘图标。
pub(crate) fn handle_second_instance(show_main_arg_count: usize, show_main_window: impl FnOnce()) {
    log::info!("检测到重复启动，正在唤醒现有主窗口：参数数量={show_main_arg_count}");
    show_main_window();
}

#[cfg(test)]
mod tests;
