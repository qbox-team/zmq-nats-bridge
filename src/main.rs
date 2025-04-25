use clap::Parser;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info, debug};

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
    
    // Log information about loaded configuration
    info!("Loaded configuration from {}", args.config);
    info!("Starting with {} forward mappings", config.forward_mappings.len());
    
    // Debug log for entire configuration content 
    debug!("Complete configuration content:");
    debug!("-------------------------------------------");
    if let Ok(config_str) = serde_json::to_string_pretty(&config) {
        // Split by newlines and log each line separately for better formatting
        for line in config_str.lines() {
            debug!("{}", line);
        }
    } else {
        debug!("Failed to serialize configuration for debug logging");
    }
    debug!("-------------------------------------------");

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
    info!("Sending shutdown signal to all tasks...");
    let _ = shutdown_tx.send(());

    // Wait for all tasks to complete with a timeout
    info!("Waiting for tasks to complete (timeout: 5s)..." );
    let timeout = tokio::time::sleep(tokio::time::Duration::from_secs(5));
    tokio::select! {
        _ = async {
            for handle in handles {
                if let Err(e) = handle.await {
                    error!("Task join error: {}", e);
                }
            }
        } => {
            info!("All tasks completed gracefully");
        }
        _ = timeout => {
            error!("Timeout waiting for tasks to complete. Forcing exit (some resources may not be cleaned up)." );
            // Force exit if tasks don't complete
            std::process::exit(1);
        }
    }

    info!("Exiting.");
    Ok(())
}
