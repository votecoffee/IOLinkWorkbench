# Roadmap

## Windows preview

- Shared one-port capture engine, portable GUI and CLI, saved profiles, manual mapping, and offline IODD inspection and decoding.
- Headless checks for equal consecutive updates, change-only recording, snapshot labeling, invalid and stale inputs, reconnects, queue backpressure, and CSV rotation.
- Capture reliability checks cover frozen counters with zero reported age, retrying dropped changes, and reporting unconfirmed rows after a CSV rotation failure.
- IODD checks cover display scaling, explicit no-unit metadata, ambiguous variant and datatype IDs, and overlapping supported input fields.
- Offline capture verification checks rotated CSV parts against the saved profile and report, with optional combined CSV output. Rejected reconnects retain the accepted capture identity and record the rejected candidate separately.
- Documented ifm AL1350 IoT Core adapter with an isolated JSON loopback fixture. Physical acceptance begins once the master and sensor are selected.

## First physical bench

1. Record the master model, hardware and firmware revision, IoT endpoint, actual configured port mode, sensor model, and IODD revision. Record the intended signal and unit.
2. Verify the device IDs and selected variant, active process-data layout, width, validity behavior, and a known value against the vendor's viewer or another independent read. Record the negotiated IO-Link cycle and the source that confirms its unit.
3. Capture a timed trace at the requested 1 ms host interval and at a sustainable interval. Compare host poll and receive distributions, invalid data, terminal silence, retries, and writer overruns. For AL1350 snapshots, report the gateway update rate and source loss as unknown. For an adapter with per-update metadata, compare source counters and timestamps independently of the host.
4. Record a separate capture with an explicit ISDU read on its own worker and measure any gateway-side effect on host timing. Keep real bench results separate from simulator and loopback results.

## Follow-on work

- Add one documented source-counter or timestamp transport once selected hardware exposes it. Qualify timestamp clock, counter wrap, update semantics and firmware-specific behavior.
- Add multiple independent port sessions with per-port identity, status, and capture accounting.
- Expand IODD handling for arrays, external language files, profile snippets, unit definitions and schema validation. Add representative sanitized fixtures for supported device families.
- Consider any configuration or output features only after explicit authorization for the specific operation and bench setup.
