use super::*;

fn isolated_store() -> Arc<AccountStore> {
    let directory =
        std::env::temp_dir().join(format!("codex-account-check-{}", rand::random::<u64>()));
    fs::create_dir_all(&directory).unwrap();
    Arc::new(AccountStore {
        directory,
        registry: Arc::new(Mutex::new(Registry::default())),
        operation: Arc::new(AsyncMutex::new(())),
    })
}

fn auth_document(access: &str) -> Value {
    serde_json::json!({"tokens":{"account_id":"isolated-account","access_token":access,"refresh_token":"fake-refresh-secret"},"expires_at":Utc::now().timestamp()+60})
}

#[test]
fn codex_switch_is_guarded_preserves_provider_and_rolls_back_registry_failure() {
    let store = isolated_store();
    let source = store.directory.join("source.json");
    save_doc(&source, &auth_document("test-secret")).unwrap();
    let id = store.import_with_binding(&source, None).unwrap();
    let codex = store.directory.join("codex");
    fs::create_dir(&codex).unwrap();
    let target = codex.join("auth.json");
    let original = br#"{"OPENAI_API_KEY":"fake-key","auth_mode":"apikey"}"#;
    atomic_write(&target, original).unwrap();
    let config = codex.join("config.toml");
    atomic_write(&config, b"model_provider = 'custom'\n").unwrap();
    assert_eq!(
        store
            .apply_at(&id, &target, || Err("codexRunning".into()))
            .unwrap_err(),
        "codexRunning"
    );
    assert_eq!(fs::read(&target).unwrap(), original);
    store.apply_at(&id, &target, || Ok(())).unwrap();
    assert_eq!(document_id(&read_document(&target).unwrap()).unwrap(), id);
    assert_eq!(fs::read(&config).unwrap(), b"model_provider = 'custom'\n");
    store.restore_at(&target, || Ok(())).unwrap();
    let restored: Value = serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
    assert_eq!(restored["OPENAI_API_KEY"], "fake-key");
    assert!(store.bound_id().is_none());
    fs::remove_file(store.directory.join("accounts.json")).unwrap();
    fs::create_dir(store.directory.join("accounts.json")).unwrap();
    let before = fs::read(&target).unwrap();
    assert!(store.apply_at(&id, &target, || Ok(())).is_err());
    assert_eq!(fs::read(&target).unwrap(), before);
    let absent = codex.join("absent.json");
    assert!(store.apply_at(&id, &absent, || Ok(())).is_err());
    assert!(!absent.exists());
    clean(&store);
}

fn clean(store: &AccountStore) {
    assert!(store.directory.starts_with(std::env::temp_dir()));
    assert!(store
        .directory
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("codex-account-check-"));
    fs::remove_dir_all(&store.directory).unwrap();
}

#[test]
fn import_deduplicates_rolls_back_failed_save_and_removes_only_copy() {
    let store = isolated_store();
    let source = store.directory.join("source.json");
    save_doc(&source, &auth_document("first-secret")).unwrap();
    let source_bytes = fs::read(&source).unwrap();
    let id = store.import_with_binding(&source, None).unwrap();
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    save_doc(&source, &auth_document("second-secret")).unwrap();
    assert_eq!(store.import_with_binding(&source, None).unwrap(), id);
    assert_eq!(store.configs().len(), 1);
    assert!(!store.config(&id).unwrap().auto_continue);
    let previous = fs::read(store.auth_path(&id)).unwrap();
    fs::remove_file(store.directory.join("accounts.json")).unwrap();
    fs::create_dir(store.directory.join("accounts.json")).unwrap();
    save_doc(&source, &auth_document("third-secret")).unwrap();
    assert!(store.import_with_binding(&source, None).is_err());
    assert_eq!(fs::read(store.auth_path(&id)).unwrap(), previous);
    assert!(store.remove(&id).is_err());
    assert_eq!(fs::read(store.auth_path(&id)).unwrap(), previous);
    fs::remove_dir(store.directory.join("accounts.json")).unwrap();
    store.remove(&id).unwrap();
    assert!(source.exists());
    assert!(!store.auth_path(&id).exists());
    clean(&store);
}

