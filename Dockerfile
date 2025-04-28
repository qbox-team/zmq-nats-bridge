# Stage 1: Build the application
FROM rust:latest as builder

# Create a dummy project to cache dependencies
WORKDIR /usr/src/app
RUN USER=root cargo init --bin .
COPY Cargo.toml Cargo.lock ./
RUN cargo fetch

# Copy the actual source code and build
COPY src ./src
RUN cargo build --release

# Stage 2: Create the final, smaller image
FROM debian:bookworm-slim

# Copy the configuration file
COPY config.yaml /app/config.yaml

# Copy the built binary from the builder stage
COPY --from=builder /usr/src/app/target/release/zmq-nats-bridge /usr/local/bin/zmq-nats-bridge

# Set the working directory
WORKDIR /app

# Expose the Prometheus port if enabled (adjust if needed)
EXPOSE 9090

# Set the entrypoint
ENTRYPOINT ["/usr/local/bin/zmq-nats-bridge", "-c", "/app/config.yaml"] 