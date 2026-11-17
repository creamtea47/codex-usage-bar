use super::*;
use crate::domain::settings::StoredSettings;
use chrono::Utc;

#[tokio::test]
pub(crate) async fn main_window_account_response_excludes_profile_and_login_paths() {
    let root = std::env::temp_dir().join(format!(
        "codex-account-view-check-{}",
        rand::random::<u64>()
    ));
    fs::create_dir_all(&root).unwrap();
    let state = Arc::new(
        AppState::new(
            UsageClient::new().unwrap(),
            StoredSettings::default(),
            root.join("settings.json"),
        )
        .unwrap(),
    );
    let store = AccountStore::open(root.join("accounts")).unwrap();
    let source = root.join("source.json");
    accounts::atomic_write(
        &source,
        br#"{"tokens":{"account_id":"test-identity","access_token":"test-secret"}}"#,
    )
    .unwrap();
    let id = store.import_with_binding(&source, None).unwrap();
    let hub = AccountHub {
        store,
        root: state,
        runtimes: StdMutex::new(HashMap::new()),
    };
    hub.ensure_runtimes().unwrap();
    let runtime = hub.resolve(Some(&id)).unwrap();
    *runtime.account_profile.lock().unwrap() = Some(
        crate::infra::account_profile::parse_usage_profile(
            &serde_json::json!({"email":"private@example.com","user_id":"raw-user","account_id":"test-identity"}),
            Utc::now(),
        ),
    );
    let json = serde_json::to_string(&hub.response(false).await).unwrap();
    for value in [
        "private@example.com",
        "raw-user",
        "test-identity",
        "test-secret",
        "managedAuthPath",
        "codexLogin",
        "details",
    ] {
        assert!(!json.contains(value), "private field escaped: {value}");
    }
    assert!(root.starts_with(std::env::temp_dir()));
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("codex-account-view-check-"));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn account_runtimes_share_preferences_and_history_but_isolate_refresh_and_delivery_state() {
    use crate::domain::usage::DashboardStatus;
    use std::sync::atomic::Ordering;

    let root = std::env::temp_dir().join(format!("codex-hub-isolation-{}", rand::random::<u64>()));
    let store = AccountStore::open(root.join("accounts")).unwrap();
    let state = AppState::new(
        UsageClient::new().unwrap(),
        StoredSettings::default(),
        root.join("settings.json"),
    )
    .unwrap();
    let hub = AccountHub {
        store,
        root: Arc::new(state),
        runtimes: StdMutex::new(HashMap::new()),
    };
    let import = |id: &str| {
        hub.store
            .import_document(
                &serde_json::json!({"tokens":{"account_id":id,"access_token":"fixture"}}),
                accounts::ImportSource::BrowserLogin,
            )
            .unwrap()
    };
    let (a, b) = (import("fixture-a"), import("fixture-b"));
    // A child must never migrate or rewrite a history file beside its credential file.
    let decoy = hub.store.account_directory(&a).join("usage-history.json");
    fs::write(&decoy, b"legacy child file").unwrap();
    assert_eq!(hub.ensure_runtimes().unwrap().len(), 2);
    assert!(hub.ensure_runtimes().unwrap().is_empty());
    let first = hub.resolve(Some(&a)).unwrap();
    let second = hub.resolve(Some(&b)).unwrap();
    assert!(Arc::ptr_eq(&first.shared, &second.shared));
    assert!(Arc::ptr_eq(&hub.root.shared, &first.shared));
    first.stored_settings().preferences.lock_position = true;
    assert!(second.current_settings().lock_position);
    first.snapshot.lock().await.status = DashboardStatus::Ready;
    first.consecutive_failures.store(3, Ordering::Relaxed);
    assert_eq!(
        second.current_snapshot().await.status,
        DashboardStatus::Loading
    );
    assert_eq!(second.consecutive_failures(), 0);
    let _first_refresh = first.refresh_guard.lock().await;
    assert!(second.refresh_guard.try_lock().is_ok());
    let _first_send = first.quota_auto_continue.execution_guard().unwrap();
    assert!(second.quota_auto_continue.execution_guard().is_ok());
    assert_eq!(fs::read(decoy).unwrap(), b"legacy child file");
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn legacy_files_survive_open_observation_save_and_reopen() {
    use crate::domain::history::{AccountIdentity, HistoryWindowInput};
    use crate::infra::continuation_store::load_runtime_state;
    use crate::infra::{history_store, settings};

    let root = std::env::temp_dir().join(format!("codex-upgrade-{}", rand::random::<u64>()));
    let directory = root.join("accounts");
    let store = AccountStore::open(directory.clone()).unwrap();
    let id = store
        .import_document(
            &serde_json::json!({"tokens":{"account_id":"fixture","access_token":"fixture"}}),
            accounts::ImportSource::BrowserLogin,
        )
        .unwrap();
    // Legacy registry omitted per-account continuation flags and Codex binding fields.
    fs::write(
        directory.join("accounts.json"),
        serde_json::to_vec(&serde_json::json!({
            "selectedId": id, "accounts": [{"id":id,"label":"Fixture","enabled":true}]
        }))
        .unwrap(),
    )
    .unwrap();
    drop(store);
    let settings_path = root.join("settings.json");
    let preferences = serde_json::json!({"preferences": {"theme":"dark","alwaysOnTop":true,"lockPosition":false,"refreshIntervalSeconds":180,
        "notifications":{"enabled":false,"lowQuotaEnabled":true,"lowQuotaThresholdPercent":10,"paceEnabled":false,"paceDeficitThresholdPercent":10,
        "resetEnabled":true,"resetCreditEnabled":false,"quietHoursEnabled":false,"quietHoursStart":"22:00","quietHoursEnd":"08:00"}}});
    fs::write(&settings_path, preferences.to_string()).unwrap();
    let history_path = root.join("usage-history.json");
    let salt = "12".repeat(32);
    fs::write(
        &history_path,
        serde_json::json!({"schemaVersion":1,"salt":salt,"accountFingerprint":null,"streams":[]})
            .to_string(),
    )
    .unwrap();
    let continuation_path = directory.join(&id).join("quota-auto-continue.json");
    fs::create_dir_all(continuation_path.parent().unwrap()).unwrap();
    fs::write(
        &continuation_path,
        serde_json::json!({"schemaVersion":1,"salt":salt,"lastSuccessAt":"2026-02-01T00:00:00Z"})
            .to_string(),
    )
    .unwrap();
    let open = || {
        let state = AppState::new(
            UsageClient::new().unwrap(),
            settings::load_settings(&settings_path),
            settings_path.clone(),
        )
        .unwrap();
        AccountHub::new(Arc::new(state)).unwrap()
    };
    let hub = open();
    let account = hub.resolve(None).unwrap();
    assert_eq!(account.account.as_ref().unwrap().id, id);
    let mut prefs = account.current_settings();
    assert!(!prefs.notifications.reset_credit_enabled);
    assert!(prefs.notifications.extra_credit_enabled && prefs.notifications.connection_enabled);
    prefs.lock_position = true;
    account.save_preferences(prefs).unwrap();
    let now = Utc::now();
    {
        let mut runtime = account.shared.usage_history.lock().await;
        runtime
            .history
            .record_successful_snapshot(
                AccountIdentity::AccountId("fixture"),
                now,
                &[HistoryWindowInput {
                    window_id: "primary".into(),
                    window_seconds: 18000,
                    cycle_reset_at: Some(now + chrono::Duration::hours(5)),
                    remaining_percent: 70,
                }],
            )
            .unwrap();
        history_store::save_history(&history_path, &runtime.history).unwrap();
    }
    drop(account);
    drop(hub);
    let reopened = open();
    let account = reopened.resolve(None).unwrap();
    assert_eq!(reopened.store.selected_id(), Some(id));
    let prefs = account.current_settings();
    assert!(prefs.lock_position && prefs.notifications.connection_enabled);
    assert!(!prefs.notifications.reset_credit_enabled);
    assert_eq!(
        account
            .shared
            .usage_history
            .lock()
            .await
            .history
            .sample_count(),
        1
    );
    assert!(history_path.with_extension("v1.backup.json").is_file());
    let (continuation, needs_migration) = load_runtime_state(&continuation_path);
    assert!(!needs_migration);
    assert!(continuation
        .last_manual_result
        .unwrap()
        .success_at
        .is_some());
    fs::remove_dir_all(root).unwrap();
}
