use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub zmq: ZmqConfig,
    #[serde(default)]
    pub nats: NatsConfig,
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

// Helper function to load configuration
pub fn load_config(path: &str) -> Result<Config, config::ConfigError> {
    let settings = config::Config::builder()
        .add_source(config::File::with_name(path).format(config::FileFormat::Toml))
        // Add environment variable overrides if needed
        // .add_source(config::Environment::with_prefix("APP"))
        .build()?;

    settings.try_deserialize()
} 