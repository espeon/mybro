// ── Streaming & piping (spec §18) ────────────────────────────────────────────────

use bytes::Bytes;
use futures::StreamExt;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Convert a reqwest response into an axum Body by streaming chunks.
/// Backpressure is inherent: Body::from_stream suspends until the client consumes.
///
/// `on_first_chunk` fires once when the first byte/chunk arrives — used to
/// capture TTFT (time-to-first-token) for streaming responses.
///
/// `on_chunk` fires for every chunk as it streams past — used to accumulate
/// token/cache usage from the SSE body without buffering the whole response.
///
/// `on_complete` fires once when the upstream stream finishes — used to
/// record final stats with total duration and accumulated usage.
pub fn pipe_response<F1, F2, F3>(
    resp: reqwest::Response,
    on_first_chunk: F1,
    on_chunk: F3,
    on_complete: F2,
) -> axum::body::Body
where
    F1: FnOnce() + Send + 'static,
    F2: FnOnce() + Send + 'static,
    F3: Fn(&Bytes) + Send + 'static,
{
    let first_arrived = Arc::new(AtomicBool::new(false));
    let first_cb = Arc::new(Mutex::new(Some(on_first_chunk)));
    let complete_cb = Arc::new(Mutex::new(Some(on_complete)));

    let stream = async_stream::stream! {
        let mut byte_stream = resp.bytes_stream();
        while let Some(result) = byte_stream.next().await {
            let bytes: Bytes = result.map_err(|e| std::io::Error::other(e.to_string()))?;
            if !first_arrived.swap(true, Ordering::Relaxed) {
                if let Some(cb) = first_cb.lock().unwrap().take() {
                    cb();
                }
            }
            on_chunk(&bytes);
            yield Ok::<Bytes, std::io::Error>(bytes);
        }
        if let Some(cb) = complete_cb.lock().unwrap().take() {
            cb();
        }
    };

    axum::body::Body::from_stream(stream)
}

/// Non-streaming helper that captures TTFT for the single response chunk.
pub async fn read_body_with_timing<F>(
    resp: reqwest::Response,
    on_first_chunk: F,
) -> Result<Bytes, crate::upstream::UpstreamError>
where
    F: FnOnce() + Send + 'static,
{
    let bytes = crate::upstream::read_body(resp).await?;
    on_first_chunk();
    Ok(bytes)
}