use super::*;
use std::fs;
#[tokio::test]
async fn explicit_model_bypasses_manifest_and_401_rotates_once() {
    use crate::test_support::http::{mock_http_server, MockResponse};
    let (token_base, token_requests, token_server) = mock_http_server(vec![MockResponse::json(
        r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}"#,
    )]);
    let account = crate::infra::accounts::account_for_protocol_test(
        format!("{token_base}/token"),
        "chosen-model".into(),
    );
    let (base,requests,server)=mock_http_server(vec![MockResponse::status("401 Unauthorized"),MockResponse::sse("data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"model\":\"actual-model\"}}\n\n")]);
    let mut client = QuotaAutoContinueClient::with_endpoints(
        format!("{base}/must-not-call-models"),
        format!("{base}/responses"),
    );
    client.account = Some(account.clone());
    assert_eq!(client.send_greeting(None).await.unwrap(), "chosen-model");
    let first = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    let second = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(first.starts_with("POST /responses") && second.starts_with("POST /responses"));
    assert!(first.contains("test-access-token") && second.contains("new-access"));
    assert!(token_requests
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .starts_with("POST /token"));
    server.join().unwrap();
    token_server.join().unwrap();
    assert!(requests.try_recv().is_err());
    assert!(token_requests.try_recv().is_err());
    let details = client.last_details().unwrap();
    assert_eq!(details.text, "Hello");
    assert!(!details.delivery_uncertain);
    let directory = &account.store.directory;
    assert!(directory.starts_with(std::env::temp_dir()));
    assert!(directory
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("codex-account-check-"));
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn complete_uses_final_text_without_duplicating_delta() {
    let c = AuthCredentials {
        access_token: "secret".into(),
        account_id: None,
    };
    let mut d = ResponseDetails::default();
    consume_event(
        r#"{"type":"response.output_text.delta","delta":"Hi"}"#,
        &mut d,
        &c,
    );
    assert_eq!(
        consume_event(
            r#"{"type":"response.completed","response":{"status":"completed","model":"actual","output":[{"content":[{"type":"output_text","text":"Hi secret"}]}]}}"#,
            &mut d,
            &c
        ),
        SseSignal::Completed
    );
    assert_eq!(d.text, "Hi [redacted]");
    assert_eq!(d.response_model.as_deref(), Some("actual"));
    assert_eq!(consume_event("[DONE]", &mut d, &c), SseSignal::Continue);
    set_text(&mut d, &"x".repeat(TEXT_LIMIT + 1), false, &c);
    assert_eq!(d.text.len(), TEXT_LIMIT);
    assert!(d.truncated);
}
