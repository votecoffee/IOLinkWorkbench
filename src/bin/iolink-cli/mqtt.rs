use clap::Args;
use iolink_workbench::mqtt::Config;
use std::net::SocketAddr;

#[derive(Args)]
pub struct Arguments {
    /// Opt-in MQTT 3.1.1 TCP QoS 0 publishing; numeric IP:PORT, no TLS/auth.
    #[arg(long)]
    mqtt_broker: Option<SocketAddr>,
    #[arg(long, requires = "mqtt_broker")]
    mqtt_topic: Option<String>,
    /// Latest-state interval, 10–60000 ms (default 100).
    #[arg(long, requires = "mqtt_broker")]
    mqtt_interval_ms: Option<u64>,
    /// Independent MQTT queue, 1–16384 events (default 256).
    #[arg(long, requires = "mqtt_broker")]
    mqtt_queue_capacity: Option<usize>,
}
impl Arguments {
    pub fn config(self) -> Option<Config> {
        self.mqtt_broker.map(|broker| Config {
            broker,
            topic: self
                .mqtt_topic
                .unwrap_or_else(|| "iolink-workbench/input".into()),
            interval_ms: self.mqtt_interval_ms.unwrap_or(100),
            queue_capacity: self.mqtt_queue_capacity.unwrap_or(256),
        })
    }
}
