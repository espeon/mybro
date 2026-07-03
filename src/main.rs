// Allow dead code across the crate — many helpers are public API for tests
// or kept for planned features. Re-enable specific lint groups as code matures.
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

mod catalog;
mod config;
mod constants;
mod db;
mod errlog;
mod gate;
mod handoff_cache;
mod keypool;
mod mock_upstream;
mod otel;
mod payload;
mod persist;
mod routes;
mod schema_norm;
mod sessions;
mod stats;
mod upstream;
mod usage;
mod usage_parse;
mod vision;

use std::sync::Arc;
use std::time::Instant;

use config::{Config, ConfigStore};
use gate::Gate;
use handoff_cache::HandoffCache;
use keypool::{KeyEntry, KeyPool};
use routes::AppState;
use sessions::ConvMap;
use upstream::Upstream;

#[tokio::main]
async fn main() {
    // Parse CLI args before doing anything else.
    let cli = parse_cli_args();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("mybro starting up");

    // 1. Load config + env overrides + validate
    let cfg = Config::load();
    let listen_addr = cfg.listen_addr.clone();
    let upstream_base = cfg.upstream_base_url.clone();
    let request_timeout = cfg.request_timeout_duration();
    let override_concurrency = cfg.override_concurrency;

    tracing::info!("listen_addr={}, upstream={}", listen_addr, upstream_base);

    // 2. Init handoff cache
    let handoff_cache = Arc::new(HandoffCache::new(
        constants::HANDOFF_CACHE_SIZE,
        cfg.handoff_cache_ttl_duration(),
    ));

    // 3. Init key pool from config.keys (or single default from API_KEY)
    let key_entries: Vec<KeyEntry> = if cfg.keys.is_empty() {
        if !cfg.api_key.is_empty() {
            vec![KeyEntry::from_config(&config::ConfigKey {
                name: "Default".to_string(),
                key: cfg.api_key.clone(),
                session: String::new(),
            })]
        } else {
            Vec::new()
        }
    } else {
        cfg.keys.iter().map(KeyEntry::from_config).collect()
    };

    let keypool = Arc::new(KeyPool::new(key_entries));
    let total_keys = keypool.total();
    tracing::info!("key pool initialized: {} entries", total_keys);

    // 3.5 Start mock upstream server if requested
    let upstream_base = if let Some(mock_port) = cli.mock_upstream {
        let mock = crate::mock_upstream::MockUpstream::with_options(
            mock_port,
            cli.mock_delay_ms,
            cli.mock_concurrency,
        );
        let url = mock.url();
        let _handle = mock.start().await;
        // Wait a moment for the mock server to bind
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        tracing::info!("mock upstream enabled at {}", url);
        url
    } else {
        upstream_base
    };

    // 4. Init upstream client
    let upstream = Arc::new(Upstream::new(&upstream_base, request_timeout));

    // 5. Init config store
    let config_store = Arc::new(ConfigStore::new(cfg));

    // 6. Init gate with safe default 4 — resized after upstream concurrency fetch
    let gate = Gate::new(None);

    // 7. Init conversation map
    let conv_map = Arc::new(ConvMap::new());

    // 8. Init error log
    let error_log = Arc::new(errlog::ErrorLog::new());

    // 9. Init stats collector + SQLite backend
    let stats_db = Arc::new(crate::db::StatsDB::new());
    let stats = Arc::new(crate::stats::StatsCollector::with_db(stats_db));

    // 10. Init OTel from CLI args
    let otel_config = crate::otel::OtelConfig {
        endpoint: cli.otel_endpoint.clone(),
        service_name: cli.otel_service_name.clone(),
        enabled: cli.otel_enabled && !cli.otel_endpoint.is_empty(),
        protocol: "http".to_string(),
    };
    let otel_providers = if otel_config.enabled {
        let providers = crate::otel::OtelProviders::init(&otel_config);
        tracing::info!("OTel initialized → {}", otel_config.endpoint);
        providers
    } else {
        crate::otel::OtelProviders::disabled()
    };

    // 9. Init debounced saver
    let (saver, _saver_handle) = persist::DebouncedSaver::new(config_store.clone());
    let debounced_saver = Arc::new(saver);

    // 10. Dev proxy detection (CLI overrides env)
    let dev_proxy = cli.dev_proxy.clone().or_else(|| {
        if std::env::var("DEV_PROXY").is_ok() || std::env::var("UMANS_DEV").is_ok() {
            Some(
                std::env::var("DEV_PROXY")
                    .unwrap_or_else(|_| "http://localhost:5173".to_string()),
            )
        } else {
            None
        }
    });
    if dev_proxy.is_some() {
        tracing::info!("dev mode: proxying non-API routes to Vite dev server");
    }

    // 11. Build app state
    let start_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let state = Arc::new(AppState {
        config: config_store,
        upstream: upstream.clone(),
        keypool,
        gate: gate.clone(),
        conv_map,
        handoff_cache,
        error_log,
        debounced_saver,
        stats,
        started_at: Instant::now(),
        start_unix,
        dev_proxy,
    });

    // 12. Validate API key → seed catalog (non-fatal on failure)
    if let Some(key) = state.active_key() {
        if !key.is_empty() {
            tracing::info!("validating API key and seeding catalog...");
            match state.upstream.get_user_info(&key).await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        match crate::upstream::read_body(resp).await {
                            Ok(body) => {
                                if let Ok(map) =
                                    serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&body)
                                {
                                    let cat = catalog::Catalog::from_info(map);
                                    let count = cat.ordered_ids.len();
                                    catalog::set_catalog(cat);
                                    tracing::info!("catalog seeded: {} models", count);
                                }
                            }
                            Err(e) => tracing::warn!("catalog body read error: {}", e),
                        }
                    } else {
                        tracing::warn!("API key validation failed: HTTP {}", resp.status());
                    }
                }
                Err(e) => tracing::warn!("API key validation error: {}", e),
            }
        } else {
            tracing::warn!("no API key configured — catalog will be empty until one is set");
        }
    } else {
        tracing::warn!("no active key — skipping catalog validation");
    }

    // 13. Fetch concurrency → size the gate
    if let Some(key) = state.active_key() {
        if !key.is_empty() {
            tracing::info!("fetching initial concurrency data...");
            if let Some(conc) = crate::usage::fetch_concurrency(&state.upstream, &key, true).await {
                tracing::info!(
                    "concurrency: concurrent={}, limit={:?}, hard_cap={:?}",
                    conc.concurrent,
                    conc.limit,
                    conc.hard_cap
                );
                let eff = crate::usage::get_effective_concurrency(override_concurrency);
                state.gate.reconcile(eff.hard_cap.or(eff.limit).map(|n| n as usize));
            }
        }
    }

    // 14. Spawn background usage refresh task (5-min cadence)
    {
        let state_bg = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(constants::USAGE_CACHE_TTL).await;
                if let Some(key) = state_bg.active_key() {
                    if !key.is_empty() {
                        if let Some(conc) =
                            crate::usage::fetch_concurrency(&state_bg.upstream, &key, false).await
                        {
                            let cfg = state_bg.config.load();
                            let eff =
                                crate::usage::get_effective_concurrency(cfg.override_concurrency);
                            state_bg.gate.reconcile(eff.hard_cap.or(eff.limit).map(|n| n as usize));
                            tracing::debug!(
                                "background concurrency refresh: concurrent={}",
                                conc.concurrent
                            );
                        }
                    }
                }
            }
        });
    }

    // 15. Build router
    let app = routes::build_router(state.clone());

    // 16. Bind TcpListener with port retry
    let listener = bind_with_retry(&listen_addr).await;
    let actual_port = listener
        .local_addr()
        .map(|a| a.port())
        .unwrap_or(8084);
    tracing::info!("listening on 127.0.0.1:{}", actual_port);

    // 17. Graceful shutdown
    let shutdown = async move {
        let ctrl_c = async {
            tokio::signal::ctrl_c().await.expect("ctrl_c");
        };

        #[cfg(unix)]
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => tracing::info!("received SIGINT"),
            _ = terminate => tracing::info!("received SIGTERM"),
        }

        tracing::info!("shutdown signal received, draining...");
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .expect("server error");

    tracing::info!("server stopped");

    // Flush OTel exporters before exit
    otel_providers.shutdown();
}

