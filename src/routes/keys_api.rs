// ── Key management API (spec §28) ─────────────────────────────────────────────

use crate::config::{ConfigKey, mask_token};
use crate::keypool::KeyEntry;
use crate::routes::AppState;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

// ── GET /api/keys (spec §28.1) ──────────────────────────────────────────────

pub async fn get_keys(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let cfg = state.config.load();

    let safe: Vec<serde_json::Value> = cfg
        .keys
        .iter()
        .map(|k| {
            json!({
                "name": k.name,
                "token_masked": mask_token(&k.key),
                "has_token": !k.key.is_empty(),
                "has_session": !k.session.is_empty()
            })
        })
        .collect();

    let raw: Vec<serde_json::Value> = cfg
        .keys
        .iter()
        .map(|k| {
            json!({
                "name": k.name,
                "key": k.key,
                "session": k.session
            })
        })
        .collect();

    Json(json!({
        "keys": raw,
        "safe": safe
    }))
    .into_response()
}

// ── POST /api/keys (spec §28.2) ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
pub enum KeyAction {
    #[serde(rename = "add")]
    Add {
        name: String,
        key: String,
        #[serde(default)]
        session: String,
    },
    #[serde(rename = "update")]
    Update {
        index: usize,
        name: String,
        key: String,
        #[serde(default)]
        session: String,
    },
    #[serde(rename = "delete")]
    Delete {
        index: usize,
    },
}

pub async fn post_keys(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(action): Json<KeyAction>,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let (new_cfg, _) = state.config.update(|cfg| {
        match action {
            KeyAction::Add { name, key, session } => {
                cfg.keys.push(ConfigKey { name, key, session });
                // If no api_key set, use this key
                if cfg.api_key.is_empty() {
                    if let Some(last) = cfg.keys.last() {
                        cfg.api_key = last.key.clone();
                    }
                }
            }
            KeyAction::Update {
                index,
                name,
                key,
                session,
            } => {
                if index < cfg.keys.len() {
                    cfg.keys[index] = ConfigKey { name, key, session };
                    // If index 0 and key present, set as api_key
                    if index == 0 && !cfg.keys[0].key.is_empty() {
                        cfg.api_key = cfg.keys[0].key.clone();
                    }
                }
            }
            KeyAction::Delete { index } => {
                if index < cfg.keys.len() {
                    cfg.keys.remove(index);
                    // If list empties, push placeholder
                    if cfg.keys.is_empty() {
                        cfg.keys.push(ConfigKey {
                            name: "Key 1".to_string(),
                            key: String::new(),
                            session: String::new(),
                        });
                    }
                    // If index 0, refresh api_key
                    if index == 0 {
                        cfg.api_key = cfg
                            .keys
                            .iter()
                            .find(|k| !k.key.is_empty())
                            .map(|k| k.key.clone())
                            .unwrap_or_default();
                    }
                }
            }
        }
    });

    // Rebuild key pool
    let entries: Vec<KeyEntry> = new_cfg
        .keys
        .iter()
        .map(KeyEntry::from_config)
        .collect();
    state.keypool.rebuild(entries);

    // Persist
    state.debounced_saver.ping();

    Json(json!({"success": true})).into_response()
}
