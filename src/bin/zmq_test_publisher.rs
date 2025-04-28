use std::error::Error;
use std::time::Duration;
use std::thread;
use tracing;
use zmq::{Context, SocketType};
use clap::Parser;

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

fn main() -> Result<(), Box<dyn Error>> {
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
    let context = Context::new();
    let socket = context.socket(SocketType::PUB)?;
    socket.bind(&args.endpoint)?;

    println!("✅ Successfully bound to endpoint");
    println!("📤 Starting to publish messages... (Press Ctrl+C to exit)\n");

    // Main message sending loop
    let mut counter = 0;
    loop {
        let payload_string = format!("{} {}", args.message_prefix, counter);
        println!("📤 Sending message #{}", counter);
        println!("   Topic: {}", args.topic);
        println!("   Data:  {}", payload_string);

        // Prepare multi-part message using slices &[u8]
        let topic_bytes = args.topic.as_bytes();
        let payload_bytes = payload_string.as_bytes();
        let message_parts = vec![topic_bytes, payload_bytes];

        // Use blocking send_multipart (flags = 0)
        match socket.send_multipart(&message_parts, 0) {
            Ok(_) => {
                println!("   ✅ Message sent successfully\n");
            }
            Err(e) => {
                eprintln!("   ❌ Error sending message: {}\n", e);
                // Consider breaking or returning error on send failure
            }
        }

        counter += 1;
        // Use std::thread::sleep
        thread::sleep(Duration::from_secs(args.interval));
    }
} 