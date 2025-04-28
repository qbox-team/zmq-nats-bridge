use std::error::Error;
use zmq::{Context, SocketType};

// --- Configuration (Hardcoded for simplicity) ---
const ZMQ_ENDPOINT: &str = "tcp://127.0.0.1:5555"; // Adjust if your publisher uses a different endpoint
const ZMQ_TOPIC: &str = "test_topic";         // Adjust if your publisher uses a different topic

fn main() -> Result<(), Box<dyn Error>> {
    println!("--- Minimal ZMQ Debug Subscriber (Blocking) ---");
    println!("Endpoint: {}", ZMQ_ENDPOINT);
    println!("Topic:    {}", ZMQ_TOPIC);
    println!("------------------------------------");

    // --- ZMQ Connection ---
    let context = Context::new();
    let zmq_socket = context.socket(SocketType::SUB)?;
    println!("Creating new ZMQ subscriber socket");

    println!("Attempting to connect to ZMQ endpoint: {}", ZMQ_ENDPOINT);
    match zmq_socket.connect(ZMQ_ENDPOINT) {
        Ok(_) => {
            println!("Successfully connected to ZMQ endpoint: {}", ZMQ_ENDPOINT);
        }
        Err(e) => {
            eprintln!("Failed to connect to ZMQ endpoint: {}", e);
            return Err(e.into());
        }
    }

    println!("Attempting to subscribe to ZMQ topic: {}", ZMQ_TOPIC);
    match zmq_socket.set_subscribe(ZMQ_TOPIC.as_bytes()) {
        Ok(_) => {
            println!("Successfully subscribed to ZMQ topic: {}", ZMQ_TOPIC);
        }
        Err(e) => {
            eprintln!("Failed to subscribe to ZMQ topic: {}", e);
            return Err(e.into());
        }
    }

    println!("ZMQ socket connected and subscribed. Waiting for messages...");

    // --- Main Receiving Loop (Blocking) ---
    loop {
        match zmq_socket.recv_multipart(0) {
            Ok(message_parts) => {
                println!("
📥 Received ZMQ message with {} frames", message_parts.len());

                // Print each frame
                for (i, frame) in message_parts.iter().enumerate() {
                    match String::from_utf8(frame.clone()) {
                        Ok(text) => {
                            println!("   Frame {}: {}", i, text);
                        }
                        Err(_) => {
                            println!("   Frame {}: [binary data, length: {}]", i, frame.len());
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("❌ Error receiving ZMQ message: {}", e);
                if e == zmq::Error::EINTR {
                    println!("Interrupt received, continuing...");
                    continue;
                }
                return Err(e.into());
            }
        }
    }
} 