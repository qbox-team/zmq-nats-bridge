# zmq-nats-bridge

## Feature Requirements

This application acts as a bridge, forwarding messages subscribed from ZeroMQ (ZMQ) topics to NATS subjects based on a flexible configuration.

**Core Functionality:**

*   **ZMQ Subscription:** Subscribe to specified topics on one or more ZMQ PUB endpoints (using `zmq::SUB` sockets).
*   **NATS Publishing:** Publish received ZMQ message payloads to a configured NATS subject.
*   **Raw Forwarding:** Forward the raw byte payload of the ZMQ message directly to NATS without inspection or modification. The ZMQ topic used for filtering is *not* forwarded.
*   **Configuration Driven:** Define all forwarding rules, connection details, and behavior via a configuration file (e.g., `config.toml`).
*   **Multiple Mappings:** Support multiple independent forwarding mappings, each potentially connecting to different ZMQ endpoints and NATS subjects.

**Reliability and Robustness:**

*   **ZMQ Connection Liveness:** Monitor the health of ZMQ connections. If no messages are received on a subscription within a configurable timeout (`heartbeat * 2`), log a warning, assuming the connection might be stale. (Note: This relies on the ZMQ publisher sending data periodically or having its own keepalive mechanism).
*   **Automatic Reconnection:**
    *   **NATS:** Leverage the `nats.rs` library's built-in reconnection capabilities. Implement retry logic for the initial connection attempt.
    *   **ZMQ:** Handle ZMQ errors during receive operations. Currently logs errors and continues; more advanced reconnection strategies (like socket recreation) could be added if needed.
*   **Graceful Shutdown:** (TODO/Future) Implement handling for signals (like Ctrl+C) to shut down cleanly, closing connections properly.

**Operational Requirements:**

*   **CLI Interface:** Provide a command-line interface (using `clap`) to start the bridge, specifying the configuration file path.
*   **Logging:** Implement structured logging (using `tracing`) for observability, including connection events, errors, and message forwarding details (e.g., message size). Supports both console and file logging with configurable levels and daily rotation.
*   **Service:** Run as a long-running process. Specific OS-level service integration (systemd, etc.) is out of scope unless explicitly added later.

**Technology Stack:**

*   **Runtime:** Tokio (Async Rust)
*   **ZMQ:** `zmq.rs` crate
*   **NATS:** `nats.rs` crate (async client)
*   **Configuration:** `config-rs` crate (supporting TOML)
*   **CLI Parsing:** `clap` crate
*   **Logging:** `tracing` ecosystem with daily log rotation

## Configuration Example (`config.toml`)

```toml
# Global ZMQ settings
[zmq]
# If no message is received within heartbeat * 2, log a warning.
# Publisher should send data frequently enough or implement its own keepalives.
# Value uses humantime format (e.g., "5s", "1m")
heartbeat = "5s"

# Global NATS settings (optional credentials)
[nats]
# user = "my_user"
# password = "my_password"

# Define one or more forwarding mappings
[[forward_mappings]]
# Optional name for easier logging/identification
name = "FuturesTicks"
# List of ZMQ PUB endpoints to connect to (SUB socket)
zmq_endpoints = ["tcp://192.168.6.7:1402"]
# List of ZMQ topics to subscribe to
zmq_topics = ["data/api/Tick"]
# NATS server URI for this mapping
nats_uri = "nats://localhost:4222"
# Target NATS subject to publish messages to
nats_subject = "market_data.FUT_CN.tick"

# Example 2: Forwarding Future 1m Bars
[[forward_mappings]]
name = "FuturesBars1m"
zmq_endpoints = ["tcp://192.168.6.7:1422"]
zmq_topics = ["data/api/Bar"]
nats_uri = "nats://localhost:4222"
nats_subject = "market_data.FUT_CN.bar.1m"

# Example 3: Forwarding Stock Ticks
[[forward_mappings]]
name = "StockTicks"
zmq_endpoints = ["tcp://192.168.6.7:25121"]
zmq_topics = ["data/api/Tick"]
nats_uri = "nats://localhost:4222"
nats_subject = "market_data.STK_CN.tick"

# Example 4: Forwarding Stock 1m Bars
[[forward_mappings]]
name = "StockBars1m"
zmq_endpoints = ["tcp://192.168.6.7:6120"]
zmq_topics = ["data/api/Bar"]              # Assuming Bar based on NATS subject
nats_uri = "nats://localhost:4222"
nats_subject = "market_data.STK_CN.bar.1m"

# Logging configuration
[logging]
# Console logging settings
[logging.console]
# Enable or disable console logging
enabled = true
# Log level for console: trace, debug, info, warn, error
level = "info"
# Enable ANSI colors in console output
colors = true

# File logging settings
[logging.file]
# Enable or disable file logging
enabled = false
# Log level for file: trace, debug, info, warn, error
level = "info"
# Path to the log file
path = "logs/zmq-nats-bridge.log"
# Whether to append to existing log file or create new one
append = true
```

## Logging Configuration

The application supports both console and file logging with the following features:

### Console Logging
- Enabled by default
- Configurable log level (default: "info")
- ANSI colors support
- Shows thread IDs, file locations, and line numbers

### File Logging
- Disabled by default
- Configurable log level (default: "info")
- Daily log rotation (files named with date suffix)
- Default path: "logs/zmq-nats-bridge.log"
- Shows thread IDs, file locations, and line numbers
- No ANSI colors for better file readability

To enable file logging, set `enabled = true` in the `[logging.file]` section of your configuration.
