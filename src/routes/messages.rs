// ── Anthropic pipeline: /v1/messages, /messages (spec §17) ───────────────────

use crate::payload;
use crate::schema_norm;
use crate::catalog;
use crate::vision;
use crate::routes::{AppState, ApiFormat};
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::json;
use std::sync::Arc;

pub async fn messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Response {
    // 1. Auth
    if super::check_auth(&state, &headers, ApiFormat::Anthropic).is_err() {
        return super::auth_error_response(ApiFormat::Anthropic).into_response();
    }

    // 2. Read body (≤ 5 MiB)
    let body_bytes = match read_body_limited(body).await {
        Ok(b) => b,
        Err(status) => return (status, "Body too large").into_response(),
    };

    // 3. Parse JSON
    let mut payload: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(e) => {
            return super::anthropic_error(
                StatusCode::BAD_REQUEST,
                &format!("Invalid JSON: {}", e),
            )
            .into_response();
        }
    };

    let is_stream = payload
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    if state.keypool.total() == 0 {
        return super::anthropic_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "No API keys configured",
        )
        .into_response();
    }

    // 4. Gate admission
    let _guard = match state.gate.acquire().await {
        Ok(g) => g,
        Err(_) => {
            return super::anthropic_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Queue full",
            )
            .into_response();
        }
    };

    // 5. Fingerprint → session → key acquire
    let fp = payload::fingerprint_payload(&payload);
    let preferred = state.conv_map.touch(&fp).and_then(|t| t.token_index);

    let slot = match state.keypool.acquire(preferred) {
        Some(s) => s,
        None => {
            return super::anthropic_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "No healthy API keys available",
            )
            .into_response();
        }
    };

    let session = state.conv_map.track(&fp, slot.index);
    if session.is_new {
        let prompt = payload::extract_user_prompt(&payload);
        let truncated = if prompt.len() > 80 { &prompt[..80] } else { &prompt };
        tracing::info!("[session {}] first prompt: {}", session.sess_num, truncated);
    }

    // 6. normalize_thinking_payload
    payload::normalize_thinking_payload(&mut payload);

    // 7. resolve_model_id → write back
    let cfg = state.config.load();
    let requested_model = payload
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let resolved_model = catalog::resolve_model_id(&requested_model, &cfg);
    payload["model"] = json!(resolved_model);

    // 8. limit_images
    payload::limit_images_in_messages(&mut payload, cfg.max_images);

    // 9. perform_vision_handoff
    let needs_handoff = catalog::needs_vision_handoff(&resolved_model, &cfg);
    if needs_handoff {
        let cache = if cfg.vision_handoff_cache_enabled {
            Some(state.handoff_cache.as_ref())
        } else {
            None
        };
        let _ = vision::perform_vision_handoff(
            &mut payload,
            &resolved_model,
            &cfg,
            &state.upstream,
            &slot.key,
            cache,
        )
        .await;
    }

    // 10. Tool normalization
    if let Some(tools) = payload.get_mut("tools") {
        schema_norm::normalize_tool_schemas(tools);
    }

    // 11. Serialize once → Bytes
    let serialized = serde_json::to_vec(&payload).unwrap_or_default();
    let body_bytes = Bytes::from(serialized);

    // 12. Upstream call (no retry loop)
    let req_start = std::time::Instant::now();
    // Resolve websearch_provider: client header overrides, else config default
    let websearch = headers
        .get("x-umans-websearch-provider")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.websearch_provider.clone());
    let upstream_start = std::time::Instant::now();
    let result = state
        .upstream
        .messages(&slot.key, body_bytes, is_stream, &websearch)
        .await;

    match result {
        Ok(resp) => {
            let status = resp.status();
            let status_u16 = status.as_u16();
            let headers_clone = resp.headers().clone();

            if status.is_success() {
                state.keypool.mark_healthy(slot.index);
                // Streaming: capture TTFT on first chunk, record stats on completion.
                let record = std::sync::Arc::new(std::sync::Mutex::new(
                    crate::stats::RequestRecord {
                        ts_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0),
                        duration_ms: 0,
                        ttft_ms: None,
                        status: 200,
                        model: resolved_model.clone(),
                        pipeline: "anthropic",
                        key_name: slot.name.to_string(),
                        tokens_in: 0,
                        tokens_out: 0,
                        cached_tokens: 0,
                        cache_creation_tokens: 0,
                        cached: false,
                        error: None,
                    },
                ));
                let record_for_first = record.clone();
                let record_for_complete = record.clone();
                let state_for_complete = state.clone();
                let req_start_for_first = upstream_start;
                let req_start_for_complete = req_start;

                let on_first = move || {
                    if let Ok(mut r) = record_for_first.lock() {
                        r.ttft_ms = Some(req_start_for_first.elapsed().as_millis() as u64);
                    }
                };
                let on_complete = move || {
                    let final_record = if let Ok(mut r) = record_for_complete.lock() {
                        r.duration_ms = req_start_for_complete.elapsed().as_millis() as u64;
                        r.clone()
                    } else {
                        return;
                    };
                    state_for_complete.stats.record(final_record);
                };

                let stream = super::stream::pipe_response(resp, on_first, on_complete);

                let mut builder = Response::builder().status(status);
                for (k, v) in &headers_clone {
                    builder = builder.header(k, v);
                }
                builder = builder.header(header::CACHE_CONTROL, "no-cache");
                return builder.body(stream).unwrap();
            }

            if status_u16 >= 400 {
                let error_body = match crate::upstream::read_body(resp).await {
                    Ok(b) => String::from_utf8_lossy(&b).to_string(),
                    Err(_) => String::new(),
                };

                log_upstream_error(&state, 1, session.sess_num, &slot.name, status_u16, &error_body);

                if status_u16 == 503 {
                    state.keypool.mark_unhealthy(slot.index, status_u16);
                }
                if status_u16 == 429 || status_u16 == 503 {
                    state.gate.bump_throttled();
                }

                record_stat(&state, req_start, None, status_u16, &resolved_model, "anthropic", &slot.name, 0, 0, 0, 0, Some(&error_body));

                let mut builder = Response::builder().status(status);
                for (k, v) in &headers_clone {
                    builder = builder.header(k, v);
                }
                builder.body(Body::from(error_body)).unwrap()
            } else {
                let body = crate::upstream::read_body(resp).await.unwrap_or_default();
                Response::builder()
                    .status(status)
                    .body(Body::from(body))
                    .unwrap()
            }
        }
        Err(e) => {
            state.keypool.mark_unhealthy(slot.index, 502);
            log_upstream_error(&state, 1, session.sess_num, &slot.name, 502, &e.to_string());
            record_stat(&state, req_start, None, 502, &resolved_model, "anthropic", &slot.name, 0, 0, 0, 0, Some(&e.to_string()));
            super::anthropic_error(
                StatusCode::BAD_GATEWAY,
                &format!("Upstream error: {}", e),
            )
            .into_response()
        }
    }
}

