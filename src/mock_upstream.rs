// ── Built-in mock upstream server for local testing ────────────────────────────
//
// Responds like the UMANS API so you can test the proxy without external
// network access. Start with: cargo run -- --mock-upstream 9001

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bytes::Bytes;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub struct MockUpstream {
    port: u16,
    request_count: AtomicU64,
    error_every_n: AtomicUsize, // 0 = no errors, N = fail every Nth request
    delay_ms: AtomicUsize,      // artificial latency per response
    concurrency_limit: AtomicUsize, // reported in /usage so proxy gate matches
}

impl MockUpstream {
    pub fn new(port: u16) -> Arc<Self> {
        Arc::new(Self {
            port,
            request_count: AtomicU64::new(0),
            error_every_n: AtomicUsize::new(0),
            delay_ms: AtomicUsize::new(0),
            concurrency_limit: AtomicUsize::new(8),
        })
    }

    pub fn with_options(port: u16, delay_ms: usize, concurrency_limit: usize) -> Arc<Self> {
        Arc::new(Self {
            port,
            request_count: AtomicU64::new(0),
            error_every_n: AtomicUsize::new(0),
            delay_ms: AtomicUsize::new(delay_ms),
            concurrency_limit: AtomicUsize::new(concurrency_limit),
        })
    }

    pub fn with_delay(port: u16, delay_ms: usize) -> Arc<Self> {
        Self::with_options(port, delay_ms, 8)
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    pub fn set_error_every_n(&self, n: usize) {
        self.error_every_n.store(n, Ordering::Relaxed);
    }

    pub fn set_delay_ms(&self, ms: usize) {
        self.delay_ms.store(ms, Ordering::Relaxed);
    }

    async fn simulate_delay(&self) {
        let ms = self.delay_ms.load(Ordering::Relaxed) as u64;
        if ms > 0 {
            // Add ±50% jitter for realistic latency distribution
            let jitter = if ms > 100 {
                rand::random::<u64>() % (ms / 2)
            } else {
                0
            };
            let actual = ms.saturating_sub(ms / 4) + jitter;
            tokio::time::sleep(std::time::Duration::from_millis(actual)).await;
        }
    }

    pub async fn start(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let app = axum::Router::new()
            .route("/v1/models/info", get(models_info))
            .route("/v1/usage", get(usage))
            .route("/v1/models", get(models))
            .route("/v1/chat/completions", post(chat_completions))
            .route("/v1/messages", post(messages))
            .with_state(self.clone());

        let addr = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", self.port))
            .await
            .expect("mock upstream bind");
        tracing::info!("mock upstream listening on http://127.0.0.1:{}/v1", self.port);

        tokio::spawn(async move {
            axum::serve(addr, app).await.expect("mock upstream serve");
        })
    }
}

async fn models_info(State(state): State<Arc<MockUpstream>>) -> Response {
    state.simulate_delay().await;
    json_response(json!({
        "umans-coder": {
            "id": "umans-coder",
            "display_name": "Coder",
            "capabilities": {
                "context_window": 200000,
                "supports_vision": "true",
                "supports_tools": true,
                "reasoning": { "supported": true, "can_disable": false, "levels": [] }
            }
        },
        "umans-vision": {
            "id": "umans-vision",
            "display_name": "Vision",
            "capabilities": {
                "context_window": 128000,
                "supports_vision": "via-handoff",
                "supports_tools": true,
                "reasoning": { "supported": false, "can_disable": false, "levels": [] }
            }
        }
    }))
}

async fn usage(State(state): State<Arc<MockUpstream>>) -> Response {
    state.simulate_delay().await;
    let limit = state.concurrency_limit.load(Ordering::Relaxed);
    json_response(json!({
        "usage": {
            "requests_in_window": 246,
            "tokens_in": 24000000,
            "tokens_out": 11732073,
            "tokens_cached": 9360000,
            "concurrent_sessions": 2
        },
        "limits": {
            "concurrency": { "limit": limit, "hard_cap": limit * 2 }
        },
        "window": { "started_at": "2026-07-01T00:00:00Z" },
        "plan": { "display_name": "Pro" },
        "user_id": "mock-user-123"
    }))
}

async fn models(State(state): State<Arc<MockUpstream>>) -> Response {
    state.simulate_delay().await;
    json_response(json!({
        "data": [
            { "id": "umans-coder", "pricing": { "input": 0.000001, "output": 0.000003 } },
            { "id": "umans-vision", "pricing": { "input": 0.000002, "output": 0.000005 } }
        ]
    }))
}

async fn chat_completions(
    State(state): State<Arc<MockUpstream>>,
    _headers: HeaderMap,
    body: axum::body::Body,
) -> Response {
    let n = state.request_count.fetch_add(1, Ordering::Relaxed) + 1;
    let err_every = state.error_every_n.load(Ordering::Relaxed);
    if err_every > 0 && n % err_every as u64 == 0 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            json_body(json!({"error": {"message": "mock upstream overloaded", "type": "overloaded_error"}})),
        )
            .into_response();
    }

    let body_bytes = axum::body::to_bytes(body, 5 * 1024 * 1024).await.unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap_or(json!({}));
    let stream = payload.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    let model = payload.get("model").and_then(|m| m.as_str()).unwrap_or("umans-coder");

    // Initial delay applies to both streaming and non-streaming paths
    state.simulate_delay().await;

    if stream {
        let delay_ms = state.delay_ms.load(Ordering::Relaxed) as u64;
        let chunks = vec![
            format!(
                "data: {}\n\n",
                serde_json::to_string(&json!({
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "model": model,
                    "choices": [{"delta": {"role": "assistant", "content": "Mock "}, "index": 0}]
                }))
                .unwrap()
            ),
            format!(
                "data: {}\n\n",
                serde_json::to_string(&json!({
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "model": model,
                    "choices": [{"delta": {"content": "response from the mock upstream."}, "index": 0}]
                }))
                .unwrap()
            ),
            // Final usage chunk (emitted when stream_options.include_usage is set).
            format!(
                "data: {}\n\n",
                serde_json::to_string(&json!({
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "model": model,
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 8,
                        "total_tokens": 18,
                        "prompt_tokens_details": {"cached_tokens": 4}
                    }
                }))
                .unwrap()
            ),
            "data: [DONE]\n\n".to_string(),
        ];
        let stream = async_stream::stream! {
            for (i, chunk) in chunks.into_iter().enumerate() {
                if i > 0 {
                    let chunk_delay = delay_ms.max(50);
                    let jitter = rand::random::<u64>() % (chunk_delay / 3 + 1);
                    tokio::time::sleep(std::time::Duration::from_millis(chunk_delay + jitter)).await;
                }
                yield Ok::<_, std::io::Error>(Bytes::from(chunk));
            }
        };
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from_stream(stream))
            .unwrap()
    } else {
        json_response(json!({
            "id": "chatcmpl-mock",
            "object": "chat.completion",
            "model": model,
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Mock response from the upstream." },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 8, "total_tokens": 18 }
        }))
    }
}

