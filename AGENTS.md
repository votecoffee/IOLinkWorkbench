# Working on IOLinkWorkbench

Start with the repository skill at [.agents/skills/iolink-workbench/SKILL.md](.agents/skills/iolink-workbench/SKILL.md). Use [operations](docs/OPERATIONS.md) for setup and CLI workflows, [development](docs/DEVELOPMENT.md) for ownership and verification, and [interfaces](docs/INTERFACES.md) for integration plans.

## Scope

- Keep transport acquisition separate from IODD/manual decoding and CSV writing. Explicit acyclic reads use an independent client/worker; they must not hold the cyclic sampler's locks.
- The initial scope is one master, one configured port and one sensor. The ifm AL1350 adapter is a documented target, not evidence that bench hardware has been selected or accepted.
- Preserve existing profiles, recorded bytes, sample indexes and schema compatibility. Imported fields remain beneath manual overrides. Host profile edits do not configure a device.
- Do not change port mode/configuration, issue device commands, write ISDU parameters or send process outputs without the user's explicit approval for the proposed operation. Current implementation is read-only. Routine offline editing, tests and packaging need no extra approval.
- Do not interpret code changes or simulator tests as authorization to connect to physical hardware. Use the current session's hardware scope.
- Desktop input and desktop/window screenshots or recording require an explicit request in the current chat. The hidden native self-test operates only on its own process and performs no input injection or pixel readback.

## Evidence and delivery

- Check status, branch, remote and applicable instructions before editing; preserve unrelated changes and read-only references.
- Keep vendor IODDs/SDKs, private profiles, raw captures and local receipts under ignored `.local/`. Commit only original synthetic or explicitly cleared fixtures. Check ignore behavior before storing private material.
- Distinguish declared port cycle, gateway update rate, source timestamps/counters, host poll/receive spacing and logger timing. Snapshot rows are host polls; absent freshness metadata means source loss is unknown. Keep simulator, loopback, offscreen GUI and physical results separate.
- The opt-in loopback HTTP API uses independent bounded ingest/history and API loss counters; see [its contract](docs/LOCAL_API.md). Keep it separate from CSV and acyclic reads. The optional MQTT feed has its own bounded queue and latest-state coalescing; see [its contract](docs/MQTT.md). Broker delivery is unknown with QoS 0. Shared memory and C FFI remain roadmap items.
- Update public command/profile/schema documentation and the skill when behavior changes. Follow [development](docs/DEVELOPMENT.md) for meaningful local checks and clean-commit packaging. Do not add automatic GitHub Actions spending.
