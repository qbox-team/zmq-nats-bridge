use thiserror::Error;
use std::path::PathBuf;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ZMQ error: {0}")]
    Zmq(#[from] zeromq::ZmqError),

    #[error("NATS error: {0}")]
    Nats(#[from] async_nats::Error),

    #[error("Task join error: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("Forwarder error: {0}")]
    Forwarder(String),
}

#[derive(Error, Debug)]
pub enum LoggingError {
    #[error("Failed to create log directory at {path}: {source}")]
    LogDirectoryCreation {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    
    #[error("Invalid log level '{level}'. Must be one of: trace, debug, info, warn, error")]
    InvalidLogLevel { level: String },
    
    #[error("Failed to initialize logging: {0}")]
    InitializationError(String),
}

// Define a type alias for convenience
pub type Result<T> = std::result::Result<T, AppError>;
pub type LoggingResult<T> = std::result::Result<T, LoggingError>; 