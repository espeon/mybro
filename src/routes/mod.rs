use crate::config::{ConfigStore, mask_token};
use crate::errlog::ErrorLog;
use crate::gate::Gate;
use crate::handoff_cache::HandoffCache;
use crate::keypool::KeyPool;
use crate::persist::DebouncedSaver;
use crate::sessions::ConvMap;
use crate::upstream::Upstream;
use std::sync::Arc;
use std::time::Instant;

// ── Shared application state ─────────────────────────────────────────────────

pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub upstream: Arc<Upstream>,
    pub keypool: Arc<KeyPool>,
    pub gate: Arc<Gate>,
    pub conv_map: Arc<ConvMap>,
    pub handoff_cache: Arc<HandoffCache>,
    pub error_log: Arc<ErrorLog>,
    pub debounced_saver: Arc<DebouncedSaver>,
    pub stats: Arc<crate::stats::StatsCollector>,
    pub started_at: Instant,
    pub start_unix: u64,
    /// Dev mode: proxy non-API routes to this Vite dev server URL.
    pub dev_proxy: Option<String>,
}

impl AppState {
    pub fn active_key(&self) -> Option<String> {
        let cfg = self.config.load();
        if !cfg.api_key.is_empty() {
            Some(cfg.api_key.clone())
        } else {
            // Use first key from the pool
            let state = self.keypool.state();
            state.iter().find(|s| !s.token.is_empty() && s.token != "***").map(|_| {
                // We need the raw key — get it from config
                cfg.keys
                    .iter()
                    .find(|k| !k.key.is_empty())
                    .map(|k| k.key.clone())
                    .unwrap_or_default()
            })
        }
    }
}

// ── Auth (spec §15.1) ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    OpenAI,
    Anthropic,
}

pub fn check_auth(state: &AppState, headers: &axum::http::HeaderMap, format: ApiFormat) -> Result<(), axum::http::StatusCode> {
    let cfg = state.config.load();
    if cfg.api_keys.is_empty() {
        return Ok(()); // open access
    }

    // Accept X-Api-Key or Authorization: Bearer
    let provided = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "))
        });

    let Some(provided) = provided else {
        return Err(axum::http::StatusCode::UNAUTHORIZED);
    };

    // Constant-time comparison
    for valid in &cfg.api_keys {
        if subtle::ConstantTimeEq::ct_eq(
            provided.as_bytes(),
            valid.as_bytes(),
        )
        .into()
        {
            return Ok(());
        }
    }

    Err(axum::http::StatusCode::UNAUTHORIZED)
}

/// Helper to build an error response in the correct format.
pub fn auth_error_response(format: ApiFormat) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    match format {
        ApiFormat::OpenAI => (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "error": {
                    "message": "Invalid API key",
                    "type": "authentication_error",
                    "code": "invalid_api_key"
                }
            })),
        ),
        ApiFormat::Anthropic => (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "type": "error",
                "error": {
                    "type": "authentication_error",
                    "message": "Invalid API key"
                }
            })),
        ),
    }
}

// ── Error helpers (spec §15.3) ───────────────────────────────────────────────

pub fn openai_error(status: axum::http::StatusCode, msg: &str, error_type: &str) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "error": {
            "message": msg,
            "type": error_type,
        }
    }))
}

pub fn anthropic_error(status: axum::http::StatusCode, msg: &str) -> axum::Json<serde_json::Value> {
    let error_type = match status.as_u16() {
        400 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        429 => "rate_limit_error",
        500 => "api_error",
        503 => "overloaded_error",
        _ => "api_error",
    };
    axum::Json(serde_json::json!({
        "type": "error",
        "error": {
            "type": error_type,
            "message": msg
        }
    }))
}

// ── mask_token re-export for convenience ─────────────────────────────────────

pub use crate::config::mask_token as _mask_token;

// ── Router ───────────────────────────────────────────────────────────────────

pub fn build_router(state: Arc<AppState>) -> axum::Router {
    use axum::routing::{get, post};

    axum::Router::new()
        // Dashboard
        .route("/", get(dashboard::serve_dashboard))
        .route("/dashboard", get(dashboard::serve_dashboard))
        // Health
        .route("/healthz", get(healthz::healthz))
        // API — config
        .route("/api/config", get(config_api::get_config).post(config_api::post_config))
        .route("/api/validate", get(config_api::validate))
        // API — models
        .route("/api/models", get(models_api::get_models))
        // API — stats (time-series)
        .route("/api/stats", get(stats_api::get_stats))
        .route("/api/stats/tokens", get(stats_api::get_token_stats))
        // API — keys
        .route("/api/keys", get(keys_api::get_keys).post(keys_api::post_keys))
        // API — usage
        .route("/api/umans/usage", get(usage_api::get_usage))
        .route("/api/umans/concurrency", get(usage_api::get_concurrency))
        .route("/api/umans/usage-history", get(usage_api::get_usage_history))
        .route("/api/umans/user", get(usage_api::get_user))
        // API — restart
        .route("/api/restart", post(restart::restart))
        // API — wallpaper
        .route("/api/bg", get(wallpaper::bing_wallpaper))
        .route("/api/bg-wallhaven", get(wallpaper::wallhaven_wallpaper))
        // OpenAI-compatible
        .route("/v1/models", get(models_api::v1_models))
        .route("/v1/models/info", get(models_api::v1_models_info))
        .route("/v1/chat/completions", post(chat::chat_completions))
        // Anthropic-compatible
        .route("/v1/messages", post(messages::messages))
        .route("/messages", post(messages::messages))
        // Fallback for embedded assets (SPA)
        .fallback(dashboard::serve_asset)
        .with_state(state)
}

// ── Module declarations ─────────────────────────────────────────────────────

pub mod chat;
pub mod dashboard;
pub mod config_api;
pub mod healthz;
pub mod keys_api;
pub mod messages;
pub mod models_api;
pub mod restart;
pub mod stats_api;
pub mod stream;
pub mod usage_api;
pub mod wallpaper;
