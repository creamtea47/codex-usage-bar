use crate::app::bootstrap::handle_second_instance;
use crate::infra::desktop::MAIN_WINDOW_LABEL;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use std::cell::Cell;

#[test]
fn second_instance_delegates_once_to_existing_main_window_activation() {
    let activation_count = Cell::new(0_u8);

    handle_second_instance(2, || activation_count.set(activation_count.get() + 1));

    assert_eq!(activation_count.get(), 1);
}

#[test]
fn macos_transparent_main_window_enables_private_api() {
    let config: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tauri.conf.json"
    )))
    .unwrap();
    let main_window = config["app"]["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|window| window["label"] == MAIN_WINDOW_LABEL)
        .unwrap();

    assert_eq!(main_window["transparent"], true);
    assert_eq!(main_window["acceptFirstMouse"], true);
    assert_eq!(config["app"]["macOSPrivateApi"], true);
}

#[test]
fn settings_titlebar_can_follow_the_webview_theme() {
    let config: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tauri.conf.json"
    )))
    .unwrap();
    let settings_window = config["app"]["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|window| window["label"] == SETTINGS_WINDOW_LABEL)
        .unwrap();
    assert_eq!(settings_window["titleBarStyle"], "Transparent");

    let capability: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/capabilities/settings.json"
    )))
    .unwrap();
    let permissions = capability["permissions"].as_array().unwrap();
    assert!(permissions
        .iter()
        .any(|permission| permission == "core:window:allow-set-theme"));
    assert!(permissions
        .iter()
        .any(|permission| { permission == "core:window:allow-set-background-color" }));
    assert!(permissions
        .iter()
        .any(|permission| permission == "clipboard-manager:allow-write-text"));
    assert!(!permissions
        .iter()
        .any(|permission| permission == "clipboard-manager:allow-read-text"));
    let opener = permissions
        .iter()
        .find(|permission| permission["identifier"] == "opener:allow-open-url")
        .unwrap();
    assert_eq!(
        opener["allow"],
        serde_json::json!([
            {"url": "https://github.com/creamtea47/codex-usage-bar"},
            {"url": "https://github.com/creamtea47/codex-usage-bar/releases"},
            {"url": "https://github.com/creamtea47/codex-usage-bar/releases/**"},
            {"url": "https://github.com/creamtea47/codex-usage-bar/issues/new"},
            {"url": "https://github.com/creamtea47/codex-usage-bar/issues/new/**"}
        ])
    );

    let main_capability: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/capabilities/default.json"
    )))
    .unwrap();
    let main_permissions = main_capability["permissions"].as_array().unwrap();
    assert!(!main_permissions.iter().any(|permission| {
        permission.as_str().is_some_and(|value| {
            value.starts_with("notification:")
                || value.starts_with("clipboard-manager:")
                || value.starts_with("opener:")
        }) || permission["identifier"]
            .as_str()
            .is_some_and(|value| value.starts_with("opener:"))
    }));
}

#[test]
fn native_bundle_defaults_are_platform_specific() {
    let base: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tauri.conf.json"
    )))
    .unwrap();
    let windows: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tauri.windows.conf.json"
    )))
    .unwrap();
    let macos: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tauri.macos.conf.json"
    )))
    .unwrap();
    let unsigned: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tauri.unsigned.conf.json"
    )))
    .unwrap();

    assert!(base["bundle"].get("targets").is_none());
    assert_eq!(windows["bundle"]["targets"], serde_json::json!(["nsis"]));
    assert_eq!(macos["bundle"]["targets"], serde_json::json!(["dmg"]));
    assert_eq!(unsigned["bundle"]["createUpdaterArtifacts"], false);
}
