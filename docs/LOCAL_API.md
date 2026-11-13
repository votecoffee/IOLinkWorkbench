# Read-only local API

The GUI and CLI optionally export one acquisition through HTTP/1.1 on numeric IPv4 `127.0.0.1`. The API is disabled by default and stops when that monitor/capture finishes. It shares acquired inputs, opens no additional hardware connection, performs no acyclic queries and has no control/write endpoints. These are runtime options, not saved profile keys.

## Start and discover

In Monitor & CSV, enable **Read-only local API**, choose a free port (default 8765), then start monitoring or CSV capture. From an extracted portable package:

```powershell
.\iolink-cli.exe capture samples/simulator.toml trial-api --seconds 30 --api-listen 127.0.0.1:8765
```

The actual URL/session go to stderr; the finalized TOML report remains on stdout. Port 0 chooses a free port. `--api-queue-capacity N` and `--api-history-capacity N` set independent bounds of 1–16384 events (defaults 1024 and 4096); both require `--api-listen`. Non-loopback/IPv6 addresses, invalid bounds and unavailable ports fail before creating the capture directory. An API worker failure makes capture success false and is recorded separately from transport/CSV errors.

Any local process able to reach the socket can read telemetry, including device identity, while enabled. There is no authentication token or remote binding. Requests require the bound numeric `Host: 127.0.0.1:PORT`; any `Origin` header is rejected. There is no browser CORS contract. The session ID identifies cursor lifetime and is not a secret. Proxy deployments would need separate authorization design.

## Endpoints and cursor use

| GET path | Body |
|---|---|
| `/v1/schema` | Effective mappings/origins, input width, configured gateway/poll interval, declared communication, logging settings and clock/row semantics |
| `/v1/status` | Latest ingested event, host age, accepted master/port/identity for the same generation and interface counters |
| `/v1/history` | First retained page, default limit 64 |
| `/v1/history?after=N&session=ID&limit=128` | Events after API sequence N in that session; limit 1–128 |

Responses contain `format_version: 1`, `session_id`, `capture_utc_start`, current monotonic `host_elapsed_us`, and `body`. Data appears under `body.schema`, `body.latest_event` or `body.events`. Configured endpoints, passwords and vendor-file paths are omitted. Numeric decoded values remain strings to retain exact 64-bit values.

Begin without a cursor. Save the opaque URL-safe session ID and send `body.next_after` with `session` on subsequent pages. Both cursor keys must appear together. A different session returns 409; future cursors, invalid/duplicate keys or invalid limits return 400. Empty pages retain the cursor. Follow `more_available`; `size_limited` means byte-size pagination stopped early, leaving the next event available while retained. Do not reuse cursors silently across sessions.

`metrics.oldest_sequence`/`newest_sequence` identify retained bounds. `missed_before_oldest` counts sequence positions before the oldest retained event after the requested cursor. This can include API queue drops and history eviction; it is not source loss. Check individual sequence gaps as well as aggregate counters. A client's exact losses cannot be inferred from total eviction alone.

## Event and freshness semantics

Every event contains API `sequence`, monotonic `host_elapsed_us`, reconnect `generation`, and `data`. Kinds are `connected`, `input`, `disconnected` and `capture_finished`. Connected status comes from the acquisition's accepted identity. Exported status text is capped at 256 UTF-8 bytes; `text_truncated`/`master_text_truncated` reports truncation without changing original checks/captures. Master metadata is withheld when it belongs to a different generation, including when a connection event was dropped.

Input events represent every normalized input response offered by acquisition, including duplicate/late replies, independently of CSV modes/columns. `classification` is `first`, `forward`, `duplicate`, `out_of_order` or `host_poll`. `receive_index` counts responses. A late reply is historical data, not a new current value. `csv_sample_index`/`csv_enqueued` describe a selected CSV attempt, including logger rejection; null means no CSV selection or monitor-only operation. API sequence and CSV sample index differ.

Inputs retain raw hex, validity, stale/unknown state, available source counter/time/age, diagnostic code, source gaps, capture counts, typed values and decoding errors. Typed values cover all effective fields with origin; invalid or known-stale inputs have no typed values. Unknown staleness stays unknown. Schema mappings expose bit offset/width, signedness/type, byte order, scale/offset, units and decimals.

Equal values can be distinct forward updates. Snapshot rows stay `host_poll`; source loss, source age and gateway cadence remain unknown on the supported AL1350 path. Counter gaps apply only within observed generations, not during disconnects. API gaps do not change that meaning. Event host age is not sensor age. UTC start anchors the session's monotonic clock; source clocks retain their transport/generation domain. Declared sensor cycle, reported port cycle, requested host interval and measured cadence remain separate. There is no physical 1 ms guarantee.

## Bounds, loss and shutdown

The sampler offers fixed-size input frames to an independent preallocated queue without HTTP work, decoding, disk I/O or waiting for clients. A full queue drops the new API event and counts `queue_dropped`; CSV/source accounting is unchanged. A separate ingest worker maintains bounded preallocated history. HTTP serialization/history-lock contention can delay ingestion and cause API drops while acquisition continues. History replacement counts `history_evicted`. Control events share API sequence/drop accounting and use bounded text copies.

One HTTP worker serves one accepted connection at a time. Headers have a 4096-byte limit and absolute 250 ms read deadline; response writes have a separate absolute 250 ms deadline. Replies are at most 1 MiB; history pages also respect this size. Bodies/pipelining are unsupported. All replies close the connection and disable caching. Other methods return 405. Export allows at most 128 effective fields with names/units up to 256 UTF-8 bytes and rejects oversized schema. These bounds apply only when the API is enabled.

Live counters are concurrent observations, not an atomic equation. After normal drain, saved `local_api.events_offered = events_ingested + queue_dropped` and `history_evicted = max(0, events_ingested - history_capacity)`. `capture.toml` also saves URL/session, capacities, requests, rejected/failed requests and `worker_failed`. Capture verification checks accounting when present; older readers may ignore it. It does not prove client delivery. HTTP rejection/timeouts are separate from source/CSV loss.

At finish CSV drains, a final API event is offered, API events drain and socket workers stop. Control timestamps can extend into final drain time. The listener is not a persistent saved-capture service.

`tools/verify_local_api.py` qualifies synthetic gateway/loopback clients, including stale/invalid/duplicate inputs, equal updates, reconnects, cursor bounds, slow headers, independent CSV overruns and shutdown. Rust tests add deterministic queue overflow and response-size pagination. Physical timing and visible GUI interaction remain separate. MQTT, shared memory and C FFI remain [future work](INTERFACES.md).
