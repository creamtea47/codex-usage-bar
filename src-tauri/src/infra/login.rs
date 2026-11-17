//! Browser OAuth flow. The verifier, callback and tokens never cross the webview boundary.
use crate::infra::accounts;
use crate::infra::network::HttpClient;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::Url;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{sync::Mutex, time::Duration};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};

const REDIRECT: &str = "http://localhost:1455/auth/callback";
const AUTHORIZE: &str = "https://auth.openai.com/oauth/authorize";
pub(crate) const TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Default)]
pub struct BrowserLogin(pub(crate) Mutex<Option<watch::Sender<bool>>>);

impl BrowserLogin {
    pub(crate) fn begin(&self) -> Result<watch::Receiver<bool>, String> {
        let mut active = self.0.lock().unwrap_or_else(|v| v.into_inner());
        if active.is_some() {
            return Err("loginBusy".into());
        }
        let (tx, rx) = watch::channel(false);
        *active = Some(tx);
        Ok(rx)
    }
}

struct Authorization {
    verifier: String,
    state: String,
}

impl Authorization {
    fn new() -> Self {
        Self {
            verifier: URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()),
            state: URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()),
        }
    }
    fn url(&self) -> Url {
        let mut url = Url::parse(AUTHORIZE).expect("static authorization URL");
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", accounts::CLIENT_ID),
            ("redirect_uri", REDIRECT),
            ("scope", "openid profile email offline_access"),
            ("state", self.state.as_str()),
            ("code_challenge_method", "S256"),
            (
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes())),
            ),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
        ]);
        url
    }
}

enum Callback {
    Ignore,
    Denied,
    Code(String),
}

fn parse_callback(request: &str, state: &str) -> Callback {
    let mut lines = request.split("\r\n");
    let mut line = lines.next().unwrap_or_default().split_ascii_whitespace();
    if line.next() != Some("GET") {
        return Callback::Ignore;
    }
    let target = line.next().unwrap_or_default();
    if !target.starts_with("/auth/callback?")
        || !lines.any(|line| {
            line.split_once(':').is_some_and(|(k, v)| {
                k.eq_ignore_ascii_case("host") && v.trim().eq_ignore_ascii_case("localhost:1455")
            })
        })
    {
        return Callback::Ignore;
    }
    let Ok(url) = Url::parse(REDIRECT).and_then(|url| url.join(target)) else {
        return Callback::Ignore;
    };
    let mut states = url.query_pairs().filter(|(k, _)| k == "state");
    if states.next().is_none_or(|(_, v)| v != state) || states.next().is_some() {
        return Callback::Ignore;
    }
    if url.query_pairs().any(|(k, _)| k == "error") {
        return Callback::Denied;
    }
    let mut codes = url.query_pairs().filter(|(k, _)| k == "code");
    match (codes.next(), codes.next()) {
        (Some((_, code)), None) if !code.is_empty() => Callback::Code(code.into_owned()),
        _ => Callback::Ignore,
    }
}

async fn read_callback(socket: &mut TcpStream, state: &str) -> Callback {
    let mut data = Vec::new();
    let mut buf = [0; 1024];
    while data.len() < 16384 {
        let Ok(n) = socket.read(&mut buf).await else {
            break;
        };
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(end) = data.windows(4).position(|v| v == b"\r\n\r\n") {
            return std::str::from_utf8(&data[..end])
                .ok()
                .map(|s| parse_callback(s, state))
                .unwrap_or(Callback::Ignore);
        }
    }
    Callback::Ignore
}

async fn wait_for_code(listener: TcpListener, state: &str) -> Result<String, String> {
    loop {
        let (mut socket, _) = listener.accept().await.map_err(|_| "loginCallback")?;
        let outcome =
            tokio::time::timeout(Duration::from_secs(2), read_callback(&mut socket, state))
                .await
                .unwrap_or(Callback::Ignore);
        let (status, message) = match &outcome {
            Callback::Ignore => ("400 Bad Request", "Invalid callback."),
            _ => ("200 OK", "Return to CodexUsageBar to view the sign-in result. 请返回 CodexUsageBar 查看登录结果。"),
        };
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{message}", message.len());
        let _ = socket.write_all(response.as_bytes()).await;
        match outcome {
            Callback::Code(code) => return Ok(code),
            Callback::Denied => return Err("loginCancelled".into()),
            Callback::Ignore => {}
        }
    }
}

async fn exchange(
    client: reqwest::Client,
    endpoint: &str,
    code: &str,
    verifier: &str,
) -> Result<Value, String> {
    let mut response = client
        .post(endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", accounts::CLIENT_ID),
            ("redirect_uri", REDIRECT),
            ("code", code),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|_| "network")?;
    if !response.status().is_success() {
        return Err("loginExchange".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "network")? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("authInvalid".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "authInvalid")?;
    let token = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("authInvalid")
    };
    let id_token = token("id_token")?;
    let claims = accounts::token_claims(Some(id_token));
    let mut doc = json!({"auth_mode":"chatgpt","last_refresh":chrono::Utc::now(),"tokens":{
        "id_token":id_token,"access_token":token("access_token")?,"refresh_token":token("refresh_token")?
    }});
    if let Some(id) = claims
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(Value::as_str)
    {
        doc["tokens"]["account_id"] = Value::from(id);
    }
    accounts::document_id(&doc)?;
    Ok(doc)
}

