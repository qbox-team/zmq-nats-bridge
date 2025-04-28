use thiserror::Error;
use std::path::PathBuf;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Configuration error: {0}")]
    #[allow(dead_code)]
    InvalidConfig(String),
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Logging setup error: {0}")]
    Logging(String),

    #[error("ZMQ error: {0}")]
    Zmq(#[from] zmq::Error),

    #[error("NATS error: {0}")]
    Nats(#[from] async_nats::Error),

    #[error("NATS connection error: {0}")]
    NatsConnection(String),

    #[error("Forwarder error: {0}")]
    Forwarder(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),

    #[error("Shutdown signal received: {0}")]
    Shutdown(String),

    #[error("Task execution error: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

#[derive(Error, Debug)]
pub enum LoggingError {
    #[error("Failed to create log directory at {path}: {source}")]
    #[allow(dead_code)]
    LogDirectoryCreation {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    
    #[error("Invalid log level '{level}'. Must be one of: trace, debug, info, warn, error")]
    #[allow(dead_code)]
    InvalidLogLevel { level: String },
    
    #[error("Failed to initialize logging: {0}")]
    #[allow(dead_code)]
    InitializationError(String),
}

// Define a unified Result type for the application
pub type Result<T, E = AppError> = std::result::Result<T, E>;

#[allow(dead_code)]
pub type LoggingResult<T> = std::result::Result<T, LoggingError>; 