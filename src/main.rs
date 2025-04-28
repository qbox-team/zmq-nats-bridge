use clap::Parser;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info, debug, warn};
use tokio::signal;

mod config;
mod error;
mod forwarder;
mod logging;
mod topic_mapping;

use config::load_config;
use error::{AppError, Result};
use logging::setup_logging;
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

        let task = tokio::spawn(async move {
            // Pass mapping, shutdown_rx, task_tuning. 
            // forwarder::run will need to create its own mapper internally.
            forwarder::run(mapping, shutdown_rx, task_tuning).await 
        });
        tasks.push(task);
    }

    if tasks.is_empty() {
        warn!("No enabled forward mappings found in configuration. Exiting.");
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
        }
    }

    info!("Sending shutdown signal to tasks...");
    // Send shutdown signal, ignore errors (tasks might have already stopped)
    let _ = shutdown_tx.send(());

    info!("Waiting for tasks to complete...");
    for (i, task) in tasks.into_iter().enumerate() {
        match task.await {
            Ok(Ok(())) => info!("Task {} completed successfully.", i),
            Ok(Err(e)) => error!(task_index = i, error = %e, "Task {} completed with an error.", i),
            Err(e) => error!(task_index = i, error = %e, "Task {} failed (panic or cancellation).", i),
        }
    }

    info!("Shutdown complete.");
    Ok(())
}
