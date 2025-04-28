use crate::config::{ForwardMapping, NatsConfig, TuningConfig};
use crate::error::{AppError, Result};
use crate::topic_mapping::TopicMapper;
use async_nats::{self, Client, ConnectOptions};
use bytes::Bytes;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
// Use async_channel for sync thread <-> async task communication
use async_channel::{self, Receiver, Sender};
use tracing::{debug, error, info, instrument, warn, trace};
// Use blocking zmq API from zmq = "0.10"
use zmq::{Context, SocketType}; // Use zmq crate types
use std::thread; // Add back std::thread
// Remove futures-util import if no longer needed elsewhere (assuming it's not)
// use futures_util::TryStreamExt;
// use futures_util::future::TryFutureExt;

// Constants
const NATS_CONNECT_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_CONSECUTIVE_NATS_ERRORS: u32 = 10;

// Message struct for the channel - simple Vecs now
// Use Arc<[u8]> to potentially reduce copying if payload is large?
// Start with Vec<Vec<u8>> for simplicity.
#[derive(Debug, Clone)] // Clone needed if we want thread to maybe hold onto it
struct ForwardData {
    // topic: Vec<u8>,
    // payload: Vec<u8>,
    // Or send the whole multipart message
    message_parts: Vec<Vec<u8>>,
    recv_time: Instant,
}

// NATS publisher task metrics
#[derive(Debug, Default)]
struct NatsMetrics {
    messages_received_from_channel: u64,
    messages_forwarded_to_nats: u64,
    nats_publish_errors: u64,
    last_nats_error: Option<String>,
    last_nats_error_time: Option<Instant>,
}

// ZMQ thread stats
#[derive(Debug, Default)]
struct ZmqStats {
    success_count: u64,
    drop_count: u64,
    error_count: u64,
}

#[instrument(
    name = "forwarder_task",
    skip(mapping, shutdown_rx, tuning),
    fields(mapping_name = %mapping.name)
)]
pub async fn run(
    mapping: ForwardMapping,
    mut shutdown_rx: broadcast::Receiver<()>, 
    tuning: Arc<TuningConfig>,
) -> Result<()> {
    let mapping_name = &mapping.name;
    info!(mapping_name, "Starting forwarder task (Blocking ZMQ Thread).");

    let mut reconnect_attempts = 0;

    // Create the TopicMapper (once)
    let mapper = Arc::new(TopicMapper::new(&mapping.topic_mapping));
    info!(mapping_name, "Created topic mapper.");

    // Outer loop for handling reconnects if inner loop fails
    loop {
        if shutdown_rx.try_recv().is_ok() {
            info!(mapping_name, "Shutdown signal received before connection attempt");
            return Ok(());
        }

        let loop_shutdown_rx = shutdown_rx.resubscribe();
        let inner_mapper = Arc::clone(&mapper);
        let inner_tuning = Arc::clone(&tuning);
        // ForwardMapping needs to derive Clone
        let inner_mapping = mapping.clone(); 

        match run_forwarder_components(
            inner_mapping,
            inner_mapper,
            loop_shutdown_rx,
            inner_tuning,
        )
        .await
        {
            Ok(_) => {
                info!(mapping_name, "Forwarder components finished normally. Task exiting.");
                return Ok(()); // Normal exit
            }
            Err(e) => {
                error!(mapping_name, error = %e, "Forwarder components exited with error.");
                if matches!(e, AppError::Shutdown(_)) {
                     info!(mapping_name, "Exiting due to shutdown signal during operation.");
                     return Ok(());
                }
                if reconnect_attempts >= tuning.task_max_retries {
                    error!(mapping_name, final_error = %e, attempts = reconnect_attempts, max_attempts = tuning.task_max_retries, "Max reconnects reached.");
                    return Err(e);
                }
                reconnect_attempts += 1;
                let delay: Duration = tuning.task_retry_delay_secs;
                warn!(mapping_name, error = %e, attempt = reconnect_attempts, max_attempts = tuning.task_max_retries, delay = ?delay, "Attempting reconnect...");
                tokio::select! {
                    _ = tokio::time::sleep(delay) => { /* Continue loop */ }
                    _ = shutdown_rx.recv() => {
                        info!(mapping_name, "Shutdown during reconnect delay.");
                        return Ok(());
                    }
                }
            }
        }
    }
}

