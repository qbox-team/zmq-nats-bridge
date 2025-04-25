use crate::config::LoggingConfig;
use crate::error::{AppError, Result};
use std::path::PathBuf;
use tracing::Level;
use tracing_subscriber::{
    fmt, 
    layer::SubscriberExt, 
    util::SubscriberInitExt,
    filter::LevelFilter,
    prelude::*,
};

/// Set up logging based on configuration
pub fn setup_logging(config: &LoggingConfig) -> Result<()> {
    let mut layers = Vec::new();

    // Console logging
    if config.console.enabled {
        let level_filter = match config.console.level.parse::<LevelFilter>() {
            Ok(level) => level,
            Err(_) => LevelFilter::INFO, // Default to INFO if parse fails
        };

        let console_layer = fmt::layer()
            .with_target(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(config.console.colors);
        
        // Apply filter separately
        let filtered_layer = console_layer
            .with_filter(level_filter);
        
        layers.push(filtered_layer.boxed());
    }

    // File logging
    if config.file.enabled {
        // Create owned PathBuf to avoid temporary value reference issues
        let file_path = config.file.path.clone();
        let file_dir = file_path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let file_name = file_path.file_name()
            .map(|os| os.to_string_lossy().to_string())
            .unwrap_or_else(|| "app.log".to_string());
        
        // Ensure log directory exists
        if !file_dir.exists() {
            std::fs::create_dir_all(&file_dir)
                .map_err(|e| AppError::Io(e))?;
        }

        let level_filter = match config.file.level.parse::<LevelFilter>() {
            Ok(level) => level,
            Err(_) => LevelFilter::INFO, // Default to INFO if parse fails
        };

        let file_appender = tracing_appender::rolling::daily(&file_dir, file_name);
        let file_layer = fmt::layer()
            .with_target(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(false)
            .with_writer(file_appender);
        
        // Apply filter separately
        let filtered_layer = file_layer
            .with_filter(level_filter);
        
        layers.push(filtered_layer.boxed());
    }

    // Initialize tracing
    tracing_subscriber::registry()
        .with(layers)
        .try_init()
        .map_err(|e| AppError::Forwarder(format!("Failed to initialize logging: {}", e)))?;

    Ok(())
}

/// Helper function to parse log level string to tracing::Level
#[allow(dead_code)]
pub fn parse_log_level(level: &str) -> Level {
    match level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO, // Default to INFO for unrecognized levels
    }
} 