async fn messages(
    State(state): State<Arc<MockUpstream>>,
    _headers: HeaderMap,
    body: axum::body::Body,
) -> Response {
    let body_bytes = axum::body::to_bytes(body, 5 * 1024 * 1024).await.unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap_or(json!({}));
    let stream = payload.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    let model = payload.get("model").and_then(|m| m.as_str()).unwrap_or("umans-coder");

    if stream {
        let delay_ms = state.delay_ms.load(Ordering::Relaxed) as u64;
        let chunks = vec![
            format!(
                "event: message_start\ndata: {}\n\n",
                serde_json::to_string(&json!({"type":"message_start","message":{"id":"msg-mock","type":"message","role":"assistant","model":model,"content":[],"usage":{"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":4,"cache_creation_input_tokens":0}}})).unwrap()
            ),
            format!(
                "event: content_block_delta\ndata: {}\n\n",
                serde_json::to_string(&json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Mock "}})).unwrap()
            ),
            format!(
                "event: content_block_delta\ndata: {}\n\n",
                serde_json::to_string(&json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"response from the mock upstream."}})).unwrap()
            ),
            format!(
                "event: message_delta\ndata: {}\n\n",
                serde_json::to_string(&json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":8}})).unwrap()
            ),
            "event: message_stop\ndata: {}\n\n".to_string(),
        ];
        let stream = async_stream::stream! {
            for (i, chunk) in chunks.into_iter().enumerate() {
                if i > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms.max(50))).await;
                }
                yield Ok::<_, std::io::Error>(Bytes::from(chunk));
            }
        };
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from_stream(stream))
            .unwrap()
    } else {
        json_response(json!({
            "id": "msg-mock",
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": [{"type": "text", "text": "Mock response from the upstream."}],
            "usage": { "input_tokens": 10, "output_tokens": 8 }
        }))
    }
}

fn json_response(value: serde_json::Value) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}

fn json_body(value: serde_json::Value) -> Body {
    Body::from(value.to_string())
}
