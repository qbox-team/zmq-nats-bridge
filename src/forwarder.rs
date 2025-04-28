use crate::config::{ForwardMapping, NatsConfig, TuningConfig};
use crate::error::{AppError, Result};
use crate::topic_mapping::TopicMapper;
use async_nats::{self, Client, ConnectOptions};
use bytes::Bytes;
use std::time::{Duration, Instant};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn, trace};
use zeromq::{Socket, SocketRecv, SubSocket};

// Constants for NATS connection retries (within connect_nats)
const NATS_CONNECT_RETRY_DELAY: Duration = Duration::from_secs(2);

// Constants for NATS publish error handling (within NATS publisher loop)
const MAX_CONSECUTIVE_NATS_ERRORS: u32 = 10; // Max errors before triggering outer reconnect

// Message struct for the internal channel
#[derive(Debug)]
struct ForwardMsg {
    topic: Bytes,
    payload: Bytes,
    recv_time: Instant,
}

// Performance metrics (remains simple for now, can be enhanced with metrics crate)
#[derive(Debug, Default)]
struct Metrics {
    messages_received: u64, // Counted when received from channel
    messages_forwarded: u64, // Counted after successful NATS publish
    errors: u64, // Counted on NATS publish error or ZMQ error
    last_error: Option<String>,
    last_error_time: Option<Instant>,
}

