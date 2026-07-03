use crate::catalog;
use crate::config::Config;
use crate::constants::DEFAULT_HANDOFF_PROMPT;
use crate::handoff_cache::HandoffCache;
use crate::upstream::{Upstream, read_body};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;

// ── Vision handoff (spec §9) ─────────────────────────────────────────────────

/// Reference to an image part in the payload, stored as a JSON pointer path.
pub struct ImageRef {
    pub path: String,
    pub data_uri: String,
}

/// Walk system + all message content arrays collecting image parts (spec §9.3).
pub fn collect_image_parts(payload: &Value) -> Vec<ImageRef> {
    let mut refs = Vec::new();

    // Walk system array
    if let Some(system) = payload.get("system").and_then(|s| s.as_array()) {
        for (i, part) in system.iter().enumerate() {
            if let Some(uri) = extract_image_uri(part) {
                refs.push(ImageRef {
                    path: format!("/system/{}", i),
                    data_uri: uri,
                });
            }
        }
    }

    // Walk messages
    if let Some(messages) = payload.get("messages").and_then(|m| m.as_array()) {
        for (mi, msg) in messages.iter().enumerate() {
            if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                for (ci, part) in content.iter().enumerate() {
                    if let Some(uri) = extract_image_uri(part) {
                        refs.push(ImageRef {
                            path: format!("/messages/{}/content/{}", mi, ci),
                            data_uri: uri,
                        });
                    }
                    // Recurse into tool_result blocks (nested content arrays)
                    if part.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        if let Some(nested) = part.get("content").and_then(|c| c.as_array()) {
                            for (ni, np) in nested.iter().enumerate() {
                                if let Some(uri) = extract_image_uri(np) {
                                    refs.push(ImageRef {
                                        path: format!(
                                            "/messages/{}/content/{}/content/{}",
                                            mi, ci, ni
                                        ),
                                        data_uri: uri,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    refs
}

/// Extract the data URI from an image part (OpenAI or Anthropic format).
fn extract_image_uri(part: &Value) -> Option<String> {
    let part_type = part.get("type").and_then(|t| t.as_str())?;
    match part_type {
        "image_url" => {
            // OpenAI: {"type":"image_url","image_url":{"url":u}}
            part.get("image_url")
                .and_then(|iu| iu.get("url"))
                .and_then(|u| u.as_str())
                .map(|s| s.to_string())
        }
        "image" => {
            // Anthropic: {"type":"image","source":{"type":"base64","media_type":"...","data":"..."}}
            // or {"type":"image","source":{"type":"url","url":"..."}}
            let source = part.get("source")?;
            let src_type = source.get("type").and_then(|t| t.as_str())?;
            match src_type {
                "base64" => {
                    let media_type = source
                        .get("media_type")
                        .and_then(|m| m.as_str())
                        .unwrap_or("image/png");
                    let data = source.get("data").and_then(|d| d.as_str())?;
                    Some(format!("data:{};base64,{}", media_type, data))
                }
                "url" => source.get("url").and_then(|u| u.as_str()).map(|s| s.to_string()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Perform vision handoff: analyze all images and replace with text (spec §9.5).
/// Returns the number of images replaced.
pub async fn perform_vision_handoff(
    payload: &mut Value,
    resolved_model: &str,
    config: &Config,
    upstream: &Upstream,
    key: &str,
    cache: Option<&HandoffCache>,
) -> usize {
    if !catalog::needs_vision_handoff(resolved_model, config) {
        return 0;
    }

    let parts = collect_image_parts(payload);
    if parts.is_empty() {
        return 0;
    }

    // Analyze all images concurrently
    let futures: Vec<_> = parts
        .iter()
        .map(|img| analyze_image(&img.data_uri, key, config, upstream, cache))
        .collect();
    let descriptions = futures::future::join_all(futures).await;

    // Replace each part in place
    let count = parts.len();
    for (i, (img, desc)) in parts.iter().zip(descriptions.iter()).enumerate() {
        let label = if count > 1 {
            format!(
                "[Image {} content — analyzed by vision module, shown as text because the active model cannot see images:]\n{}",
                i + 1,
                desc
            )
        } else {
            format!(
                "[Image content — analyzed by vision module, shown as text because the active model cannot see images:]\n{}",
                desc
            )
        };

        if let Some(target) = navigate_pointer_mut(payload, &img.path) {
            *target = serde_json::json!({
                "type": "text",
                "text": label
            });
        }
    }

    count
}

/// Analyze a single image via the handoff model (spec §9.4).
/// Never panics — on failure returns "[Image analysis failed: ...]".
async fn analyze_image(
    data_uri: &str,
    key: &str,
    config: &Config,
    upstream: &Upstream,
    cache: Option<&HandoffCache>,
) -> String {
    // Compute SHA-256 of the data URI for cache key
    let digest: [u8; 32] = {
        let mut hasher = Sha256::new();
        hasher.update(data_uri.as_bytes());
        hasher.finalize().into()
    };

    // Check cache
    if let Some(cache) = cache {
        if let Some(desc) = cache.get(&digest) {
            return desc.to_string();
        }
    }

    let prompt = if config.vision_handoff_prompt.is_empty() {
        DEFAULT_HANDOFF_PROMPT
    } else {
        &config.vision_handoff_prompt
    };

    let body = serde_json::json!({
        "model": config.vision_handoff_model,
        "stream": false,
        "messages": [
            {"role": "system", "content": prompt},
            {"role": "user", "content": [
                {"type": "text", "text": "Describe this image."},
                {"type": "image_url", "image_url": {"url": data_uri}}
            ]}
        ]
    });
    let body_bytes = serde_json::to_vec(&body).unwrap_or_default().into();

    match upstream.chat_completions(key, body_bytes, false, "none").await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return format!("[Image analysis failed: HTTP {}]", resp.status());
            }
            match read_body(resp).await {
                Ok(body) => {
                    let v: Value = match serde_json::from_slice(&body) {
                        Ok(v) => v,
                        Err(e) => {
                            return format!("[Image analysis failed: parse error: {}]", e);
                        }
                    };
                    let desc = extract_chat_content(&v);
                    // Cache the result
                    if let Some(cache) = cache {
                        cache.set(digest, Arc::from(desc.as_str()));
                    }
                    desc
                }
                Err(e) => {
                    format!("[Image analysis failed: {}]", e)
                }
            }
        }
        Err(e) => {
            format!("[Image analysis failed: {}]", e)
        }
    }
}

/// Extract content from a chat completion response.
fn extract_chat_content(resp: &Value) -> String {
    // string content
    if let Some(content) = resp
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
    {
        if let Some(s) = content.as_str() {
            return s.to_string();
        }
        if let Some(arr) = content.as_array() {
            return arr
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("");
        }
    }
    String::new()
}

/// Navigate to a JSON pointer path mutably.
fn navigate_pointer_mut<'a>(value: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let mut current = value;
    for part in parts {
        if let Ok(idx) = part.parse::<usize>() {
            current = current.as_array_mut()?.get_mut(idx)?;
        } else {
            current = current.as_object_mut()?.get_mut(part)?;
        }
    }
    Some(current)
}
