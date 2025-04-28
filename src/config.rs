use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::path::PathBuf;
use humantime_serde;
use config::{Config as ConfigCrate, ConfigError, File};

// --- Default value functions for TuningConfig ---
fn default_stats_interval() -> Duration { Duration::from_secs(30) }
fn default_task_retry_delay() -> Duration { Duration::from_secs(5) }
fn default_task_max_retries() -> u32 { 5 }
fn default_channel_buffer_size() -> usize { 10_000 }
fn default_true() -> bool { true }
fn default_false() -> bool { false }
fn default_console_level() -> String { "info".to_string() }
fn default_file_level() -> String { "debug".to_string() }
fn default_log_path() -> PathBuf { PathBuf::from("logs/zmq-nats-bridge.log") }
fn default_max_consecutive_nats_errors() -> u32 { 10 }
fn default_nats_connect_retry_delay() -> Duration { Duration::from_secs(2) }
fn default_prometheus_enabled() -> bool { false }
fn default_prometheus_listen_address() -> String { "0.0.0.0:9090".to_string() }

/// Tuning configuration parameters
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)] // Allows missing fields in TOML to use default values
pub struct TuningConfig {
    /// Size of the buffer for the internal async_channel connecting ZMQ thread to NATS task.
    #[serde(default = "default_channel_buffer_size")]
    pub channel_buffer_size: usize,
    /// Maximum number of consecutive NATS publish errors before the forwarder task triggers a reconnect.
    #[serde(default = "default_max_consecutive_nats_errors")]
    pub max_consecutive_nats_errors: u32,
    /// Interval (in seconds) for reporting forwarder statistics.
    #[serde(with = "humantime_serde", default = "default_stats_interval")]
    pub stats_report_interval_secs: Duration,
    /// Maximum number of times a forwarder task will attempt to restart after an error.
    #[serde(default = "default_task_max_retries")]
    pub task_max_retries: u32,
    /// Delay (in seconds) between forwarder task restart attempts.
    #[serde(with = "humantime_serde", default = "default_task_retry_delay")]
    pub task_retry_delay_secs: Duration,
    /// Delay (in seconds) between NATS connection attempts during initial connection.
    #[serde(with = "humantime_serde", default = "default_nats_connect_retry_delay")]
    pub nats_connect_retry_delay: Duration,
}

// Implement Default for TuningConfig
impl Default for TuningConfig {
    fn default() -> Self {
        TuningConfig {
            channel_buffer_size: default_channel_buffer_size(),
            max_consecutive_nats_errors: default_max_consecutive_nats_errors(),
            stats_report_interval_secs: default_stats_interval(),
            task_max_retries: default_task_max_retries(),
            task_retry_delay_secs: default_task_retry_delay(),
            nats_connect_retry_delay: default_nats_connect_retry_delay(),
        }
    }
}

/// Prometheus exporter configuration.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct PrometheusConfig {
    /// Enable the Prometheus metrics exporter HTTP server.
    #[serde(default = "default_prometheus_enabled")]
    pub enabled: bool,
    /// Listen address (host:port) for the Prometheus exporter.
    #[serde(default = "default_prometheus_listen_address")]
    pub listen_address: String,
}

impl Default for PrometheusConfig {
    fn default() -> Self {
        Self {
            enabled: default_prometheus_enabled(),
            listen_address: default_prometheus_listen_address(),
        }
    }
}

/// Main application configuration structure.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct Config {
    /// List of ZMQ -> NATS forwarding pipelines.
    #[serde(default)]
    pub forward_mappings: Vec<ForwardMapping>,
    /// Logging configuration (console, file).
    #[serde(default)]
    pub logging: LoggingConfig,
    /// Global tuning parameters.
    #[serde(default)]
    pub tuning: TuningConfig,
    /// Prometheus exporter configuration.
    #[serde(default)]
    pub prometheus: PrometheusConfig,
}

/// Defines a single ZMQ -> NATS forwarding pipeline.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ForwardMapping {
    /// Unique name for this mapping (used in logs).
    pub name: String,
    /// Optional description.
    #[serde(default)]
    pub desc: Option<String>,
    /// Whether this mapping is active.
    #[serde(default = "default_true")]
    pub enable: bool,
    /// ZMQ subscriber configuration.
    pub zmq: ZmqConfig,
    /// NATS publisher configuration.
    pub nats: NatsConfig,
    /// Rules for transforming ZMQ topics to NATS subjects.
    #[serde(default)]
    pub topic_mapping: TopicMapping,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ZmqConfig {
    #[serde(default)]
    pub endpoints: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(with = "humantime_serde", default)]
    pub heartbeat: Option<Duration>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct NatsConfig {
    #[serde(default)]
    pub uris: Vec<String>,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct TopicMapping {
    #[serde(default)]
    pub subject_prefix: String,
    #[serde(default)]
    pub topic_transforms: Vec<TopicTransform>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct TopicTransform {
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub replacement: String,
}

/// Logging configuration settings.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct LoggingConfig {
    /// Console (stdout) logging settings.
    #[serde(default)]
    pub console: ConsoleLogging,
    /// File logging settings.
    #[serde(default)]
    pub file: FileLogging,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConsoleLogging {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_console_level")]
    pub level: String,
    #[serde(default = "default_true")]
    pub colors: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
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

// Manually implement Default for Logging sub-structs
impl Default for ConsoleLogging {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            level: default_console_level(),
            colors: default_true(),
        }
    }
}

impl Default for FileLogging {
    fn default() -> Self {
        Self {
            enabled: default_false(),
            level: default_file_level(),
            path: default_log_path(),
            append: default_true(),
        }
    }
}

// --- Config Loading Function (Using config crate) ---
pub fn load_config(path: &str) -> Result<Config, ConfigError> {
    let settings = ConfigCrate::builder()
        .add_source(File::with_name(path))
        .build()?;

    let config: Config = settings.try_deserialize()?;
    validate_config(&config)?;
    Ok(config)
}

// --- Helper Functions ---

/// Performs basic validation checks on the loaded configuration.
fn validate_config(config: &Config) -> Result<(), ConfigError> {
    if config.forward_mappings.is_empty() {
         tracing::warn!("Configuration loaded, but no forward_mappings are defined.");
    }
    for mapping in &config.forward_mappings {
        if mapping.enable {
            if mapping.zmq.endpoints.is_empty() {
                return Err(ConfigError::Message(format!(
                    "Enabled forward_mapping '{}' has no ZMQ endpoints defined.",
                    mapping.name
                )));
            }
            if mapping.nats.uris.is_empty() {
                return Err(ConfigError::Message(format!(
                    "Enabled forward_mapping '{}' has no NATS URIs defined.",
                    mapping.name
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loading() {
        // Create a dummy config file for testing if config.yaml doesn't exist
        // For now, assume config.yaml might exist for a basic check
        let config_path = "config.yaml";
        if std::path::Path::new(config_path).exists() {
             match load_config(config_path) {
                Ok(config) => {
                    // Basic assertion: if mappings exist, the first one should have a name
                    if !config.forward_mappings.is_empty() {
                         assert!(!config.forward_mappings[0].name.is_empty());
                    }
                    // Add more specific tests based on expected config content
                }
                Err(e) => {
                     panic!("Failed to load test config '{}': {}", config_path, e);
                }
             }
        } else {
             println!("Skipping config loading test: '{}' not found.", config_path);
        }
    }
}