// ── Parse token and cache usage from upstream responses ──────────────────────
//
// Both OpenAI and Anthropic report usage in their responses, but with different
// shapes and different conventions for what `input`/`prompt` tokens include:
//
//   OpenAI:     usage.prompt_tokens        (INCLUDES cached tokens)
//               usage.completion_tokens
//               usage.prompt_tokens_details.cached_tokens
//               usage.prompt_tokens_details.cache_creation_tokens
//   Anthropic:  usage.input_tokens         (EXCLUDES cache read + creation)
//               usage.output_tokens
//               usage.cache_read_input_tokens
//               usage.cache_creation_input_tokens
//
// We normalize both into `Usage`, where `tokens_in` is the TOTAL prompt input
// including any cache-read and cache-creation tokens. That makes the cache hit
// rate (`cached / tokens_in`) meaningful and ≤ 1 for both providers.
//
// For non-streaming responses the usage block is top-level JSON. For streaming
// it is spread across SSE events: OpenAI emits a final `usage` chunk (only when
// the request set `stream_options.include_usage`), Anthropic reports input +
// cache in `message_start` and the final `output_tokens` in `message_delta`.

use bytes::Bytes;
use serde_json::Value;

/// Normalized token + cache usage for a single upstream response.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// Total prompt input tokens, including cache read + creation.
    pub tokens_in: u64,
    /// Completion / output tokens.
    pub tokens_out: u64,
    /// Input tokens served from cache (cache hits).
    pub cached: u64,
    /// Input tokens written to cache this request (cache warming).
    pub creation: u64,
}

impl Usage {
    pub fn any(&self) -> bool {
        self.tokens_in > 0 || self.tokens_out > 0 || self.cached > 0 || self.creation > 0
    }

    /// Merge another usage sample, keeping the largest value seen for each field.
    /// Streaming events report different fields at different points (e.g. input in
    /// `message_start`, final output in `message_delta`), and all counts grow
    /// monotonically, so a per-field max reconstructs the complete picture
    /// regardless of event order.
    fn merge(&mut self, other: Usage) {
        self.tokens_in = self.tokens_in.max(other.tokens_in);
        self.tokens_out = self.tokens_out.max(other.tokens_out);
        self.cached = self.cached.max(other.cached);
        self.creation = self.creation.max(other.creation);
    }
}

fn field(u: &Value, key: &str) -> u64 {
    u.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn from_openai_usage(u: &Value) -> Usage {
    let details = u.get("prompt_tokens_details");
    let cached = details.map(|d| field(d, "cached_tokens")).unwrap_or(0);
    let creation = details.map(|d| field(d, "cache_creation_tokens")).unwrap_or(0);
    // OpenAI's prompt_tokens already includes cached tokens, so use it as-is.
    Usage {
        tokens_in: field(u, "prompt_tokens"),
        tokens_out: field(u, "completion_tokens"),
        cached,
        creation,
    }
}

fn from_anthropic_usage(u: &Value) -> Usage {
    let cached = field(u, "cache_read_input_tokens");
    let creation = field(u, "cache_creation_input_tokens");
    // Anthropic's input_tokens excludes cache read/creation; add them for a total.
    Usage {
        tokens_in: field(u, "input_tokens") + cached + creation,
        tokens_out: field(u, "output_tokens"),
        cached,
        creation,
    }
}

/// Extract normalized usage from a parsed JSON value (a full response body or a
/// single SSE event). Handles OpenAI top-level `usage`, Anthropic top-level
/// `usage`, and Anthropic streaming `message.usage` (the `message_start` event).
pub fn extract_usage_from_value(v: &Value) -> Usage {
    // Anthropic streaming `message_start` nests usage under `message`.
    if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
        return from_anthropic_usage(u);
    }
    let Some(u) = v.get("usage") else {
        return Usage::default();
    };
    // OpenAI uses prompt_tokens/completion_tokens; Anthropic uses input/output.
    if u.get("prompt_tokens").is_some() || u.get("completion_tokens").is_some() {
        from_openai_usage(u)
    } else {
        from_anthropic_usage(u)
    }
}

/// Extract usage from a non-streaming (single JSON object) response body.
pub fn extract_usage(body: &Bytes) -> Usage {
    match serde_json::from_slice::<Value>(body) {
        Ok(v) => extract_usage_from_value(&v),
        Err(_) => Usage::default(),
    }
}

/// Incrementally accumulates usage from SSE chunks as they stream past, without
/// buffering the whole response body. Feed each raw chunk to `ingest`, then call
/// `finish` once the stream ends to get the merged usage.
#[derive(Default)]
pub struct SseUsageAccumulator {
    /// Bytes of an SSE line not yet terminated by a newline.
    line_buf: Vec<u8>,
    usage: Usage,
}

