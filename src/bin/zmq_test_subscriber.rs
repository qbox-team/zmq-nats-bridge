use std::error::Error;
use std::time::{Duration, Instant};
use clap::Parser;
use zmq::{Context, SocketType};

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
    println!("\n=== ZMQ Subscriber Configuration ===");
    println!("Endpoint:  {}", args.endpoint);
    println!("Topic:     {}", args.topic);
    println!("Heartbeat: {} seconds", args.heartbeat);
    println!("==================================\n");

    // Create and configure ZMQ socket
    let context = Context::new();
    let socket = context.socket(SocketType::SUB)?;
    socket.connect(&args.endpoint)?;
    socket.set_subscribe(&args.topic.as_bytes())?;

    println!("✅ Successfully connected and subscribed");
    println!("📥 Waiting for messages... (Press Ctrl+C to exit)\n");

    // Set up periodic reporting using std::time
    let report_interval = Duration::from_secs(2);
    let mut message_count: u64 = 0;
    let mut last_report_time = Instant::now();

    // Main message receiving loop (Blocking)
    loop {
        // Use blocking recv_multipart
        match socket.recv_multipart(0) {
            Ok(_message_parts) => {
                message_count += 1;
                // Message received, no detailed print for performance
            }
            Err(e) => {
                 eprintln!("❌ Error receiving message: {}\n", e);
                 // Check for interrupt and continue if desired, otherwise break/return
                if e == zmq::Error::EINTR {
                    println!("Interrupt received, continuing...");
                    continue;
                }
                 // Check for EAGAIN if using non-blocking flags (not used here, but good practice)
                // if e == zmq::Error::EAGAIN {
                //     // No message available yet (if non-blocking)
                //     thread::sleep(Duration::from_millis(1)); // Avoid busy-waiting
                //     continue;
                // }
                break; // Exit loop on other socket errors
            }
        }

        // Check if it's time to report stats
        let now = Instant::now();
        let elapsed_since_last = now.duration_since(last_report_time);

        if elapsed_since_last >= report_interval {
            let rate = message_count as f64 / elapsed_since_last.as_secs_f64();
            println!("📊 Stats: Received {} messages in {:.2?} (~{:.1} msgs/sec)",
                     message_count, elapsed_since_last, rate);

            // Reset for next interval
            message_count = 0;
            last_report_time = now;
        }
        // No explicit timeout check needed with blocking recv unless recv itself times out
        // (which requires setting RECVTIMEO socket option)
    }

    // Loop exited, likely due to error
    eprintln!("Exiting due to receive error.");
    Ok(())
} 