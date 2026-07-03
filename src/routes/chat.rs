// ── OpenAI pipeline: /v1/chat/completions (spec §16) ──────────────────────────

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

pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Response {
    // 1. Auth
    if super::check_auth(&state, &headers, ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(ApiFormat::OpenAI).into_response();
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
            return super::openai_error(
                StatusCode::BAD_REQUEST,
                &format!("Invalid JSON: {}", e),
                "invalid_request_error",
            )
            .into_response();
        }
    };

    let is_stream = payload
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    // 4. Gate admission (§5.2)
    let _guard = match state.gate.acquire().await {
        Ok(g) => g,
        Err(_) => {
            return super::openai_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Queue full",
                "queue_full",
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
            return super::openai_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "No healthy API keys available",
                "api_error",
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

    // 6. strip_reasoning_content
    payload::strip_reasoning_content(&mut payload);

    // 7. resolve_model_id → write back
    let cfg = state.config.load();
    let requested_model = payload
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let resolved_model = catalog::resolve_model_id(&requested_model, &cfg);
    payload["model"] = json!(resolved_model);

    // 8. Tool normalization
    if let Some(tools) = payload.get_mut("tools") {
        schema_norm::normalize_tool_schemas(tools);
    }

    // 9. If not handoff model: limit_images
    if !catalog::needs_vision_handoff(&resolved_model, &cfg) {
        payload::limit_images_in_messages(&mut payload, cfg.max_images);
    }

    // 10. Reasoning caps auto-think
    payload::apply_auto_think(&mut payload, &resolved_model);

    // 11. normalize_thinking_payload
    payload::normalize_thinking_payload(&mut payload);

    // 12. Vision handoff
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

    // 13. Serialize once → Bytes
    let serialized = serde_json::to_vec(&payload).unwrap_or_default();
    let body_bytes = Bytes::from(serialized);

    // 14. Retry loop (§13)
    let max_retries = crate::constants::MAX_RETRIES;
    let mut current_slot = slot.clone();
    let mut last_error_body: String;
    let req_start = std::time::Instant::now();

    for attempt in 1..=max_retries {
        if attempt > 1 && state.keypool.total() > 1 {
            if let Some(new_slot) = state.keypool.acquire(None) {
                current_slot = new_slot;
            }
        }

        let body_clone = body_bytes.clone();
        let result = state
            .upstream
            .chat_completions(&current_slot.key, body_clone, is_stream)
            .await;

        match result {
            Ok(resp) => {
                let status = resp.status();
                let status_u16 = status.as_u16();
                let headers_clone = resp.headers().clone();

                if status.is_success() {
                    state.keypool.mark_healthy(current_slot.index);

                    let content_type = headers_clone
                        .get(header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();

                    let is_sse = content_type.contains("text/event-stream");

                    if is_sse {
                        record_stat(&state, req_start, 200, &resolved_model, "openai", &current_slot.name, None);
                        let stream = super::stream::pipe_response(resp);
                        return Response::builder()
                            .status(StatusCode::OK)
                            .header(header::CONTENT_TYPE, "text/event-stream")
                            .header(header::CACHE_CONTROL, "no-cache")
                            .body(stream)
                            .unwrap();
                    } else {
                        let body = crate::upstream::read_body(resp).await.unwrap_or_default();
                        record_stat(&state, req_start, 200, &resolved_model, "openai", &current_slot.name, None);
                        if is_stream {
                            return wrap_json_as_sse(body);
                        } else {
                            return Response::builder()
                                .status(StatusCode::OK)
                                .header(header::CONTENT_TYPE, "application/json")
                                .body(Body::from(body))
                                .unwrap();
                        }
                    }
                } else if status_u16 == 500 || status_u16 == 503 {
                    last_error_body = read_error_body(resp).await;
                    state.keypool.mark_unhealthy(current_slot.index, status_u16);
                    log_upstream_error(&state, attempt, session.sess_num, &current_slot.name, status_u16, &last_error_body);

                    if attempt == max_retries {
                        if status_u16 == 503 {
                            state.gate.bump_throttled();
                        }
                        record_stat(&state, req_start, status_u16, &resolved_model, "openai", &current_slot.name, Some(&last_error_body));
                        return Response::builder()
                            .status(status)
                            .header(header::CONTENT_TYPE, "application/json")
                            .body(Body::from(last_error_body))
                            .unwrap();
                    }
                    let delay = std::time::Duration::from_millis(3000 + 3000 * (attempt as u64 - 1));
                    tokio::time::sleep(delay).await;
                } else {
                    if status_u16 == 429 {
                        state.gate.bump_throttled();
                    }
                    let body = crate::upstream::read_body(resp).await.unwrap_or_default();
                    record_stat(&state, req_start, status_u16, &resolved_model, "openai", &current_slot.name, None);
                    let mut builder = Response::builder().status(status);
                    for (k, v) in &headers_clone {
                        builder = builder.header(k, v);
                    }
                    return builder.body(Body::from(body)).unwrap();
                }
            }
            Err(e) => {
                last_error_body = e.to_string();
                state.keypool.mark_unhealthy(current_slot.index, 502);
                log_upstream_error(&state, attempt, session.sess_num, &current_slot.name, 502, &last_error_body);

                if attempt == max_retries {
                    record_stat(&state, req_start, 502, &resolved_model, "openai", &current_slot.name, Some(&last_error_body));
                    return super::openai_error(
                        StatusCode::BAD_GATEWAY,
                        &format!("Upstream error: {}", e),
                        "upstream_error",
                    )
                    .into_response();
                }
                let delay = std::time::Duration::from_millis(3000 + 3000 * (attempt as u64 - 1));
                tokio::time::sleep(delay).await;
            }
        }
    }

    record_stat(&state, req_start, 500, &resolved_model, "openai", &current_slot.name, Some("max retries exhausted"));
    super::openai_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Max retries exhausted",
        "api_error",
    )
    .into_response()
}

// ── Stats helper ─────────────────────────────────────────────────────────────

fn record_stat(
    state: &AppState,
    start: std::time::Instant,
    status: u16,
    model: &str,
    pipeline: &'static str,
    key_name: &str,
    error: Option<&str>,
) {
    use crate::stats::RequestRecord;
    state.stats.record(RequestRecord {
        ts_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        duration_ms: start.elapsed().as_millis() as u64,
        status,
        model: model.to_string(),
        pipeline,
        key_name: key_name.to_string(),
        tokens_in: 0,
        tokens_out: 0,
        cached: false,
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

async fn read_error_body(resp: reqwest::Response) -> String {
    match crate::upstream::read_body(resp).await {
        Ok(b) => String::from_utf8_lossy(&b).to_string(),
        Err(_) => String::new(),
    }
}

fn wrap_json_as_sse(body: Bytes) -> Response {
    let mut sse = String::new();
    sse.push_str("data: ");
    sse.push_str(&String::from_utf8_lossy(&body));
    sse.push_str("\n\n");
    sse.push_str("data: [DONE]\n\n");

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(sse))
        .unwrap()
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
        stage: if status == 500 || status == 503 {
            "retryable_attempt"
        } else {
            "final_attempt"
        }
        .to_string(),
        attempt,
        sess_num,
        slot_name: slot_name.to_string(),
        request: RequestLog {
            method: "POST".to_string(),
            url: "/v1/chat/completions".to_string(),
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