#[instrument(
    name = "forwarder_task",
    skip(mapping, shutdown_rx, tuning),
    fields(
        mapping_name = %mapping.name,
    )
)]
pub async fn run(
    mapping: ForwardMapping,
    mut shutdown_rx: broadcast::Receiver<()>, 
    tuning: Arc<TuningConfig>,
) -> Result<()> {
    let mapping_name = &mapping.name;
    let mut metrics = Metrics::default();
    let mut reconnect_attempts = 0;

    // Create the TopicMapper. Assume `new` returns TopicMapper directly or panics.
    let mapper = Arc::new(
        TopicMapper::new(&mapping.topic_mapping)
    );
    info!(mapping_name, "Created topic mapper for task");
    debug!(mapping_name, ?tuning, "Using tuning parameters");

    loop {
        if shutdown_rx.try_recv().is_ok() {
            info!(mapping_name, "Shutdown signal received before connection attempt");
            return Ok(());
        }

        let loop_shutdown_rx = shutdown_rx.resubscribe();
        
        // Clone Arc<TuningConfig> for the inner loop
        let loop_tuning = Arc::clone(&tuning);
        let loop_mapper = Arc::clone(&mapper);

        match run_forwarder_loop(
            &mapping, 
            loop_mapper,
            &mut metrics, 
            loop_shutdown_rx, 
            loop_tuning,
        ).await {
            Ok(_) => {
                info!(mapping_name, "Forwarder task completed normally");
                break;
            }
            Err(e) => {
                metrics.errors += 1;
                metrics.last_error = Some(e.to_string());
                metrics.last_error_time = Some(Instant::now());

                if reconnect_attempts >= tuning.task_max_retries {
                    error!(
                        mapping_name,
                        error = %e,
                        attempts = reconnect_attempts,
                        max_attempts = tuning.task_max_retries,
                        "Maximum reconnection attempts reached. Exiting."
                    );
                    return Err(e);
                }

                reconnect_attempts += 1;
                let delay: Duration = tuning.task_retry_delay_secs;
                warn!(
                    mapping_name,
                    error = %e,
                    attempt = reconnect_attempts,
                    max_attempts = tuning.task_max_retries,
                    delay = ?delay,
                    "Forwarder loop error. Attempting to reconnect..."
                );

                tokio::select! {
                    _ = tokio::time::sleep(delay) => { /* Continue */ }
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


// Main loop responsible for running the ZMQ receiver and NATS publisher tasks
async fn run_forwarder_loop(
    mapping: &ForwardMapping,
    mapper: Arc<TopicMapper>,
    metrics: &mut Metrics,
    mut shutdown_rx: broadcast::Receiver<()>, 
    tuning: Arc<TuningConfig>,
) -> Result<()> {
    let mapping_name = Arc::new(mapping.name.clone()); // Use Arc for cheap cloning into tasks
    let heartbeat_duration = mapping.zmq.heartbeat.unwrap_or_else(|| Duration::from_secs(30));
    let zmq_recv_timeout = heartbeat_duration * 2;
    
    info!(mapping_name=%mapping_name, timeout=?zmq_recv_timeout, "Calculated ZMQ recv() timeout");

    // --- NATS Connection (with retries) ---
    let nats_client = connect_nats(mapping_name.as_str(), &mapping.nats, tuning.task_max_retries).await?;

    // --- ZMQ Connection & Subscription (extracted to helper) ---
    let mut zmq_socket = setup_zmq_subscriber(mapping_name.as_str(), &mapping.zmq, shutdown_rx.resubscribe()).await?;

    // --- Create the Internal Channel (Using ForwardMsg with Bytes) ---
    let channel_capacity = tuning.channel_buffer_size;
    let (zmq_to_nats_tx, mut zmq_to_nats_rx) = mpsc::channel::<ForwardMsg>(channel_capacity);
    info!(mapping_name=%mapping_name, capacity = channel_capacity, "Created internal forwarder channel.");

    // --- Spawn ZMQ Receiver Task ---
    let mut zmq_shutdown_rx = shutdown_rx.resubscribe();
    let zmq_task_mapping_name = Arc::clone(&mapping_name);
    let mut zmq_task_handle: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async move {
        trace!(mapping_name=%zmq_task_mapping_name, "ZMQ receiver task started.");
        loop {
            tokio::select! {
                _ = zmq_shutdown_rx.recv() => {
                    info!(mapping_name=%zmq_task_mapping_name, "ZMQ receiver task received shutdown signal.");
                    break Ok(());
                }
                
                zmq_result = timeout(zmq_recv_timeout, zmq_socket.recv()) => {
                    let recv_instant = Instant::now();
                    match zmq_result {
                        Ok(Ok(zmq_message)) => {
                             trace!(mapping_name=%zmq_task_mapping_name, num_frames = zmq_message.len(), "ZMQ receiver got message");
                            if zmq_message.len() >= 2 {
                                // Convert frames to Bytes - requires copy from ZMQ buffer
                                let topic = zmq_message.get(0).map_or(Bytes::new(), |f| Bytes::copy_from_slice(f));
                                let payload = zmq_message.get(1).map_or(Bytes::new(), |f| Bytes::copy_from_slice(f));

                                // Create message with timestamp and Bytes
                                let msg = ForwardMsg { topic, payload, recv_time: recv_instant };

                                trace!(mapping_name=%zmq_task_mapping_name, "ZMQ receiver sending to channel...");
                                // Send the ForwardMsg (containing Bytes)
                                if zmq_to_nats_tx.send(msg).await.is_err() {
                                    warn!(mapping_name=%zmq_task_mapping_name, "Forwarder channel closed unexpectedly (NATS task likely stopped). ZMQ receiver task stopping.");
                                    break Err(AppError::Forwarder("Internal channel closed".to_string()));
                                }
                                trace!(mapping_name=%zmq_task_mapping_name, "ZMQ receiver sent to channel.");
                            } else {
                                warn!(mapping_name=%zmq_task_mapping_name, num_frames = zmq_message.len(), "ZMQ receiver: Insufficient frames received, skipping.");
                            }
                        }
                        Ok(Err(e)) => {
                            error!(mapping_name=%zmq_task_mapping_name, error = %e, "ZMQ receiver task failed on recv()");
                            break Err(AppError::Zmq(e)); // Propagate ZMQ error
                        }
                        Err(_) => {
                            // Timeout occurred
                            warn!(mapping_name=%zmq_task_mapping_name, timeout=?zmq_recv_timeout, "ZMQ receiver task recv() timeout. Assuming ZMQ source/connection stalled.");
                            // Return an error to trigger the outer reconnect loop - safest default
                            break Err(AppError::Forwarder(format!(
                                "ZMQ heartbeat timeout in receiver task after {:?}",
                                zmq_recv_timeout
                            )));
                        }
                    }
                }
            }
        }
    });

    // --- NATS Publisher Loop (within run_forwarder_loop) ---
    info!(mapping_name=%mapping_name, "Starting NATS publisher loop.");
    let mut last_report_time = Instant::now();
    let stats_interval = tuning.stats_report_interval_secs;
    let mut consecutive_nats_errors: u32 = 0;

    'publisher_loop: loop {
        tokio::select! {
            biased;

            // Option 1: Shutdown Signal
            _ = shutdown_rx.recv() => {
                info!(mapping_name=%mapping_name, "Shutdown signal received in NATS publisher loop.");
                break 'publisher_loop; 
            }

            // Option 2: Message received from ZMQ receiver task via channel
            maybe_msg = zmq_to_nats_rx.recv() => {
                match maybe_msg {
                    Some(msg) => {
                        // Received message containing Bytes and timestamp
                        metrics.messages_received += 1; // Count when dequeued
                        
                        // Calculate E2E latency NOW before potential awaits
                        let latency = msg.recv_time.elapsed(); 
                        
                        // Destructure msg to get Bytes fields
                        let ForwardMsg { topic, payload, .. } = msg;

                        // Use the passed mapper
                        let topic_str = String::from_utf8_lossy(&topic);
                        let subject = mapper.map_topic(&topic_str);

                        // Publish to NATS (payload is already Bytes)
                        let publish_start_time = Instant::now();
                        let publish_result = nats_client.publish(subject.clone(), payload).await;
                        let publish_duration = publish_start_time.elapsed();

                        match publish_result {
                            Ok(_) => {
                                consecutive_nats_errors = 0; // Reset error count on success
                                metrics.messages_forwarded += 1;
                                // Log with latency
                                debug!(mapping_name=%mapping_name, zmq_topic = %topic_str, nats_subject = %subject, ?latency, ?publish_duration, "Message forwarded");
                            }
                            Err(e) => {
                                consecutive_nats_errors += 1;
                                metrics.errors += 1;
                                metrics.last_error = Some(format!("NATS publish: {}", e));
                                metrics.last_error_time = Some(Instant::now());
                                
                                // Log with latency
                                error!(mapping_name=%mapping_name, error = %e, nats_subject=%subject, ?latency, ?publish_duration, consecutive_errors = consecutive_nats_errors, "Failed to publish message to NATS");
                                warn!(mapping_name=%mapping_name, "NATS client will attempt internal reconnect, but monitoring consecutive errors.");

                                // *** Add Circuit Breaker Check ***
                                if consecutive_nats_errors >= MAX_CONSECUTIVE_NATS_ERRORS {
                                     error!(mapping_name=%mapping_name, error=%e, attempts=consecutive_nats_errors, threshold=MAX_CONSECUTIVE_NATS_ERRORS, "Exceeded max consecutive NATS publish errors! Triggering full forwarder reconnect.");
                                     // Exit the inner loop with an error to force outer reconnect logic
                                     return Err(AppError::Forwarder(format!("Persistent NATS publish failure after {} consecutive errors: {}", consecutive_nats_errors, e))); 
                                }
                                // If below threshold, the loop continues processing next message
                            }
                        }
                    }
                    None => {
                        // Channel closed
                        info!(mapping_name=%mapping_name, "Internal channel closed. Assuming ZMQ receiver task stopped. Exiting NATS publisher loop.");
                        break 'publisher_loop; 
                    }
                }
            }

            // Option 3: ZMQ Receiver Task finished
            result = &mut zmq_task_handle => {
                match result {
                    Ok(Ok(())) => {
                        // ZMQ task finished normally (likely due to shutdown signal)
                        info!(mapping_name=%mapping_name, "ZMQ receiver task completed successfully.");
                    }
                    Ok(Err(e)) => {
                        // ZMQ task finished with an error it reported
                        error!(mapping_name=%mapping_name, error=%e, "ZMQ receiver task exited with error. Stopping forwarder loop.");
                        return Err(e); // Propagate the specific error
                    }
                    Err(e) => {
                        // ZMQ task panicked
                        error!(mapping_name=%mapping_name, panic=%e, "ZMQ receiver task panicked! Stopping forwarder loop.");
                        return Err(AppError::Forwarder(format!("ZMQ receiver task panicked: {}", e)));
                    }
                }
                // If the ZMQ task is finished (for any reason), stop the publisher loop.
                info!(mapping_name=%mapping_name, "ZMQ receiver task finished, exiting NATS publisher loop.");
                break 'publisher_loop;
            }
        }

        // --- Periodic Stats Reporting ---
        if last_report_time.elapsed() >= stats_interval {
            // Can add channel length reporting here if needed (using zmq_to_nats_tx.len() if available/stable)
            info!(mapping_name=%mapping_name, 
                "Stats Update: ReceivedTotal={}, ForwardedTotal={}, ErrorsTotal={}", 
                metrics.messages_received, metrics.messages_forwarded, metrics.errors
            );
            if let Some(err_str) = &metrics.last_error {
                 if let Some(err_time) = metrics.last_error_time {
                      debug!(mapping_name=%mapping_name, last_error = %err_str, last_error_age = ?err_time.elapsed(), "Last recorded error details");
                 }
            }
            last_report_time = Instant::now(); 
        }
    }

    // --- Loop Cleanup --- 
    info!(mapping_name=%mapping_name, "NATS publisher loop finished.");

    // The select above already handles joining/awaiting the ZMQ task result.
    // If the loop exited for reasons other than the ZMQ task finishing (e.g. shutdown signal),
    // we might optionally want to ensure the ZMQ task is fully stopped, but the broadcast signal
    // should handle that via its zmq_shutdown_rx.

    info!(mapping_name=%mapping_name, 
        "Final Loop Stats: Received={}, Forwarded={}, Errors={}", 
        metrics.messages_received, metrics.messages_forwarded, metrics.errors
    );

    Ok(()) // Normal exit from the inner loop
}


// --- Helper: Connect to NATS with Retries ---
#[instrument(name = "connect_nats", skip(config), fields(nats_uris = ?config.uris))] // Changed skip
async fn connect_nats(
    mapping_name: &str, // For logging context
    config: &NatsConfig, 
    max_connect_retries: u32
) -> Result<Client> {
    let mut attempts = 0;
    // Use first URI for initial attempt, client handles others if cluster specified correctly
    let primary_uri = config.uris.first().ok_or_else(|| AppError::Forwarder("No NATS URI specified".to_string()))?.as_str();
    
    info!(mapping_name, nats_uri=%primary_uri, "Attempting NATS connection...");

    loop {
        let mut options = ConnectOptions::new();
        if !config.user.is_empty() && !config.password.is_empty() {
             debug!(mapping_name, nats_uri=%primary_uri, user=%config.user, "Configuring NATS authentication (User/Pass)");
            options = options.user_and_password(config.user.clone(), config.password.clone());
        }
        // Add NKey/Creds file handling here if needed
        // options = options.max_reconnects(Some(max_connect_retries as usize)); // Configure client internal retries? Maybe not needed if we handle connect explicitly.
        
        // Construct connection string potentially joining multiple URIs
        let connection_string = config.uris.join(",");

        match options.connect(&connection_string).await {
            Ok(client) => {
                info!(mapping_name, nats_uri=%connection_string, "NATS connection successful.");
                return Ok(client);
            }
            Err(e) => {
                attempts += 1;
                if attempts > max_connect_retries { // Use > to allow exactly max_connect_retries attempts
                    error!(
                        mapping_name,
                        nats_uri = %connection_string,
                        error = %e,
                        attempts,
                        max_attempts = max_connect_retries,
                        "Failed to connect to NATS after maximum attempts."
                    );
                    // Return specific error for NATS connection failure
                    return Err(AppError::NatsConnection(e.to_string())); 
                }

                error!(
                    mapping_name,
                    nats_uri = %connection_string,
                    error = %e,
                    attempt = attempts,
                    max_attempts = max_connect_retries,
                    delay = ?NATS_CONNECT_RETRY_DELAY,
                    "Failed to connect to NATS. Retrying..."
                );
                tokio::time::sleep(NATS_CONNECT_RETRY_DELAY).await;
            }
        }
    }
}

// --- Helper: Setup ZMQ Subscriber Socket ---
#[instrument(name = "setup_zmq", skip(shutdown_rx), fields(endpoints = ?config.endpoints, topics = ?config.topics))]
async fn setup_zmq_subscriber(
    mapping_name: &str, // For logging context
    config: &super::config::ZmqConfig, // Use full path to avoid ambiguity 
    mut shutdown_rx: broadcast::Receiver<()>, // To allow cancellation during setup
) -> Result<SubSocket> {
    let mut socket = SubSocket::new();
    info!(mapping_name, "Creating ZMQ subscriber socket.");

    if config.endpoints.is_empty() {
        error!(mapping_name, "No ZMQ endpoints specified in configuration.");
        return Err(AppError::Config("No ZMQ endpoint specified".to_string()));
    }

    // Connect to endpoints
    let mut successful_connections = 0;
    for endpoint in &config.endpoints {
        info!(mapping_name, zmq_endpoint = %endpoint, "Attempting ZMQ connection...");
        tokio::select! {
            connect_result = socket.connect(endpoint) => {
                match connect_result {
                    Ok(_) => {
                        info!(mapping_name, zmq_endpoint = %endpoint, "ZMQ connection successful");
                        successful_connections += 1;
                    }
                    Err(e) => {
                        warn!(mapping_name, zmq_endpoint = %endpoint, error = %e, "ZMQ connection failed, continuing with others...");
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!(mapping_name, zmq_endpoint=%endpoint, "Shutdown received during ZMQ connect attempt.");
                return Err(AppError::Shutdown("Shutdown during ZMQ connect".to_string()));
            }
        }
    }

    if successful_connections == 0 {
        error!(mapping_name, endpoints=?config.endpoints, "Failed to connect to any specified ZMQ endpoints.");
        return Err(AppError::ZmqConnection("Failed to connect to any ZMQ endpoints".to_string()));
    }
    info!(mapping_name, count = successful_connections, "Completed ZMQ connection attempts.");

    // Subscribe to topics
    if config.topics.is_empty() {
        warn!(mapping_name, "No specific ZMQ topics defined, subscribing to all ('').");
        // ZMQ requires subscribing to *something*, empty string means all.
        if let Err(e) = socket.subscribe("").await {
             error!(mapping_name, zmq_topic = "<ALL>", error = %e, "Failed to subscribe to all ZMQ topics.");
             return Err(AppError::Zmq(e));
        }
    } else {
        for topic in &config.topics {
            info!(mapping_name, zmq_topic = %topic, "Attempting ZMQ subscription...");
            tokio::select! {
                subscribe_result = socket.subscribe(topic) => {
                    match subscribe_result {
                        Ok(_) => {
                            info!(mapping_name, zmq_topic = %topic, "ZMQ subscription successful");
                        }
                        Err(e) => {
                            error!(mapping_name, zmq_topic = %topic, error = %e, "ZMQ subscription failed");
                            // Treat subscription failure as critical for this forwarder
                            return Err(AppError::Zmq(e)); 
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!(mapping_name, zmq_topic=%topic, "Shutdown received during ZMQ subscribe attempt.");
                     return Err(AppError::Shutdown("Shutdown during ZMQ subscribe".to_string()));
                }
            }
        }
    }

    info!(mapping_name, "ZMQ subscriber setup complete.");
    Ok(socket)
}