#[derive(Debug, Clone)]
struct CliArgs {
    otel_endpoint: String,
    otel_service_name: String,
    otel_enabled: bool,
    dev_proxy: Option<String>,
    mock_upstream: Option<u16>,
    mock_delay_ms: usize,
    mock_concurrency: usize,
}

fn parse_cli_args() -> CliArgs {
    let mut args = std::env::args().skip(1).peekable();
    let mut cli = CliArgs {
        otel_endpoint: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_default(),
        otel_service_name: "mybro".to_string(),
        otel_enabled: false,
        dev_proxy: None,
        mock_upstream: None,
        mock_delay_ms: 0,
        mock_concurrency: 8,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--otel-endpoint" => {
                cli.otel_endpoint = args.next().unwrap_or_default();
            }
            "--otel-service-name" => {
                cli.otel_service_name = args.next().unwrap_or_default();
            }
            "--otel-enabled" => {
                cli.otel_enabled = true;
            }
            "--dev-proxy" => {
                cli.dev_proxy = Some(args.next().unwrap_or_else(|| "http://localhost:5173".to_string()));
            }
            "--mock-upstream" => {
                if let Some(p) = args.next().and_then(|s| s.parse::<u16>().ok()) {
                    cli.mock_upstream = Some(p);
                }
            }
            "--mock-delay-ms" => {
                if let Some(n) = args.next().and_then(|s| s.parse::<usize>().ok()) {
                    cli.mock_delay_ms = n;
                }
            }
            "--mock-concurrency" => {
                if let Some(n) = args.next().and_then(|s| s.parse::<usize>().ok()) {
                    cli.mock_concurrency = n;
                }
            }
            "--help" | "-h" => {
                eprintln!("mybro [options]");
                eprintln!("  --otel-endpoint <url>    OTLP HTTP collector endpoint");
                eprintln!("  --otel-enabled           Enable OTel export (requires endpoint)");
                eprintln!("  --otel-service-name <n>  OTel service name (default: umans-proxy)");
                eprintln!("  --dev-proxy [url]        Proxy non-API routes to Vite dev server");
                eprintln!("  --mock-upstream <port>   Start a fake UMANS server on that port for testing");
                eprintln!("  --mock-delay-ms <ms>     Add artificial latency to mock responses");
                eprintln!("  --mock-concurrency <n>   Mock's reported concurrency limit (default 8)");
                std::process::exit(0);
            }
            _ => {}
        }
    }

    cli
}

async fn bind_with_retry(listen_addr: &str) -> tokio::net::TcpListener {
    let mut addr = listen_addr.to_string();
    let mut retries = 0u32;

    loop {
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => return l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                if retries < 3 {
                    tracing::warn!("address {} in use, retrying in 2s...", addr);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    retries += 1;
                } else {
                    // Increment port
                    let parts: Vec<&str> = addr.split(':').collect();
                    if parts.len() == 2 {
                        if let Ok(port) = parts[1].parse::<u16>() {
                            let new_addr = format!("{}:{}", parts[0], port + 1);
                            tracing::warn!("port {} in use, trying {}", port, new_addr);
                            addr = new_addr;
                            retries = 0;
                            continue;
                        }
                    }
                    panic!("failed to bind: {}", e);
                }
            }
            Err(e) => panic!("failed to bind {}: {}", addr, e),
        }
    }
}
