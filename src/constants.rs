// ── Constants (spec §23) ────────────────────────────────────────────────────

pub const UMANS_API_BASE: &str = "https://api.code.umans.ai/v1";
pub const API_KEY_ENV_VAR: &str = "UMANS_API_KEY";
pub const MAX_RETRIES: u32 = 10;
pub const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(3);
pub const MAX_QUEUE_SIZE: usize = 256;
pub const MAX_BODY_SIZE: usize = 5 * 1024 * 1024;
pub const CONVERSATION_MAP_MAX: usize = 10_000;
pub const CATALOG_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);
pub const USAGE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);
pub const DEFAULT_KEY_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);
pub const HANDOFF_CACHE_SIZE: usize = 50;
pub const HANDOFF_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(86_400);

// Reasoning-level budgets (§8.7 / §23).  medium == high is intentional.
pub const REASONING_LEVEL_BUDGETS: &[(&str, u32)] = &[
    ("low", 8_000),
    ("medium", 16_000),
    ("high", 16_000),
    ("max", 32_000),
];

/// Built-in default vision-handoff prompt (spec §9.7).
pub const DEFAULT_HANDOFF_PROMPT: &str = r#"You are an image captioning module. Your output is fed verbatim into another model as the sole visual content of the image — it cannot see the image itself, only your text.

Produce a factual, third-person description of the image contents. Do NOT use first person ("I see..."). Do NOT address the reader. Do NOT speculate about what the user wants.

Cover:
- Type of image (screenshot, photograph, diagram, UI, log, etc.) and overall layout
- All visible elements (objects, UI widgets, people, regions) and their spatial arrangement
- Exact transcription of any visible text, code, or labels (use quotes)
- Salient technical details (file paths, error messages, colors, dimensions, filenames)

Write as a single coherent description, not a bulleted list. Be thorough but concise."#;
