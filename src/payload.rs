use md5::Digest as _;
use serde_json::Value;

// ── Payload normalization (spec §11) ─────────────────────────────────────────

/// Strip reasoning_content / reasoningContent from assistant messages (§11.1).
pub fn strip_reasoning_content(payload: &mut Value) {
    let Some(messages) = payload.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return;
    };
    for msg in messages.iter_mut() {
        if msg.get("role").and_then(|r| r.as_str()) == Some("assistant") {
            if let Some(obj) = msg.as_object_mut() {
                obj.remove("reasoning_content");
                obj.remove("reasoningContent");
            }
        }
    }
}

/// Rename thinking.budgetTokens → thinking.budget_tokens (§11.2).
pub fn normalize_thinking_payload(payload: &mut Value) {
    let Some(thinking) = payload.get_mut("thinking").and_then(|t| t.as_object_mut()) else {
        return;
    };
    if thinking.contains_key("budgetTokens") && !thinking.contains_key("budget_tokens") {
        if let Some(v) = thinking.remove("budgetTokens") {
            thinking.insert("budget_tokens".to_string(), v);
        }
    }
}

/// Limit image parts in messages to max_images, replacing oldest with text (§11.3).
pub fn limit_images_in_messages(payload: &mut Value, max_images: usize) {
    if max_images == 0 {
        return;
    }

    // Collect all image parts with their positions
    #[derive(Clone)]
    struct ImagePos {
        path: String, // JSON pointer path
        msg_index: i64, // -1 for system
    }

    let mut positions: Vec<ImagePos> = Vec::new();

    // Walk system array
    if let Some(system) = payload.get("system").and_then(|s| s.as_array()) {
        for (i, part) in system.iter().enumerate() {
            if is_image_part(part) {
                positions.push(ImagePos {
                    path: format!("/system/{}", i),
                    msg_index: -1,
                });
            }
        }
    }

    // Walk messages
    if let Some(messages) = payload.get("messages").and_then(|m| m.as_array()) {
        for (mi, msg) in messages.iter().enumerate() {
            if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                for (ci, part) in content.iter().enumerate() {
                    if is_image_part(part) {
                        positions.push(ImagePos {
                            path: format!("/messages/{}/content/{}", mi, ci),
                            msg_index: mi as i64,
                        });
                    }
                }
            }
        }
    }

    let count = positions.len();
    if count <= max_images {
        return;
    }

    // Sort by message index ascending (oldest first), stable
    positions.sort_by_key(|p| p.msg_index);

    // Replace the oldest (count - max) with text
    let to_replace = count - max_images;
    for pos in positions.iter().take(to_replace) {
        // Navigate to the path and replace
        if let Some(target) = navigate_pointer_mut(payload, &pos.path) {
            *target = serde_json::json!({
                "type": "text",
                "text": "(Image previously shared)"
            });
        }
    }
}

fn is_image_part(part: &Value) -> bool {
    part.get("type")
        .and_then(|t| t.as_str())
        .map(|t| t == "image_url" || t == "image")
        .unwrap_or(false)
}

/// Navigate to a JSON pointer path (e.g. "/messages/3/content/1") mutably.
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

/// MD5 of the first user message's text, hex-truncated to 12 chars (§11.4).
pub fn fingerprint_payload(payload: &Value) -> String {
    let Some(messages) = payload.get("messages").and_then(|m| m.as_array()) else {
        return String::new();
    };
    for msg in messages {
        if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
            let text = msg_text(msg);
            if !text.is_empty() {
                let digest = md5::Md5::digest(text.as_bytes());
                let hex = format!("{:x}", digest);
                return hex[..12].to_string();
            }
        }
    }
    String::new()
}

/// Extract text from a message (§11.5).
pub fn msg_text(m: &Value) -> String {
    // String content → itself
    if let Some(s) = m.get("content").and_then(|c| c.as_str()) {
        return s.to_string();
    }
    // Array content → text of first {type: text} part
    if let Some(arr) = m.get("content").and_then(|c| c.as_array()) {
        for part in arr {
            if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    return text.to_string();
                }
            }
        }
    }
    String::new()
}

/// Text of the last user message, with leading [...] bracket prefix stripped (§11.6).
pub fn extract_user_prompt(payload: &Value) -> String {
    let Some(messages) = payload.get("messages").and_then(|m| m.as_array()) else {
        return String::new();
    };
    for msg in messages.iter().rev() {
        if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
            let text = msg_text(msg);
            return strip_bracket_prefix(&text);
        }
    }
    String::new()
}

/// Strip leading `[...]` prefix: equivalent to ^\[[^\]]+\]\s*
fn strip_bracket_prefix(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes[0] != b'[' {
        return s.to_string();
    }
    // Find closing ]
    if let Some(end) = s.find(']') {
        let after = &s[end + 1..];
        // Skip leading whitespace
        return after.trim_start().to_string();
    }
    s.to_string()
}

/// Auto-think: if reasoning.supported && !can_disable, set thinking = adaptive (§11.7).
pub fn apply_auto_think(payload: &mut Value, resolved_model: &str) {
    let Some(cat) = crate::catalog::catalog() else {
        return;
    };
    let Some(info) = cat.info.get(resolved_model) else {
        return;
    };
    let Some(caps) = info.get("capabilities") else {
        return;
    };
    let Some(reasoning) = caps.get("reasoning") else {
        return;
    };

    let supported = reasoning
        .get("supported")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let can_disable = reasoning
        .get("can_disable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if supported && !can_disable {
        payload["thinking"] = serde_json::json!({"type": "adaptive"});
    }
}
