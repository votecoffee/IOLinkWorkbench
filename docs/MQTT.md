# MQTT latest-state publishing

The GUI/CLI can optionally publish acquired inputs to a numeric TCP broker endpoint. This is a lossy latest-state feed, independent of CSV and the local API. It is disabled by default and active only during a monitor/capture session. It adds no hardware connection, subscriptions, acyclic queries or device/output command path. Runtime options are not saved profile keys.

## Enable and connect

In Monitor & CSV, enable **MQTT latest-state publishing**, enter the broker IP:port, exact topic and interval. From the portable package directory:

```powershell
.\iolink-cli.exe capture samples/simulator.toml trial-mqtt --seconds 10 --mqtt-broker 127.0.0.1:1883 --mqtt-topic lab/iolink/input --mqtt-interval-ms 100
```

An existing broker is required; no broker is bundled. `--mqtt-broker IP:PORT` opts in. Numeric IPv4/IPv6 unicast endpoints with a nonzero port are accepted; DNS resolution is not performed. `--mqtt-topic` defaults to `iolink-workbench/input`; it permits 1–512 UTF-8 bytes without wildcards, controls or noncharacters. Choose a topic appropriate to the device/session; the payload also identifies its session and port. `--mqtt-interval-ms` is 10–60000, default 100. `--mqtt-queue-capacity` is 1–16384 events, default 256. Additional MQTT options require `--mqtt-broker`. Invalid host settings fail before creating the CSV destination.

This publisher supports MQTT 3.1.1 over plain TCP, QoS 0, Clean Session, DUP=false and RETAIN=false. It has no TLS, credentials, will, QoS 1/2 or subscription support. Use a broker whose endpoint accepts this contract; authentication/TLS support remains future work. A generated alphanumeric client ID stays constant across broker reconnects in one capture and differs between publisher instances. The CLI prints the requested destination to stderr; the finalized capture TOML stays on stdout. That startup message is not evidence of an accepted broker connection.

## Delivery and lifecycle

The sampler offers fixed-size input frames to an independent preallocated queue without waiting for MQTT. A full queue drops the new MQTT event and counts `queue_dropped`. A dedicated worker consumes queued inputs/control changes and replaces its pending state, counting each replaced event as `coalesced`. At most one pending state survives for the next publication. The interval is a requested minimum spacing between ordinary publish attempts, not a source/sensor cycle or delivery guarantee. A final shutdown attempt may occur before that interval expires.

Broker connect, CONNACK and writes have separate absolute 250 ms phase deadlines. Connection/handshake failures retry after 500 ms while acquisition continues. After connection, the worker consumes queued changes before sending its latest state. Socket/protocol errors abandon that connection and count `connection_losses`. A failed publication is not retried as the same message; any partial write has unknown delivery. Broker reconnects and IO-Link generation changes are different events and counters.

Keepalive is 10 seconds; after five seconds without a control packet the worker sends PINGREQ and allows one second for PINGRESP. Fragmented replies are supported. Malformed, refused or unexpected packets are rejected; no received packet can command the device. Responses and message sizes are bounded. JSON is limited to 1 MiB, with at most 128 effective mappings and names/units up to 256 UTF-8 bytes. Exported master text is capped at 256 UTF-8 bytes with a truncation flag. These bounds apply to this optional export, not ordinary offline/CSV use.

