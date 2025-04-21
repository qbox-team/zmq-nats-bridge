use thiserror::Error;

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

// Define a type alias for convenience
pub type Result<T> = std::result::Result<T, AppError>; 