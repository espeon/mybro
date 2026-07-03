use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

// ── Config key entry (as stored in the JSON file) ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigKey {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub session: String,
}

// ── Wallpaper source enum ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperSource {
    None,
    Bing,
    Wallhaven,
}

impl Default for WallpaperSource {
    fn default() -> Self {
        Self::Bing
    }
}

impl std::fmt::Display for WallpaperSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Bing => write!(f, "bing"),
            Self::Wallhaven => write!(f, "wallhaven"),
        }
    }
}

// ── Main config struct ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "LISTEN_ADDR", default = "default_listen_addr")]
    pub listen_addr: String,

    #[serde(rename = "UPSTREAM_BASE_URL", default = "default_upstream")]
    pub upstream_base_url: String,

    #[serde(rename = "REQUEST_TIMEOUT", default = "default_request_timeout")]
    pub request_timeout: String,

    #[serde(rename = "API_KEY", default)]
    pub api_key: String,

    #[serde(rename = "API_KEYS", default)]
    pub api_keys: Vec<String>,

    #[serde(rename = "KEYS", default)]
    pub keys: Vec<ConfigKey>,

    #[serde(rename = "ENABLED_MODELS", default)]
    pub enabled_models: Vec<String>,

    #[serde(rename = "MODEL_DISPLAY_NAMES", default)]
    pub model_display_names: std::collections::HashMap<String, String>,

    #[serde(rename = "OVERRIDE_CONCURRENCY", default)]
    pub override_concurrency: u32,

    #[serde(rename = "MAX_IMAGES", default = "default_max_images")]
    pub max_images: usize,

    #[serde(rename = "DISABLED_MODELS", default)]
    pub disabled_models: Vec<String>,

    #[serde(rename = "VISION_HANDOFF_ENABLED", default)]
    pub vision_handoff_enabled: bool,

    #[serde(rename = "VISION_HANDOFF_MODEL", default = "default_handoff_model")]
    pub vision_handoff_model: String,

    #[serde(rename = "VISION_HANDOFF_PROMPT", default)]
    pub vision_handoff_prompt: String,

    #[serde(rename = "VISION_HANDOFF_CACHE_ENABLED", default)]
    pub vision_handoff_cache_enabled: bool,

    #[serde(rename = "VISION_HANDOFF_CACHE_TTL", default = "default_cache_ttl")]
    pub vision_handoff_cache_ttl: String,

    #[serde(rename = "wallpaper_source", default)]
    pub wallpaper_source: WallpaperSource,
}

fn default_listen_addr() -> String {
    "127.0.0.1:8084".to_string()
}
fn default_upstream() -> String {
    crate::constants::UMANS_API_BASE.to_string()
}
fn default_request_timeout() -> String {
    "15m".to_string()
}
fn default_max_images() -> usize {
    9
}
fn default_handoff_model() -> String {
    "umans-coder".to_string()
}
fn default_cache_ttl() -> String {
    "24h".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            upstream_base_url: default_upstream(),
            request_timeout: default_request_timeout(),
            api_key: String::new(),
            api_keys: Vec::new(),
            keys: Vec::new(),
            enabled_models: Vec::new(),
            model_display_names: Default::default(),
            override_concurrency: 0,
            max_images: default_max_images(),
            disabled_models: Vec::new(),
            vision_handoff_enabled: false,
            vision_handoff_model: default_handoff_model(),
            vision_handoff_prompt: String::new(),
            vision_handoff_cache_enabled: false,
            vision_handoff_cache_ttl: default_cache_ttl(),
            wallpaper_source: WallpaperSource::Bing,
        }
    }
}

impl Config {
    /// Load from `.config/config.json`, apply env overrides, validate.
    pub fn load() -> Self {
        let mut cfg = Self::load_file();
        cfg.apply_env_overrides();
        cfg.validate();
        cfg
    }

