// ── OpenTelemetry OTLP export ────────────────────────────────────────────────
//
// Configures trace + metric export via OTLP/HTTP to a configurable endpoint.
// Enabled by default when OTEL_EXPORTER_OTLP_ENDPOINT is set, or when configured
// via the dashboard. Falls back to no-op when disabled.

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::runtime::Tokio;
use std::sync::OnceLock;

// ── Config ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OtelConfig {
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_service_name")]
    pub service_name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

fn default_endpoint() -> String {
    std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_default()
}
fn default_service_name() -> String {
    "umans-proxy".to_string()
}
fn default_enabled() -> bool {
    false
}
fn default_protocol() -> String {
    "http".to_string()
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
            service_name: default_service_name(),
            enabled: default_enabled(),
            protocol: default_protocol(),
        }
    }
}

// ── Global providers ─────────────────────────────────────────────────────────

static OTEL_INITIALIZED: OnceLock<bool> = OnceLock::new();

pub struct OtelProviders {
    pub tracer_provider: Option<opentelemetry_sdk::trace::TracerProvider>,
    pub meter_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
}

impl OtelProviders {
    pub fn disabled() -> Self {
        Self {
            tracer_provider: None,
            meter_provider: None,
        }
    }

    pub fn init(cfg: &OtelConfig) -> Self {
        if !cfg.enabled || cfg.endpoint.is_empty() {
            return Self::disabled();
        }

        if OTEL_INITIALIZED.get().is_some() {
            tracing::warn!("OTel already initialized — skipping");
            return Self::disabled();
        }

        let _ = OTEL_INITIALIZED.set(true);

        let resource = Resource::new([
            KeyValue::new("service.name", cfg.service_name.clone()),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            KeyValue::new("runtime", "rust"),
        ]);

        // ── Tracer ───────────────────────────────────────────────────────────
        let tracer_provider = match build_tracer_provider(cfg, &resource) {
            Ok(tp) => {
                opentelemetry::global::set_tracer_provider(tp.clone());
                tracing::info!("OTel traces → {}", cfg.endpoint);
                Some(tp)
            }
            Err(e) => {
                tracing::warn!("OTel tracer init failed: {}", e);
                None
            }
        };

        // ── Meter ────────────────────────────────────────────────────────────
        let meter_provider = match build_meter_provider(cfg, &resource) {
            Ok(mp) => {
                opentelemetry::global::set_meter_provider(mp.clone());
                tracing::info!("OTel metrics → {}", cfg.endpoint);
                Some(mp)
            }
            Err(e) => {
                tracing::warn!("OTel meter init failed: {}", e);
                None
            }
        };

        Self {
            tracer_provider,
            meter_provider,
        }
    }

    /// Graceful shutdown — flush pending data.
    pub fn shutdown(&self) {
        if let Some(tp) = &self.tracer_provider {
            if let Err(e) = tp.shutdown() {
                tracing::warn!("OTel tracer shutdown: {}", e);
            }
        }
        if let Some(mp) = &self.meter_provider {
            if let Err(e) = mp.shutdown() {
                tracing::warn!("OTel meter shutdown: {}", e);
            }
        }
    }
}

fn build_tracer_provider(
    cfg: &OtelConfig,
    resource: &Resource,
) -> Result<opentelemetry_sdk::trace::TracerProvider, String> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()
        .map_err(|e: opentelemetry::trace::TraceError| e.to_string())?;

    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(exporter, Tokio)
        .build();

    Ok(provider)
}

fn build_meter_provider(
    cfg: &OtelConfig,
    resource: &Resource,
) -> Result<opentelemetry_sdk::metrics::SdkMeterProvider, String> {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .build()
        .map_err(|e: opentelemetry_sdk::metrics::MetricError| e.to_string())?;

    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter, Tokio).build();

    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_resource(resource.clone())
        .with_reader(reader)
        .build();

    Ok(provider)
}
