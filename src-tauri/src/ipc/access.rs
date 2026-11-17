use crate::infra::desktop::MAIN_WINDOW_LABEL;
use tauri::WebviewWindow;

use crate::infra::desktop::SETTINGS_WINDOW_LABEL;

/// Command 的调用窗口不是前端可伪造参数。这里在 Rust 端二次限定窗口标签，
/// 避免独立设置页取得主悬浮卡不需要的用量数据，或主卡直接执行设置副作用。
pub(crate) fn require_window_label(
    window: &WebviewWindow,
    expected_label: &str,
) -> Result<(), String> {
    if has_exact_window_label(window.label(), expected_label) {
        Ok(())
    } else {
        Err("当前窗口无权执行该操作。".to_owned())
    }
}

pub(crate) fn require_known_window(window: &WebviewWindow) -> Result<(), String> {
    if is_known_window_label(window.label()) {
        Ok(())
    } else {
        Err("当前窗口无权执行该操作。".to_owned())
    }
}

pub(crate) fn has_exact_window_label(actual_label: &str, expected_label: &str) -> bool {
    actual_label == expected_label
}

pub(crate) fn is_known_window_label(label: &str) -> bool {
    matches!(label, MAIN_WINDOW_LABEL | SETTINGS_WINDOW_LABEL)
}

#[cfg(test)]
mod tests;
