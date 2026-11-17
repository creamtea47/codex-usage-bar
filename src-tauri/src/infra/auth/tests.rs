use super::*;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn temporary_directory(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = env::temp_dir().join(format!("codex-usage-bar-{name}-{suffix}"));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn resolves_existing_files_in_documented_priority_order() {
    let root = temporary_directory("auth-priority");
    let exe = root.join("exe");
    let home = root.join("home");
    let profile = root.join("profile");
    fs::create_dir_all(&exe).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(profile.join(".codex")).unwrap();
    fs::write(home.join("auth.json"), "{}").unwrap();
    fs::write(profile.join(".codex/auth.json"), "{}").unwrap();
    assert_eq!(
        resolve_auth_json_path_from(Some(&exe), Some(&home), Some(&profile)).unwrap(),
        home.join("auth.json")
    );
    fs::write(exe.join("auth.json"), "{}").unwrap();
    assert_eq!(
        resolve_auth_json_path_from(Some(&exe), Some(&home), Some(&profile)).unwrap(),
        exe.join("auth.json")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn falls_back_to_user_profile_and_reports_missing_files_without_paths() {
    let root = temporary_directory("auth-profile-fallback");
    let exe = root.join("exe");
    let home = root.join("home");
    let profile = root.join("profile");
    fs::create_dir_all(&exe).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(profile.join(".codex")).unwrap();
    fs::write(profile.join(".codex/auth.json"), "{}").unwrap();

    assert_eq!(
        resolve_auth_json_path_from(Some(&exe), Some(&home), Some(&profile)).unwrap(),
        profile.join(".codex/auth.json")
    );

    fs::remove_file(profile.join(".codex/auth.json")).unwrap();
    let error = resolve_auth_json_path_from(Some(&exe), Some(&home), Some(&profile))
        .unwrap_err()
        .to_string();
    assert!(error.contains("Codex"));
    assert!(!error.contains("auth.json"));
    assert!(!error.contains(&root.display().to_string()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reads_only_the_fields_needed_for_usage_requests() {
    let parsed: Value = serde_json::json!({
        "tokens": { "access_token": "secret", "account_id": "account" }
    });
    let access = parsed["tokens"]["access_token"].as_str().unwrap();
    assert_eq!(access, "secret");
    assert!(parsed.get("refresh_token").is_none());
}

#[test]
fn resolves_home_for_macos_style_auth_location() {
    let root = temporary_directory("auth-macos-home");
    let macos_home = root.join("macos-home");
    fs::create_dir_all(macos_home.join(".codex")).unwrap();
    fs::write(macos_home.join(".codex/auth.json"), "{}").unwrap();

    assert_eq!(
        resolve_auth_json_path_from(None, None, Some(&macos_home)).unwrap(),
        macos_home.join(".codex/auth.json")
    );
    fs::remove_dir_all(root).unwrap();
}
