// ── Parse cache-hit and cache-creation token counts from upstream responses ──
//
// Both OpenAI and Anthropic report prompt-cache stats in their `usage` blocks.
//   OpenAI:      usage.prompt_tokens_details.cached_tokens
//                usage.prompt_tokens_details.cache_creation_tokens
//   Anthropic:   usage.cache_read_input_tokens
//                usage.cache_creation_input_tokens
//
// For streaming, the `usage` object appears in the final SSE event.
// For non-streaming, it's a top-level field in the JSON.

use bytes::Bytes;
use serde_json::Value;

#[derive(Debug, Default, Clone, Copy)]
pub struct CacheStats {
    /// Tokens served from cache (cache hits)
    pub cached: u64,
    /// Tokens written to cache this request (cache warming)
    pub creation: u64,
}

impl CacheStats {
    pub fn any(&self) -> bool {
        self.cached > 0 || self.creation > 0
    }
}

/// Extract cache stats from an upstream non-streaming response body.
pub fn extract_cache_stats(body: &Bytes) -> CacheStats {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return CacheStats::default(),
    };
    extract_from_value(&v)
}

fn extract_from_value(v: &Value) -> CacheStats {
    let mut stats = CacheStats::default();

    // Anthropic shape: { usage: { cache_read_input_tokens, cache_creation_input_tokens } }
    if let Some(usage) = v.get("usage") {
        if let Some(n) = usage.get("cache_read_input_tokens").and_then(|n| n.as_u64()) {
            stats.cached = n;
        }
        if let Some(n) = usage
            .get("cache_creation_input_tokens")
            .and_then(|n| n.as_u64())
        {
            stats.creation = n;
        }
        if stats.any() {
            return stats;
        }

        // OpenAI shape: { usage: { prompt_tokens_details: { cached_tokens, cache_creation_tokens } } }
        if let Some(details) = usage.get("prompt_tokens_details") {
            if let Some(n) = details.get("cached_tokens").and_then(|n| n.as_u64()) {
                stats.cached = n;
            }
            if let Some(n) = details
                .get("cache_creation_tokens")
                .and_then(|n| n.as_u64())
            {
                stats.creation = n;
            }
        }
    }

    stats
}

/// Extract cache stats from a stream of SSE chunks (the final one will have usage).
pub fn extract_cache_stats_from_sse(chunks: &[Bytes]) -> CacheStats {
    // The last chunk with data is most likely to contain the usage object.
    for chunk in chunks.iter().rev() {
        let s = match std::str::from_utf8(chunk) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for line in s.lines() {
            if let Some(payload) = line.strip_prefix("data: ") {
                let payload = payload.trim();
                if payload.is_empty() || payload == "[DONE]" {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(payload) {
                    // Anthropic streaming: { message: { usage: {...} } }
                    if let Some(usage) = v.get("message").and_then(|m| m.get("usage")) {
                        let mut stats = CacheStats::default();
                        if let Some(n) = usage
                            .get("cache_read_input_tokens")
                            .and_then(|n| n.as_u64())
                        {
                            stats.cached = n;
                        }
                        if let Some(n) = usage
                            .get("cache_creation_input_tokens")
                            .and_then(|n| n.as_u64())
                        {
                            stats.creation = n;
                        }
                        if stats.any() {
                            return stats;
                        }
                    }
                    // OpenAI streaming: { usage: { ... } }
                    let stats = extract_from_value(&v);
                    if stats.any() {
                        return stats;
                    }
                }
            }
        }
    }
    CacheStats::default()
}

/// Extract both prompt and completion token counts from a non-streaming response.
pub fn extract_token_counts(body: &Bytes) -> (u64, u64) {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (0, 0),
    };

    let usage = v.get("usage");

    // OpenAI
    if let Some(u) = usage {
        if let (Some(p), Some(c)) = (
            u.get("prompt_tokens").and_then(|n| n.as_u64()),
            u.get("completion_tokens").and_then(|n| n.as_u64()),
        ) {
            return (p, c);
        }
    }

    // Anthropic
    if let Some(u) = usage {
        if let (Some(i), Some(o)) = (
            u.get("input_tokens").and_then(|n| n.as_u64()),
            u.get("output_tokens").and_then(|n| n.as_u64()),
        ) {
            return (i, o);
        }
    }

    (0, 0)
}