# zmq-nats-bridge

## Feature Requirements

This application acts as a bridge, forwarding messages subscribed from ZeroMQ (ZMQ) topics to NATS subjects based on a flexible YAML configuration.

**Core Functionality:**

*   **ZMQ Subscription:** Subscribe to specified topics on one or more ZMQ PUB endpoints (using `zmq::SUB` sockets).
*   **NATS Publishing:** Publish received ZMQ message payloads to a configured NATS subject.
*   **Raw Forwarding:** Forward the raw byte payload of the ZMQ message (typically the second frame) directly to NATS without inspection or modification. The ZMQ topic (typically the first frame) is used for filtering and subject mapping.
*   **Configuration Driven:** Define all forwarding rules, connection details, logging, and tuning parameters via a YAML configuration file (default: `config.yaml`).
*   **Multiple Mappings:** Support multiple independent forwarding mappings, each potentially connecting to different ZMQ endpoints, applying different topic-to-subject transformations, and publishing to NATS.
*   **Topic/Subject Mapping:** Transform ZMQ topics into NATS subjects using a configurable prefix and regex-based replacement rules.

**Reliability and Robustness:**

*   **ZMQ Connection Liveness:** Uses a heartbeat mechanism. If no messages are received on a subscription within a configurable timeout (`heartbeat * 2`), logs a `trace` message. (Note: Requires the ZMQ publisher to send data periodically).
*   **Automatic Reconnection:**
    *   **NATS:** Leverages the `nats.rs` library's built-in reconnection capabilities. Implements retry logic (configurable attempts/delay) for the initial connection attempt.
    *   **ZMQ:** Implements an outer retry loop for the entire forwarder task if the inner loop fails (e.g., ZMQ connection or subscription fails). Configurable retry delay and max attempts.
*   **Graceful Shutdown:** Implemented handling for signals (Ctrl+C) to shut down cleanly, signaling all forwarder tasks and waiting for them to complete (with a timeout).

**Operational Requirements:**

*   **CLI Interface:** Provide a command-line interface (using `clap`) to start the bridge, specifying the configuration file path.
*   **Logging:** Implement structured logging (using `tracing`) for observability, including connection events, errors, message forwarding details, and periodic stats. Supports both console and file logging with configurable levels.
*   **Periodic Stats:** Logs total message counts (received, forwarded, errors) for each mapping at a configurable interval.
*   **Service:** Run as a long-running process.

**Technology Stack:**

*   **Runtime:** Tokio (Async Rust)
*   **ZMQ:** `zmq.rs` crate
*   **NATS:** `nats.rs` crate (async client)
*   **Configuration:** `config-rs` crate (supporting YAML)
*   **CLI Parsing:** `clap` crate
*   **Logging:** `tracing` ecosystem
*   **Serialization:** `serde`

## Configuration (`config.yaml`)

The application is configured using a YAML file (default: `config.yaml`). See `config.example.yaml` for a template.

**Structure:**

*   `forward_mappings`: An array of mapping objects.
*   `logging`: Configuration for console and file logging.
*   `tuning` (Optional): Internal tuning parameters with defaults.

**Example `config.yaml` Snippet:**

```yaml
# Logging configuration
logging:
  # Console logging settings
  console:
    enabled: true
    # Available levels: trace, debug, info, warn, error
    level: "info"
    colors: true
  # File logging settings
  file:
    enabled: false
    level: "debug"
    # Path to log file (will be created if doesn't exist)
    path: "logs/zmq-nats-bridge.log"
    # Whether to append to existing log file
    append: true

# Optional section for internal tuning parameters
tuning:
  stats_report_interval_secs: 60 # Interval (seconds) for logging periodic stats
  task_retry_delay_secs: 5       # Delay (seconds) between retrying a failed forwarder task
  task_max_retries: 5            # Max attempts to retry a failed forwarder task

# Forward mappings define how messages flow from ZMQ to NATS
forward_mappings:
  # Example: Futures Market Data
  - name: "FuturesMarketData"      # Unique name for this mapping
    desc: "Forward futures market data from ZMQ to NATS" # Optional description
    enable: true                  # Set to false to disable this mapping

    # ZMQ Configuration
    zmq:
      endpoints:
        - "tcp://your-zmq-server:5555" # ZMQ PUB endpoint(s) to connect to
      topics:
        - "data.api.Tick"           # ZMQ topic(s) to subscribe to (prefix match)
        - "data.api.Bar"
      heartbeat: "30s"              # Optional: Used to calculate receive timeout (heartbeat * 2)

    # NATS Configuration
    nats:
      uris:
        - "nats://localhost:4222"   # NATS server URI(s)
      user: "my_user"               # Optional NATS user
      password: "my_password"       # Optional NATS password

    # Topic to Subject Mapping Rules
    topic_mapping:
      # Optional prefix added to all generated NATS subjects
      # Format: {source}.{instance}.{protocol}
      subject_prefix: "zmq.line1.pb"

      # Topic transformation rules applied sequentially to the ZMQ topic
      topic_transforms:
        # Example: Replace path separators with dots (e.g., 30s/CZCE/SH601 -> 30s.CZCE.SH601)
        - pattern: "/"
          replacement: "."
        # Example: Replace dots with hyphens (e.g., data.api.Tick -> data-api-Tick)
        - pattern: "."
          replacement: "-"

# Optional Prometheus exporter configuration
prometheus:
  enabled: true                 # Set to true to enable the exporter
  listen_address: "0.0.0.0:9090" # Address and port to listen on

**Topic/Subject Mapping Details:**

1.  The ZMQ topic received (e.g., `data.api.Bar/30s/CZCE/SH601`) is processed.
2.  Each rule in `topic_transforms` is applied:
    *   `pattern`: A string (currently simple string replacement, could be regex in future) to find.
    *   `replacement`: The string to replace the pattern with.
    *   Example 1 (`/` -> `.`): `data.api.Bar.30s.CZCE.SH601`
    *   Example 2 (`.` -> `-`): `data-api-Bar.30s.CZCE.SH601`
3.  The `subject_prefix` is prepended.
4.  Final NATS Subject: `zmq.line1.pb.data-api-Bar.30s.CZCE.SH601`

**Example Mappings:**
- Original ZMQ topic -> Transformed NATS subject
  - `data.api.Bar/30s/CZCE/SH601` -> `zmq.line1.pb.data-api-Bar.30s.CZCE.SH601`
  - `data.api.Tick/SHSE/688176` -> `zmq.line1.pb.data-api-tick.SHSE.688176`

## Prometheus Exporter

The application can optionally expose metrics in Prometheus format.

**Configuration:**

Enable and configure the exporter in your `config.yaml` under the `prometheus` section:

```yaml
prometheus:
  enabled: true                 # Set to true to enable the exporter
  listen_address: "0.0.0.0:9090" # Address and port to listen on