// ── Stats helper ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn record_stat(
    state: &AppState,
    start: std::time::Instant,
    ttft_ms: Option<u64>,
    status: u16,
    model: &str,
    pipeline: &'static str,
    key_name: &str,
    tokens_in: u64,
    tokens_out: u64,
    cached_tokens: u64,
    cache_creation_tokens: u64,
    error: Option<&str>,
) {
    use crate::stats::RequestRecord;
    state.stats.record(RequestRecord {
        ts_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        duration_ms: start.elapsed().as_millis() as u64,
        ttft_ms,
        status,
        model: model.to_string(),
        pipeline,
        key_name: key_name.to_string(),
        tokens_in,
        tokens_out,
        cached_tokens,
        cache_creation_tokens,
        cached: cached_tokens > 0 || cache_creation_tokens > 0,
        error: error.map(|s| s.to_string()),
    });
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn read_body_limited(body: axum::body::Body) -> Result<Bytes, StatusCode> {
    use http_body_util::BodyExt;
    match BodyExt::collect(http_body_util::Limited::new(body, crate::constants::MAX_BODY_SIZE)).await {
        Ok(c) => Ok(c.to_bytes()),
        Err(_) => Err(StatusCode::PAYLOAD_TOO_LARGE),
    }
}

fn log_upstream_error(
    state: &AppState,
    attempt: u32,
    sess_num: u64,
    slot_name: &str,
    status: u16,
    error_body: &str,
) {
    use crate::errlog::{ErrorRecord, RequestLog, UpstreamLog};
    let record = ErrorRecord {
        timestamp: chrono::Local::now().to_rfc3339(),
        error_type: "upstream_http_error".to_string(),
        stage: "final_attempt".to_string(),
        attempt,
        sess_num,
        slot_name: slot_name.to_string(),
        request: RequestLog {
            method: "POST".to_string(),
            url: "/v1/messages".to_string(),
            headers: json!({}),
            body: json!("[redacted]"),
        },
        upstream: Some(UpstreamLog {
            url: state.upstream.base.clone(),
            method: "POST".to_string(),
            headers: json!({}),
            status,
            status_text: StatusCode::from_u16(status)
                .map(|s| s.canonical_reason().unwrap_or("").to_string())
                .unwrap_or_default(),
            body: serde_json::from_str(error_body).unwrap_or(json!(error_body)),
        }),
    };
    state.error_log.log_error(&record);
}
