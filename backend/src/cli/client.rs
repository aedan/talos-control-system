//! Thin authenticated HTTP/SSE/WebSocket client for the `tcs` CLI.
//!
//! Talks to the TCS REST API using a bearer token. The kubeconfig never
//! touches the CLI — all K8s access is proxied server-side.

use std::io;
use std::time::Duration;

use futures_util::StreamExt;

use super::config::CliConfig;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("not authenticated: run `tcs login` or set TCS_TOKEN")]
    NotAuthenticated,
    #[error("no cluster selected: pass --cluster or set TCS_CLUSTER")]
    NoCluster,
    #[error("server error: {0}")]
    Server(String),
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Other(String),
}

pub type CliResult<T> = Result<T, CliError>;

pub struct Client {
    pub http: reqwest::Client,
    pub server: String,
    pub token: String,
}

impl Client {
    /// Build a client from the resolved config. Fails if no token is available.
    pub fn new(server_flag: Option<&str>, token_flag: Option<&str>) -> CliResult<Self> {
        let server = CliConfig::resolve_server(server_flag);
        let token = CliConfig::resolve_token(token_flag)
            .ok_or(CliError::NotAuthenticated)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(CliError::Network)?;
        Ok(Self { http, server, token })
    }

    /// Absolute WebSocket URL for a path that already contains its query string.
    pub fn absolute(&self, path: &str) -> String {
        ws_url(&self.server, path)
    }

    /// GET a JSON endpoint, returning the parsed body.
    pub async fn get_json(&self, path: &str) -> CliResult<serde_json::Value> {
        let res = self
            .http
            .get(format!("{}{}", self.server.trim_end_matches('/'), path))
            .bearer_auth(&self.token)
            .send()
            .await?;
        self.json(res).await
    }

    /// POST a JSON body, returning the parsed response.
    pub async fn post_json(&self, path: &str, body: &serde_json::Value) -> CliResult<serde_json::Value> {
        let res = self
            .http
            .post(format!("{}{}", self.server.trim_end_matches('/'), path))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await?;
        self.json(res).await
    }

    /// DELETE an endpoint, returning the parsed response.
    pub async fn delete_json(&self, path: &str) -> CliResult<serde_json::Value> {
        let res = self
            .http
            .delete(format!("{}{}", self.server.trim_end_matches('/'), path))
            .bearer_auth(&self.token)
            .send()
            .await?;
        self.json(res).await
    }

    async fn json(&self, res: reqwest::Response) -> CliResult<serde_json::Value> {
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            let msg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(|s| s.to_string()))
                .unwrap_or(text);
            return Err(CliError::Server(format!("{status}: {msg}")));
        }
        if text.trim().is_empty() {
            return Ok(serde_json::json!({}));
        }
        serde_json::from_str(&text).map_err(|e| CliError::Other(format!("bad JSON from server: {e}")))
    }

    /// GET a text endpoint (e.g. non-follow logs).
    pub async fn get_text(&self, path: &str) -> CliResult<String> {
        let res = self
            .http
            .get(format!("{}{}", self.server.trim_end_matches('/'), path))
            .bearer_auth(&self.token)
            .send()
            .await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(CliError::Server(format!("{status}: {text}")));
        }
        Ok(text)
    }

    /// Stream an SSE endpoint, invoking `on_line` for each `data:` payload.
    pub async fn stream_sse<F>(&self, path: &str, mut on_line: F) -> CliResult<()>
    where
        F: FnMut(&str) + Send,
    {
        let res = self
            .http
            .get(format!("{}{}", self.server.trim_end_matches('/'), path))
            .bearer_auth(&self.token)
            .send()
            .await?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            return Err(CliError::Server(format!("{status}: {text}")));
        }
        let mut stream = res.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buf.extend_from_slice(&chunk);
            // Process complete SSE frames (terminated by \n\n).
            while let Some(pos) = find_frame_end(&buf) {
                let frame: Vec<u8> = buf.drain(..=pos).collect();
                let frame = String::from_utf8_lossy(&frame);
                for line in frame.lines() {
                    if let Some(data) = line.strip_prefix("data:") {
                        on_line(data.trim());
                    }
                }
            }
        }
        Ok(())
    }
}

/// Index of the byte just past the end of the first complete SSE frame, or None.
fn find_frame_end(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n").map(|i| i + 1)
}

/// Build an absolute WebSocket URL from an http(s) server base + path (with query).
/// The WS client requires a `ws://` or `wss://` scheme, so the server's http(s)
/// scheme is upgraded accordingly.
fn ws_url(server: &str, path: &str) -> String {
    let base = server.trim_end_matches('/');
    let upgraded = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{}{}", upgraded, path)
}

#[cfg(test)]
mod tests {
    use super::ws_url;

    #[test]
    fn http_upgrades_to_ws() {
        assert_eq!(
            ws_url("http://10.0.0.1:8081", "/api/clusters/x/k8s/exec?ns=a&name=b"),
            "ws://10.0.0.1:8081/api/clusters/x/k8s/exec?ns=a&name=b"
        );
    }

    #[test]
    fn https_upgrades_to_wss() {
        assert_eq!(
            ws_url("https://tcs.example.com/", "/api/clusters/x/k8s/exec"),
            "wss://tcs.example.com/api/clusters/x/k8s/exec"
        );
    }

    #[test]
    fn trailing_slash_stripped() {
        assert_eq!(
            ws_url("http://10.0.0.1:8081///", "/p"),
            "ws://10.0.0.1:8081/p"
        );
    }
}
