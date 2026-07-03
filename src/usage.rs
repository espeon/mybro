use arc_swap::{ArcSwapOption, ArcSwap};
use serde::Serialize;
use serde_json::Value;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

// ── Usage tracking & effective concurrency (spec §6) ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct UsageData {
    pub requests_in_window: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cached: u64,
    pub window_started_at: String,
    pub plan_display_name: String,
    pub raw: Value,
}

#[derive(Debug, Clone)]
pub struct LastConcurrency {
    pub concurrent: u32,
    pub limit: Option<u32>,
    pub hard_cap: Option<u32>,
    pub user_id: String,
}

#[derive(Debug, Clone)]
pub struct Effective {
    pub hard_cap: Option<u32>,
    pub limit: Option<u32>,
    pub overridden: bool,
}

struct CachedUsage {
    data: Arc<UsageData>,
    fetched_at: Instant,
    window_started_at: String,
}

struct CachedConcurrency {
    data: Arc<LastConcurrency>,
    fetched_at: Instant,
}

static USAGE_CACHE: LazyLock<ArcSwapOption<CachedUsage>> = LazyLock::new(ArcSwapOption::empty);
static CONCURRENCY_CACHE: LazyLock<ArcSwapOption<CachedConcurrency>> = LazyLock::new(ArcSwapOption::empty);
static EFFECTIVE_CACHE: LazyLock<ArcSwapOption<Effective>> = LazyLock::new(ArcSwapOption::empty);

static FETCH_GUARD: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

