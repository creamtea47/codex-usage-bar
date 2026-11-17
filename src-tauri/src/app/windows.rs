use crate::app::account_hub;
use crate::app::state::resolved_language;
use crate::app::state::AppState;
use crate::domain::settings::Language;
use crate::domain::settings::MainWindowSizeMode;
use crate::domain::settings::StoredSettings;
use crate::domain::settings::WindowPlacement;
use crate::domain::usage::DashboardSnapshot;
use crate::domain::usage::ForecastStatus as ModelForecastStatus;
use crate::domain::usage::QuotaForecast;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::infra::settings::save_settings as persist_settings;
use crate::infra::settings::update_main_window_placement;
use crate::infra::settings::update_settings_window_placement;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tauri::LogicalSize;
use tauri::Manager;
use tauri::PhysicalPosition;
use tauri::PhysicalSize;
use tauri::Position;
use tauri::Size;
use tauri::WebviewWindow;
use tauri::Window;

impl AppState {
    pub(crate) fn save_window_placement_for_label(&self, app: &AppHandle, label: &str) {
        let Some(window) = app.get_webview_window(label) else {
            return;
        };
        let Ok(position) = window.outer_position() else {
            return;
        };
        let Ok(size) = window.outer_size() else {
            return;
        };
        let placement = WindowPlacement {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        };
        let mut stored = self.stored_settings();
        match label {
            MAIN_WINDOW_LABEL => update_main_window_placement(&mut stored, placement),
            SETTINGS_WINDOW_LABEL => update_settings_window_placement(&mut stored, placement),
            _ => return,
        }
        if persist_settings(&self.shared.settings_path, &stored).is_err() {
            log::warn!("无法保存窗口位置与大小：窗口={label}。");
        }
    }

    pub(crate) fn next_geometry_generation(&self, label: &str) -> u64 {
        let mut generations = self
            .shared
            .geometry_generations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let generation = generations.entry(label.to_owned()).or_insert(0);
        *generation += 1;
        *generation
    }

    pub(crate) fn is_current_geometry_generation(&self, label: &str, generation: u64) -> bool {
        self.shared
            .geometry_generations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(label)
            .is_some_and(|current| *current == generation)
    }

    /// 只有主卡真实开始边缘拖拽时才切换手动尺寸。不能根据 `Resized` 推断，
    /// 因为启动恢复、DPI 切换和 Tauri 的程序化调整都会产生同样的窗口事件。
    pub(crate) fn mark_main_size_manual(&self) -> Result<(), String> {
        let mut stored = self.stored_settings();
        if stored.main_window_size_mode == MainWindowSizeMode::Manual {
            return Ok(());
        }
        let mut candidate = stored.clone();
        candidate.main_window_size_mode = MainWindowSizeMode::Manual;
        persist_settings(&self.shared.settings_path, &candidate)
            .map_err(|_| "无法保存手动窗口尺寸模式。".to_owned())?;
        *stored = candidate;
        log::info!("主悬浮卡已切换为手动尺寸模式。");
        Ok(())
    }

    /// 自动模式只按窗口与可靠建议数量收敛高度，不触碰用户的位置和宽度。
    pub(crate) fn apply_auto_main_height(&self, app: &AppHandle, snapshot: &DashboardSnapshot) {
        if !account_hub::is_selected(app, self) {
            return;
        }
        let (size_mode, locked) = {
            let stored = self.stored_settings();
            (
                stored.main_window_size_mode,
                stored.preferences.lock_position,
            )
        };
        if size_mode != MainWindowSizeMode::Auto || locked {
            return;
        }
        let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
            return;
        };
        let Ok(current_size) = window.outer_size() else {
            return;
        };
        let Ok(scale_factor) = window.scale_factor() else {
            return;
        };
        let advice_count = reliable_advice_count(snapshot);
        let target_height = compact_height(snapshot.quota_windows.len(), advice_count)
            + if snapshot.extra_credits.is_some() {
                24
            } else {
                0
            };
        // tauri.conf.json 的尺寸是逻辑像素；自动高度必须保持同一单位，
        // 否则 125%/150% DPI 下会把 260 错当作物理像素而压扁主卡。
        let current_logical = current_size.to_logical::<f64>(scale_factor);
        let target_size = LogicalSize::new(
            current_logical.width.max(f64::from(MAIN_MIN_WIDTH)),
            f64::from(target_height),
        );
        if (current_logical.height - target_size.height).abs() < 0.5 {
            return;
        }

        if window.set_size(Size::Logical(target_size)).is_err() {
            log::warn!("更新主悬浮卡自动高度失败：类别=window-size。");
        }
    }
}

use crate::infra::desktop::MAIN_WINDOW_LABEL;

pub(crate) const GEOMETRY_SAVE_DELAY: Duration = Duration::from_millis(250);

// 账号切换位于标题栏，不再占用正文高度；底部每条短建议仅占一行。
// 窗口数量或可靠建议增加时只扩展高度，最高后由前端分别滚动额度与建议区域。
pub(crate) const MAIN_COMPACT_HEIGHT: u32 = 260;

pub(crate) const MAIN_COMPACT_HEIGHT_PER_EXTRA_WINDOW: u32 = 145;

pub(crate) const MAIN_ADVICE_SECTION_BASE_HEIGHT: u32 = 16;

