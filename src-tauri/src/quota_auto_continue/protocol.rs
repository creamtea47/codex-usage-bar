use super::*;
use crate::accounts::ManagedAccount;
use std::sync::Arc;

const TEXT_LIMIT: usize = 8192;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResponseDetails {
    pub started_at: Option<DateTime<Utc>>,
    pub response_model: Option<String>,
    pub http_status: Option<u16>,
    pub duration_ms: u64,
    pub text: String,
    pub truncated: bool,
    pub error_message: Option<String>,
    /// POST 的交付状态不确定时不补发，避免连接超时造成重复消费。
    pub delivery_uncertain: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
}

#[derive(Clone)]
pub(super) struct QuotaAutoContinueClient {
    client: Client,
    models_endpoint: String,
    responses_endpoint: String,
    pub account: Option<Arc<ManagedAccount>>,
    pub details: Arc<StdMutex<Option<ResponseDetails>>>,
}

impl QuotaAutoContinueClient {
    pub fn new() -> Result<Self, QuotaAutoContinueErrorCode> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| QuotaAutoContinueErrorCode::Network)?,
            models_endpoint: MODEL_MANIFEST_ENDPOINT.into(),
            responses_endpoint: RESPONSES_ENDPOINT.into(),
            account: None,
            details: Arc::new(StdMutex::new(None)),
        })
    }
    #[cfg(test)]
    pub fn with_endpoints(models_endpoint: String, responses_endpoint: String) -> Self {
        Self {
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            models_endpoint,
            responses_endpoint,
            account: None,
            details: Arc::new(StdMutex::new(None)),
        }
    }
    pub fn last_details(&self) -> Option<ResponseDetails> {
        self.details
            .lock()
            .unwrap_or_else(|v| v.into_inner())
            .clone()
    }

    #[cfg(test)]
    pub fn with_test_timeout(mut self, timeout: Duration) -> Self {
        self.client = Client::builder()
            .no_proxy()
            .timeout(timeout)
            .build()
            .unwrap();
        self
    }

    async fn credentials(
        &self,
        force: bool,
    ) -> Result<AuthCredentials, QuotaAutoContinueErrorCode> {
        match &self.account {
            Some(a) => a.credentials(force).await,
            None => read_auth_credentials(),
        }
        .map_err(auth_error_code)
    }

    pub async fn models(&self) -> Result<Vec<ModelOption>, QuotaAutoContinueErrorCode> {
        let credentials = self.credentials(false).await?;
        match self.fetch_models(&credentials).await {
            Err(QuotaAutoContinueErrorCode::AuthInvalid) if self.account.is_some() => {
                self.fetch_models(&self.credentials(true).await?).await
            }
            result => result,
        }
    }

    pub async fn send_greeting(
        &self,
        expected: Option<(&str, &str)>,
    ) -> Result<String, SendFailure> {
        // 先清除上次传输信息，凭证获取失败不能复用之前成功的回复。
        *self.details.lock().unwrap_or_else(|v| v.into_inner()) = Some(ResponseDetails {
            started_at: Some(Utc::now()),
            ..Default::default()
        });
        let credentials = self
            .credentials(false)
            .await
            .map_err(|code| SendFailure { code, model: None })?;
        self.send_greeting_with_credentials(credentials, expected)
            .await
    }

    pub async fn send_greeting_with_credentials(
        &self,
        mut credentials: AuthCredentials,
        expected: Option<(&str, &str)>,
    ) -> Result<String, SendFailure> {
        let started = std::time::Instant::now();
        let mut details = ResponseDetails {
            started_at: Some(Utc::now()),
            ..Default::default()
        };
        let result = async {
            if let Some((expected, salt)) = expected {
                let identity = credentials
                    .account_id
                    .as_deref()
                    .map(AccountIdentity::AccountId)
                    .unwrap_or_else(|| AccountIdentity::Token(&credentials.access_token));
                if account_fingerprint(salt, identity).ok().as_deref() != Some(expected) {
                    return Err(SendFailure {
                        code: QuotaAutoContinueErrorCode::AccountChanged,
                        model: None,
                    });
                }
            }
            let configured = self
                .account
                .as_ref()
                .and_then(|a| a.config())
                .and_then(|a| a.model);
            let model = match configured {
                Some(model) => model,
                None => {
                    let mut models = self.fetch_models(&credentials).await;
                    if matches!(models, Err(QuotaAutoContinueErrorCode::AuthInvalid))
                        && self.account.is_some()
                    {
                        credentials = self
                            .credentials(true)
                            .await
                            .map_err(|code| SendFailure { code, model: None })?;
                        models = self.fetch_models(&credentials).await;
                    }
                    models
                        .map_err(|code| SendFailure { code, model: None })?
                        .first()
                        .map(|v| v.id.clone())
                        .ok_or(SendFailure {
                            code: QuotaAutoContinueErrorCode::NoTextModel,
                            model: None,
                        })?
                }
            };
            let failure = |code| SendFailure {
                code,
                model: Some(model.clone()),
            };
            let mut response = self.post(&credentials, &model).await.map_err(|_| {
                details.delivery_uncertain = true;
                failure(QuotaAutoContinueErrorCode::Network)
            })?;
            // 只有明确的 401 拒绝允许刷新后重发一次；连接中断不在这里重试。
            if response.status() == StatusCode::UNAUTHORIZED && self.account.is_some() {
                credentials = self.credentials(true).await.map_err(failure)?;
                response = self.post(&credentials, &model).await.map_err(|_| {
                    details.delivery_uncertain = true;
                    failure(QuotaAutoContinueErrorCode::Network)
                })?;
            }
            details.http_status = Some(response.status().as_u16());
            let status = response.status();
            details.delivery_uncertain = status.is_success();
            if response
                .content_length()
                .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
            {
                return Err(failure(QuotaAutoContinueErrorCode::InvalidResponse));
            }
            let mut received = 0;
            let mut pending = Vec::new();
            let mut frame = String::new();
            let mut failed_body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| {
                details.delivery_uncertain = status.is_success();
                failure(QuotaAutoContinueErrorCode::Network)
            })? {
                received += chunk.len();
                if received > MAX_RESPONSE_BYTES {
                    details.delivery_uncertain = status.is_success();
                    return Err(failure(QuotaAutoContinueErrorCode::InvalidResponse));
                }
                if !status.is_success() {
                    failed_body.extend_from_slice(&chunk);
                    continue;
                }
                pending.extend_from_slice(&chunk);
                while let Some(position) = pending.iter().position(|b| *b == b'\n') {
                    let line = pending.drain(..=position).collect::<Vec<_>>();
                    let line = std::str::from_utf8(&line)
                        .map_err(|_| failure(QuotaAutoContinueErrorCode::InvalidResponse))?
                        .trim_end_matches(['\r', '\n']);
                    if line.is_empty() {
                        match consume_event(&frame, &mut details, &credentials) {
                            SseSignal::Completed => return Ok(model),
                            SseSignal::Failed => {
                                return Err(failure(QuotaAutoContinueErrorCode::InvalidResponse))
                            }
                            SseSignal::Continue => {}
                        }
                        frame.clear();
                    } else if let Some(data) = line.strip_prefix("data:") {
                        if !frame.is_empty() {
                            frame.push('\n');
                        }
                        frame.push_str(data.trim_start());
                    }
                }
            }
            if !status.is_success() {
                if let Ok(value) = serde_json::from_slice::<Value>(&failed_body) {
                    details.error_message = value
                        .pointer("/error/message")
                        .or_else(|| value.get("message"))
                        .and_then(Value::as_str)
                        .map(|v| sanitize(v, &credentials));
                }
                return Err(failure(status_error_code(status)));
            }
            if let Ok(line) = std::str::from_utf8(&pending) {
                if let Some(data) = line.trim().strip_prefix("data:") {
                    frame.push_str(data.trim_start());
                }
            }
            if consume_event(&frame, &mut details, &credentials) == SseSignal::Completed {
                Ok(model)
            } else {
                details.delivery_uncertain = true;
                Err(failure(QuotaAutoContinueErrorCode::InvalidResponse))
            }
        }
        .await;
        details.duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        // Token 若跨多个 delta 边界出现，完成后仍需对拼接文本统一脱敏。
        details.text = sanitize(&details.text, &credentials);
        *self.details.lock().unwrap_or_else(|v| v.into_inner()) = Some(details);
        result
    }

    async fn post(
        &self,
        credentials: &AuthCredentials,
        model: &str,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let mut request = self
            .client
            .post(&self.responses_endpoint)
            .bearer_auth(&credentials.access_token)
            .header("Accept", "text/event-stream")
            .header("OpenAI-Beta", "responses=experimental")
            .header("Originator", "codex_cli_rs")
            .header("Version", CODEX_CLIENT_VERSION)
            .header("User-Agent", CODEX_USER_AGENT)
            .json(&build_greeting_payload(model));
        if let Some(id) = &credentials.account_id {
            request = request.header("ChatGPT-Account-Id", id);
        }
        request.send().await
    }

    async fn fetch_models(
        &self,
        credentials: &AuthCredentials,
    ) -> Result<Vec<ModelOption>, QuotaAutoContinueErrorCode> {
        let mut request = self
            .client
            .get(&self.models_endpoint)
            .query(&[("client_version", CODEX_CLIENT_VERSION)])
            .bearer_auth(&credentials.access_token)
            .header("Accept", "application/json")
            .header("Originator", "codex_cli_rs")
            .header("Version", CODEX_CLIENT_VERSION)
            .header("User-Agent", CODEX_USER_AGENT);
        if let Some(id) = &credentials.account_id {
            request = request.header("ChatGPT-Account-Id", id);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| QuotaAutoContinueErrorCode::Network)?;
        if !response.status().is_success() {
            return Err(status_error_code(response.status()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| QuotaAutoContinueErrorCode::Network)?
        {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(QuotaAutoContinueErrorCode::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| QuotaAutoContinueErrorCode::InvalidResponse)?;
        let models = value
            .get("models")
            .and_then(Value::as_array)
            .ok_or(QuotaAutoContinueErrorCode::InvalidResponse)?;
        let options: Vec<_> = models
            .iter()
            .filter(|m| m.get("disabled").and_then(Value::as_bool) != Some(true))
            .filter_map(|m| {
                let id = m.get("slug").and_then(Value::as_str)?.trim();
                if id.is_empty()
                    || id.len() > 128
                    || !is_text_model(id)
                    || !id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-._:/".contains(&c))
                {
                    return None;
                }
                Some(ModelOption {
                    id: id.into(),
                    label: m
                        .get("display_name")
                        .and_then(Value::as_str)
                        .unwrap_or(id)
                        .chars()
                        .take(160)
                        .collect(),
                })
            })
            .collect();
        if options.is_empty() {
            Err(QuotaAutoContinueErrorCode::NoTextModel)
        } else {
            Ok(options)
        }
    }
}

fn sanitize(text: &str, credentials: &AuthCredentials) -> String {
    let mut result = text.replace(&credentials.access_token, "[redacted]");
    if let Some(id) = &credentials.account_id {
        result = result.replace(id, "[redacted]");
    }
    result
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(TEXT_LIMIT)
        .collect()
}

fn set_text(
    details: &mut ResponseDetails,
    text: &str,
    append: bool,
    credentials: &AuthCredentials,
) {
    if !append {
        details.text.clear();
    }
    let available = TEXT_LIMIT.saturating_sub(details.text.chars().count());
    details.truncated |= text.chars().count() > available;
    details
        .text
        .extend(sanitize(text, credentials).chars().take(available));
}

fn consume_event(
    data: &str,
    details: &mut ResponseDetails,
    credentials: &AuthCredentials,
) -> SseSignal {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return SseSignal::Continue;
    };
    match value.get("type").and_then(Value::as_str) {
        Some("response.output_text.delta") => {
            if let Some(text) = value.get("delta").and_then(Value::as_str) {
                set_text(details, text, true, credentials);
            }
            SseSignal::Continue
        }
        Some("response.completed") => {
            if value
                .pointer("/response/status")
                .and_then(Value::as_str)
                .is_some_and(|v| v != "completed")
            {
                return SseSignal::Failed;
            }
            details.response_model = value
                .pointer("/response/model")
                .and_then(Value::as_str)
                .map(|v| sanitize(v, credentials));
            let output = value
                .pointer("/response/output")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.get("content").and_then(Value::as_array))
                        .flatten()
                        .filter(|v| v.get("type").and_then(Value::as_str) == Some("output_text"))
                        .filter_map(|v| v.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                });
            if let Some(text) = output.filter(|v| !v.is_empty()) {
                set_text(details, &text, false, credentials);
            }
            details.delivery_uncertain = false;
            SseSignal::Completed
        }
        Some("response.failed" | "error" | "response.incomplete") => {
            details.delivery_uncertain =
                value.get("type").and_then(Value::as_str) == Some("response.incomplete");
            details.error_message = value
                .pointer("/response/error/message")
                .or_else(|| value.pointer("/error/message"))
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .map(|v| sanitize(v, credentials));
            SseSignal::Failed
        }
        _ => SseSignal::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn explicit_model_bypasses_manifest_and_401_rotates_once() {
        use super::super::tests::{mock_http_server, MockResponse};
        let (token_base, token_requests, token_server) =
            mock_http_server(vec![MockResponse::json(
                r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}"#,
            )]);
        let account = crate::accounts::account_for_protocol_test(
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
}
