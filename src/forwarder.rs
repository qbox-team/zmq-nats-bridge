use crate::config::{ForwardMapping, NatsConfig};
use crate::error::{AppError, Result};
use crate::topic_mapping::TopicMapper;
use async_nats::{self, Client, ConnectOptions};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn, trace};
use zeromq::{Socket, SocketRecv, SubSocket};

// Removed constants, now passed as arguments
// const RECONNECT_DELAY: Duration = Duration::from_secs(5);
// const MAX_RECONNECT_ATTEMPTS: u32 = 5;
// const STATS_REPORT_INTERVAL: Duration = Duration::from_secs(60);

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
    skip(mapping, mapper, shutdown_rx, stats_interval, task_retry_delay, task_max_retries),
    fields(
        mapping_name = %mapping.name,
    )
)]
pub async fn run(
    mapping: ForwardMapping,
    mapper: Arc<TopicMapper>,
    mut shutdown_rx: broadcast::Receiver<()>, 
    stats_interval: Duration,
    task_retry_delay: Duration,
    task_max_retries: u32,
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
        match run_forwarder_loop(
            &mapping, 
            &mapper, 
            &mut metrics, 
            loop_shutdown_rx, 
            stats_interval, 
            task_max_retries // Pass down max retries for NATS connect
        ).await {
            Ok(_) => {
                info!(mapping_name, "Forwarder task completed normally");
                break;
            }
            Err(e) => {
                metrics.errors += 1;
                metrics.last_error = Some(e.to_string());
                metrics.last_error_time = Some(Instant::now());

                // Use configured max retries
                if reconnect_attempts >= task_max_retries {
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

                // Use configured retry delay
                tokio::select! {
                    _ = tokio::time::sleep(task_retry_delay) => { /* Continue */ }
                    _ = shutdown_rx.recv() => {
                        info!(mapping_name, "Shutdown signal received during reconnect delay. Exiting.");
                        return Ok(());
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
    stats_report_interval: Duration, // Receive stats interval
    max_connect_retries: u32,      // Receive max retries (for NATS connect)
) -> Result<()> {
    let mapping_name = &mapping.name;
    let heartbeat_timeout = mapping.zmq.heartbeat.unwrap_or_else(|| Duration::from_secs(30)) * 2;

    // --- NATS Connection ---
    let nats_uri = mapping.nats.uris.first()
        .ok_or_else(|| AppError::Forwarder("No NATS URI specified".to_string()))?
        .clone();

    info!(mapping_name, nats_uri = %nats_uri, "Attempting NATS connection...");
    // Use max_connect_retries for NATS connection attempts
    let nats_client = connect_nats(&nats_uri, &mapping.nats, max_connect_retries).await?;
    info!(mapping_name, nats_uri = %nats_uri, "NATS connection successful");

    // --- ZMQ Connection ---
    let mut zmq_socket = SubSocket::new();
    info!(mapping_name, "Creating new ZMQ subscriber socket");

    if mapping.zmq.endpoints.is_empty() {
        error!(mapping_name, "No ZMQ endpoints specified in configuration.");
        return Err(AppError::Forwarder("No ZMQ endpoint specified".to_string()));
    }

    let mut successful_zmq_connections = 0;
    for endpoint in &mapping.zmq.endpoints {
        info!(mapping_name, zmq_endpoint = %endpoint, "Attempting ZMQ connection...");
        // We need to await the connect future
        let connect_result = zmq_socket.connect(endpoint).await;

        match connect_result {
            Ok(_) => {
                info!(mapping_name, zmq_endpoint = %endpoint, "ZMQ connection successful");
                successful_zmq_connections += 1;
            }
            Err(e) => {
                // Log error but continue trying other endpoints
                warn!(mapping_name, zmq_endpoint = %endpoint, error = %e, "ZMQ connection failed");
            }
        }
        // Allow checking for shutdown between connection attempts
        if shutdown_rx.try_recv().is_ok() {
            info!(mapping_name, "Shutdown signal received during ZMQ connection attempts");
            return Ok(());
        }
    }

    if successful_zmq_connections == 0 {
        error!(mapping_name, "Failed to connect to any of the specified ZMQ endpoints.");
        // Returning a specific ZMQ error might be better if we had the last error,
        // but a general Forwarder error is okay here.
        return Err(AppError::Forwarder(
            "Failed to connect to any ZMQ endpoint".to_string(),
        ));
    }
    info!(mapping_name, count = successful_zmq_connections, "Completed ZMQ connection attempts.");

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
    let mut last_report_time = Instant::now();
    loop {
        tokio::select! {
            // Branch 1: Handle Shutdown Signal
            _ = shutdown_rx.recv() => {
                info!(mapping_name, "Shutdown signal received, exiting forwarder loop");
                // Log final stats before exiting
                info!(mapping_name, 
                    "Final Stats: Received={}, Forwarded={}, Errors={}", 
                    metrics.messages_received, metrics.messages_forwarded, metrics.errors
                );
                return Ok(());
            }

            // Branch 2: Process ZMQ messages with timeout
            zmq_result = timeout(heartbeat_timeout, zmq_socket.recv()) => {
                match zmq_result {
                    Ok(Ok(zmq_message)) => {
                        metrics.messages_received += 1;
                        trace!(mapping_name, num_frames = zmq_message.len(), "Received ZMQ message via select!");

                        if zmq_message.len() < 2 {
                            warn!(mapping_name, num_frames = zmq_message.len(), "Insufficient frames, skipping.");
                            // No continue here, proceed to stats report check
                        } else {
                            let topic_frame = zmq_message.get(0).map(|b| b.as_ref()).unwrap_or(&[][..]);
                            let payload_frame = zmq_message.get(1).map(|b| b.as_ref()).unwrap_or(&[][..]);
                            let topic_str = String::from_utf8_lossy(topic_frame);
                            let subject = mapper.map_topic(&topic_str);
    
                            if let Err(e) = nats_client.publish(subject.clone(), payload_frame.to_vec().into()).await {
                                error!(mapping_name, error = %e, nats_subject=%subject, "Failed to publish message to NATS");
                                warn!(mapping_name, "NATS publish error. NATS client will attempt to reconnect automatically.");
                                // Increment error count on publish failure
                                metrics.errors += 1;
                                metrics.last_error = Some(format!("NATS publish: {}", e));
                                metrics.last_error_time = Some(Instant::now());
                            } else {
                                metrics.messages_forwarded += 1;
                                debug!(mapping_name, zmq_topic = %topic_str, nats_subject = %subject, "Message forwarded via select!");
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        error!(mapping_name, error = %e, "Error receiving ZMQ message in select!");
                        // Log final stats before exiting due to error
                        info!(mapping_name, 
                            "Final Stats on Error: Received={}, Forwarded={}, Errors={}", 
                            metrics.messages_received, metrics.messages_forwarded, metrics.errors
                        );
                        // Propagate ZMQ error to the outer loop for full task restart
                        return Err(AppError::Zmq(e)); // Exit loop on ZMQ error
                    }
                    Err(_) => {
                        // Timeout occurred (no message received within heartbeat_timeout)
                        warn!(mapping_name, timeout = ?heartbeat_timeout, "ZMQ Heartbeat timeout occurred. Triggering reconnect.");
                        // Return an error to trigger the outer reconnect loop
                        return Err(AppError::Forwarder(format!(
                            "ZMQ heartbeat timeout after {:?}",
                            heartbeat_timeout
                        )));
                    }
                }
            }
        }

        // --- Periodic Stats Reporting ---
        if last_report_time.elapsed() >= stats_report_interval {
            info!(mapping_name, 
                "Stats Update: ReceivedTotal={}, ForwardedTotal={}, ErrorsTotal={}", 
                metrics.messages_received, metrics.messages_forwarded, metrics.errors
            );
            last_report_time = Instant::now(); 
        }
    }
    // Note: Loop is infinite, exit happens via return Ok(()) or return Err(...)
}

async fn connect_nats(
    uri: &str, 
    config: &NatsConfig, 
    max_connect_retries: u32 // Use max retries passed down
) -> Result<Client> {
    let mut attempts = 0;
    // Define retry delay locally for NATS connect, or make it configurable too?
    // For now, keeping it simple with a fixed internal delay.
    let nats_retry_delay = Duration::from_secs(2);
    loop {
        let mut options = ConnectOptions::new();
        if !config.user.is_empty() && !config.password.is_empty() {
            options = options.user_and_password(config.user.clone(), config.password.clone());
        }

        match options.connect(uri).await {
            Ok(client) => return Ok(client),
            Err(e) => {
                attempts += 1;
                // Use configured max retries
                if attempts >= max_connect_retries {
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
                    nats_retry_delay 
                );
                tokio::time::sleep(nats_retry_delay).await;
            }
        }
    }
} 