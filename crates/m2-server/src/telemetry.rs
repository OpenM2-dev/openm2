//! Logs, traces and metrics.
//!
//! Logs always go to stderr. Traces and metrics go out through OpenTelemetry
//! (OTLP over HTTP) when `OTEL_EXPORTER_OTLP_ENDPOINT` is set, to a collector
//! next to the server; otherwise the instruments cost next to nothing. Nothing
//! recorded names a ticket, a provider's code or a chat line.

use std::io::IsTerminal as _;
use std::time::Duration;

use anyhow::{Context as _, Result};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Histogram};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::prelude::*;

/// The service name traces and metrics carry.
const SERVICE: &str = "openm2-server";

/// The environment variable that turns export on.
pub const ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// Keeps the exporters running; dropping it sends what is left and stops
/// them. Create it before the tokio runtime and drop it after (the HTTP
/// client it uses runs its own threads).
#[must_use = "dropping it stops exporting"]
pub struct Telemetry {
    tracer: Option<SdkTracerProvider>,
    meter: Option<SdkMeterProvider>,
}

impl Telemetry {
    /// True if traces and metrics are exported.
    pub fn exporting(&self) -> bool {
        self.tracer.is_some()
    }
}

impl Drop for Telemetry {
    fn drop(&mut self) {
        if let Some(tracer) = self.tracer.take() {
            let _ = tracer.shutdown_with_timeout(Duration::from_secs(5));
        }
        if let Some(meter) = self.meter.take() {
            let _ = meter.shutdown_with_timeout(Duration::from_secs(5));
        }
    }
}

/// Sets up logging and, when the endpoint is set, the exporters.
/// `RUST_LOG` picks what is logged (default: info).
pub fn init() -> Result<Telemetry> {
    let filter = std::env::var("RUST_LOG")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| value.parse::<Targets>().ok())
        .unwrap_or_else(|| {
            Targets::new()
                .with_default(LevelFilter::INFO)
                // The exporter's own HTTP client stays quiet, so exporting
                // does not trace itself.
                .with_target("reqwest", LevelFilter::WARN)
                .with_target("hyper", LevelFilter::WARN)
                .with_target("hyper_util", LevelFilter::WARN)
                .with_target("sqlx", LevelFilter::WARN)
        });
    // Colours only on a terminal: container logs stay plain text.
    let logs = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal());
    // An empty value (compose.yaml's default) means no export.
    if std::env::var_os(ENDPOINT_ENV).is_none_or(|value| value.is_empty()) {
        tracing_subscriber::registry()
            .with(logs)
            .with(filter)
            .try_init()
            .context("setting up logging")?;
        return Ok(Telemetry {
            tracer: None,
            meter: None,
        });
    }
    let resource = Resource::builder().with_service_name(SERVICE).build();
    let spans = SpanExporter::builder()
        .with_http()
        .build()
        .context("setting up the trace exporter")?;
    let tracer = SdkTracerProvider::builder()
        .with_batch_exporter(spans)
        .with_resource(resource.clone())
        .build();
    let metrics = MetricExporter::builder()
        .with_http()
        .build()
        .context("setting up the metric exporter")?;
    let meter = SdkMeterProvider::builder()
        .with_periodic_exporter(metrics)
        .with_resource(resource)
        .build();
    opentelemetry::global::set_meter_provider(meter.clone());
    tracing_subscriber::registry()
        .with(logs)
        .with(tracing_opentelemetry::layer().with_tracer(tracer.tracer(SERVICE)))
        .with(filter)
        .try_init()
        .context("setting up logging")?;
    Ok(Telemetry {
        tracer: Some(tracer),
        meter: Some(meter),
    })
}

/// The server's metrics. Without an exporter they record into nothing.
#[derive(Clone)]
pub(crate) struct Metrics {
    connections: Gauge<u64>,
    players: Gauge<u64>,
    entities: Gauge<u64>,
    groups_awake: Gauge<u64>,
    ground_items: Gauge<u64>,
    logins: Counter<u64>,
    sign_ins: Counter<u64>,
    handshakes: Counter<u64>,
    messages_in: Counter<u64>,
    messages_out: Counter<u64>,
    store: Histogram<f64>,
}

impl Metrics {
    pub(crate) fn new() -> Self {
        let meter = opentelemetry::global::meter(SERVICE);
        Self {
            connections: meter
                .u64_gauge("openm2.connections")
                .with_description("Open connections, auth and game")
                .build(),
            players: meter
                .u64_gauge("openm2.players")
                .with_description("Players in game")
                .build(),
            entities: meter
                .u64_gauge("openm2.entities")
                .with_description("Monsters and NPCs in the world")
                .build(),
            groups_awake: meter
                .u64_gauge("openm2.groups.awake")
                .with_description("Groups of monsters with a player near")
                .build(),
            ground_items: meter
                .u64_gauge("openm2.ground_items")
                .with_description("Items lying on the ground")
                .build(),
            logins: meter
                .u64_counter("openm2.logins")
                .with_description(
                    "Sign-ins given a ticket, and sign-ins and tickets refused, by result",
                )
                .build(),
            sign_ins: meter
                .u64_counter("openm2.sign_ins")
                .with_description("Providers' sign-ins checked, by provider and result")
                .build(),
            handshakes: meter
                .u64_counter("openm2.tls.handshakes")
                .with_description("TLS handshakes, by result")
                .build(),
            messages_in: meter
                .u64_counter("openm2.messages.received")
                .with_description("Messages from clients")
                .build(),
            messages_out: meter
                .u64_counter("openm2.messages.sent")
                .with_description("Messages to clients")
                .build(),
            store: meter
                .f64_histogram("openm2.store.duration")
                .with_description("How long storage takes to answer, by request")
                .with_unit("s")
                .build(),
        }
    }

    pub(crate) fn counts(
        &self,
        connections: usize,
        players: usize,
        entities: usize,
        awake: usize,
        ground: usize,
    ) {
        self.connections.record(connections as u64, &[]);
        self.players.record(players as u64, &[]);
        self.entities.record(entities as u64, &[]);
        self.groups_awake.record(awake as u64, &[]);
        self.ground_items.record(ground as u64, &[]);
    }

    pub(crate) fn login(&self, result: &'static str) {
        self.logins.add(1, &[KeyValue::new("result", result)]);
    }

    pub(crate) fn sign_in(
        &self,
        provider: String,
        result: &Result<m2_server_core::Identity, m2_server_core::SignInRefusal>,
    ) {
        let result = match result {
            Ok(_) => "ok",
            Err(m2_server_core::SignInRefusal::Refused) => "refused",
            Err(m2_server_core::SignInRefusal::Unavailable) => "unavailable",
        };
        self.sign_ins.add(
            1,
            &[
                KeyValue::new("provider", provider),
                KeyValue::new("result", result),
            ],
        );
    }

    pub(crate) fn handshake(&self, result: &'static str) {
        self.handshakes.add(1, &[KeyValue::new("result", result)]);
    }

    pub(crate) fn message_in(&self) {
        self.messages_in.add(1, &[]);
    }

    pub(crate) fn message_out(&self) {
        self.messages_out.add(1, &[]);
    }

    pub(crate) fn stored(&self, request: &'static str, seconds: f64) {
        self.store
            .record(seconds, &[KeyValue::new("request", request)]);
    }
}
