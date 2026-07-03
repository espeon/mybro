// ── /healthz (spec §19) ──────────────────────────────────────────────────────

use crate::routes::AppState;
use axum::Json;
use serde_json::json;
use std::sync::Arc;

pub async fn healthz(axum::extract::State(state): axum::extract::State<Arc<AppState>>) -> Json<serde_json::Value> {
    let cfg = state.config.load();
    let token_state = state.keypool.state();
    let valid_tokens = token_state
        .iter()
        .filter(|t| t.status != "none")
        .count();
    let total_tokens = token_state.len();

    let models_count = crate::catalog::catalog()
        .map(|c| c.ordered_ids.len())
        .unwrap_or(0);

    let uptime = state.started_at.elapsed().as_secs();

    let cache_stats = state.handoff_cache.stats();

    // Refresh user info if older than 5 min (fire-and-forget)
    // For now, just return what we have

    Json(json!({
        "ok": true,
        "started_at": chrono::DateTime::from_timestamp(state.start_unix as i64, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default(),
        "uptime_sec": uptime,
        "api_key_valid": valid_tokens > 0,
        "provider": "umans",
        "token_state": token_state,
        "valid_tokens": valid_tokens,
        "total_tokens": total_tokens,
        "models_count": models_count,
        "runtime": "rust",
        "runtime_version": env!("CARGO_PKG_VERSION"),
        "port": cfg.listen_addr.split(':').nth(1).and_then(|p| p.parse::<u16>().ok()).unwrap_or(8084),
        "visionHandoff": {
            "enabled": cfg.vision_handoff_enabled,
            "cacheEnabled": cfg.vision_handoff_cache_enabled,
            "cache": {
                "size": cache_stats.size,
                "maxSize": cache_stats.max_size,
                "ttlMs": cache_stats.ttl_ms,
                "hits": cache_stats.hits,
                "misses": cache_stats.misses,
                "evictions": cache_stats.evictions
            }
        }
    }))
}
