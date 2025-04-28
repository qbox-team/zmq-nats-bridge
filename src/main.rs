use clap::Parser;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info, debug, warn};
use tokio::signal;

// Prometheus & Axum imports
use prometheus_client::registry::Registry;
use prometheus_client::encoding::text::encode;
use axum::{routing::get, Router};
use std::net::SocketAddr;

mod config;
mod error;
mod forwarder;
mod logging;
mod topic_mapping;
mod metrics; // Added metrics module

use config::load_config;
use error::{AppError, Result};
use logging::setup_logging;
use metrics::ForwarderMetrics; // Import metrics struct
// Remove TopicMapper import if it's no longer created globally
// use topic_mapping::TopicMapper;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the configuration file
    #[arg(short, long, default_value = "config.yaml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Load configuration using the updated function
    let config = load_config(&args.config)
        // Convert ConfigError to AppError::Config(String)
        .map_err(|e| AppError::Config(e.to_string()))?;

    // Setup logging (using config values)
    setup_logging(&config.logging).map_err(|e| AppError::Logging(e.to_string()))?; // Assuming setup_logging returns a specific error type

    info!("Configuration loaded from {}", args.config);
    debug!("Loaded config: {:?}", config);

    // --- Prometheus Setup (Optional) ---
    let mut metrics_registry: Option<Registry> = None;
    let forwarder_metrics: Option<Arc<ForwarderMetrics>> = if config.prometheus.enabled {
        info!("Prometheus exporter enabled.");
        let mut registry = Registry::default();
        let metrics = Arc::new(ForwarderMetrics::register(&mut registry));
        metrics_registry = Some(registry); // Store registry for server
        Some(metrics) // Store Arc<Metrics> to pass to tasks
    } else {
        info!("Prometheus exporter disabled.");
        None
    };

    // --- Start Prometheus Exporter Server (Optional) ---
    let mut maybe_prometheus_server_handle = None;
    if let Some(registry) = metrics_registry { // Only if enabled
        if let Ok(listen_addr) = config.prometheus.listen_address.parse::<SocketAddr>() {
            info!("Starting Prometheus exporter server on {}", listen_addr);
            
            // Use Arc for the registry to share with the handler
            let shared_registry = Arc::new(registry); 

            let app = Router::new().route("/metrics", get(move || metrics_handler(shared_registry)));
            
            // Spawn the server in a separate task
             maybe_prometheus_server_handle = Some(tokio::spawn(async move {
                let listener = match tokio::net::TcpListener::bind(listen_addr).await {
                    Ok(l) => l,
                    Err(e) => {
                         error!(error = %e, addr = %listen_addr, "Failed to bind Prometheus exporter TCP listener");
                         return;
                    }
                };
                if let Err(e) = axum::serve(listener, app).await {
                     error!(error = %e, "Prometheus exporter server failed");
                }
            }));
        } else {
             warn!(addr = %config.prometheus.listen_address, "Invalid Prometheus listen address format. Exporter disabled.");
        }
    }
    
    // Create shutdown channel
    let (shutdown_tx, _) = broadcast::channel::<()>(1); // Underscore indicates rx is unused here

    // Remove global TopicMapper creation
    // let mapper = Arc::new(
    //     TopicMapper::new(&config.topic_mapping_rules) 
    //         .map_err(|e| AppError::TopicMapping(e.to_string()))?, 
    // );

    // Spawn forwarder tasks based on configuration
    let mut tasks = Vec::new();
    // Clone Arc for task spawning loop
    let tuning_config = Arc::new(config.tuning.clone()); 

    for mapping in config.forward_mappings.iter().filter(|m| m.enable) {
        info!("Starting forwarder task for mapping: {}", mapping.name);
        let mapping = mapping.clone(); // Clone mapping for the task
        // Do not clone or pass the global mapper
        // let mapper = Arc::clone(&mapper);
        let shutdown_rx = shutdown_tx.subscribe();
        // Clone Arc<TuningConfig> for each task
        let task_tuning = Arc::clone(&tuning_config); 
        // Clone the Option<Arc<ForwarderMetrics>> for each task
        let task_metrics = forwarder_metrics.clone(); 

        let task = tokio::spawn(async move {
            // Pass mapping, shutdown_rx, task_tuning, task_metrics. 
            // forwarder::run will need to create its own mapper internally.
            forwarder::run(mapping, shutdown_rx, task_tuning, task_metrics).await 
        });
        tasks.push(task);
    }

    if tasks.is_empty() {
        warn!("No enabled forward mappings found in configuration. Exiting.");
        // Should we also shutdown the prometheus server if it started? Yes.
        if let Some(handle) = maybe_prometheus_server_handle {
            info!("Aborting Prometheus server task as no forwarders started.");
            handle.abort();
        }
        return Ok(());
    }

    info!("{} forwarder tasks started. Waiting for shutdown signal (Ctrl+C)...", tasks.len());

    // Wait for shutdown signal (Ctrl+C)
    match signal::ctrl_c().await {
        Ok(()) => {
            info!("Shutdown signal received.");
        }
        Err(err) => {
            error!(error = %err, "Failed to listen for shutdown signal");
            // Still proceed with shutdown attempt
        }
    }
    
    // Abort Prometheus server if it's running
    if let Some(handle) = maybe_prometheus_server_handle {
        info!("Aborting Prometheus server task due to shutdown signal.");
        handle.abort();
    }

    info!("Sending shutdown signal to tasks...");
    // Send shutdown signal, ignore errors (tasks might have already stopped)
    let _ = shutdown_tx.send(());

    info!("Waiting for tasks to complete...");
    for (i, task) in tasks.into_iter().enumerate() {
        match task.await {
            Ok(Ok(())) => info!("Task {} completed successfully.", i),
            Ok(Err(e)) => error!(task_index = i, error = %e, "Task {} completed with an error.", i),
            Err(e) => {
                 // Check if it's an abort error (from Prometheus server abort)
                if e.is_cancelled() {
                    info!("Task {} was cancelled (likely Prometheus server).", i);
                } else {
                    error!(task_index = i, error = %e, "Task {} failed (panic or join error).", i);
                }
            }
        }
    }

    info!("Shutdown complete.");
    Ok(())
}

// Axum handler for /metrics endpoint
async fn metrics_handler(registry: Arc<Registry>) -> String {
    let mut buffer = String::new();
    match encode(&mut buffer, &registry) {
        Ok(_) => buffer,
        Err(e) => {
            error!(error = %e, "Failed to encode Prometheus metrics");
            // Consider returning an internal server error status code here
            // For simplicity, returning an empty string or error message
            format!("# Failed to encode metrics: {}", e)
        }
    }
}
