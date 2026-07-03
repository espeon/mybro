// ── Restart API (spec §31) ──────────────────────────────────────────────────

use crate::routes::AppState;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::sync::Arc;

pub async fn restart(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    // Respond immediately, then initiate graceful shutdown after 500ms
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        tracing::info!("restart requested — exiting with code 42");
        // Flush error log
        // Graceful shutdown is handled by the main process signal handler
        std::process::exit(42);
    });

    Json(json!({
        "success": true,
        "message": "Restarting..."
    }))
    .into_response()
}
