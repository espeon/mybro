// ── Model API routes (spec §20, §20.1) ───────────────────────────────────────

use crate::catalog;
use crate::routes::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::sync::Arc;

// ── GET /v1/models (spec §20) ────────────────────────────────────────────────

pub async fn v1_models(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err(_) = super::check_auth(&state, &headers, super::ApiFormat::OpenAI) {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let cfg = state.config.load();
    let cat = catalog::catalog().unwrap_or_else(catalog::Catalog::empty);

    let models: Vec<serde_json::Value> = cat
        .effective_models(&cfg)
        .iter()
        .map(|id| {
            let display_name = cat
                .display
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.strip_prefix("umans-").unwrap_or(id).to_string());

            let info = cat.info.get(id).cloned().unwrap_or(json!({}));
            let caps = info.get("capabilities").cloned().unwrap_or(json!({}));

            let context_window = caps
                .get("context_window")
                .and_then(|v| v.as_u64())
                .filter(|&v| v > 0);

            let max_output_tokens = info
                .get("recommended_max_tokens")
                .and_then(|v| v.as_u64())
                .or_else(|| info.get("max_completion_tokens").and_then(|v| v.as_u64()))
                .or(context_window);

            let mut model = json!({
                "id": id,
                "object": "model",
                "created": state.start_unix,
                "owned_by": "umans",
                "root": id,
                "permission": [],
                "display_name": display_name
            });

            if let Some(cw) = context_window {
                model["context_length"] = json!(cw);
                if let Some(max) = max_output_tokens {
                    model["max_output_tokens"] = json!(max);
                }
            }

            model
        })
        .collect();

    // Enrich with pricing if available (spec §8.5, §20)
    if let Some(key) = state.active_key() {
        if let Ok(resp) = state.upstream.get_models_pricing(&key).await {
            if resp.status().is_success() {
                if let Ok(body) = crate::upstream::read_body(resp).await {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                        if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
                            for model in models.iter() {
                                // pricing enrichment would happen here
                                // (each model's id matched against data array)
                            }
                            // Build pricing map
                            let pricing_map: std::collections::HashMap<&str, &serde_json::Value> =
                                data
                                    .iter()
                                    .filter_map(|m| {
                                        let id = m.get("id").and_then(|i| i.as_str())?;
                                        let pricing = m.get("pricing")?;
                                        Some((id, pricing))
                                    })
                                    .collect();

                            // Rebuild models with pricing
                            let models_with_pricing: Vec<serde_json::Value> = cat
                                .effective_models(&cfg)
                                .iter()
                                .map(|id| {
                                    let display_name = cat
                                        .display
                                        .get(id)
                                        .cloned()
                                        .unwrap_or_else(|| {
                                            id.strip_prefix("umans-").unwrap_or(id).to_string()
                                        });

                                    let info = cat.info.get(id).cloned().unwrap_or(json!({}));
                                    let caps =
                                        info.get("capabilities").cloned().unwrap_or(json!({}));

                                    let context_window = caps
                                        .get("context_window")
                                        .and_then(|v| v.as_u64())
                                        .filter(|&v| v > 0);

                                    let max_output_tokens = info
                                        .get("recommended_max_tokens")
                                        .and_then(|v| v.as_u64())
                                        .or_else(|| {
                                            info.get("max_completion_tokens")
                                                .and_then(|v| v.as_u64())
                                        })
                                        .or(context_window);

                                    let mut model = json!({
                                        "id": id,
                                        "object": "model",
                                        "created": state.start_unix,
                                        "owned_by": "umans",
                                        "root": id,
                                        "permission": [],
                                        "display_name": display_name
                                    });

                                    if let Some(cw) = context_window {
                                        model["context_length"] = json!(cw);
                                        if let Some(max) = max_output_tokens {
                                            model["max_output_tokens"] = json!(max);
                                        }
                                    }

                                    // Add pricing
                                    if let Some(pricing) = pricing_map.get(id.as_str()) {
                                        if let Some(input) =
                                            pricing.get("input").and_then(|v| v.as_f64())
                                        {
                                            model["pricing"]["prompt"] =
                                                json!(input / 1_000_000.0);
                                        }
                                        if let Some(output) =
                                            pricing.get("output").and_then(|v| v.as_f64())
                                        {
                                            model["pricing"]["completion"] =
                                                json!(output / 1_000_000.0);
                                        }
                                    }

                                    model
                                })
                                .collect();

                            return Json(json!({
                                "object": "list",
                                "data": models_with_pricing
                            }))
                            .into_response();
                        }
                    }
                }
            }
        }
    }

    Json(json!({
        "object": "list",
        "data": models
    }))
    .into_response()
}

// ── GET /v1/models/info ──────────────────────────────────────────────────────

pub async fn v1_models_info(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }
    let cat = catalog::catalog().unwrap_or_else(catalog::Catalog::empty);
    let info_map: serde_json::Map<String, serde_json::Value> = cat
        .info
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Json(json!(info_map)).into_response()
}

// ── GET /api/models (spec §20.1) ─────────────────────────────────────────────

pub async fn get_models(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err(_) = super::check_auth(&state, &headers, super::ApiFormat::OpenAI) {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let cfg = state.config.load();
    let cat = catalog::catalog().unwrap_or_else(catalog::Catalog::empty);

    let models: Vec<serde_json::Value> = cat
        .all_catalog_models(&cfg)
        .iter()
        .map(|id| {
            let info = cat.info.get(id).cloned().unwrap_or(json!({}));
            let caps = info.get("capabilities").cloned().unwrap_or(json!({}));

            let reasoning = catalog::reasoning_mode(&caps);
            let variants = catalog::reasoning_variants(&caps);
            let supports_tools = caps
                .get("supports_tools")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let supports_vision = caps
                .get("supports_vision")
                .and_then(|v| v.as_str())
                .unwrap_or("false");
            let context_window = caps
                .get("context_window")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let display_name = cat
                .display
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.strip_prefix("umans-").unwrap_or(id).to_string());

            let mut model = json!({
                "id": id,
                "displayName": display_name,
                "reasoning": reasoning,
                "supportsTools": supports_tools,
                "supportsVision": supports_vision,
                "contextWindow": context_window
            });

            if let Some(v) = variants {
                model["variants"] = v.into();
            }

            model
        })
        .collect();

    Json(json!({
        "models": models,
        "disabled": cfg.disabled_models,
        "displayNames": cat.display
    }))
    .into_response()
}