// Function to set up and run components for one iteration
async fn run_forwarder_components(
    mapping: ForwardMapping, // Takes ownership via clone
    mapper: Arc<TopicMapper>,
    shutdown_rx: broadcast::Receiver<()>, // Takes ownership
    tuning: Arc<TuningConfig>,
) -> Result<()> {
    let mapping_name = Arc::new(mapping.name.clone());
    info!(mapping_name=%mapping_name, "Initializing forwarder components (blocking ZMQ).");

    // --- NATS Connection (Async) ---
    let nats_client = connect_nats(mapping_name.as_str(), &mapping.nats, tuning.task_max_retries).await?;
    let nats_client = Arc::new(nats_client);

    // --- Create Async Channel ---
    let channel_capacity = tuning.channel_buffer_size;
    let (tx, rx) = async_channel::bounded::<ForwardData>(channel_capacity);
    info!(mapping_name=%mapping_name, capacity = channel_capacity, "Created async_channel.");

    // --- Spawn Dedicated Blocking ZMQ Receiver Thread --- 
    let zmq_thread_handle = {
        // Use mapping.name directly, clone String for thread
        let thread_mapping_name = mapping.name.clone(); 
        let thread_endpoints = mapping.zmq.endpoints.clone();
        let thread_topics = mapping.zmq.topics.clone();
        let thread_tx = tx; // ZMQ thread owns the sender end
        // No shutdown_rx needed for the blocking thread

        thread::spawn(move || { // Use std::thread::spawn
            // Run the blocking ZMQ loop
            let result = run_zmq_receiver_thread(
                thread_mapping_name.clone(), // Clone again for logging inside
                thread_endpoints,
                thread_topics,
                thread_tx, 
            );
            // Log final thread status
            match &result { // Log based on ZmqStats result
                Ok(stats) => info!(mapping_name=%thread_mapping_name, "ZMQ thread finished. Stats: Received={}, Dropped={}", stats.success_count, stats.drop_count),
                Err(e) => error!(mapping_name=%thread_mapping_name, error=%e, "ZMQ thread finished with error."),
            }
            result // Return the Result<ZmqStats>
        })
    };

    // --- Run NATS Publisher Task (Async) ---
    let publisher_result = run_nats_publisher_loop(
        Arc::clone(&mapping_name),
        nats_client,
        mapper,
        rx, // Publisher owns the receiver end
        tuning,
        shutdown_rx, // Publisher owns the original shutdown receiver
    ).await;

    // Ensure thread handle doesn't leak if publisher exits early.
    // Joining might block, dropping is okay if we don't need the thread's return value.
    drop(zmq_thread_handle); 

    publisher_result
}

// ========================================
// Helper: Connect to NATS with Retries 
// ========================================
#[instrument(name = "connect_nats", skip(config), fields(nats_uris = ?config.uris))]
async fn connect_nats(
    mapping_name: &str,
    config: &NatsConfig, 
    max_connect_retries: u32
) -> Result<Client> {
    let mut attempts = 0;
    let _primary_uri = config.uris.first().ok_or_else(|| AppError::Config("No NATS URI specified".to_string()))?.as_str();
    let connection_string = config.uris.join(",");
    
    info!(mapping_name, nats_uris=%connection_string, "Attempting NATS connection...");

    loop {
        let mut options = ConnectOptions::new();
        if !config.user.is_empty() && !config.password.is_empty() {
             debug!(mapping_name, user=%config.user, "Configuring NATS authentication (User/Pass)");
            options = options.user_and_password(config.user.clone(), config.password.clone());
        }
        
        match options.connect(&connection_string).await {
            Ok(client) => {
                info!(mapping_name, "NATS connection successful.");
                return Ok(client);
            }
            Err(e) => {
                attempts += 1;
                if attempts > max_connect_retries { 
                    error!(mapping_name, error = %e, attempts, max_attempts = max_connect_retries, "Failed to connect to NATS after max attempts.");
                    return Err(AppError::NatsConnection(e.to_string())); 
                }
                error!(mapping_name, error = %e, attempt = attempts, max_attempts = max_connect_retries, delay = ?NATS_CONNECT_RETRY_DELAY, "Failed NATS connect. Retrying...");
                tokio::time::sleep(NATS_CONNECT_RETRY_DELAY).await;
            }
        }
    }
}

