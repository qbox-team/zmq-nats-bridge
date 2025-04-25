use thiserror::Error;
use std::path::PathBuf;
use std::error::Error as StdError;
use std::fmt;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Configuration error: {0}")]
    InvalidConfig(String),
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),

    #[error("ZMQ error: {0}")]
    Zmq(#[from] zeromq::ZmqError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Task join error: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("NATS error: {0}")]
    Nats(#[from] async_nats::error::Error<async_nats::client::PublishErrorKind>),

    #[error("Other error: {0}")]
    Other(#[from] Box<dyn StdError + Send + Sync>),

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