#[tokio::test]
async fn bound_account_follows_codex_and_rejects_external_identity_change() {
    let store = isolated_store();
    let source = store.directory.join("codex-auth.json");
    save_doc(&source, &auth_document("first-secret")).unwrap();
    let id = store
        .import_with_binding(&source, Some(source.clone()))
        .unwrap();
    let account = ManagedAccount::new(id.clone(), store.clone());
    save_doc(&source, &auth_document("new-secret")).unwrap();
    assert_eq!(
        account.credentials(false).await.unwrap().access_token,
        "new-secret"
    );
    let view = serde_json::to_string(&account.view()).unwrap();
    assert!(!view.contains("new-secret") && !view.contains("fake-refresh-secret"));
    let mut other = auth_document("other-secret");
    other["tokens"]["account_id"] = "other-account".into();
    save_doc(&source, &other).unwrap();
    assert!(account.credentials(false).await.is_err());
    assert_eq!(
        credentials(&read_document(&store.auth_path(&id)).unwrap())
            .unwrap()
            .access_token,
        "new-secret"
    );
    clean(&store);
}

#[tokio::test]
async fn oauth_rotation_is_persisted_before_next_read_without_touching_source() {
    use std::io::Read;
    let store = isolated_store();
    let source = store.directory.join("source.json");
    save_doc(&source, &auth_document("old-secret")).unwrap();
    let id = store.import_with_binding(&source, None).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0);
            request.extend_from_slice(&buffer[..n]);
            let text = String::from_utf8_lossy(&request);
            if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.parse::<usize>().ok())
                    })
                    .unwrap();
                if body.len() >= length {
                    assert!(body.contains("grant_type=refresh_token"));
                    break;
                }
            }
        }
        let body = r#"{"access_token":"rotated-secret","refresh_token":"new-refresh-secret","expires_in":3600}"#;
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    let mut account = ManagedAccount::new(id.clone(), store.clone());
    Arc::get_mut(&mut account).unwrap().token_endpoint = Some(endpoint);
    let (one, two) = tokio::join!(account.credentials(false), account.credentials(false));
    assert_eq!(one.unwrap().access_token, "rotated-secret");
    assert_eq!(two.unwrap().access_token, "rotated-secret");
    server.join().unwrap();
    let saved = read_document(&store.auth_path(&id)).unwrap();
    assert_eq!(saved["tokens"]["refresh_token"], "new-refresh-secret");
    assert_eq!(document_id(&saved).unwrap(), id);
    assert_eq!(
        read_document(&source).unwrap()["tokens"]["access_token"],
        "old-secret"
    );
    clean(&store);
}
#[test]
fn rotation_keeps_identity_and_missing_refresh_token() {
    let mut doc = serde_json::json!({"tokens":{"access_token":"old", "refresh_token":"keep", "account_id":"test-account"}});
    let id = document_id(&doc).unwrap();
    merge_tokens(&mut doc, &serde_json::json!({"access_token":"new"})).unwrap();
    assert_eq!(document_id(&doc).unwrap(), id);
    assert_eq!(doc["tokens"]["refresh_token"], "keep");
    assert!(document_id(&serde_json::json!({"tokens":{"access_token":"no-identity"}})).is_err());
}
#[test]
fn atomic_replacement_retains_valid_json() {
    let path = std::env::temp_dir().join(format!("usage-account-{}.json", rand::random::<u64>()));
    atomic_write(&path, b"{\"old\":true}").unwrap();
    atomic_write(&path, b"{\"new\":true}").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"{\"new\":true}");
    fs::remove_file(path).unwrap();
}
