use crate::config::{Mapping, NatsConfig, ZmqConfig};
use crate::error::{AppError, Result};
use async_nats::{self, Client, ConnectOptions};
use std::time::{Duration, Instant};
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};
use zeromq::{Socket, SocketRecv};

const RECONNECT_DELAY: Duration = Duration::from_secs(5);
const MAX_RECONNECT_ATTEMPTS: u32 = 5;

// Performance metrics
#[derive(Debug, Default)]
struct Metrics {
    messages_received: u64,
    messages_forwarded: u64,
    errors: u64,
    last_error: Option<String>,
    last_error_time: Option<Instant>,
}

#[instrument(
    name = "forwarder_task",
    skip(mapping, nats_config, zmq_config),
    fields(
        mapping_name = %mapping.name.as_deref().unwrap_or("unnamed"),
        zmq_endpoints = ?mapping.zmq_endpoints,
        nats_subject = %mapping.nats_subject
    )
)]
pub async fn run(
    mapping: Mapping,
    nats_config: NatsConfig,
    zmq_config: ZmqConfig,
) -> Result<()> {
    let mapping_name = mapping.name.as_deref().unwrap_or("unnamed");
    let mut metrics = Metrics::default();
    let mut reconnect_attempts = 0;

    info!(mapping_name, "Starting forwarder task");
    info!(
        mapping_name,
        "Configuration: ZMQ topics: {:?}, NATS subject: {}",
        mapping.zmq_topics,
        mapping.nats_subject
    );

    // Main connection loop with reconnection logic
    loop {
        match run_forwarder_loop(&mapping, &nats_config, &zmq_config, &mut metrics).await {
            Ok(_) => {
                // Normal exit
                info!(mapping_name, "Forwarder task completed normally");
                break;
            }
            Err(e) => {
                metrics.errors += 1;
                metrics.last_error = Some(e.to_string());
                metrics.last_error_time = Some(Instant::now());

                if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                    error!(
                        mapping_name,
                        error = %e,
                        attempts = reconnect_attempts,
                        "Maximum reconnection attempts reached. Exiting."
                    );
                    return Err(e);
                }

                reconnect_attempts += 1;
                warn!(
                    mapping_name,
                    error = %e,
                    attempt = reconnect_attempts,
                    "Connection error. Attempting to reconnect..."
                );
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        }
    }

    Ok(())
}

async fn run_forwarder_loop(
    mapping: &Mapping,
    nats_config: &NatsConfig,
    zmq_config: &ZmqConfig,
    metrics: &mut Metrics,
) -> Result<()> {
    let mapping_name = mapping.name.as_deref().unwrap_or("unnamed");
    let heartbeat_timeout_duration = zmq_config.heartbeat + zmq_config.heartbeat;

    // --- NATS Connection ---
    let nats_client = connect_nats(&mapping.nats_uri, nats_config).await?;
    info!(mapping_name, nats_uri = %mapping.nats_uri, "NATS connection established");

    // --- ZMQ Connection ---
    let mut zmq_socket = zeromq::SubSocket::new();

    // Connect to all specified ZMQ endpoints
    for endpoint in &mapping.zmq_endpoints {
        info!(mapping_name, zmq_endpoint = %endpoint, "Connecting ZMQ");
        zmq_socket.connect(endpoint).await?;
    }

    // Subscribe to all specified topics
    for topic in &mapping.zmq_topics {
        info!(
            mapping_name,
            zmq_topic = %topic,
            "Subscribing to ZMQ topic"
        );
        zmq_socket.subscribe(topic).await?;
    }

    info!(mapping_name, "ZMQ socket connected and subscribed");

    // --- Main Forwarding Loop ---
    loop {
        let start_time = Instant::now();
        
        match timeout(heartbeat_timeout_duration, zmq_socket.recv()).await {
            Ok(Ok(zmq_message)) => {
                metrics.messages_received += 1;
                
                // Only process messages with at least 2 frames (topic + payload)
                if zmq_message.len() < 2 {
                    warn!(
                        mapping_name,
                        num_frames = zmq_message.len(),
                        "Received ZMQ message with insufficient frames, skipping."
                    );
                    continue;
                }

                // Get topic and payload frames
                let topic_frame = zmq_message.get(0).unwrap();
                let payload_frame = zmq_message.get(1).unwrap();

                // Convert topic to string for logging (only if needed)
                let topic_str = if cfg!(debug_assertions) {
                    String::from_utf8_lossy(topic_frame).to_string()
                } else {
                    String::new()
                };

                // Forward message to NATS
                if let Err(e) = nats_client
                    .publish(mapping.nats_subject.clone(), payload_frame.clone())
                    .await
                {
                    error!(mapping_name, error = %e, "Failed to publish message to NATS");
                    warn!(mapping_name, "NATS publish error. Will rely on auto-reconnect.");
                    return Err(AppError::Nats(e));
                }

                metrics.messages_forwarded += 1;

                // Debug logging only in development
                if cfg!(debug_assertions) {
                    debug!(
                        mapping_name,
                        zmq_topic = %topic_str,
                        nats_subject = %mapping.nats_subject,
                        payload_size = payload_frame.len(),
                        processing_time_ms = start_time.elapsed().as_millis(),
                        "Message forwarded"
                    );
                }
            }
            Ok(Err(e)) => {
                error!(mapping_name, error = %e, "Error receiving ZMQ message");
                return Err(AppError::Zmq(e));
            }
            Err(_elapsed) => {
                warn!(
                    mapping_name,
                    timeout_ms = heartbeat_timeout_duration.as_millis(),
                    "ZMQ Heartbeat timeout. No message received. Assuming connection stale."
                );
            }
        }
    }
}

async fn connect_nats(uri: &str, config: &NatsConfig) -> Result<Client> {
    let mut attempts = 0;
    loop {
        let mut options = ConnectOptions::new();
        if let (Some(user), Some(pass)) = (&config.user, &config.password) {
            options = options.user_and_password(user.clone(), pass.clone());
        }

        match options.connect(uri).await {
            Ok(client) => return Ok(client),
            Err(e) => {
                attempts += 1;
                error!(
                    nats_uri = %uri,
                    error = %e,
                    attempt = attempts,
                    "Failed to connect to NATS. Retrying in {:?}...",
                    RECONNECT_DELAY
                );
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        }
    }
} 