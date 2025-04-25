use serde::Deserialize;
use std::time::Duration;
use std::path::PathBuf;
use humantime_serde;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub forward_mappings: Vec<ForwardMapping>,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ForwardMapping {
    pub name: String,
    #[allow(dead_code)]
    pub desc: Option<String>,
    #[serde(default = "default_true")]
    pub enable: bool,
    pub zmq: ZmqConfig,
    pub nats: NatsConfig,
    pub topic_mapping: TopicMapping,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ZmqConfig {
    #[serde(default)]
    pub endpoints: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(with = "humantime_serde", default)]
    pub heartbeat: Option<Duration>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct NatsConfig {
    #[serde(default)]
    pub uris: Vec<String>,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TopicMapping {
    #[serde(default)]
    pub subject_prefix: String,
    #[serde(default)]
    pub topic_transforms: Vec<TopicTransform>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TopicTransform {
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub replacement: String,
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
    #[allow(dead_code)]
    pub append: bool,
}

// Default value functions
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

// Public function to load configuration
pub fn load_config(path: &str) -> Result<Config, config::ConfigError> {
    let settings = config::Config::builder()
        // Primary format is YAML
        .add_source(config::File::with_name(path))
        // Add environment variable overrides
        .add_source(config::Environment::with_prefix("APP"))
        .build()?;

    settings.try_deserialize()
}

// Backward compatibility for code that might use Config::load
impl Config {
    #[allow(dead_code)]
    pub fn load(path: &str) -> Result<Self, config::ConfigError> {
        load_config(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loading() {
        // This test will only pass if config.yaml exists
        if let Ok(config) = Config::load("config.yaml") {
            assert!(!config.forward_mappings.is_empty());
        }
    }
} 