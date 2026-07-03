// ── Streaming & piping (spec §18) ────────────────────────────────────────────

use bytes::Bytes;
use futures::StreamExt;

/// Convert a reqwest response into an axum Body by streaming chunks.
/// Backpressure is inherent: Body::from_stream suspends until the client consumes.
pub fn pipe_response(resp: reqwest::Response) -> axum::body::Body {
    let stream = resp.bytes_stream().map(|result| {
        result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    });
    axum::body::Body::from_stream(stream)
}