static THROTTLED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn throttled_count() -> u64 {
    THROTTLED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn bump_throttled() {
    THROTTLED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub fn reset_throttled() {
    THROTTLED.store(0, std::sync::atomic::Ordering::Relaxed);
}

// ── Fetch usage (spec §6.1) ──────────────────────────────────────────────────

pub async fn fetch_usage(
    upstream: &crate::upstream::Upstream,
    key: &str,
    fresh: bool,
) -> Option<Arc<UsageData>> {
    if !fresh {
        if let Some(cached) = USAGE_CACHE.load_full() {
            if cached.fetched_at.elapsed() < crate::constants::USAGE_CACHE_TTL {
                return Some(cached.data.clone());
            }
        }
    }

    let guard = FETCH_GUARD.get_or_init(|| tokio::sync::Mutex::new(()));
    let _g = guard.lock().await;

    if !fresh {
        if let Some(cached) = USAGE_CACHE.load_full() {
            if cached.fetched_at.elapsed() < crate::constants::USAGE_CACHE_TTL {
                return Some(cached.data.clone());
            }
        }
    }

    match upstream.get_usage(key).await {
        Ok(resp) => {
            if !resp.status().is_success() {
                tracing::warn!("usage fetch status: {}", resp.status());
                return USAGE_CACHE.load_full().map(|c| c.data.clone());
            }
            match crate::upstream::read_body(resp).await {
                Ok(body) => {
                    let v: Value = match serde_json::from_slice(&body) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!("usage body parse error: {}", e);
                            return USAGE_CACHE.load_full().map(|c| c.data.clone());
                        }
                    };

                    let window_started_at = v
                        .get("window")
                        .and_then(|w| w.get("started_at"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();

                    let old_window = USAGE_CACHE
                        .load_full()
                        .map(|c| c.window_started_at.clone());
                    if old_window != Some(window_started_at.clone()) {
                        reset_throttled();
                    }

                    let data = Arc::new(UsageData {
                        requests_in_window: v
                            .get("usage")
                            .and_then(|u| u.get("requests_in_window"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        tokens_in: v
                            .get("usage")
                            .and_then(|u| u.get("tokens_in"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        tokens_out: v
                            .get("usage")
                            .and_then(|u| u.get("tokens_out"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        tokens_cached: v
                            .get("usage")
                            .and_then(|u| u.get("tokens_cached"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        window_started_at: window_started_at.clone(),
                        plan_display_name: v
                            .get("plan")
                            .and_then(|p| p.get("display_name"))
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string(),
                        raw: v,
                    });

                    USAGE_CACHE.store(Some(Arc::new(CachedUsage {
                        data: data.clone(),
                        fetched_at: Instant::now(),
                        window_started_at,
                    })));

                    Some(data)
                }
                Err(e) => {
                    tracing::warn!("usage body read error: {}", e);
                    USAGE_CACHE.load_full().map(|c| c.data.clone())
                }
            }
        }
        Err(e) => {
            tracing::warn!("usage fetch error: {}", e);
            USAGE_CACHE.load_full().map(|c| c.data.clone())
        }
    }
}

// ── Fetch concurrency (spec §6.2) ────────────────────────────────────────────

pub async fn fetch_concurrency(
    upstream: &crate::upstream::Upstream,
    key: &str,
    fresh: bool,
) -> Option<Arc<LastConcurrency>> {
    if !fresh {
        if let Some(cached) = CONCURRENCY_CACHE.load_full() {
            if cached.fetched_at.elapsed() < crate::constants::USAGE_CACHE_TTL {
                return Some(cached.data.clone());
            }
        }
    }

    let usage = fetch_usage(upstream, key, fresh).await?;

    let concurrent = usage
        .raw
        .get("usage")
        .and_then(|u| u.get("concurrent_sessions"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let limit = usage
        .raw
        .get("limits")
        .and_then(|l| l.get("concurrency"))
        .and_then(|c| c.get("limit"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    let hard_cap = usage
        .raw
        .get("limits")
        .and_then(|l| l.get("concurrency"))
        .and_then(|c| c.get("hard_cap"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    let user_id = usage
        .raw
        .get("user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let data = Arc::new(LastConcurrency {
        concurrent,
        limit,
        hard_cap,
        user_id,
    });

    CONCURRENCY_CACHE.store(Some(Arc::new(CachedConcurrency {
        data: data.clone(),
        fetched_at: Instant::now(),
    })));

    recompute_effective(data.clone(), 0);

    Some(data)
}

// ── Effective concurrency (spec §6.3) ────────────────────────────────────────

pub fn get_effective_concurrency(config_override: u32) -> Effective {
    if let Some(eff) = EFFECTIVE_CACHE.load_full() {
        return (*eff).clone();
    }
    compute_effective(config_override)
}

fn recompute_effective(conc: Arc<LastConcurrency>, config_override: u32) {
    let eff = compute_effective_from_conc(&conc, config_override);
    EFFECTIVE_CACHE.store(Some(Arc::new(eff)));
}

fn compute_effective(config_override: u32) -> Effective {
    if let Some(conc) = CONCURRENCY_CACHE.load_full() {
        return compute_effective_from_conc(&conc.data, config_override);
    }
    // No upstream data yet: use the safe default of 4.
    Effective {
        hard_cap: None,
        limit: Some(crate::gate::DEFAULT_CONCURRENCY_LIMIT as u32),
        overridden: false,
    }
}

fn compute_effective_from_conc(conc: &LastConcurrency, config_override: u32) -> Effective {
    if config_override > 0 {
        Effective {
            hard_cap: conc
                .hard_cap
                .map(|c| config_override.min(c))
                .or(Some(config_override)),
            limit: Some(config_override),
            overridden: true,
        }
    } else {
        Effective {
            hard_cap: conc.hard_cap,
            limit: Some(
                conc.limit
                    .unwrap_or(crate::gate::DEFAULT_CONCURRENCY_LIMIT as u32),
            ),
            overridden: false,
        }
    }
}

pub fn last_concurrency() -> Option<Arc<LastConcurrency>> {
    CONCURRENCY_CACHE.load_full().map(|c| c.data.clone())
}

#[allow(unused_imports)]
use ArcSwap as _ArcSwap;
