use std::error::Error;
use std::time::Duration;
use tokio::time::timeout;
use zeromq::{Socket, SocketRecv, SubSocket};

// --- Configuration (Hardcoded for simplicity) ---
const ZMQ_ENDPOINT: &str = "tcp://127.0.0.1:5555"; // Adjust if your publisher uses a different endpoint
const ZMQ_TOPIC: &str = "test_topic";         // Adjust if your publisher uses a different topic
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(60); // Increased timeout

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("--- Minimal ZMQ Debug Subscriber ---");
    println!("Endpoint: {}", ZMQ_ENDPOINT);
    println!("Topic:    {}", ZMQ_TOPIC);
    println!("------------------------------------");

    // --- ZMQ Connection ---
    let mut zmq_socket = SubSocket::new();
    println!("Creating new ZMQ subscriber socket");

    println!("Attempting to connect to ZMQ endpoint: {}", ZMQ_ENDPOINT);
    match zmq_socket.connect(ZMQ_ENDPOINT).await {
        Ok(_) => {
            println!("Successfully connected to ZMQ endpoint: {}", ZMQ_ENDPOINT);
        }
        Err(e) => {
            eprintln!("Failed to connect to ZMQ endpoint: {}", e);
            return Err(e.into());
        }
    }

    println!("Attempting to subscribe to ZMQ topic: {}", ZMQ_TOPIC);
    match zmq_socket.subscribe(ZMQ_TOPIC).await {
        Ok(_) => {
            println!("Successfully subscribed to ZMQ topic: {}", ZMQ_TOPIC);
        }
        Err(e) => {
            eprintln!("Failed to subscribe to ZMQ topic: {}", e);
            return Err(e.into());
        }
    }

    println!("ZMQ socket connected and subscribed. Waiting for messages...");

    // --- Main Receiving Loop (Simplified) ---
    loop {
        match timeout(HEARTBEAT_TIMEOUT, zmq_socket.recv()).await {
            Ok(Ok(zmq_message)) => {
                println!("
📥 Received ZMQ message with {} frames", zmq_message.len());

                // Print each frame
                for i in 0..zmq_message.len() {
                    if let Some(frame) = zmq_message.get(i) {
                        match String::from_utf8(frame.to_vec()) {
                            Ok(text) => {
                                println!("   Frame {}: {}", i, text);
                            }
                            Err(_) => {
                                println!("   Frame {}: [binary data, length: {}]", i, frame.len());
                            }
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("❌ Error receiving ZMQ message: {}", e);
                return Err(e.into()); // Exit on error
            }
            Err(_) => {
                // Timeout occurred
                println!("⏳ No message received within {} seconds.", HEARTBEAT_TIMEOUT.as_secs());
            }
        }
    }
    // Unreachable, loop is infinite
    // Ok(())
} 