async fn bind_callback(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|_| "loginPort".to_owned())
}

pub(crate) async fn authorize(app: &AppHandle) -> Result<Value, String> {
    let listener = bind_callback(1455).await?;
    let auth = Authorization::new();
    app.opener()
        .open_url(auth.url().as_str(), None::<&str>)
        .map_err(|_| "loginBrowser")?;
    let code = wait_for_code(listener, &auth.state).await?;
    let client = HttpClient::new(30).build().map_err(|_| "network")?;
    exchange(client, accounts::TOKEN_ENDPOINT, &code, &auth.verifier).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn occupied_port_and_concurrent_login_are_reported() {
        let listener = bind_callback(0).await.unwrap();
        assert_eq!(
            bind_callback(listener.local_addr().unwrap().port())
                .await
                .unwrap_err(),
            "loginPort"
        );
        let login = BrowserLogin::default();
        let mut receiver = login.begin().unwrap();
        assert_eq!(login.begin().unwrap_err(), "loginBusy");
        login.0.lock().unwrap().as_ref().unwrap().send_replace(true);
        receiver.changed().await.unwrap();
        assert!(*receiver.borrow());
    }

    #[tokio::test]
    async fn pkce_exchange_imports_and_renews_only_the_managed_session() {
        let auth = Authorization::new();
        let another = Authorization::new();
        assert_ne!(auth.state, another.state);
        let url = auth.url();
        let get = |key| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .unwrap()
                .1
                .into_owned()
        };
        assert_eq!(get("state"), auth.state);
        assert_eq!(get("code_challenge_method"), "S256");
        assert_eq!(
            get("code_challenge"),
            URL_SAFE_NO_PAD.encode(Sha256::digest(auth.verifier.as_bytes()))
        );
        assert!(!url.as_str().contains(&auth.verifier));

        let listener = bind_callback(0).await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let verifier = auth.verifier.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut body = String::new();
            let mut reader = tokio::io::BufReader::new(&mut socket);
            use tokio::io::AsyncBufReadExt;
            let mut length = 0;
            loop {
                body.clear();
                assert!(reader.read_line(&mut body).await.unwrap() > 0);
                if body == "\r\n" {
                    break;
                }
                if let Some(value) = body.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            let mut form = vec![0; length];
            reader.read_exact(&mut form).await.unwrap();
            let form = String::from_utf8(form).unwrap();
            assert!(form.contains(&format!("code_verifier={verifier}")));
            assert!(form.contains("code=synthetic-code"));
            let claims =
                json!({"https://api.openai.com/auth":{"chatgpt_account_id":"fixture-account"}});
            let id = format!("header.{}.test", URL_SAFE_NO_PAD.encode(claims.to_string()));
            let reply = json!({"id_token":id,"access_token":"new-session","refresh_token":"new-refresh","unrelated":"omit"}).to_string();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                        reply.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let doc = exchange(client, &endpoint, "synthetic-code", &auth.verifier)
            .await
            .unwrap();
        server.await.unwrap();
        assert!(doc.get("unrelated").is_none());
        let dir =
            std::env::temp_dir().join(format!("codex-login-fixture-{}", rand::random::<u64>()));
        let store = accounts::AccountStore::open(dir.clone()).unwrap();
        let official = dir.join("fixture-codex.json");
        let mut old = doc.clone();
        old["tokens"]["access_token"] = json!("original-session");
        let original = old.to_string();
        std::fs::write(&official, &original).unwrap();
        let id = store
            .import_with_binding(&official, Some(official.clone()))
            .unwrap();
        assert_eq!(store.bound_id().as_ref(), Some(&id));
        let mut config = store.config(&id).unwrap();
        config.label = "My account".into();
        store.update(config).unwrap();
        assert_eq!(
            store
                .import_document(&doc, accounts::ImportSource::BrowserLogin)
                .unwrap(),
            id
        );
        store.sync_bound().unwrap();
        assert_eq!(store.configs().len(), 1);
        assert_eq!(store.config(&id).unwrap().label, "My account");
        assert_eq!(
            accounts::read_document(&store.auth_path(&id)).unwrap()["tokens"]["access_token"],
            "new-session"
        );
        assert_eq!(std::fs::read_to_string(official).unwrap(), original);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn loopback_accepts_only_matching_state_and_handles_denial() {
        for (query, ok) in [("code=sample", true), ("error=access_denied", false)] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let flow = tokio::spawn(async move { wait_for_code(listener, "expected").await });
            for state in ["wrong", "expected"] {
                let mut stream = TcpStream::connect(address).await.unwrap();
                stream.write_all(format!("GET /auth/callback?{query}&state={state} HTTP/1.1\r\nHost: localhost:1455\r\n\r\n").as_bytes()).await.unwrap();
                let mut reply = String::new();
                stream.read_to_string(&mut reply).await.unwrap();
                assert!(reply.starts_with(if state == "wrong" {
                    "HTTP/1.1 400"
                } else {
                    "HTTP/1.1 200"
                }));
            }
            assert_eq!(flow.await.unwrap().is_ok(), ok);
        }
    }
}
