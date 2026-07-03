// ─<arg_value>/~ Config API (spec §26–27) ──────────────────────────────────────────────────

use crate::config::{ConfigKey, mask_token, WallpaperSource};
use crate::routes::AppState;
use crate::keypool::KeyEntry;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

// ── GET /api/config (spec §26) ──────────────────────────────────────────────

pub async fn get_config(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let cfg = state.config.load();

    Json(json!({
        "listenAddr": cfg.listen_addr,
        "upstreamBaseURL": cfg.upstream_base_url,
        "apiKey": mask_token(&cfg.api_key),
        "enabledModels": cfg.enabled_models,
        "modelDisplayNames": cfg.model_display_names,
        "wallpaperSource": cfg.wallpaper_source.to_string(),
        "overrideConcurrency": cfg.override_concurrency,
        "maxImages": cfg.max_images,
        "disabledModels": cfg.disabled_models,
        "visionHandoffEnabled": cfg.vision_handoff_enabled,
        "visionHandoffModel": cfg.vision_handoff_model,
        "visionHandoffPrompt": cfg.vision_handoff_prompt,
        "visionHandoffCacheEnabled": cfg.vision_handoff_cache_enabled
    }))
    .into_response()
}

// ── POST /api/config (spec §27) ──────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ConfigUpdate {
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    #[serde(rename = "apiKeys")]
    pub api_keys: Option<Vec<String>>,
    #[serde(rename = "listenAddr")]
    pub listen_addr: Option<String>,
    #[serde(rename = "enabledModels")]
    pub enabled_models: Option<Vec<String>>,
    #[serde(rename = "modelDisplayNames")]
    pub model_display_names: Option<std::collections::HashMap<String, String>>,
    #[serde(rename = "wallpaperSource")]
    pub wallpaper_source: Option<String>,
    #[serde(rename = "overrideConcurrency")]
    pub override_concurrency: Option<u32>,
    #[serde(rename = "maxImages")]
    pub max_images: Option<usize>,
    #[serde(rename = "disabledModels")]
    pub disabled_models: Option<Vec<String>>,
    #[serde(rename = "visionHandoffEnabled")]
    pub vision_handoff_enabled: Option<bool>,
    #[serde(rename = "visionHandoffModel")]
    pub vision_handoff_model: Option<String>,
    #[serde(rename = "visionHandoffPrompt")]
    pub vision_handoff_prompt: Option<String>,
    #[serde(rename = "visionHandoffCacheEnabled")]
    pub vision_handoff_cache_enabled: Option<bool>,
    pub keys: Option<Vec<ConfigKey>>,
}

pub async fn post_config(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(update): Json<ConfigUpdate>,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let mut restart_required = false;
    let mut keypool_rebuild = false;

    let mut resize_cache = false;
    let (new_cfg, _) = state.config.update(|cfg| {
        if let Some(v) = update.api_key {
            cfg.api_key = v;
        }
        if let Some(v) = update.api_keys {
            cfg.api_keys = v;
        }
        if let Some(v) = &update.listen_addr {
            if *v != cfg.listen_addr {
                restart_required = true;
            }
            cfg.listen_addr = v.clone();
        }
        if let Some(v) = update.enabled_models {
            cfg.enabled_models = v;
        }
        if let Some(v) = update.model_display_names {
            cfg.model_display_names = v;
        }
        if let Some(v) = &update.wallpaper_source {
            cfg.wallpaper_source = match v.as_str() {
                "none" => WallpaperSource::None,
                "wallhaven" => WallpaperSource::Wallhaven,
                _ => WallpaperSource::Bing,
            };
        }
        if let Some(v) = update.override_concurrency {
            cfg.override_concurrency = v;
        }
        if let Some(v) = update.max_images {
            cfg.max_images = v;
        }
        if let Some(v) = update.disabled_models {
            cfg.disabled_models = v;
        }
        if let Some(v) = update.vision_handoff_enabled {
            cfg.vision_handoff_enabled = v;
        }
        if let Some(v) = update.vision_handoff_model {
            cfg.vision_handoff_model = v;
        }
        if let Some(v) = update.vision_handoff_prompt {
            cfg.vision_handoff_prompt = v;
            resize_cache = true;
        }
        if let Some(v) = update.vision_handoff_cache_enabled {
            cfg.vision_handoff_cache_enabled = v;
        }
        if let Some(keys) = update.keys {
            cfg.keys = keys;
            keypool_rebuild = true;
        }
    });

    // Side effects
    if update.override_concurrency.is_some() {
        let eff = crate::usage::get_effective_concurrency(new_cfg.override_concurrency);
        state.gate.reconcile(eff.hard_cap.or(eff.limit).map(|n| n as usize));
    }
    if update.vision_handoff_cache_enabled.is_some() || resize_cache {
        // resize cache if needed
        let cfg = state.config.load();
        state.handoff_cache.resize(
            crate::constants::HANDOFF_CACHE_SIZE,
            cfg.handoff_cache_ttl_duration(),
        );
    }
    if keypool_rebuild {
        let entries: Vec<KeyEntry> = new_cfg
            .keys
            .iter()
            .map(KeyEntry::from_config)
            .collect();
        state.keypool.rebuild(entries);
    }

    // Persist
    state.debounced_saver.ping();

    // Build response
    let mut resp = serde_json::to_value(&*new_cfg).unwrap_or_default();
    if let Some(obj) = resp.as_object_mut() {
        obj.insert("restartRequired".to_string(), json!(restart_required));
    }

    Json(resp).into_response()
}

// ── GET /api/validate (spec §20.1) ──────────────────────────────────────────

pub async fn validate(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let key = match state.active_key() {
        Some(k) if !k.is_empty() => k,
        _ => return Json(json!({"valid": false})).into_response(),
    };

    match state.upstream.get_user_info(&key).await {
        Ok(resp) => {
            if resp.status().is_success() {
                // Apply catalog snapshot
                if let Ok(body) = crate::upstream::read_body(resp).await {
                    if let Ok(map) = serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&body) {
                        let cat = crate::catalog::Catalog::from_info(map);
                        crate::catalog::set_catalog(cat);
                    }
                }
                Json(json!({"valid": true})).into_response()
            } else {
                Json(json!({"valid": false})).into_response()
            }
        }
        Err(_) => Json(json!({"valid": false})).into_response(),
    }
}
