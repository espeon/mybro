// ── Wallpaper proxies (spec §30) ─────────────────────────────────────────────

use crate::routes::AppState;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use std::sync::Arc;

static BING_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static WALLHAVEN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ── GET /api/bg — Bing wallpaper (daily) (spec §30.1) ───────────────────────

pub async fn bing_wallpaper(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let cache_path = ".cache/wallpaper.jpg";
    let _ = std::fs::create_dir_all(".cache");

    // Check if cached file is from today
    if let Ok(metadata) = std::fs::metadata(cache_path) {
        if let Ok(modified) = metadata.modified() {
            let mod_dt: chrono::DateTime<Utc> = modified.into();
            let now = Utc::now();
            if mod_dt.date_naive() == now.date_naive() {
                return serve_image(cache_path, now).await;
            }
        }
    }

    // Single-flight: guard with per-source mutex
    let _guard = BING_LOCK.lock().await;

    // Double-check after acquiring lock
    if let Ok(metadata) = std::fs::metadata(cache_path) {
        if let Ok(modified) = metadata.modified() {
            let mod_dt: chrono::DateTime<Utc> = modified.into();
            let now = Utc::now();
            if mod_dt.date_naive() == now.date_naive() {
                return serve_image(cache_path, now).await;
            }
        }
    }

    // Fetch from peapix
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap();

    match client
        .get("https://peapix.com/bing/feed")
        .send()
        .await
    {
        Ok(resp) => {
            if resp.status().is_success() {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(item) = json.as_array().and_then(|a| a.first()) {
                        let url = item
                            .get("fullUrl")
                            .and_then(|v| v.as_str())
                            .or_else(|| item.get("imageUrl").and_then(|v| v.as_str()))
                            .or_else(|| item.get("url").and_then(|v| v.as_str()));

                        if let Some(url) = url {
                            if let Ok(image_data) = download_image(&client, url).await {
                                let _ = save_atomic(cache_path, &image_data);
                                return serve_image(cache_path, Utc::now()).await;
                            }
                        }
                    }
                }
            }
            // Fallback to stale cache
            serve_stale(cache_path).await
        }
        Err(_) => serve_stale(cache_path).await,
    }
}

// ── GET /api/bg-wallhaven — Wallhaven (hourly) (spec §30.2) ──────────────────

pub async fn wallhaven_wallpaper(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if super::check_auth(&state, &headers, super::ApiFormat::OpenAI).is_err() {
        return super::auth_error_response(super::ApiFormat::OpenAI).into_response();
    }

    let cache_path = ".cache/wallpaper-haven.jpg";
    let _ = std::fs::create_dir_all(".cache");

    // Check if cached file is < 1 hour old
    if let Ok(metadata) = std::fs::metadata(cache_path) {
        if let Ok(modified) = metadata.modified() {
            let age = modified.elapsed().unwrap_or_default();
            if age < std::time::Duration::from_secs(3600) {
                return serve_image(cache_path, Utc::now()).await;
            }
        }
    }

    // Single-flight
    let _guard = WALLHAVEN_LOCK.lock().await;

    // Double-check
    if let Ok(metadata) = std::fs::metadata(cache_path) {
        if let Ok(modified) = metadata.modified() {
            let age = modified.elapsed().unwrap_or_default();
            if age < std::time::Duration::from_secs(3600) {
                return serve_image(cache_path, Utc::now()).await;
            }
        }
    }

    let client = reqwest::Client::builder()
        .user_agent("umans-proxy/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap();

    match client
        .get("https://wallhaven.cc/api/v1/search?categories=100&purity=100&topRange=1M&sorting=toplist&order=desc&page=3")
        .send()
        .await
    {
        Ok(resp) => {
            if resp.status().is_success() {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
                        if !data.is_empty() {
                            // Pick random entry
                            let idx = rand::random::<usize>() % data.len();
                            if let Some(path) = data[idx].get("path").and_then(|p| p.as_str()) {
                                if let Ok(image_data) = download_image(&client, path).await {
                                    let _ = save_atomic(cache_path, &image_data);
                                    return serve_image(cache_path, Utc::now()).await;
                                }
                            }
                        }
                    }
                }
            }
            serve_stale(cache_path).await
        }
        Err(_) => serve_stale(cache_path).await,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn download_image(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}

fn save_atomic(path: &str, data: &[u8]) -> std::io::Result<()> {
    let tmp = format!("{}.tmp", path);
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

async fn serve_image(path: &str, now: chrono::DateTime<Utc>) -> Response {
    match std::fs::read(path) {
        Ok(data) => {
            // Set Expires to end of today (UTC)
            let end_of_day = now
                .date_naive()
                .and_hms_opt(23, 59, 59)
                .unwrap()
                .and_utc();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/jpeg")
                .header(header::CACHE_CONTROL, "public, max-age=86400")
                .header(
                    header::EXPIRES,
                    end_of_day.format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
                )
                .body(Body::from(data))
                .unwrap()
        }
        Err(_) => (StatusCode::NOT_FOUND, "Wallpaper not found").into_response(),
    }
}

async fn serve_stale(path: &str) -> Response {
    match std::fs::read(path) {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/jpeg")
            .body(Body::from(data))
            .unwrap(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Wallpaper fetch failed").into_response(),
    }
}
