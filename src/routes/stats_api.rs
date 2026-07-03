// ── /api/stats — time-series request stats ───────────────────────────────────

use crate::routes::AppState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::sync::Arc;

pub async fn get_stats(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let window = params
        .get("window")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);
    let bucket = params
        .get("bucket")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(10);
    let mode = params.get("mode").map(|s| s.as_str()).unwrap_or("buckets");
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100);
    let model = params.get("model").filter(|m| !m.is_empty()).map(|m| m.as_str());

    let window_ms = window * 1000;
    let bucket_ms = bucket * 1000;

    match mode {
        "summary" => {
            let summary = state.stats.summary(window_ms, model);
            Json(json!(summary)).into_response()
        }
        "recent" => {
            let records = state.stats.recent(limit, model);
            Json(json!({ "records": records })).into_response()
        }
        "models" => {
            let models = state.stats.distinct_models(window_ms);
            Json(json!({ "models": models })).into_response()
        }
        _ => {
            let buckets = state.stats.buckets(window_ms, bucket_ms, model);
            let summary = state.stats.summary(window_ms, model);
            Json(json!({
                "buckets": buckets,
                "summary": summary,
                "window_sec": window,
                "bucket_sec": bucket,
            }))
            .into_response()
        }
    }
}

pub async fn get_token_stats(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let window = params
        .get("window")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);
    let model = params.get("model").filter(|m| !m.is_empty()).map(|m| m.as_str());
    let window_ms = window * 1000;

    let tokens = state.stats.token_stats(window_ms, model);
    Json(json!({
        "window_sec": window,
        "tokens": tokens,
    }))
    .into_response()
}

/// `/api/umans/gate` — current concurrency state (in-flight, queued, limits).
/// This is the live "what's happening right now" endpoint.
pub async fn get_gate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let cfg = state.config.load();
    let eff = crate::usage::get_effective_concurrency(cfg.override_concurrency);

    Json(json!({
        "active": state.gate.active(),
        "queued": state.gate.queued(),
        "throttled": state.gate.throttled(),
        "limit": eff.limit,
        "hard_cap": eff.hard_cap,
        "overridden": eff.overridden,
        "max_queue_size": crate::constants::MAX_QUEUE_SIZE,
    }))
    .into_response()
}
