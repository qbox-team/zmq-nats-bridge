use crate::config::LoggingConfig;
use crate::error::{LoggingError, LoggingResult};
use std::path::PathBuf;
use tracing::Level;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{
    fmt::{self, format::FmtSpan, time::UtcTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
    Layer,
};

pub fn init_logging(config: &LoggingConfig) -> LoggingResult<()> {
    let mut layers = Vec::new();

    // Configure console logging if enabled
    if config.console.enabled {
        let console_layer = fmt::layer()
            .with_target(false)
            .with_level(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(config.console.colors)
            .with_span_events(FmtSpan::CLOSE)
            .with_timer(UtcTime::rfc_3339());

        // Apply log level filter
        let level = validate_log_level(&config.console.level)?;
        let filtered_layer = console_layer.with_filter(EnvFilter::new(&level));
        layers.push(filtered_layer.boxed());
    }

    // Configure file logging if enabled
    if config.file.enabled {
        // Ensure log directory exists
        let log_path = PathBuf::from(&config.file.path);
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| LoggingError::LogDirectoryCreation {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        // Create rotation configuration
        let rotation = Rotation::DAILY;

        // Create file appender
        let file_appender = RollingFileAppender::new(
            rotation,
            log_path.parent().unwrap_or(&log_path),
            log_path.file_name().unwrap_or_default(),
        );

        let file_layer = fmt::layer()
            .with_target(true)
            .with_level(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(false)
            .with_timer(UtcTime::rfc_3339())
            .with_writer(file_appender);

        // Apply log level filter
        let level = validate_log_level(&config.file.level)?;
        let filtered_layer = file_layer.with_filter(EnvFilter::new(&level));
        layers.push(filtered_layer.boxed());
    }

    // Initialize the subscriber with all configured layers
    tracing_subscriber::registry()
        .with(layers)
        .try_init()
        .map_err(|e| LoggingError::InitializationError(e.to_string()))?;

    Ok(())
}

/// Validates and normalizes log level string
fn validate_log_level(level: &str) -> LoggingResult<String> {
    let normalized = level.to_lowercase();
    match normalized.as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => Ok(normalized),
        _ => Err(LoggingError::InvalidLogLevel {
            level: level.to_string(),
        }),
    }
}

/// Helper function to convert log level string to tracing Level
pub fn parse_log_level(level: &str) -> Level {
    match level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    }
}

/// Helper macro for structured logging
#[macro_export]
macro_rules! log_with_context {
    ($level:expr, $msg:expr, $($field:tt)*) => {
        tracing::event!(
            $level,
            target: module_path!(),
            $($field)*,
            message = $msg
        );
    };
} 