use crate::config::{ForwardMapping, NatsConfig};
use crate::error::{AppError, Result};
use crate::topic_mapping::TopicMapper;
use async_nats::{self, Client, ConnectOptions};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{debug, error, info, instrument, warn, trace};
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
        // Check for shutdown signal before attempting to connect
        if shutdown_rx.try_recv().is_ok() {
            info!(mapping_name, "Shutdown signal received before connection attempt");
            return Ok(());
        }

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

    // --- NATS Connection ---
    let nats_uri = mapping.nats.uris.first()
        .ok_or_else(|| AppError::Forwarder("No NATS URI specified".to_string()))?
        .clone();

    info!(mapping_name, nats_uri = %nats_uri, "Attempting NATS connection...");
    let nats_client = connect_nats(&nats_uri, &mapping.nats).await?;
    info!(mapping_name, nats_uri = %nats_uri, "NATS connection successful");

    // --- ZMQ Connection ---
    let mut zmq_socket = SubSocket::new();
    info!(mapping_name, "Creating new ZMQ subscriber socket");

    let endpoint = mapping.zmq.endpoints.first()
        .ok_or_else(|| AppError::Forwarder("No ZMQ endpoint specified".to_string()))?;
    
    info!(mapping_name, zmq_endpoint = %endpoint, "Attempting ZMQ connection...");
    tokio::select! {
        connect_result = zmq_socket.connect(endpoint) => {
            match connect_result {
                Ok(_) => {
                    info!(mapping_name, zmq_endpoint = %endpoint, "ZMQ connection successful");
                }
                Err(e) => {
                    error!(mapping_name, zmq_endpoint = %endpoint, error = %e, "ZMQ connection failed");
                    return Err(AppError::Zmq(e));
                }
            }
        }
        _ = shutdown_rx.recv() => {
            info!(mapping_name, zmq_endpoint = %endpoint, "Shutdown during ZMQ connection");
            return Ok(());
        }
    }

    // Subscribe to all specified topics
    for topic in &mapping.zmq.topics {
        info!(mapping_name, zmq_topic = %topic, "Attempting ZMQ subscription...");
        tokio::select! {
            subscribe_result = zmq_socket.subscribe(topic) => {
                match subscribe_result {
                    Ok(_) => {
                        info!(mapping_name, zmq_topic = %topic, "ZMQ subscription successful");
                    }
                    Err(e) => {
                        error!(mapping_name, zmq_topic = %topic, error = %e, "ZMQ subscription failed");
                        // Decide if one failed subscription should stop the whole forwarder
                        // For now, let's return the error
                        return Err(AppError::Zmq(e)); 
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!(mapping_name, zmq_topic = %topic, "Shutdown during ZMQ subscription");
                return Ok(());
            }
        }
    }

    info!(mapping_name, "ZMQ setup complete. Entering main forward loop...");

    // --- Main Forwarding Loop (Manual Polling Workaround) ---
    // CRITICAL WORKAROUND:
    // We are using manual polling with non-blocking checks for shutdown
    // and a blocking `recv().await` followed by `yield_now()` instead of
    // the more idiomatic `tokio::select!`.
    // REASON: Testing revealed that `zmq_socket.recv().await` does not function
    // correctly when used as a branch within `tokio::select!` in this context.
    // The `select!` macro would wait indefinitely on the `recv()` future
    // without waking up, even when messages were available (confirmed by 
    // diagnostic tests using `recv()` outside of `select!`).
    // This manual loop ensures responsiveness by yielding after each attempt.
    // Avoid reverting to `tokio::select!` for the ZMQ receive branch unless
    // the underlying issue in `zmq.rs` or `tokio` interaction is resolved.
    loop {
        // 1. Check for shutdown signal (non-blocking)
        match shutdown_rx.try_recv() {
            Ok(_) => {
                info!(mapping_name, "Shutdown signal received, exiting forwarder loop");
                return Ok(());
            }
            Err(broadcast::error::TryRecvError::Empty) => {
                // No shutdown signal, continue
            }
            Err(broadcast::error::TryRecvError::Lagged(_)) => {
                warn!(mapping_name, "Shutdown receiver lagged! Exiting to be safe.");
                return Ok(()); // Or Err depending on desired behavior
            }
            Err(broadcast::error::TryRecvError::Closed) => {
                 warn!(mapping_name, "Shutdown channel closed unexpectedly. Exiting.");
                 return Ok(()); // Or Err
            }
        }

        // 2. Try receiving from ZMQ non-blockingly
        match zmq_socket.recv().await { // NOTE: Using blocking recv as try_recv isn't available easily.
                                       // This might impact responsiveness if ZMQ blocks for long.
                                       // Consider revisiting if performance issues arise.
            Ok(zmq_message) => {
                // Message received!
                metrics.messages_received += 1;
                trace!(mapping_name, num_frames = zmq_message.len(), "Received ZMQ message");

                if zmq_message.len() < 2 {
                    warn!(mapping_name, num_frames = zmq_message.len(), "Insufficient frames, skipping.");
                    continue; // Go to next loop iteration
                }

                let topic_frame = zmq_message.get(0).map(|b| b.as_ref()).unwrap_or(&[][..]);
                let payload_frame = zmq_message.get(1).map(|b| b.as_ref()).unwrap_or(&[][..]);
                let topic_str = String::from_utf8_lossy(topic_frame);
                let subject = mapper.map_topic(&topic_str);

                if let Err(e) = nats_client.publish(subject.clone(), payload_frame.to_vec().into()).await {
                    error!(mapping_name, error = %e, nats_subject=%subject, "Failed to publish message to NATS");
                    warn!(mapping_name, "NATS publish error. NATS client will attempt to reconnect automatically.");
                } else {
                    metrics.messages_forwarded += 1;
                    debug!(mapping_name, zmq_topic = %topic_str, nats_subject = %subject, "Message forwarded");
                }
            }
            Err(e) => {
                // Actual ZMQ error
                error!(mapping_name, error = %e, "Error receiving ZMQ message");
                // Consider if specific errors should trigger reconnect vs exit
                return Err(AppError::Zmq(e)); // Exit loop on ZMQ error
            }
        }
        // Yield control after attempting receive/process to allow other tasks to run.
        tokio::task::yield_now().await;
    }
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