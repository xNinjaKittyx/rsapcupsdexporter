mod apcaccess;

use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicU64;
use tokio::time::{interval, Duration};

use actix_web::middleware::Compress;
use actix_web::{web, App, HttpResponse, HttpServer, Result};
use log::{debug, info};
use prometheus_client::encoding::text::encode;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::info::Info;
use prometheus_client::registry::Registry;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ApcInfoLabels {
    pub apc: String,
    pub hostname: String,
    pub upsname: String,
    pub version: String,
    pub cable: String,
    pub model: String,
    pub upsmode: String,
    pub driver: String,
    pub apcmodel: String,
}

pub struct AppState {
    pub registry: Registry,
    pub stats: std::collections::BTreeMap<String, String>,
}

pub async fn metrics_handler(state: web::Data<Arc<Mutex<AppState>>>) -> Result<HttpResponse> {
    let state = state.lock().unwrap();
    let mut body = String::new();
    encode(&mut body, &state.registry).unwrap();
    Ok(HttpResponse::Ok()
        .content_type("text/plain; charset=utf-8")
        .body(body))
}

fn update_metrics(state: &mut AppState) {
    // Clear and re-register all metrics
    state.registry = Registry::default();

    // Create APC info metric with labels
    let apc_labels = ApcInfoLabels {
        apc: state.stats.get("APC").cloned().unwrap_or_default(),
        hostname: state.stats.get("HOSTNAME").cloned().unwrap_or_default(),
        upsname: state.stats.get("UPSNAME").cloned().unwrap_or_default(),
        version: state.stats.get("VERSION").cloned().unwrap_or_default(),
        cable: state.stats.get("CABLE").cloned().unwrap_or_default(),
        model: state.stats.get("MODEL").cloned().unwrap_or_default(),
        upsmode: state.stats.get("UPSMODE").cloned().unwrap_or_default(),
        driver: state.stats.get("DRIVER").cloned().unwrap_or_default(),
        apcmodel: state.stats.get("APCMODEL").cloned().unwrap_or_default(),
    };
    let apc_info = Info::new(apc_labels);
    state.registry.register(
        "apcupsd",
        "APC UPS daemon information",
        apc_info,
    );

    // Register all numeric metrics as gauges
    for (key, value) in &state.stats {
        // Skip the tag keys that are already in the info metric
        if matches!(key.as_str(), "APC" | "HOSTNAME" | "UPSNAME" | "VERSION" | "CABLE" | "MODEL" | "UPSMODE" | "DRIVER" | "APCMODEL") {
            continue;
        }

        // Try to parse as f64
        if let Ok(numeric_value) = value.parse::<f64>() {
            let gauge: Gauge<f64, AtomicU64> = Gauge::default();
            gauge.set(numeric_value);
            let metric_name = format!("apcupsd_{}", key.to_lowercase());
            let description = format!("APC UPS {}", key);
            state.registry.register(metric_name, description, gauge);
        }
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {

    env_logger::init();
    let apcupsd_host = std::env::var("APCUPSD_HOST").unwrap_or_else(|_| "localhost".to_string());
    let apcupsd_port: u16 = std::env::var("APCUPSD_PORT")
        .unwrap_or_else(|_| "3551".to_string())
        .parse()
        .unwrap_or(3551);
    let port_bind: u16 = std::env::var("METRICS_PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse()
        .unwrap_or(8080);
    let fetch_interval: u64 = std::env::var("INTERVAL")
        .unwrap_or_else(|_| "10".to_string())
        .parse()
        .unwrap_or(10);
    let timeout: u64 = std::env::var("TIMEOUT")
        .unwrap_or_else(|_| "15".to_string())
        .parse()
        .unwrap_or(15);

    // Initial fetch
    debug!("Fetching initial APC UPS stats from {}:{}", apcupsd_host, apcupsd_port);
    let stats = apcaccess::fetch_stats(&apcupsd_host, apcupsd_port, timeout, true)
        .expect("Failed to fetch initial APC UPS stats");
    debug!("Fetched stats: {:?}", stats);
    info!("Successfully fetched initial APC UPS stats");
    let state = Arc::new(Mutex::new(AppState {
        registry: Registry::default(),
        stats: stats.clone(),
    }));

    // Initialize metrics
    {
        let mut state_guard = state.lock().unwrap();
        update_metrics(&mut state_guard);
    }

    // Spawn background task to fetch stats periodically
    let state_clone = Arc::clone(&state);
    let host_clone = apcupsd_host.clone();

    debug!("Starting background task to fetch APC UPS stats every {} seconds", fetch_interval);
    tokio::spawn(async move {
        let mut interval_timer = interval(Duration::from_secs(fetch_interval));
        loop {
            interval_timer.tick().await;

            match apcaccess::fetch_stats(&host_clone, apcupsd_port, timeout, true) {
                Ok(new_stats) => {
                    let mut state_guard = state_clone.lock().unwrap();
                    state_guard.stats = new_stats;
                    update_metrics(&mut state_guard);
                }
                Err(e) => {
                    eprintln!("Failed to fetch APC UPS stats: {}", e);
                }
            }
        }
    });
    info!("Started background task to fetch APC UPS stats every {} seconds", fetch_interval);

    let state = web::Data::new(state);

    debug!("Starting HTTP server on 0.0.0.0:{}", port_bind);
    HttpServer::new(move || {
        App::new()
            .wrap(Compress::default())
            .app_data(state.clone())
            .service(web::resource("/metrics").route(web::get().to(metrics_handler)))
    })
    .bind(("0.0.0.0", port_bind))?
    .run()
    .await
}
