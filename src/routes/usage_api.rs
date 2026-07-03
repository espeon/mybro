// ── Usage API endpoints (spec §29) ──────────────────────────────────────────

use crate::routes::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::sync::Arc;

// ── GET /api/umans/usage (spec §29.1) ────────────────────────────────────────

pub async fn get_usage(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let fresh = params.get("fresh").map(|v| v == "1").unwrap_or(false);
    let key = match state.active_key() {
        Some(k) if !k.is_empty() => k,
        _ => {
            return Json(json!({
                "usage": {
                    "requests_in_window": 0,
                    "tokens_in": 0,
                    "tokens_out": 0,
                    "tokens_cached": 0
                },
                "window": {"started_at": ""},
                "plan": {"display_name": ""},
                "throttled": crate::usage::throttled_count()
            }))
            .into_response();
        }
    };

    let usage = crate::usage::fetch_usage(&state.upstream, &key, fresh).await;

    match usage {
        Some(data) => Json(json!({
            "usage": {
                "requests_in_window": data.requests_in_window,
                "tokens_in": data.tokens_in,
                "tokens_out": data.tokens_out,
                "tokens_cached": data.tokens_cached
            },
            "window": {"started_at": data.window_started_at},
            "plan": {"display_name": data.plan_display_name},
            "throttled": crate::usage::throttled_count()
        }))
        .into_response(),
        None => Json(json!({
            "usage": {
                "requests_in_window": 0,
                "tokens_in": 0,
                "tokens_out": 0,
                "tokens_cached": 0
            },
            "window": {"started_at": ""},
            "plan": {"display_name": ""},
            "throttled": crate::usage::throttled_count()
        }))
        .into_response(),
    }
}

// ── GET /api/umans/concurrency (spec §29.2) ─────────────────────────────────

pub async fn get_concurrency(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let fresh = params.get("fresh").map(|v| v == "1").unwrap_or(false);
    let key = match state.active_key() {
        Some(k) if !k.is_empty() => k,
        _ => {
            return Json(json!({
                "concurrent": 0,
                "limit": null,
                "hard_cap": null,
                "user_id": "",
                "overridden": false,
                "active": state.gate.active(),
                "queued": state.gate.queued()
            }))
            .into_response();
        }
    };

    let conc = crate::usage::fetch_concurrency(&state.upstream, &key, fresh).await;
    let cfg = state.config.load();
    let eff = crate::usage::get_effective_concurrency(cfg.override_concurrency);

    match conc {
        Some(data) => Json(json!({
            "concurrent": data.concurrent,
            "limit": data.limit,
            "hard_cap": data.hard_cap,
            "user_id": data.user_id,
            "overridden": eff.overridden,
            "active": state.gate.active(),
            "queued": state.gate.queued()
        }))
        .into_response(),
        None => Json(json!({
            "concurrent": 0,
            "limit": null,
            "hard_cap": null,
            "user_id": "",
            "overridden": eff.overridden,
            "active": state.gate.active(),
            "queued": state.gate.queued()
        }))
        .into_response(),
    }
}

// ── GET /api/umans/usage-history (spec §29.3) ────────────────────────────────


static USAGE_HISTORY_CACHE: parking_lot::Mutex<Option<std::collections::HashMap<String, (std::time::Instant, serde_json::Value)>>> =
    parking_lot::Mutex::new(None);

pub async fn get_usage_history(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let key = match state.active_key() {
        Some(k) if !k.is_empty() => k,
        _ => return (StatusCode::SERVICE_UNAVAILABLE, "No active key").into_response(),
    };

    // Forward query params: from, to, granularity, scope
    let query_parts: Vec<String> = ["from", "to", "granularity", "scope"]
        .iter()
        .filter_map(|k| params.get(*k).map(|v| format!("{}={}", k, v)))
        .collect();
    let query = query_parts.join("&");

    let fresh = params.get("fresh").map(|v| v == "1").unwrap_or(false);
    let cache_key = query.clone();

    // Simple 5-min cache
    if !fresh {
        let cache = USAGE_HISTORY_CACHE.lock();
        if let Some(map) = cache.as_ref() {
            if let Some(cached) = map.get(&cache_key) {
                if cached.0.elapsed() < crate::constants::USAGE_CACHE_TTL {
                    return Json(cached.1.clone()).into_response();
                }
            }
        }
    }

    match state.upstream.get_usage_history(&key, &query).await {
        Ok(resp) => {
            if resp.status().is_success() {
                match crate::upstream::read_body(resp).await {
                    Ok(body) => {
                        let v: serde_json::Value =
                            serde_json::from_slice(&body).unwrap_or(json!({}));
                        // Store in cache
                        let mut cache = USAGE_HISTORY_CACHE.lock();
                        if cache.is_none() {
                            *cache = Some(std::collections::HashMap::new());
                        }
                        if let Some(map) = cache.as_mut() {
                            map.insert(cache_key, (std::time::Instant::now(), v.clone()));
                        }
                        Json(v).into_response()
                    }
                    Err(_) => (StatusCode::BAD_GATEWAY, "Failed to read usage history").into_response(),
                }
            } else {
                (StatusCode::BAD_GATEWAY, "Upstream error").into_response()
            }
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "Failed to fetch usage history").into_response(),
    }
}

// ── GET /api/umans/user (spec §29.4) ────────────────────────────────────────

pub async fn get_user(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let conc = crate::usage::last_concurrency();
    let user_id = conc.map(|c| c.user_id.clone()).unwrap_or_default();

    Json(json!({
        "loggedIn": !user_id.is_empty(),
        "email": "",
        "user_id": user_id
    }))
    .into_response()
}
