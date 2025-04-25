use crate::config::{ForwardMapping, NatsConfig};
use crate::error::{AppError, Result};
use crate::topic_mapping::TopicMapper;
use async_nats::{self, Client, ConnectOptions};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};
use zeromq::{Socket, SocketRecv, SubSocket};

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
    skip(mapping, mapper, shutdown_rx),
    fields(
        mapping_name = %mapping.name,
        zmq_endpoints = ?mapping.zmq.endpoints,
    )
)]
pub async fn run(
    mapping: ForwardMapping,
    mapper: Arc<TopicMapper>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<()> {
    let mapping_name = &mapping.name;
    let mut metrics = Metrics::default();
    let mut reconnect_attempts = 0;

    info!(mapping_name, "Starting forwarder task");
    info!(
        mapping_name,
        "Configuration: ZMQ topics: {:?}",
        mapping.zmq.topics,
    );

    // Main connection loop with reconnection logic
    loop {
        // Clone shutdown_rx for each attempt to connect/run the loop
        let loop_shutdown_rx = shutdown_rx.resubscribe();
        match run_forwarder_loop(&mapping, &mapper, &mut metrics, loop_shutdown_rx).await {
            Ok(_) => {
                // Normal exit (likely due to shutdown signal)
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

                // Wait before reconnecting, but also listen for shutdown signal
                tokio::select! {
                    _ = tokio::time::sleep(RECONNECT_DELAY) => { /* Continue to reconnect */ }
                    _ = shutdown_rx.recv() => {
                        info!(mapping_name, "Shutdown signal received during reconnect delay. Exiting.");
                        return Ok(()); // Exit gracefully
                    }
                }
            }
        }
    }

    Ok(())
}

async fn run_forwarder_loop(
    mapping: &ForwardMapping,
    mapper: &Arc<TopicMapper>,
    metrics: &mut Metrics,
    mut shutdown_rx: broadcast::Receiver<()>, 
) -> Result<()> {
    let mapping_name = &mapping.name;
    // Calculate heartbeat duration if specified, otherwise default to 30s
    let heartbeat_timeout = mapping.zmq.heartbeat.unwrap_or_else(|| Duration::from_secs(30)) * 2;

    // --- NATS Connection ---
    // Use the first URI for initial connection
    let nats_uri = mapping.nats.uris.first()
        .ok_or_else(|| AppError::Forwarder("No NATS URI specified".to_string()))?
        .clone();

    let nats_client = connect_nats(&nats_uri, &mapping.nats).await?;
    info!(mapping_name, nats_uri = %nats_uri, "NATS connection established");

    // --- ZMQ Connection ---
    let mut zmq_socket = SubSocket::new();

    // Connect to all specified ZMQ endpoints
    for endpoint in &mapping.zmq.endpoints {
        info!(mapping_name, zmq_endpoint = %endpoint, "Connecting ZMQ");
        zmq_socket.connect(endpoint).await?;
    }

    // Subscribe to all specified topics
    for topic in &mapping.zmq.topics {
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

        tokio::select! {
            // Bias select to check shutdown first
            biased;

            // Branch 1: Handle Shutdown Signal
            _ = shutdown_rx.recv() => {
                info!(mapping_name, "Shutdown signal received, exiting forwarder loop.");
                break; // Exit the loop gracefully
            }

            // Branch 2: Process ZMQ messages with timeout
            zmq_result = timeout(heartbeat_timeout, zmq_socket.recv()) => {
                match zmq_result {
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

                        // Get topic and payload frames - fixed to handle Option properly
                        let topic_frame = zmq_message.get(0)
                            .map(|b| b.as_ref())
                            .unwrap_or_else(|| &[][..]);
                        let payload_frame = zmq_message.get(1)
                            .map(|b| b.as_ref())
                            .unwrap_or_else(|| &[][..]);

                        // Convert topic to string for mapping (lossy for safety)
                        let topic_str = String::from_utf8_lossy(topic_frame);

                        // Map the topic to NATS subject
                        let subject = mapper.map_topic(&topic_str);

                        // Forward message to NATS
                        if let Err(e) = nats_client.publish(subject.clone(), payload_frame.to_vec().into()).await {
                            error!(mapping_name, error = %e, nats_subject=%subject, "Failed to publish message to NATS");
                            // In most cases, NATS client will handle reconnection internally
                            warn!(mapping_name, "NATS publish error. NATS client will attempt to reconnect automatically.");
                        } else {
                            metrics.messages_forwarded += 1;

                            // Debug logging
                            if cfg!(debug_assertions) {
                                debug!(
                                    mapping_name,
                                    zmq_topic = %topic_str,
                                    nats_subject = %subject,
                                    payload_size = payload_frame.len(),
                                    processing_time_ms = start_time.elapsed().as_millis(),
                                    "Message forwarded"
                                );
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        error!(mapping_name, error = %e, "Error receiving ZMQ message");
                        return Err(AppError::Zmq(e));
                    }
                    Err(_) => {
                        // Timeout occurred (no message received within heartbeat_timeout)
                        warn!(
                            mapping_name,
                            timeout_ms = heartbeat_timeout.as_millis(),
                            "ZMQ Heartbeat timeout. No message received within timeout period."
                        );
                        // Don't return an error here, just log a warning and continue listening
                    }
                }
            }
        }
    }
    Ok(())
}

async fn connect_nats(uri: &str, config: &NatsConfig) -> Result<Client> {
    let mut attempts = 0;
    loop {
        let mut options = ConnectOptions::new();
        // Setup auth if provided
        if !config.user.is_empty() && !config.password.is_empty() {
            options = options.user_and_password(config.user.clone(), config.password.clone());
        }

        match options.connect(uri).await {
            Ok(client) => return Ok(client),
            Err(e) => {
                attempts += 1;
                if attempts >= MAX_RECONNECT_ATTEMPTS {
                    error!(
                        nats_uri = %uri,
                        error = %e,
                        attempts = attempts,
                        "Failed to connect to NATS after maximum attempts."
                    );
                    return Err(AppError::Forwarder(format!("Failed to connect to NATS: {}", e)));
                }

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