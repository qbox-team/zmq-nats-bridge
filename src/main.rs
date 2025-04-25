use clap::Parser;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info};

mod config;
mod error;
mod forwarder;
mod logging;
mod topic_mapping;

use config::load_config;
use error::{AppError, Result};
use logging::setup_logging;
use topic_mapping::TopicMapper;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the configuration file
    #[arg(short, long, default_value = "config.yaml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args = Args::parse();

    // Load configuration
    let config = load_config(&args.config)
        .map_err(|e| AppError::Config(e))?;

    // Setup logging
    setup_logging(&config.logging)?;

    // Create a channel for graceful shutdown
    let (shutdown_tx, _) = broadcast::channel(1);

    // Process each forward mapping
    let mut handles = Vec::new();
    for mapping in config.forward_mappings {
        if !mapping.enable {
            info!("Skipping disabled mapping: {}", mapping.name);
            continue;
        }

        info!("Starting forward mapping: {}", mapping.name);
        
        // Create topic mapper from the configuration
        let mapper = Arc::new(TopicMapper::new(&mapping.topic_mapping));
        
        // Each task gets its own shutdown receiver
        let shutdown_rx = shutdown_tx.subscribe();
        
        // Clone mapping for the task
        let task_mapping = mapping.clone();

        // Spawn a task for each mapping
        let handle = tokio::spawn(async move {
            if let Err(e) = forwarder::run(task_mapping, mapper, shutdown_rx).await {
                error!("Error in mapping {}: {}", mapping.name, e);
            }
        });

        handles.push(handle);
    }

    // Wait for Ctrl+C signal
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Ctrl+C received, initiating graceful shutdown...");
        }
    }
    
    // Send shutdown signal to all tasks
    let _ = shutdown_tx.send(());

    // Wait for all tasks to complete
    info!("Waiting for tasks to complete...");
    for handle in handles {
        if let Err(e) = handle.await {
            error!("Task join error: {}", e);
        }
    }

    info!("All tasks completed. Exiting.");
    Ok(())
}