// ======================================== 
// ZMQ Receiver Thread Function (Blocking)
// ========================================
fn run_zmq_receiver_thread( // Changed back to fn, removed async, removed shutdown_rx
    mapping_name: String, 
    endpoints: Vec<String>,
    topics: Vec<String>,
    tx: Sender<ForwardData>, 
) -> Result<ZmqStats> { // Return Result<ZmqStats>
    info!(mapping_name=%mapping_name, "ZMQ receiver thread starting (blocking API).");
    let mut stats = ZmqStats::default();

    // --- ZMQ Connection & Subscription (Blocking API from zmq = "0.10") ---
    let context = Context::new();
    let socket = context.socket(SocketType::SUB)
        .map_err(|e| {
            error!(mapping_name=%mapping_name, error = %e, "Failed to create ZMQ SUB socket.");
            AppError::Zmq(e)
        })?;

    // Set high water mark using the Socket method from zmq = "0.10"
    match socket.set_rcvhwm(1_000_000) {
        Ok(_) => info!(mapping_name=%mapping_name, "Set ZMQ RCVHWM successfully."),
        Err(e) => warn!(mapping_name=%mapping_name, error = %e, "Failed to set ZMQ RCVHWM."),
    }

    if endpoints.is_empty() {
        error!(mapping_name=%mapping_name, "No ZMQ endpoints provided to thread.");
        return Err(AppError::Config("No ZMQ endpoint specified".to_string()));
    }
    for endpoint in &endpoints {
        info!(mapping_name=%mapping_name, zmq_endpoint = %endpoint, "Thread connecting ZMQ (blocking)...");
        // Use blocking connect
        socket.connect(endpoint).map_err(|e| {
            error!(mapping_name=%mapping_name, zmq_endpoint = %endpoint, error = %e, "ZMQ connect failed.");
            AppError::Zmq(e)
        })?;
        info!(mapping_name=%mapping_name, zmq_endpoint = %endpoint, "ZMQ Connected.");
    }

    if topics.is_empty() {
        warn!(mapping_name=%mapping_name, "No specific ZMQ topics, subscribing to all ('').");
        // Use blocking subscribe with bytes (zmq = "0.10" often uses bytes)
        socket.set_subscribe(b"").map_err(|e| { // Renamed to set_subscribe
             error!(mapping_name=%mapping_name, zmq_topic = "", error = %e, "ZMQ subscription failed.");
             AppError::Zmq(e)
        })?;
    } else {
        for topic in &topics {
             info!(mapping_name=%mapping_name, zmq_topic = %topic, "Thread subscribing ZMQ...");
             // Use blocking subscribe with bytes
             socket.set_subscribe(topic.as_bytes()).map_err(|e| { // Renamed to set_subscribe
                 error!(mapping_name=%mapping_name, zmq_topic = %topic, error = %e, "ZMQ subscription failed.");
                 AppError::Zmq(e)
            })?;
            info!(mapping_name=%mapping_name, zmq_topic=%topic, "ZMQ Subscribed.");
        }
    }

    info!(mapping_name=%mapping_name, "ZMQ receiver thread entering blocking receive loop...");
    let mut last_log_time = Instant::now();
    let log_interval = Duration::from_secs(10); 

    // --- Main Blocking Loop ---
    loop {
        // Check channel closed before blocking recv?
        if tx.is_closed() {
            info!(mapping_name=%mapping_name, "Channel closed by receiver. ZMQ thread exiting.");
            break;
        }

        // Blocking receive using recv_multipart on Socket
        match socket.recv_multipart(0) { // flags = 0 for blocking
            Ok(message_parts) => { // message_parts is Vec<Vec<u8>>
                let recv_time = Instant::now();
                
                if message_parts.is_empty() {
                    trace!(mapping_name=%mapping_name, "Received empty multipart message, skipping.");
                    continue;
                }
                let data = ForwardData { message_parts, recv_time };

                // Use non-blocking try_send
                match tx.try_send(data) {
                    Ok(_) => {
                        stats.success_count += 1;
                    }
                    Err(async_channel::TrySendError::Full(_dropped_data)) => {
                        stats.drop_count += 1;
                        if stats.drop_count == 1 || last_log_time.elapsed() > log_interval {
                            warn!(mapping_name=%mapping_name,
                                  drops = stats.drop_count,
                                  "ZMQ receiver channel full. Dropped messages.");
                            last_log_time = Instant::now();
                        }
                    }
                    Err(async_channel::TrySendError::Closed(_dropped_data)) => {
                        info!(mapping_name=%mapping_name, "Channel closed during try_send. ZMQ thread exiting.");
                        break; // Exit loop
                    }
                }
            }
            Err(e) => {
                stats.error_count += 1;
                // Check if the error is EAGAIN (non-blocking) or EINTR (interrupt), possibly continue
                if e == zmq::Error::EAGAIN || e == zmq::Error::EINTR {
                    trace!(mapping_name=%mapping_name, error=%e, "ZMQ recv non-fatal error, continuing.");
                    // Maybe short sleep to prevent spinning on EAGAIN if misconfigured?
                    thread::sleep(Duration::from_millis(1)); 
                    continue;
                }
                // Otherwise, treat as potentially fatal for now
                // Log most ZMQ recv errors and continue to maximize uptime;
                // potentially fatal errors could return Err(AppError::Zmq(e)) instead.
                error!(mapping_name=%mapping_name, error = %e, "ZMQ blocking recv error.");
                // Maybe add a small sleep to prevent tight error loop on some errors?
                thread::sleep(Duration::from_millis(100)); 
                // Consider if the error means the thread should exit.
                // For now, we log and continue, but could return Err here.
                // Example: return Err(AppError::Zmq(e));
            }
        }
    }

    info!(mapping_name=%mapping_name, "ZMQ receiver thread loop finished.");
    // Ensure the channel is closed from the sender side upon exit
    tx.close();
    Ok(stats) // Return ZmqStats
}

