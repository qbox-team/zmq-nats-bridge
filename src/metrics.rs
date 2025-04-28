use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::registry::Registry;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ForwarderLabels {
    pub mapping_name: String,
}

// Holds the metric families for a forwarder
// We'll create one instance of this globally and use labels for each mapping
#[derive(Debug, Clone)]
pub struct ForwarderMetrics {
    // ZMQ Metrics
    pub zmq_messages_received: Family<ForwarderLabels, Counter>,
    pub zmq_messages_dropped: Family<ForwarderLabels, Counter>,
    pub zmq_receive_errors: Family<ForwarderLabels, Counter>,
    
    // NATS Metrics
    pub nats_messages_received_from_channel: Family<ForwarderLabels, Counter>,
    pub nats_messages_published: Family<ForwarderLabels, Counter>,
    pub nats_publish_errors: Family<ForwarderLabels, Counter>,
    
    // TODO: Add latency histogram? Maybe later.
}

impl ForwarderMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let zmq_messages_received = Family::<ForwarderLabels, Counter>::default();
        registry.register(
            "zmq_messages_received_total",
            "Total number of messages successfully received from ZMQ",
            zmq_messages_received.clone(),
        );

        let zmq_messages_dropped = Family::<ForwarderLabels, Counter>::default();
        registry.register(
            "zmq_messages_dropped_total",
            "Total number of messages dropped by ZMQ receiver due to full channel",
            zmq_messages_dropped.clone(),
        );
        
        let zmq_receive_errors = Family::<ForwarderLabels, Counter>::default();
        registry.register(
            "zmq_receive_errors_total",
            "Total number of errors encountered during ZMQ message receive",
            zmq_receive_errors.clone(),
        );

        let nats_messages_received_from_channel = Family::<ForwarderLabels, Counter>::default();
        registry.register(
            "nats_messages_received_from_channel_total",
            "Total number of messages received by NATS publisher from the internal channel",
            nats_messages_received_from_channel.clone(),
        );

        let nats_messages_published = Family::<ForwarderLabels, Counter>::default();
        registry.register(
            "nats_messages_published_total",
            "Total number of messages successfully published to NATS",
            nats_messages_published.clone(),
        );

        let nats_publish_errors = Family::<ForwarderLabels, Counter>::default();
        registry.register(
            "nats_publish_errors_total",
            "Total number of errors encountered during NATS message publish",
            nats_publish_errors.clone(),
        );

        Self {
            zmq_messages_received,
            zmq_messages_dropped,
            zmq_receive_errors,
            nats_messages_received_from_channel,
            nats_messages_published,
            nats_publish_errors,
        }
    }
} 