mod config;
mod error;
mod forwarder;
mod logging;

use crate::config::load_config;
use crate::error::{AppError, Result};
use clap::Parser;
use logging::init_logging;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the configuration file
    #[arg(short, long, default_value = "config.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let args = Args::parse();

    tracing::info!("Loading configuration from: {}", args.config);
    let config = load_config(&args.config).map_err(AppError::Config)?;

    // Initialize logging based on configuration
    init_logging(&config.logging)
        .map_err(|e| AppError::Forwarder(format!("Failed to initialize logging: {}", e)))?;

    tracing::info!("Configuration loaded: {:?}", config);

    let mut tasks = vec![];

    for mapping in config.forward_mappings {
        let nats_config = config.nats.clone(); // Clone NATS config for the task
        let zmq_config = config.zmq.clone();   // Clone ZMQ config for the task
        let current_mapping = mapping.clone(); // Clone mapping for the task

        let task = tokio::spawn(async move {
            let mapping_name = current_mapping.name.clone().unwrap_or_else(|| "unnamed".to_string());
            tracing::info!(
                mapping_name = %mapping_name,
                zmq_endpoints = ?current_mapping.zmq_endpoints, 
                nats_subject = %current_mapping.nats_subject,
                "Setting up forwarder task"
            );

            if let Err(e) = forwarder::run(current_mapping, nats_config, zmq_config).await {
                tracing::error!(
                    error = %e,
                    mapping_name = %mapping_name,
                    "Forwarder task failed"
                );
                return Err(e);
            }
            Ok(())
        });
        tasks.push(task);
    }

    // Wait for all forwarder tasks to complete
    for task in tasks {
        match task.await {
            Ok(Ok(())) => { /* Task completed successfully */ }
            Ok(Err(e)) => tracing::error!(
                error = %e,
                "Forwarder task finished with error"
            ),
            Err(e) => tracing::error!(
                error = %e,
                "Tokio task join error"
            ),
        }
    }

    tracing::info!("All forwarder tasks finished.");
    Ok(())
}
