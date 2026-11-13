# Roadmap

## Windows preview

- Shared one-port capture engine, portable GUI and CLI, saved profiles, manual mapping, and offline IODD inspection and decoding.
- Headless checks for equal consecutive updates, change-only recording, snapshot labeling, invalid and stale inputs, reconnects, queue backpressure, and CSV rotation.
- Capture reliability checks cover frozen counters with zero reported age, retrying dropped changes, and reporting unconfirmed rows after a CSV rotation failure.
- IODD checks cover display scaling, explicit no-unit metadata, ambiguous variant and datatype IDs, and overlapping supported input fields.
- Offline capture verification checks rotated CSV parts against the saved profile and report, with optional combined CSV output. Rejected reconnects retain the accepted capture identity and record the rejected candidate separately.
- Explicit IODD variable catalogs persist names, indexes, access rights, defaults, choices, and ranges. Named scalar and text parameter reads use the acyclic client, with raw payloads retained on decoding errors.
- Optional local IODD standard definitions resolve referenced parameters into saved profiles. Numeric menu displays retain separate scaling, units, formatting, and raw values. The 22 official example XML files imported with 329 resolved standard references and 15 supported parameter display mappings in an offline check.
- Parameter records preserve member layouts, defaults, access restrictions, standard record references, and menu display metadata. Whole-record and named subindex reads share the offline ISDU decoder. An offline import of the 22 official XML examples retained 46 records and 406 members. Synthetic and loopback checks cover member reads, whole-record decoding, malformed members, read restrictions, and 232-byte payloads.
- Top-level cyclic arrays expand into named typed/CSV fields. Acyclic arrays preserve element metadata and support whole-array and permitted subindex reads, with per-element display scaling and standard count restrictions. The 22 official examples imported with 24 array definitions and 139 elements after standard restrictions. Synthetic and loopback checks cover packed ordering, octet boundaries, raw sign padding, array CSV fields, malformed responses, and read restrictions.
- Optional local XSD 1.0 validation checks the selected main XML against a supplied schema closure using Windows System.Xml. Imports can require a pass and persist XML/schema hashes. GUI inspection uses a background worker. All 22 official example IODDs passed against the supplied 2025 schema set. Tests cover failed-import preservation, local includes, blocked external resolution, DTDs, schema hints, receipts, and helper timeouts.
- Documented ifm AL1350 IoT Core adapter with an isolated JSON loopback fixture. Physical acceptance begins once the master and sensor are selected.

## First physical bench

1. Record the master model, hardware and firmware revision, IoT endpoint, actual configured port mode, sensor model, and IODD revision. Record the intended signal and unit.
2. Verify the device IDs and selected variant, active process-data layout, width, validity behavior, and a known value against the vendor's viewer or another independent read. Record the negotiated IO-Link cycle and the source that confirms its unit.
3. Capture a timed trace at the requested 1 ms host interval and at a sustainable interval. Compare host poll and receive distributions, invalid data, terminal silence, retries, and writer overruns. For AL1350 snapshots, report the gateway update rate and source loss as unknown. For an adapter with per-update metadata, compare source counters and timestamps independently of the host.
4. Record a separate capture with an explicit ISDU read on its own worker and measure any gateway-side effect on host timing. Keep real bench results separate from simulator and loopback results.

## Follow-on work

- Add one documented source-counter or timestamp transport once selected hardware exposes it. Qualify timestamp clock, counter wrap, update semantics and firmware-specific behavior.
- Add multiple independent port sessions with per-port identity, status, and capture accounting.
- Expand IODD handling for external language files, profile snippets, unit definitions, and IODD Checker semantic/stamp rules. Add representative sanitized fixtures for supported device families.
- Expand parameter decoding for nested layouts and additional display formats. Add explicit display selection when a parameter has multiple menu representations.
- Consider any configuration or output features only after explicit authorization for the specific operation and bench setup.
