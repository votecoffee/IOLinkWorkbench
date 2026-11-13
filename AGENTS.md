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
- The opt-in loopback HTTP API uses independent bounded ingest/history and API loss counters; see [its contract](docs/LOCAL_API.md). Keep it separate from CSV and acyclic reads. The optional MQTT feed has its own bounded queue and latest-state coalescing; see [its contract](docs/MQTT.md). Broker delivery is unknown with QoS 0. The optional [shared-memory snapshot](docs/SHARED_MEMORY.md) uses its own queue and zero-wait publisher lock; never read mapped bytes without the named mutex. C FFI remains roadmap work.
- Update public command/profile/schema documentation and the skill when behavior changes. Follow [development](docs/DEVELOPMENT.md) for meaningful local checks and clean-commit packaging. Do not add automatic GitHub Actions spending.
- Saved identity review must distinguish configured pins, producer claims, offline rechecks and accepted/rejected observations. Common validation checks known accepted contradictions; never replace accepted identity with a rejected reconnect or invent per-generation identity history. Comparison must account for observed context and missing identity fields. See [identity evidence](docs/CAPTURE_REVIEW.md#recorded-identity-and-reconnects).
- Saved accounting checks must use available populations only. Exact CSV/source-gap checks require a complete every-update counter population without logger drops; filtered/legacy rows do not establish source loss. Keep reconnect generations separate and check local re-decoded bundles through the same accumulator. See [accounting](docs/CAPTURE_REVIEW.md#selection-reconnect-and-source-gap-accounting).
- Saved timing validation belongs in the common manifest path for all archive operations, including local export verification. Preserve missing legacy statistics, distinguish host/source clocks, and never infer full receive statistics from selected CSV rows. See [timing checks](docs/CAPTURE_REVIEW.md#recorded-timing-consistency).
- Saved capture comparison must verify both complete bundles before results, preserve missing context, and keep deltas conditional on recorded settings and count exposure. No clock alignment or automatic performance/physical verdict. Saved interface review must stay offline and use the same verified metadata/CSV pass. Keep missing metrics distinct from zero, and coalescing/retention/unconfirmed delivery distinct from source or CSV loss. See [capture review](docs/CAPTURE_REVIEW.md).
