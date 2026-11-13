# Integration interfaces

Current integration includes the CLI, saved profiles/snapshots/CSV and an opt-in [read-only loopback HTTP API](LOCAL_API.md). The API has bounded independent ingest/history, session cursors and loss accounting. The remaining interfaces below are planned, not implemented. They must preserve the read-only hardware contract and cannot turn into configuration/control endpoints implicitly.

## Planned consumers

| Interface | Intended contract | Acceptance criteria |
|---|---|---|
| MQTT publishing | Optional telemetry feed with explicit delivery/retention policy | Document version, QoS, topic/schema identity, reconnect and retained-message meaning; bounded independent queue and visible publish/drop counters |
| Windows shared memory | Versioned latest raw snapshot with coherent status/identity/time metadata | Specify names/ACLs, ABI layout, synchronization, reader consistency and lifecycle; expose sequence/age so overwrite or missed observations are visible |
| Optional C-compatible FFI | Embedding in C# or other native hosts | Evaluate connection ownership first; specify ABI versioning, handles/lifetimes, thread/error/cancellation rules and panic containment before implementation |

For each consumer report independently: source metadata/gaps where measurable; snapshot uncertainty; acquisition/reconnect generations; logger queue drops; interface queue drops/history eviction or latest-value replacement; subscriber reconnects; consumer observation age. A host receive timestamp has a monotonic clock domain within a capture/session; include UTC start and an explicit session ID rather than implying a portable absolute monotonic epoch. Unknown source update loss remains unknown even when every host snapshot is delivered.

Consumers need bounded preallocated handoffs and bounded serialization/message sizes. They cannot hold sampler locks, trigger implicit acyclic queries or share logger backpressure. A disconnected subscriber must not stall acquisition or CSV. Define explicit shutdown/drain behavior, schema negotiation and validity/stale/diagnostic representations. Preserve equal successive source updates if the chosen delivery contract claims every update; a latest-value feed must declare its lossy nature. On-change filtering must remain distinct from transport loss.

## Reference and qualification

[EthernetIPWorkbench](https://github.com/votecoffee/EthernetIPWorkbench) provides a read-only design reference: bounded local HTTP history, MQTT 3.1.1 TCP QoS 0 latest-value publishing and a Windows shared-memory latest raw snapshot. IOLinkWorkbench uses its own HTTP schema; MQTT/shared memory remain future work here. Its optional C FFI was discussed, not implemented, when this roadmap was added.

Before choosing FFI, determine whether the C# application must own the hardware connection or can consume a separate Workbench process. Compare lifetime, driver/SDK licensing, process isolation and deployment needs. Do not assume either ownership model. If embedding is chosen, no unwind may cross the C boundary; define pointer ownership, buffer sizing, callback reentrancy and cancellation explicitly.

Qualify each interface with localhost simulations covering equal updates, stale/invalid data, reconnects, slow/disconnected consumers, overflow, shutdown and malformed requests. Measure acquisition and logger timing with feeds disabled/enabled. Keep those results separate from physical bench latency/loss testing. Hardware selection and documented SDK/API/fieldbus support remain prerequisites to any new master adapter.
