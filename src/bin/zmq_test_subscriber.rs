use std::error::Error;
use tokio::time::Duration;
use clap::Parser;
use zeromq::{Socket, SocketRecv};

/// A ZMQ subscriber for testing purposes
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// ZMQ endpoint to connect to
    #[arg(short, long, default_value = "tcp://127.0.0.1:5555")]
    endpoint: String,

    /// Topic to subscribe to
    #[arg(short, long, default_value = "test_topic")]
    topic: String,

    /// Heartbeat interval in seconds for checking message activity
    #[arg(short = 'b', long, default_value = "5")]
    heartbeat: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize tracing with console output
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .init();

    // Parse command line arguments
    let args = Args::parse();
    println!("\n=== ZMQ Subscriber Configuration ===");
    println!("Endpoint:  {}", args.endpoint);
    println!("Topic:     {}", args.topic);
    println!("Heartbeat: {} seconds", args.heartbeat);
    println!("==================================\n");

    // Create and configure ZMQ socket
    let mut socket = zeromq::SubSocket::new();
    socket.connect(&args.endpoint).await?;
    socket.subscribe(&args.topic).await?;

    println!("✅ Successfully connected and subscribed");
    println!("📥 Waiting for messages... (Press Ctrl+C to exit)\n");

    // Main message receiving loop
    loop {
        match tokio::time::timeout(Duration::from_secs(args.heartbeat), socket.recv()).await {
            Ok(Ok(message)) => {
                println!("📥 Received new message");
                println!("   Frame count: {}", message.len());

                // Print each frame's content
                for i in 0..message.len() {
                    if let Some(frame) = message.get(i) {
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
                println!(); // Add empty line for better readability
            }
            Ok(Err(e)) => {
                eprintln!("❌ Error receiving message: {}\n", e);
                break;
            }
            Err(_) => {
                println!("⏳ No message received in {} seconds\n", args.heartbeat);
            }
        }
    }

    Ok(())
} 