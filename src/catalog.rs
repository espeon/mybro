use arc_swap::ArcSwapOption;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

// ── Catalog snapshot (spec §8.3) ────────────────────────────────────────────

pub struct Catalog {
    pub info: HashMap<String, Value>,
    pub display: HashMap<String, String>,
    pub ordered_ids: Vec<String>,
}

impl Catalog {
    pub fn empty() -> Arc<Self> {
        Arc::new(Self {
            info: HashMap::new(),
            display: HashMap::new(),
            ordered_ids: Vec::new(),
        })
    }

    /// Build a new immutable snapshot from the raw `/models/info` JSON map.
    pub fn from_info(info_map: Map<String, Value>) -> Arc<Self> {
        let info: HashMap<String, Value> = info_map.into_iter().collect();
        let mut display = HashMap::new();
        let mut ids: Vec<String> = info.keys().cloned().collect();

        for (id, info_obj) in &info {
            let display_name = info_obj
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let display_name = if display_name.to_lowercase().starts_with("umans ") {
                display_name["umans ".len()..].to_string()
            } else {
                display_name
            };
            display.insert(id.clone(), display_name);
        }

        // Sort by lowercase display name, then id
        ids.sort_by(|a, b| {
            let da = display.get(a).map(|s| s.to_lowercase()).unwrap_or_default();
            let db = display.get(b).map(|s| s.to_lowercase()).unwrap_or_default();
            da.cmp(&db).then_with(|| a.cmp(b))
        });

        Arc::new(Self {
            info,
            display,
            ordered_ids: ids,
        })
    }

    /// Effective models: ordered_ids minus disabled (fallback to enabled if empty)
    pub fn effective_models(&self, config: &crate::config::Config) -> Vec<String> {
        if self.ordered_ids.is_empty() {
            return config.enabled_models.clone();
        }
        self.ordered_ids
            .iter()
            .filter(|id| !config.disabled_models.contains(id))
            .cloned()
            .collect()
    }

    /// All catalog models without the disabled filter (same fallback)
    pub fn all_catalog_models(&self, config: &crate::config::Config) -> Vec<String> {
        if self.ordered_ids.is_empty() {
            return config.enabled_models.clone();
        }
        self.ordered_ids.clone()
    }
}

// ── Global catalog storage ──────────────────────────────────────────────────

static CATALOG: LazyLock<ArcSwapOption<Catalog>> = LazyLock::new(|| ArcSwapOption::empty());
static CATALOG_FETCHED_AT: LazyLock<tokio::sync::Mutex<Option<Instant>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

pub fn catalog() -> Option<Arc<Catalog>> {
    CATALOG.load_full()
}

pub fn set_catalog(catalog: Arc<Catalog>) {
    CATALOG.store(Some(catalog));
}

pub async fn ensure_catalog_fresh(upstream: &crate::upstream::Upstream, key: &str) {
    let mu = &*CATALOG_FETCHED_AT;
    let mut guard = mu.lock().await;

    if let Some(t) = *guard {
        if t.elapsed() < crate::constants::CATALOG_CACHE_TTL {
            return;
        }
    }

    match upstream.get_user_info(key).await {
        Ok(resp) => {
            if resp.status().is_success() {
                match crate::upstream::read_body(resp).await {
                    Ok(body) => {
                        if let Ok(map) = serde_json::from_slice::<Map<String, Value>>(&body) {
                            let cat = Catalog::from_info(map);
                            set_catalog(cat);
                            *guard = Some(Instant::now());
                            tracing::info!("catalog fetched: {} models", catalog().map(|c| c.ordered_ids.len()).unwrap_or(0));
                            return;
                        }
                    }
                    Err(e) => tracing::warn!("catalog body read error: {}", e),
                }
            } else {
                tracing::warn!("catalog fetch status: {}", resp.status());
            }
        }
        Err(e) => tracing::warn!("catalog fetch error: {}", e),
    }

    if catalog().is_none() {
        tracing::warn!("catalog fetch failed and no stale snapshot");
    }
}

// ── Derived model capabilities (spec §8.7) ──────────────────────────────────

pub fn reasoning_mode(caps: &Value) -> bool {
    let reasoning = caps.get("reasoning");
    if let Some(r) = reasoning {
        if r.get("supported").and_then(|v| v.as_bool()).unwrap_or(false) {
            return true;
        }
        if r.get("levels")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

pub fn reasoning_variants(caps: &Value) -> Option<Map<String, Value>> {
    let reasoning = caps.get("reasoning")?;
    let supported = reasoning
        .get("supported")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let can_disable = reasoning
        .get("can_disable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !supported || !can_disable {
        return None;
    }

    let levels = reasoning.get("levels")?.as_array()?;
    let mut variants = Map::new();
    for level in levels {
        let level_str = level.as_str().unwrap_or("");
        if level_str == "none" || level_str.is_empty() {
            continue;
        }
        if let Some(&budget) = crate::constants::REASONING_LEVEL_BUDGETS
            .iter()
            .find(|(name, _)| *name == level_str)
            .map(|(_, b)| b)
        {
            variants.insert(
                level_str.to_string(),
                serde_json::json!({
                    "thinking": {
                        "type": "enabled",
                        "budget_tokens": budget
                    }
                }),
            );
        }
    }
    if variants.is_empty() {
        None
    } else {
        Some(variants)
    }
}

// ── Model resolution (spec §9.2) ─────────────────────────────────────────────

pub fn resolve_model_id(requested: &str, config: &crate::config::Config) -> String {
    let cat = match catalog() {
        Some(c) => c,
        None => return requested.to_string(),
    };
    let effective = cat.effective_models(config);

    if requested.starts_with("umans-") {
        return requested.to_string();
    }
    let prefixed = format!("umans-{}", requested);
    if effective.contains(&prefixed) {
        return prefixed;
    }
    if effective.iter().any(|m| m == requested) {
        return requested.to_string();
    }
    requested.to_string()
}

/// Check if a resolved model needs vision handoff (spec §9.1).
pub fn needs_vision_handoff(resolved: &str, config: &crate::config::Config) -> bool {
    if !config.vision_handoff_enabled {
        return false;
    }
    let cat = match catalog() {
        Some(c) => c,
        None => return false,
    };
    cat.info
        .get(resolved)
        .and_then(|m| m.get("capabilities"))
        .and_then(|c| c.get("supports_vision"))
        .and_then(|v| v.as_str())
        == Some("via-handoff")
}
