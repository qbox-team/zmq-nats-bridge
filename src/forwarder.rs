use crate::config::{Mapping, NatsConfig, ZmqConfig};
use crate::error::Result;
use async_nats::{self, Client, ConnectOptions};
use bytes::Bytes;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};
use zeromq::{Socket, SocketRecv};

const RECONNECT_DELAY: Duration = Duration::from_secs(5);

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
    info!(mapping_name, "Starting forwarder task");

    // --- NATS Connection ---
    let nats_client = connect_nats(&mapping.nats_uri, &nats_config).await?;
    info!(mapping_name, nats_uri = %mapping.nats_uri, "NATS connection established");

    // --- ZMQ Connection (using zeromq crate) ---
    let mut zmq_socket = zeromq::SubSocket::new();

    // Connect to all specified ZMQ endpoints
    for endpoint in &mapping.zmq_endpoints {
        debug!(mapping_name, zmq_endpoint = %endpoint, "Connecting ZMQ");
        zmq_socket.connect(endpoint).await?;
    }

    // Subscribe to all specified topics
    for topic in &mapping.zmq_topics {
        debug!(mapping_name, zmq_topic = %topic, "Subscribing ZMQ");
        zmq_socket.subscribe(topic).await?;
    }

    info!(mapping_name, "ZMQ socket connected and subscribed");

    let heartbeat_timeout_duration = zmq_config.heartbeat + zmq_config.heartbeat;

    // --- Main Forwarding Loop ---
    loop {
        match timeout(heartbeat_timeout_duration, zmq_socket.recv()).await {
            Ok(Ok(zmq_message)) => {
                debug!(mapping_name, "Received ZMQ message");

                if let Some(payload_bytes) = zmq_message.get(1) {
                    let payload: Bytes = payload_bytes.clone();

                    debug!(
                        mapping_name,
                        payload_size = payload.len(),
                        nats_subject = %mapping.nats_subject,
                        "Forwarding message"
                    );

                    if let Err(e) = nats_client
                        .publish(mapping.nats_subject.clone(), payload)
                        .await
                    {
                        error!(mapping_name, error = %e, "Failed to publish message to NATS");
                        warn!(mapping_name, "NATS publish error. Will rely on auto-reconnect.");
                    }
                } else {
                    warn!(
                        mapping_name,
                        num_frames = zmq_message.len(),
                        "Received ZMQ message with unexpected frame count, skipping."
                    );
                }
            }
            Ok(Err(e)) => {
                error!(mapping_name, error = %e, "Error receiving ZMQ message. Forwarder task ending.");
                return Err(e.into());
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
    loop {
        let mut options = ConnectOptions::new();
        if let (Some(user), Some(pass)) = (&config.user, &config.password) {
            options = options.user_and_password(user.clone(), pass.clone());
        }

        match options.connect(uri).await {
            Ok(client) => return Ok(client),
            Err(e) => {
                error!(nats_uri = %uri, error = %e, "Failed to connect to NATS. Retrying in {:?}...", RECONNECT_DELAY);
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        }
    }
} 