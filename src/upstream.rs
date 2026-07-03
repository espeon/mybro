use axum::http::{HeaderMap, HeaderValue, Method, Uri};
use bytes::Bytes;
use std::time::Duration;

// ── Upstream client (spec §7) — using reqwest ───────────────────────────────

pub struct Upstream {
    pub base: String,
    client: reqwest::Client,
    timeout: Duration,
}

impl Upstream {
    pub fn new(base_url: &str, timeout: Duration) -> Self {
        let base = if base_url.is_empty() {
            crate::constants::UMANS_API_BASE.to_string()
        } else {
            base_url.to_string()
        };

        let client = reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(128)
            .timeout(timeout)
            .build()
            .expect("failed to build upstream client");

        Self {
            base,
            client,
            timeout,
        }
    }

    pub fn build_url(&self, path: &str) -> String {
        let base = self.base.trim_end_matches('/');
        format!("{}{}", base, path)
    }

    /// Issue a request and return the response.
    async fn do_request(
        &self,
        method: Method,
        url: &str,
        key: &str,
        headers: Option<HeaderMap>,
        body: Option<Bytes>,
        timeout_override: Option<Duration>,
    ) -> Result<reqwest::Response, UpstreamError> {
        let timeout = timeout_override.unwrap_or(self.timeout);

        let mut builder = self.client.request(method, url);
        builder = builder.timeout(timeout);
        builder = builder.header(
            reqwest::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", key))
                .unwrap_or_else(|_| HeaderValue::from_static("Bearer ")),
        );
        if let Some(extra) = headers {
            for (k, v) in extra.iter() {
                builder = builder.header(k, v);
            }
        }
        if let Some(body) = body {
            builder = builder.body(body);
        }

        builder.send().await.map_err(UpstreamError::Reqwest)
    }

    // ── Convenience methods (spec §7.2) ───────────────────────────────────

    pub async fn get_user_info(&self, key: &str) -> Result<reqwest::Response, UpstreamError> {
        let url = self.build_url("/models/info");
        self.do_request(Method::GET, &url, key, None, None, Some(Duration::from_secs(10)))
            .await
    }

    pub async fn chat_completions(
        &self,
        key: &str,
        body: Bytes,
        stream: bool,
    ) -> Result<reqwest::Response, UpstreamError> {
        let url = self.build_url("/chat/completions");
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static(if stream {
                "text/event-stream"
            } else {
                "application/json"
            }),
        );
        self.do_request(Method::POST, &url, key, Some(headers), Some(body), None)
            .await
    }

    pub async fn messages(
        &self,
        key: &str,
        body: Bytes,
        stream: bool,
    ) -> Result<reqwest::Response, UpstreamError> {
        let url = self.build_url("/messages");
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static(if stream {
                "text/event-stream"
            } else {
                "application/json"
            }),
        );
        self.do_request(Method::POST, &url, key, Some(headers), Some(body), None)
            .await
    }

    pub async fn get_usage(&self, key: &str) -> Result<reqwest::Response, UpstreamError> {
        let url = self.build_url("/usage");
        self.do_request(Method::GET, &url, key, None, None, Some(Duration::from_secs(10)))
            .await
    }

    pub async fn get_usage_history(
        &self,
        key: &str,
        query: &str,
    ) -> Result<reqwest::Response, UpstreamError> {
        let path = if query.is_empty() {
            "/usage/history".to_string()
        } else {
            format!("/usage/history?{}", query)
        };
        let url = self.build_url(&path);
        self.do_request(Method::GET, &url, key, None, None, Some(Duration::from_secs(10)))
            .await
    }

    pub async fn get_models_pricing(&self, key: &str) -> Result<reqwest::Response, UpstreamError> {
        let url = self.build_url("/models");
        self.do_request(Method::GET, &url, key, None, None, Some(Duration::from_secs(10)))
            .await
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum UpstreamError {
    Reqwest(reqwest::Error),
}

impl UpstreamError {
    pub fn as_status(&self) -> u16 {
        match self {
            Self::Reqwest(e) => {
                if e.is_timeout() || e.is_connect() {
                    502
                } else {
                    500
                }
            }
        }
    }
}

impl std::fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reqwest(e) => write!(f, "upstream error: {}", e),
        }
    }
}

impl std::error::Error for UpstreamError {}

// ── Body helpers ─────────────────────────────────────────────────────────────

/// Read entire response body to Bytes.
pub async fn read_body(resp: reqwest::Response) -> Result<Bytes, UpstreamError> {
    resp.bytes().await.map_err(UpstreamError::Reqwest)
}
