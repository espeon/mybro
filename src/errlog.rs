// ── Error logging (spec §14) ─────────────────────────────────────────────────

use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

pub struct ErrorLog {
    writer: Mutex<std::io::BufWriter<File>>,
}

impl ErrorLog {
    pub fn new() -> Self {
        let dir = Path::new(".logs");
        let _ = std::fs::create_dir_all(dir);

        let ts = chrono::Local::now()
            .format("%Y-%m-%d-%H-%M-%S")
            .to_string();
        let filename = format!("errors-{}.log", ts);

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(&filename))
            .unwrap_or_else(|e| {
                tracing::warn!("failed to open error log {}: {}", filename, e);
                File::create("/dev/null").unwrap_or_else(|_| {
                    File::create(dir.join("errors-fallback.log")).unwrap()
                })
            });

        Self {
            writer: Mutex::new(std::io::BufWriter::new(file)),
        }
    }

    pub fn log_error(&self, record: &ErrorRecord) {
        let mut w = self.writer.lock();
        let json = serde_json::to_string_pretty(&record).unwrap_or_default();
        let _ = writeln!(w, "--- HTTP ERROR ---");
        let _ = writeln!(w, "{}", json);
        let _ = writeln!(w);
        let _ = w.flush();
    }
}

impl Default for ErrorLog {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ErrorRecord {
    pub timestamp: String,
    #[serde(rename = "errorType")]
    pub error_type: String,
    pub stage: String,
    pub attempt: u32,
    #[serde(rename = "sessNum")]
    pub sess_num: u64,
    #[serde(rename = "slotName")]
    pub slot_name: String,
    pub request: RequestLog,
    pub upstream: Option<UpstreamLog>,
}

#[derive(Debug, serde::Serialize)]
pub struct RequestLog {
    pub method: String,
    pub url: String,
    pub headers: Value,
    pub body: Value,
}

#[derive(Debug, serde::Serialize)]
pub struct UpstreamLog {
    pub url: String,
    pub method: String,
    pub headers: Value,
    pub status: u16,
    #[serde(rename = "statusText")]
    pub status_text: String,
    pub body: Value,
}

// ── Redaction (spec §14.1, §14.2) ────────────────────────────────────────────

/// Redact sensitive headers (spec §14.1).
pub fn redact_headers(headers: &axum::http::HeaderMap) -> Value {
    let mut map = Map::new();
    for (k, v) in headers.iter() {
        let name = k.as_str().to_lowercase();
        let redacted = should_redact_header(&name);
        let value = if redacted {
            "[REDACTED]".to_string()
        } else {
            v.to_str().unwrap_or("[binary]").to_string()
        };
        map.insert(name, Value::String(value));
    }
    Value::Object(map)
}

fn should_redact_header(name: &str) -> bool {
    let exact = ["authorization", "x-api-key", "cookie", "set-cookie", "api-key"];
    if exact.contains(&name) {
        return true;
    }
    let substrings = ["auth", "token", "key", "password", "secret"];
    substrings.iter().any(|s| name.contains(s))
}

/// Redact sensitive values in a JSON body (spec §14.2).
pub fn redact_body_json(body: &Value) -> Value {
    let mut v = body.clone();
    redact_value_in_place(&mut v, false);
    v
}

fn redact_value_in_place(value: &mut Value, in_messages: bool) {
    match value {
        Value::Object(obj) => {
            for (k, v) in obj.iter_mut() {
                let key_lower = k.to_lowercase();
                let sensitive_keys = [
                    "api_key", "apikey", "token", "password", "secret", "authorization",
                ];
                if sensitive_keys.contains(&key_lower.as_str()) {
                    *v = Value::String("[REDACTED]".to_string());
                } else if key_lower == "messages" {
                    redact_value_in_place(v, true);
                } else if in_messages && key_lower == "content" {
                    if let Some(s) = v.as_str() {
                        if s.len() > 2000 {
                            *v = Value::String(format!("{}...[truncated]", &s[..2000]));
                        }
                    }
                } else {
                    redact_value_in_place(v, false);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                redact_value_in_place(item, in_messages);
            }
        }
        _ => {}
    }
}
