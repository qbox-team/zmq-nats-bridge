mod config;
mod error;
mod forwarder;

use crate::config::load_config;
use crate::error::{AppError, Result};
use clap::Parser;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the configuration file
    #[arg(short, long, default_value = "config.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing subscriber
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    // Parse command-line arguments
    let args = Args::parse();

    tracing::info!("Loading configuration from: {}", args.config);
    let config = load_config(&args.config).map_err(AppError::Config)?;

    tracing::info!("Configuration loaded: {:?}", config);

    let mut tasks = vec![];

    for mapping in config.mappings {
        let mapping_name = mapping.name.as_deref().unwrap_or("unnamed");
        tracing::info!(
            mapping_name,
            zmq_endpoints = ?mapping.zmq_endpoints, 
            nats_subject = %mapping.nats_subject,
            "Setting up forwarder task"
        );
        let nats_config = config.nats.clone(); // Clone NATS config for the task
        let zmq_config = config.zmq.clone();   // Clone ZMQ config for the task
        let current_mapping = mapping.clone(); // Clone mapping for the task

        let task = tokio::spawn(async move {
            // Instantiate and run the forwarder logic from src/forwarder.rs
            if let Err(e) = forwarder::run(current_mapping, nats_config, zmq_config).await {
                // Error logging now happens inside the forwarder::run instrumented span
                // We just return the error here
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
            Ok(Err(e)) => tracing::error!("Forwarder task finished with error: {}", e),
            Err(e) => tracing::error!("Tokio task join error: {}", e),
        }
    }

    tracing::info!("All forwarder tasks finished.");
    Ok(())
}
