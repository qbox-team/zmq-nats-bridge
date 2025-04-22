use serde::Deserialize;
use std::time::Duration;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub zmq: ZmqConfig,
    #[serde(default)]
    pub nats: NatsConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    pub forward_mappings: Vec<Mapping>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ZmqConfig {
    #[serde(with = "humantime_serde", default = "default_heartbeat")]
    pub heartbeat: Duration,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct NatsConfig {
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct LoggingConfig {
    #[serde(default)]
    pub console: ConsoleLogging,
    #[serde(default)]
    pub file: FileLogging,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ConsoleLogging {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_console_level")]
    pub level: String,
    #[serde(default = "default_true")]
    pub colors: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FileLogging {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_file_level")]
    pub level: String,
    #[serde(default = "default_log_path")]
    pub path: PathBuf,
    #[serde(default = "default_true")]
    pub append: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Mapping {
    #[serde(default)]
    pub name: Option<String>,
    pub zmq_endpoints: Vec<String>,
    pub zmq_topics: Vec<String>,
    pub nats_uri: String,
    pub nats_subject: String,
}

// Default heartbeat duration (5 seconds)
fn default_heartbeat() -> Duration {
    Duration::from_secs(5)
}

impl Default for ZmqConfig {
    fn default() -> Self {
        ZmqConfig {
            heartbeat: default_heartbeat(),
        }
    }
}

// Default implementations
impl Default for ConsoleLogging {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "info".to_string(),
            colors: true,
        }
    }
}

impl Default for FileLogging {
    fn default() -> Self {
        Self {
            enabled: false,
            level: "debug".to_string(),
            path: PathBuf::from("logs/zmq-nats-bridge.log"),
            append: true,
        }
    }
}

// Helper functions for defaults
fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_console_level() -> String {
    "info".to_string()
}

fn default_file_level() -> String {
    "debug".to_string()
}

fn default_log_path() -> PathBuf {
    PathBuf::from("logs/zmq-nats-bridge.log")
}

// Helper function to load configuration
pub fn load_config(path: &str) -> Result<Config, config::ConfigError> {
    let settings = config::Config::builder()
        .add_source(config::File::with_name(path).format(config::FileFormat::Toml))
        // Add environment variable overrides if needed
        // .add_source(config::Environment::with_prefix("APP"))
        .build()?;

    settings.try_deserialize()
} 