    fn load_file() -> Self {
        let path = Path::new(".config/config.json");
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let cfg: Config = serde_json::from_str(&contents).unwrap_or_default();
                tracing::info!("loaded config from {}", path.display());
                cfg
            }
            Err(_) => {
                tracing::info!("no config file at {}, using defaults", path.display());
                Config::default()
            }
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("LISTEN_ADDR") {
            self.listen_addr = v;
        }
        if let Ok(v) = std::env::var("UPSTREAM_BASE_URL") {
            self.upstream_base_url = v;
        }
        if let Ok(v) = std::env::var("REQUEST_TIMEOUT") {
            self.request_timeout = v;
        }
        if let Ok(v) = std::env::var("UMANS_API_KEY") {
            self.api_key = v;
        }
        if let Ok(v) = std::env::var("API_KEYS") {
            self.api_keys = v.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Ok(v) = std::env::var("OVERRIDE_CONCURRENCY") {
            if let Ok(n) = v.parse::<u32>() {
                self.override_concurrency = n;
            }
        }
        if let Ok(v) = std::env::var("MAX_IMAGES") {
            if let Ok(n) = v.parse::<usize>() {
                self.max_images = n;
            }
        }
        if let Ok(v) = std::env::var("VISION_HANDOFF_ENABLED") {
            self.vision_handoff_enabled = !matches!(v.as_str(), "false" | "0" | "");
        }
        if let Ok(v) = std::env::var("VISION_HANDOFF_CACHE_ENABLED") {
            self.vision_handoff_cache_enabled = !matches!(v.as_str(), "false" | "0" | "");
        }
        if let Ok(v) = std::env::var("VISION_HANDOFF_CACHE_TTL") {
            self.vision_handoff_cache_ttl = v;
        }
    }

    fn validate(&self) {
        let dur = parse_duration(&self.request_timeout);
        if dur == Duration::ZERO {
            tracing::error!("REQUEST_TIMEOUT '{}' parsed to zero — invalid", self.request_timeout);
            std::process::exit(1);
        }
    }

    pub fn request_timeout_duration(&self) -> Duration {
        let d = parse_duration(&self.request_timeout);
        if d == Duration::ZERO {
            Duration::from_secs(900) // fallback 15m
        } else {
            d
        }
    }

    pub fn handoff_cache_ttl_duration(&self) -> Duration {
        let d = parse_duration(&self.vision_handoff_cache_ttl);
        if d == Duration::ZERO {
            Duration::from_secs(86_400) // fallback 24h
        } else {
            d
        }
    }

    /// Save to `.config/config.json` (called by the debounced persist task).
    pub fn save_to_file(&self) -> std::io::Result<()> {
        let dir = Path::new(".config");
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let tmp = dir.join("config.json.tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, dir.join("config.json"))?;
        Ok(())
    }
}

// ── ConfigStore: lock-free ArcSwap wrapper ───────────────────────────────────

pub struct ConfigStore(ArcSwap<Config>);

impl ConfigStore {
    pub fn new(cfg: Config) -> Self {
        Self(ArcSwap::from_pointee(cfg))
    }

    pub fn load(&self) -> Arc<Config> {
        self.0.load_full()
    }

    pub fn store(&self, cfg: Config) {
        self.0.store(Arc::new(cfg));
    }

    /// Clone current, apply mutation, store new snapshot, return the new Arc.
    pub fn update<F, R>(&self, f: F) -> (Arc<Config>, R)
    where
        F: FnOnce(&mut Config) -> R,
    {
        let mut new_cfg = (*self.load()).clone();
        let r = f(&mut new_cfg);
        let arc = Arc::new(new_cfg);
        self.0.store(arc.clone());
        (arc, r)
    }
}

// ── parse_duration: ^(\d+)(h|m|s)$ — bare digits → milliseconds ──────────────

pub fn parse_duration(s: &str) -> Duration {
    let s = s.trim();
    if s.is_empty() {
        return Duration::ZERO;
    }
    let bytes = s.as_bytes();
    let last = bytes[bytes.len() - 1];
    let (num_part, mult) = match last {
        b'h' => (&s[..s.len() - 1], 3600),
        b'm' => (&s[..s.len() - 1], 60),
        b's' => (&s[..s.len() - 1], 1),
        _ => {
            // bare digits → milliseconds
            return match s.parse::<u64>() {
                Ok(ms) => Duration::from_millis(ms),
                Err(_) => Duration::ZERO,
            };
        }
    };
    match num_part.parse::<u64>() {
        Ok(n) => Duration::from_secs(n * mult),
        Err(_) => Duration::ZERO,
    }
}

// ── mask_token ────────────────────────────────────────────────────────────────

pub fn mask_token(token: &str) -> String {
    if token.len() <= 14 {
        if token.is_empty() {
            return String::new();
        }
        return "***".to_string();
    }
    format!("{}...{}", &token[..10], &token[token.len() - 4..])
}