pub(crate) const MAIN_ADVICE_LINE_HEIGHT: u32 = 20;

pub(crate) const MAIN_ADVICE_VISIBLE_LINE_LIMIT: usize = 3;

pub(crate) const MAIN_COMPACT_MAX_HEIGHT: u32 = 560;

pub(crate) const MAIN_MIN_WIDTH: u32 = 340;

pub(crate) const MAIN_MIN_HEIGHT: u32 = 260;

pub(crate) const SETTINGS_MIN_WIDTH: u32 = 620;

pub(crate) const SETTINGS_MIN_HEIGHT: u32 = 420;

pub(crate) fn settings_window_title(language: Language) -> &'static str {
    match resolved_language(language) {
        Language::ZhCn => "CodexUsageBar 设置",
        Language::En | Language::System => "CodexUsageBar Settings",
    }
}

pub(crate) fn reliable_advice_count(snapshot: &DashboardSnapshot) -> usize {
    snapshot
        .quota_windows
        .iter()
        .filter(|window| {
            let has_valid_period = matches!(
                (window.start_at.as_ref(), window.reset_at.as_ref()),
                (Some(start), Some(reset)) if reset > start
            );
            let has_reliable_forecast = matches!(
                window.forecast.as_ref(),
                Some(QuotaForecast {
                    status: ModelForecastStatus::LastsUntilReset,
                    ..
                }) | Some(QuotaForecast {
                    status: ModelForecastStatus::ExhaustsBeforeReset,
                    exhausts_at: Some(_),
                    ..
                })
            );
            window.show_pace_marker && has_valid_period && has_reliable_forecast
        })
        .count()
}

pub(crate) fn compact_height(window_count: usize, advice_count: usize) -> u32 {
    let extra_window_height = u32::try_from(window_count.saturating_sub(1))
        .unwrap_or(u32::MAX)
        .saturating_mul(MAIN_COMPACT_HEIGHT_PER_EXTRA_WINDOW);
    let advice_height = if advice_count == 0 {
        0
    } else {
        MAIN_ADVICE_SECTION_BASE_HEIGHT.saturating_add(
            u32::try_from(advice_count.min(MAIN_ADVICE_VISIBLE_LINE_LIMIT))
                .unwrap_or(MAIN_ADVICE_VISIBLE_LINE_LIMIT as u32)
                .saturating_mul(MAIN_ADVICE_LINE_HEIGHT),
        )
    };
    MAIN_COMPACT_HEIGHT
        .saturating_add(extra_window_height)
        .saturating_add(advice_height)
        .min(MAIN_COMPACT_MAX_HEIGHT)
}

pub(crate) fn restore_main_window(
    window: &WebviewWindow,
    stored: &StoredSettings,
    _state: &AppState,
) {
    if let Some(placement) = stored.window_placement.as_ref() {
        let placement = clamp_placement(window, placement, MAIN_MIN_WIDTH, MAIN_MIN_HEIGHT);
        apply_window_placement(window, &placement);
    }
    // 只有主悬浮卡可以置顶和锁定尺寸；设置窗口始终保持普通应用窗口行为。
    let _ = window.set_always_on_top(stored.preferences.always_on_top);
    let _ = window.set_resizable(!stored.preferences.lock_position);
}

pub(crate) fn restore_settings_window(window: &WebviewWindow, stored: &StoredSettings) {
    if let Some(placement) = stored.settings_window_placement.as_ref() {
        let placement = clamp_placement(window, placement, SETTINGS_MIN_WIDTH, SETTINGS_MIN_HEIGHT);
        apply_window_placement(window, &placement);
    }
}

pub(crate) fn apply_window_placement(window: &WebviewWindow, placement: &WindowPlacement) {
    let _ = window.set_size(Size::Physical(PhysicalSize::new(
        placement.width,
        placement.height,
    )));
    let _ = window.set_position(Position::Physical(PhysicalPosition::new(
        placement.x,
        placement.y,
    )));
}

/// 恢复位置前限制在当前显示器的可用工作区，避免任务栏或显示器变更后窗口落在屏幕外。
pub(crate) fn clamp_placement(
    window: &WebviewWindow,
    placement: &WindowPlacement,
    minimum_width: u32,
    minimum_height: u32,
) -> WindowPlacement {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return placement.clone();
    };
    let work_area = monitor.work_area();
    let width = placement.width.min(work_area.size.width).max(minimum_width);
    let height = placement
        .height
        .min(work_area.size.height)
        .max(minimum_height);
    let max_x = work_area.position.x + work_area.size.width.saturating_sub(width) as i32;
    let max_y = work_area.position.y + work_area.size.height.saturating_sub(height) as i32;
    WindowPlacement {
        x: placement.x.clamp(work_area.position.x, max_x),
        y: placement.y.clamp(work_area.position.y, max_y),
        width,
        height,
    }
}

pub(crate) fn schedule_geometry_save(window: Window) {
    let app = window.app_handle().clone();
    let state = app.state::<Arc<AppState>>().inner().clone();
    let window_label = window.label().to_owned();
    let generation = state.next_geometry_generation(&window_label);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(GEOMETRY_SAVE_DELAY).await;
        if state.is_current_geometry_generation(&window_label, generation) {
            state.save_window_placement_for_label(&app, &window_label);
        }
    });
}

#[cfg(test)]
mod tests;