impl SseUsageAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a raw chunk. Chunk boundaries may fall anywhere, including mid-line,
    /// so we buffer a partial trailing line until its newline arrives.
    pub fn ingest(&mut self, chunk: &[u8]) {
        self.line_buf.extend_from_slice(chunk);
        while let Some(nl) = self.line_buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.line_buf.drain(..=nl).collect();
            self.process_line(&line);
        }
    }

    fn process_line(&mut self, line: &[u8]) {
        let s = match std::str::from_utf8(line) {
            Ok(s) => s.trim(),
            Err(_) => return,
        };
        let payload = match s.strip_prefix("data:") {
            Some(p) => p.trim(),
            None => return,
        };
        if payload.is_empty() || payload == "[DONE]" {
            return;
        }
        if let Ok(v) = serde_json::from_str::<Value>(payload) {
            self.usage.merge(extract_usage_from_value(&v));
        }
    }

    /// Flush any buffered trailing line and return the merged usage.
    pub fn finish(&mut self) -> Usage {
        if !self.line_buf.is_empty() {
            let rem: Vec<u8> = std::mem::take(&mut self.line_buf);
            self.process_line(&rem);
        }
        self.usage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_non_streaming_includes_cached_in_prompt() {
        let body = Bytes::from(
            r#"{"usage":{"prompt_tokens":100,"completion_tokens":20,
                "prompt_tokens_details":{"cached_tokens":40}}}"#,
        );
        let u = extract_usage(&body);
        // prompt_tokens already includes the 40 cached tokens.
        assert_eq!(u.tokens_in, 100);
        assert_eq!(u.tokens_out, 20);
        assert_eq!(u.cached, 40);
        assert_eq!(u.creation, 0);
        // Hit rate is a sane fraction ≤ 1.
        assert!((u.cached as f64 / u.tokens_in as f64) <= 1.0);
    }

    #[test]
    fn anthropic_non_streaming_adds_cache_to_input() {
        let body = Bytes::from(
            r#"{"usage":{"input_tokens":10,"output_tokens":8,
                "cache_read_input_tokens":5000,"cache_creation_input_tokens":200}}"#,
        );
        let u = extract_usage(&body);
        // input_tokens (10) EXCLUDES cache, so total = 10 + 5000 + 200.
        assert_eq!(u.tokens_in, 5210);
        assert_eq!(u.tokens_out, 8);
        assert_eq!(u.cached, 5000);
        assert_eq!(u.creation, 200);
        // Without normalization this would be 500% — verify it's now ≤ 1.
        assert!((u.cached as f64 / u.tokens_in as f64) <= 1.0);
    }

    #[test]
    fn no_usage_is_zero() {
        let body = Bytes::from(r#"{"choices":[]}"#);
        assert_eq!(extract_usage(&body), Usage::default());
        assert!(!extract_usage(&body).any());
    }

    #[test]
    fn openai_streaming_final_usage_chunk() {
        let mut acc = SseUsageAccumulator::new();
        acc.ingest(b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n");
        acc.ingest(
            b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":30,\"completion_tokens\":12,\
              \"prompt_tokens_details\":{\"cached_tokens\":10}}}\n\n",
        );
        acc.ingest(b"data: [DONE]\n\n");
        let u = acc.finish();
        assert_eq!(u.tokens_in, 30);
        assert_eq!(u.tokens_out, 12);
        assert_eq!(u.cached, 10);
    }

    #[test]
    fn anthropic_streaming_merges_start_and_delta() {
        let mut acc = SseUsageAccumulator::new();
        // message_start carries input + cache, output_tokens is a placeholder 1.
        acc.ingest(
            b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":\
              {\"input_tokens\":100,\"output_tokens\":1,\"cache_read_input_tokens\":40,\
              \"cache_creation_input_tokens\":10}}}\n\n",
        );
        acc.ingest(b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n");
        // message_delta carries the final output_tokens.
        acc.ingest(
            b"event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":250}}\n\n",
        );
        acc.ingest(b"event: message_stop\ndata: {}\n\n");
        let u = acc.finish();
        // tokens_in from message_start: 100 + 40 + 10.
        assert_eq!(u.tokens_in, 150);
        // output_tokens from message_delta wins over the placeholder 1.
        assert_eq!(u.tokens_out, 250);
        assert_eq!(u.cached, 40);
        assert_eq!(u.creation, 10);
    }

    #[test]
    fn accumulator_handles_chunks_split_mid_line() {
        let mut acc = SseUsageAccumulator::new();
        let full = "data: {\"usage\":{\"input_tokens\":10,\"output_tokens\":5,\
                    \"cache_read_input_tokens\":2}}\n\n";
        // Split the SSE event across arbitrary byte boundaries.
        let bytes = full.as_bytes();
        acc.ingest(&bytes[..15]);
        acc.ingest(&bytes[15..40]);
        acc.ingest(&bytes[40..]);
        let u = acc.finish();
        assert_eq!(u.tokens_in, 12);
        assert_eq!(u.tokens_out, 5);
        assert_eq!(u.cached, 2);
    }

    #[test]
    fn accumulator_flushes_final_line_without_trailing_newline() {
        let mut acc = SseUsageAccumulator::new();
        // No terminating newline — finish() must still parse it.
        acc.ingest(b"data: {\"usage\":{\"input_tokens\":7,\"output_tokens\":3}}");
        let u = acc.finish();
        assert_eq!(u.tokens_in, 7);
        assert_eq!(u.tokens_out, 3);
    }
}
