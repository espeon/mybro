// ── Dashboard serving (spec §24) — adapted for embedded Vite assets ─────────

use crate::routes::AppState;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use std::sync::Arc;

#[derive(RustEmbed)]
#[folder = "uman-frontend/dist/"]
struct FrontendAssets;

/// Serve index.html for `/` and `/dashboard` (SPA entry point).
pub async fn serve_dashboard(
    State(state): State<Arc<AppState>>,
) -> Response {
    // In dev mode, proxy to the Vite dev server
    if let Some(dev_url) = &state.dev_proxy {
        return proxy_to_dev(dev_url, "/").await;
    }

    // Production: serve embedded index.html
    match FrontendAssets::get("index.html") {
        Some(asset) => {
            let mut response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Body::from(asset.data))
                .unwrap();
            // Inject wallpaper style before </head>
            inject_wallpaper_style(&state, &mut response).await;
            response
        }
        None => (StatusCode::NOT_FOUND, "Dashboard not built. Run `pnpm build` in uman-frontend/.").into_response(),
    }
}

/// Serve static assets from the embedded frontend (JS, CSS, images, etc.).
/// Also handles SPA routing by falling back to index.html for non-asset paths.
///
/// Note: we extract the URI path manually from the Request instead of using
/// `Path<String>`, because in a fallback handler `Path<String>` requires the
/// request to have at least one path segment (e.g. it fails on bare `/`).
pub async fn serve_asset(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> Response {
    // Extract the path from the request URI.
    let raw_path = req
        .uri()
        .path()
        .trim_start_matches('/')
        .to_string();

    // In dev mode, proxy to the Vite dev server
    if let Some(dev_url) = &state.dev_proxy {
        return proxy_to_dev(dev_url, &format!("/{}", raw_path)).await;
    }

    let path = raw_path.as_str();
    let path_with_html = format!("{}.html", path);

    // Try exact path → with .html suffix → SPA fallback to index.html
    let asset = FrontendAssets::get(path)
        .or_else(|| FrontendAssets::get(&path_with_html))
        .or_else(|| FrontendAssets::get("index.html"));

    match asset {
        Some(asset) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let is_index = path == "index.html" || path.is_empty();

            let response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(
                    header::CACHE_CONTROL,
                    if is_index {
                        "no-cache, must-revalidate"
                    } else {
                        "public, max-age=31536000, immutable"
                    },
                )
                .body(Body::from(asset.data))
                .unwrap();

            // For index.html, inject wallpaper style too
            if is_index {
                let mut response = response;
                inject_wallpaper_style(&state, &mut response).await;
                return response;
            }

            response
        }
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

/// Inject wallpaper `<style>` before `</head>` (spec §24.2).
async fn inject_wallpaper_style(state: &AppState, response: &mut Response) {
    let cfg = state.config.load();
    let style = match cfg.wallpaper_source {
        crate::config::WallpaperSource::None => {
            "<style>body{background:#0d1117}</style>".to_string()
        }
        crate::config::WallpaperSource::Bing => {
            if let Some(b64) = read_wallpaper_base64(".cache/wallpaper.jpg") {
                format!(
                    "<style>body{{background-image:url(data:image/jpeg;base64,{});background-size:cover;background-position:center}}</style>",
                    b64
                )
            } else {
                "<style>body{background:#0d1117}</style>".to_string()
            }
        }
        crate::config::WallpaperSource::Wallhaven => {
            if let Some(b64) = read_wallpaper_base64(".cache/wallpaper-haven.jpg") {
                format!(
                    "<style>body{{background-image:url(data:image/jpeg;base64,{});background-size:cover;background-position:center}}</style>",
                    b64
                )
            } else {
                "<style>body{background:#0d1117}</style>".to_string()
            }
        }
    };

    // Read the body, inject, and replace
    let body = std::mem::take(response.body_mut());
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .unwrap_or_default();
    let html = String::from_utf8_lossy(&bytes);
    let injected = html.replacen("</head>", &format!("{}\n</head>", style), 1);
    *response.body_mut() = Body::from(injected);
}

fn read_wallpaper_base64(path: &str) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    Some(base64_encode(&data))
}

fn base64_encode(data: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose};
    general_purpose::STANDARD.encode(data)
}

/// Proxy a request to the Vite dev server.
async fn proxy_to_dev(dev_url: &str, path: &str) -> Response {
    let url = format!("{}{}", dev_url.trim_end_matches('/'), path);
    let client = reqwest::Client::new();

    match client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .cloned();
            let body = resp.bytes().await.unwrap_or_default();

            let mut builder = Response::builder().status(status);
            if let Some(ct) = content_type {
                builder = builder.header(header::CONTENT_TYPE, ct);
            }
            builder.body(Body::from(body)).unwrap()
        }
        Err(_) => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(Body::from("Vite dev server not reachable"))
            .unwrap(),
    }
}