// ========================================
// NATS Publisher Task Function (Async)
// ========================================
async fn run_nats_publisher_loop(
    mapping_name: Arc<String>,
    nats_client: Arc<Client>,
    mapper: Arc<TopicMapper>,
    rx: Receiver<ForwardData>, // Receiver from async_channel
    tuning: Arc<TuningConfig>,
    mut shutdown_rx: broadcast::Receiver<()>, 
) -> Result<()> {
    info!(mapping_name=%mapping_name, "Starting NATS publisher task (async).");
    let mut metrics = NatsMetrics::default();
    let mut consecutive_nats_errors: u32 = 0;

    let report_interval = tuning.stats_report_interval_secs;
    let mut interval = tokio::time::interval(report_interval);
    interval.tick().await; // Consume initial tick

    loop {
        tokio::select! {
            biased; // Prioritize receiving messages

            // Branch 1: Receive message from ZMQ thread's channel
            Ok(data) = rx.recv() => { // Use Ok() pattern as recv() returns Result
                metrics.messages_received_from_channel += 1;
                let latency_since_zmq_recv = data.recv_time.elapsed();
                
                // Extract topic and payload from Vec<Vec<u8>>
                // Assumes ZMQ message contains at least 2 frames: [topic, payload, ...]
                if data.message_parts.len() >= 2 {
                    // Assume topic is frame 0, payload is frame 1
                    let topic_slice = &data.message_parts[0];
                    let payload_slice = &data.message_parts[1];
                    
                    // Convert topic slice to string for mapping
                    let topic_str = String::from_utf8_lossy(topic_slice);
                    let subject = mapper.map_topic(&topic_str);

                    // Convert payload slice to Bytes for publishing
                    let payload_bytes = Bytes::copy_from_slice(payload_slice);

                    // Publish to NATS
                    let publish_start_time = Instant::now();
                    let publish_result = nats_client.publish(subject.clone(), payload_bytes).await;
                    let publish_duration = publish_start_time.elapsed();

                    match publish_result {
                        Ok(_) => {
                            consecutive_nats_errors = 0;
                            metrics.messages_forwarded_to_nats += 1;
                            trace!(mapping_name=%mapping_name, zmq_topic = %topic_str, nats_subject = %subject, ?latency_since_zmq_recv, ?publish_duration, "Message forwarded");
                        }
                        Err(e) => {
                            consecutive_nats_errors += 1;
                            metrics.nats_publish_errors += 1;
                            let error_string = e.to_string();
                            metrics.last_nats_error = Some(error_string.clone());
                            metrics.last_nats_error_time = Some(Instant::now());
                            error!(mapping_name=%mapping_name, error = %e, nats_subject=%subject, ?latency_since_zmq_recv, ?publish_duration, consecutive_errors = consecutive_nats_errors, "Failed to publish to NATS");
                            
                            // Check circuit breaker
                            if consecutive_nats_errors >= MAX_CONSECUTIVE_NATS_ERRORS {
                                 error!(mapping_name=%mapping_name, error=%e, attempts=consecutive_nats_errors, threshold=MAX_CONSECUTIVE_NATS_ERRORS, "Exceeded max consecutive NATS publish errors! Triggering reconnect.");
                                 return Err(AppError::Forwarder(format!(
                                    "Persistent NATS publish failure after {} errors: {}", 
                                    consecutive_nats_errors, error_string
                                ))); 
                            }
                        }
                    }
                } else {
                    warn!(mapping_name=%mapping_name, num_frames = data.message_parts.len(), "NATS publisher: Insufficient frames received, skipping.");
                }
            }
            
            // This branch handles channel closure
            Err(_) = rx.recv() => {
                info!(mapping_name=%mapping_name, "Channel closed by ZMQ thread. NATS publisher exiting.");
                break; // Exit the loop normally
            }

            // Branch 2: Report Stats periodically
            _ = interval.tick() => {
                 info!(mapping_name=%mapping_name,
                      "NATS Pub Stats: RcvdFromChan={}, FwdToNats={}, PublishErrs={}",
                      metrics.messages_received_from_channel, metrics.messages_forwarded_to_nats, metrics.nats_publish_errors
                 );
                 if let Some(err_str) = &metrics.last_nats_error {
                      if let Some(err_time) = metrics.last_nats_error_time {
                           debug!(mapping_name=%mapping_name, last_error = %err_str, last_error_age = ?err_time.elapsed(), "Last recorded NATS error");
                      }
                 }
            }

            // Branch 3: Handle shutdown signal
            _ = shutdown_rx.recv() => {
                info!(mapping_name=%mapping_name, "Shutdown signal received. Exiting NATS publisher loop.");
                // Closing the receiver end here ensures the loop terminates cleanly
                // even if the channel wasn't closed by the sender yet.
                rx.close();
                break; 
            }
        }
    }

    info!(mapping_name=%mapping_name, "NATS publisher loop finished.");
    Ok(())
}