```

*   `enabled`: Set to `true` to activate the exporter.
*   `listen_address`: The IP address and port where the metrics server will listen. `0.0.0.0` allows connections from any network interface.

**Accessing Metrics:**

Once the bridge is running with the exporter enabled, metrics will be available at `http://<bridge_ip>:<listen_port>/metrics`. For example, if running locally, it might be `http://localhost:9090/metrics` or if running in Docker (as shown in the Docker section), `http://<docker_host_ip>:9090/metrics`.

**Docker Port Mapping:**

Remember to map the `listen_address` port when running in Docker if you want to access the metrics endpoint from outside the container (e.g., using `-p 9090:9090` in the `docker run` command if the `listen_address` is `0.0.0.0:9090`).

## Building and Running

1.  **Build:**
    ```bash
    cargo build --release
    ```
2.  **Configure:**
    *   Copy `config.example.yaml` to `config.yaml`.
    *   Edit `config.yaml` to match your ZMQ endpoints, NATS details, desired topics, and mapping rules.
3.  **Run:**
    ```bash
    ./target/release/zmq-nats-bridge --config config.yaml
    ```
    (Or omit `--config` if using the default `config.yaml` path).

## Docker Support

This project includes a `Dockerfile` for building and running the application in a container.

1.  **Build the Docker Image:**
    Make sure you have Docker installed and running. Navigate to the project root directory and run:
    ```bash
    docker build -t zmq-nats-bridge .
    ```
    This command builds the image using a multi-stage process to keep the final image size small and tags it as `zmq-nats-bridge`.

2.  **Run the Docker Container:**

    *   **Using the embedded config:**
        The `Dockerfile` copies the `config.yaml` from the build context into the image at `/app/config.yaml`. To run using this configuration:
        ```bash
        # Exposes the Prometheus port 9090 (if enabled in config.yaml)
        docker run --rm -p 9090:9090 zmq-nats-bridge
        ```
        The `--rm` flag automatically removes the container when it exits.

    *   **Mounting a local config:**
        For more flexibility, especially during development or for different environments, you can mount your local `config.yaml` file into the container, overriding the one in the image:
        ```bash
        docker run --rm -p 9090:9090 -v "$(pwd)/config.yaml":/app/config.yaml zmq-nats-bridge
        ```
        *(Note: Use `%cd%` instead of `$(pwd)` on Windows Command Prompt/PowerShell)*

    *   **Specifying a Network:**
        If your NATS server or ZMQ endpoints are running in other Docker containers, you might need to connect the bridge container to the same Docker network:
        ```bash
        # Create a network if you don't have one
        # docker network create my-network

        # Run the bridge on the specified network
        docker run --rm -p 9090:9090 --network my-network -v "$(pwd)/config.yaml":/app/config.yaml zmq-nats-bridge
        ```
        Replace `my-network` with the name of your actual Docker network.

## Docker Compose

A `docker-compose.yml` file is provided for easier management, especially when running alongside other services like NATS.

**Prerequisites:**

*   Docker Compose V2 (usually included with Docker Desktop or installable as a plugin).
*   A `config.yaml` file in the project root.

**Usage:**

1.  **Customize (Optional):**
    *   Edit `docker-compose.yml` if you need to change the mapped Prometheus port (e.g., `"9091:9090"`), container name, or network settings.
    *   Uncomment and configure the `networks` section if you are running other services (like a NATS server) within the same Docker Compose setup and need the bridge to connect to them. Ensure the `nats.uris` in your `config.yaml` use the service name defined in `docker-compose.yml` (e.g., `nats://nats-server:4222`).

2.  **Start the Service:**
    Run the following command from the project root directory:
    ```bash
    docker compose up --build
    ```
    *   `docker compose up`: Starts the services defined in the `docker-compose.yml` file.
    *   `--build`: Forces Docker Compose to build the image using the `Dockerfile` before starting the container. You can omit `--build` on subsequent runs if the source code and `Dockerfile` haven't changed.
    *   Add `-d` to run in detached mode (in the background):
        ```bash
        docker compose up --build -d
        ```

3.  **Viewing Logs (if detached):**
    ```bash
    docker compose logs -f zmq-nats-bridge
    ```

4.  **Stopping the Service:**
    ```bash
    docker compose down
    ```
    This command stops and removes the containers, networks, and volumes defined in the compose file.

## Development

*   **Testing:** `cargo test`