QoS 0 socket completion confirms only that the local socket accepted a complete packet. Broker and subscriber receipt remain unknown. There is no delivery acknowledgment or replay/history service. Equal input values are not suppressed by value comparison; interval coalescing and queue drops can still skip them. CSV every-update/on-change/raw-only selection does not define MQTT selection. Use CSV or the bounded API history when that evidence is needed. These protocol choices follow the [OASIS MQTT 3.1.1 specification](https://docs.oasis-open.org/mqtt/mqtt/v3.1.1/os/mqtt-v3.1.1-os.html).

On capture completion, CSV drains, a final control state is offered and MQTT drains its queue. If connected, it makes one final pending-state attempt, then sends DISCONNECT best-effort. Without a connection, the pending state is discarded and counted; shutdown starts no new connection. Already-running I/O phases retain their finite deadlines. The queue is not durable. Ordinary broker unavailability is recorded without failing CSV acquisition; a worker panic makes overall capture success false and marks unavailable worker counters.

## JSON payload

Every packet contains `format_version: 1`, Workbench version, an opaque `session_id`, UTC capture start and monotonic `host_elapsed_us`. `event_sequence` identifies the newest consumed MQTT event; `event_elapsed_us` is that event's host timestamp. `generation` refers to the source connection. `publisher_connection` counts successful broker connections; `publish_index` counts message attempts. None of these is a CSV sample index or source counter.

`sampling_active` becomes false at finish. `gateway_connected_last_observed` is the last ingested port connection state; it does not prove current liveness. `observed_master` includes accepted master/firmware/port/identity for the matching generation, otherwise null. `master_text_truncated` identifies bounded text. A dropped connection event can make status unavailable until another accepted event supplies it. Clients must check queue drops and age, not infer fresh identity from an older generation.

`input` uses the shared raw/typed telemetry decoder: receive index, freshness/classification, validity, stale/unknown state, source metadata and gaps where measurable, diagnostic code, raw hex, typed values with origins/units, decoding error, capture counts and optional CSV attempt/index. Invalid or known-stale inputs have no typed values. The `fields` array supplies bit offsets/widths, signedness/type, endian, scale/offset, units/decimals and mapping origin. Large decoded integers remain exact strings. All effective fields are exported independently of selected CSV columns.

`input_elapsed_us` and `input_host_age_us` describe the last consumed input. Disconnect/accepted reconnect events clear that input; a finished state can retain the last input with `sampling_active=false`. If a control event was dropped, only the last ingested state is known. No subsequent messages, a high age, or a false active flag must not be treated as current sensor data. There is no retained online/offline message or will to establish consumer liveness.

Declared sensor minimum cycle, reported port cycle, requested host poll, source/gateway cadence and host receive spacing remain separate. Snapshot data stays `host_poll`, with source loss/age/gateway update cadence unknown on the supported AL1350 path. Counter gaps apply within observed source generations; disconnect loss is unknown. MQTT skips do not change source-loss meaning. This feed establishes no physical 1 ms guarantee.

## Counters and verification

Payload `mqtt_counts` is a concurrent observation; the current attempt is counted before its socket outcome is known. Final `capture.toml` includes optional `mqtt` settings, publisher identity, error text, handoff/coalescing/publish/connection counts, final DISCONNECT outcome and worker-failure flag. Live counters need not form an atomic equation. Normal final accounting is:

```text
events_offered = events_dequeued + queue_dropped
events_dequeued = coalesced + publish_attempts + shutdown_discarded
publish_attempts = socket_writes_completed + publish_write_failures + encoding_failures
broker_connect_attempts = broker_connections + connect_failures
```

Queue drops, deliberate coalescing, shutdown discard, uncertain socket delivery, source gaps, API eviction and CSV overruns are separate quantities. Subscriber losses are unknown. `verify-capture` checks saved MQTT accounting when present; it does not prove delivery. Older readers may ignore the new optional report section. Broker errors remain visible after recovery through `last_error`; original sensor/profile/capture bytes are unchanged.

`tools/verify_mqtt.py` uses an original local protocol peer, synthetic IO-Link inputs and no desktop input. It covers byte-level flags, fragmented/refused handshakes, keepalive unit cases, reconnects, slow/non-reading brokers, independent API/CSV operation, stale/invalid data, snapshots, bounds and saved-accounting rejection. This is software fixture qualification, separate from production-broker interoperability, physical master/sensor timing and visible GUI acceptance.
