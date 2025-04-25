use std::error::Error;
use tokio::time::Duration;
use tracing;
use zeromq::{Socket, SocketSend};
use clap::Parser;
use bytes::Bytes;

/// A ZMQ publisher for testing purposes
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// ZMQ endpoint to bind to
    #[arg(short, long, default_value = "tcp://127.0.0.1:5555")]
    endpoint: String,

    /// Topic to publish to
    #[arg(short, long, default_value = "test_topic")]
    topic: String,

    /// Message prefix
    #[arg(short, long, default_value = "Test message")]
    message_prefix: String,

    /// Message interval in seconds
    #[arg(short, long, default_value = "1")]
    interval: u64,
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
    println!("\n=== ZMQ Publisher Configuration ===");
    println!("Endpoint: {}", args.endpoint);
    println!("Topic:    {}", args.topic);
    println!("Prefix:   {}", args.message_prefix);
    println!("Interval: {} seconds", args.interval);
    println!("=================================\n");

    // Create and configure ZMQ socket
    let mut socket = zeromq::PubSocket::new();
    socket.bind(&args.endpoint).await?;

    println!("✅ Successfully bound to endpoint");
    println!("📤 Starting to publish messages... (Press Ctrl+C to exit)\n");

    // Main message sending loop
    let mut counter = 0;
    loop {
        let message = format!("{} {}", args.message_prefix, counter);
        println!("📤 Sending message #{}", counter);
        println!("   Topic: {}", args.topic);
        println!("   Data:  {}", message);

        // Create a multi-frame message (topic + payload)
        let mut msg = zeromq::ZmqMessage::from(args.topic.as_str());
        msg.push_back(Bytes::from(message.as_bytes().to_vec()));

        match socket.send(msg).await {
            Ok(_) => {
                println!("   ✅ Message sent successfully\n");
            }
            Err(e) => {
                eprintln!("   ❌ Error sending message: {}\n", e);
            }
        }

        counter += 1;
        tokio::time::sleep(Duration::from_secs(args.interval)).await;
    }
} 