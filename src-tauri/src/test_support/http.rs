use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};
pub(crate) struct MockResponse {
    pub(crate) status: &'static str,
    pub(crate) content_type: &'static str,
    pub(crate) body: String,
    pub(crate) declared_length: Option<usize>,
    pub(crate) delay_millis: u64,
}

impl MockResponse {
    pub(crate) fn json(body: &str) -> Self {
        Self {
            status: "200 OK",
            content_type: "application/json",
            body: body.to_owned(),
            declared_length: None,
            delay_millis: 0,
        }
    }

    pub(crate) fn sse(body: &str) -> Self {
        Self {
            status: "200 OK",
            content_type: "text/event-stream",
            body: body.to_owned(),
            declared_length: None,
            delay_millis: 0,
        }
    }

    pub(crate) fn status(status: &'static str) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: "{}".to_owned(),
            declared_length: None,
            delay_millis: 0,
        }
    }
}

/// 本地 TCP 服务只实现测试所需的最小 HTTP 子集，避免测试接触真实账号或外部网络。
pub(crate) fn mock_http_server(
    responses: Vec<MockResponse>,
) -> (String, Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let header_end = loop {
                let read = stream.read(&mut buffer).unwrap_or(0);
                if read == 0 {
                    break request.len();
                }
                request.extend_from_slice(&buffer[..read]);
                if let Some(position) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    break position + 4;
                }
            };
            let header = String::from_utf8_lossy(&request[..header_end]).to_string();
            let content_length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while request.len() < header_end + content_length {
                let read = stream.read(&mut buffer).unwrap_or(0);
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            let _ = sender.send(String::from_utf8_lossy(&request).to_string());
            if response.delay_millis > 0 {
                thread::sleep(Duration::from_millis(response.delay_millis));
            }
            let declared_length = response.declared_length.unwrap_or(response.body.len());
            let wire = format!(
                "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.status, response.content_type, declared_length, response.body
            );
            let _ = stream.write_all(wire.as_bytes());
        }
    });
    (format!("http://{address}"), receiver, handle)
}
