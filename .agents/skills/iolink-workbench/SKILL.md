---
name: iolink-workbench
description: Configure, operate and develop IOLinkWorkbench for IO-Link description import, offline decoding, read-only acquisition and capture review. Use for this application's CLI, profiles, timing, integration plans and Windows packaging.
---

# IOLinkWorkbench

Locate the user's checkout or extracted portable package. Read its `AGENTS.md` when working in source, then the relevant maintained document: `docs/OPERATIONS.md` for setup/CLI/mappings/capture review, `docs/DEVELOPMENT.md` for source flow/tests/releases, or `docs/INTERFACES.md` for planned consumers. The root README holds detailed options and the AL1350 protocol references. Prefer the executable's `--help` over guessing flags. This skill is useful with either binary-only packages or a Rust checkout.

## Choose the workflow

- For device descriptions, inspect XML/ZIP with `iolink-cli iodd`, choose an exact variant/layout, then `import` to a new profile. Resolve local standard/language/unit resources explicitly. Preserve review notes and imported definitions; import does not verify connected identity. Vendor files and raw captures belong in ignored `.local/` in source checkouts.
- For manual setup, use `validate`, `set-connection`, `set-identity`, `set-field` and `remove-field`. These edit host profiles offline. Full input-payload `decode` previews bit offsets, width, signedness/type, byte order, scale and units. An imported name becomes an override; removal restores it. Never invent unknown scaling/units.
- For logging, use `set-logging` and offline `logging` to inspect mode, bounded queue, row/time rotation, selected mappings and exact CSV headers. Selection can be raw-only, all mapped or explicit ordered names. New profile destinations must not exist; unspecified settings remain intact. Reset dangling explicit CSV selections before removing a standalone field.
- For acquisition, use an existing configured port and the current session's authorized endpoint. Check `status` and separate expected pins/IODD from observed identity. `capture` writes a new directory; `verify-capture`, `review-capture`, `redecode-capture` and `verify-export` operate on saved evidence. Keep originals unchanged.
- For saved interface diagnostics, use `review-interfaces DIRECTORY` or GUI Verify capture. Read `docs/CAPTURE_REVIEW.md`: all CSV parts are verified, raw-only captures work, exact saved interface counters/hashes are retained, and no live endpoint/mapping is opened. Missing metrics are not zero. Intentional coalescing, retention and uncertain delivery must not become source-loss totals. Offer counts are checked against acquisition only when the needed counters exist. Field review includes the same diagnostics when recorded.
- For offline comparisons, use `compare-captures LEFT RIGHT` or GUI Compare two saved captures offline. Both folders are verified; complete reviews and metadata hashes remain available. Read the comparison policy in `docs/CAPTURE_REVIEW.md`: no clock alignment or physical-origin inference, timing deltas require matching recorded context, counts also require identical duration, and missing/error context suppresses deltas. Keep each loss domain separate. Use `tools/verify_comparison.py` with explicit saved fixtures.
- For parameters, `parameters`, `defaults` and `decode-parameter` are offline. Explicit `read-parameter --snapshot`, `read-diagnostics` and `isdu` use independent acyclic clients. `review-parameter` reopens embedded context offline. A declared default is not a device value or a write instruction.

## Non-obvious constraints

No master/sensor/signal has been selected in the initial project. ifm AL1350 IoT Core 3.1.x is a documented adapter target, not physical acceptance. Confirm selected hardware's firmware/interface/Windows support and license before adding an SDK or claiming compatibility.

Keep transport, pure decoding, cyclic capture, independent acyclic reads and CSV writer responsibilities separate. Source-update mode retains equal consecutive values only when freshness metadata verifies a new update. AL1350 snapshot rows are host polls; source-update loss, source age and gateway update cadence remain unknown. Do not infer freshness from value changes. On-change compares complete raw input plus quality/diagnostic changes, independent of typed-column selection.

Compare declared sensor minimum, reported port cycle, requested host polling, source/gateway cadence and host receive spacing separately. Treat 1 ms as an evaluation target. Report p50/p95/p99/longest gaps, stale/invalid/unknown data, reconnect boundaries, measurable source gaps and logger overruns. Preserve monotonic receive times, UTC start, indexes and bounded queue accounting. Distinguish intentional filtering from loss.

Changes to port mode/configuration, device commands, ISDU writes and process outputs require explicit user approval for the proposed operation. Current software is read-only; host profile edits do not need that hardware approval. Use session authorization for physical reads. Desktop input and screenshots/recording need an explicit request in the current chat. The hidden native self-test uses only its own process, no input or pixel readback.

## Changes and delivery

Use `docs/DEVELOPMENT.md` for actual commands: Cargo formatting/tests/Clippy, CLI-only build, `tools/verify_headless.py`, capability-checked `tools/verify_gui.py` and `tools/package_windows.py`. Keep simulator, HTTP loopback, hidden native GUI and physical results distinct. Do not launch old GUI binaries to probe arguments. Test extracted portable binaries and prior CLI compatibility when changing stored schemas.

The opt-in local HTTP API runs only during acquisition: GUI Read-only local API or CLI capture `--api-listen 127.0.0.1:PORT` (0 chooses a port). Read `docs/LOCAL_API.md` for schema/status/history, numeric Host policy, session cursors, bounds and independent loss/age. It exports all normalized responses before CSV filtering, including duplicate/late classifications. Never equate API sequence, receive index and CSV sample index. Use `tools/verify_local_api.py` for synthetic/loopback checks. Runtime API options are not profile keys.

Optional MQTT publishing uses a separate latest-state worker: GUI checkbox or CLI capture `--mqtt-broker IP:PORT`. Read `docs/MQTT.md` for interval/topic/queue bounds, anonymous TCP QoS 0, no retained messages, coalescing, reconnect/shutdown and saved counters. Socket completion is not delivery confirmation. Use `tools/verify_mqtt.py` for the local protocol peer and synthetic input checks. Runtime MQTT options are not saved profile keys.

Optional Windows shared memory uses a separate queue and latest-snapshot worker: GUI checkbox or CLI capture `--shared-memory NAME`. Read `docs/SHARED_MEMORY.md` for Local object names, protected DACL, fixed little-endian header, named-mutex synchronization, raw/typed JSON, age/PID-creation checks, collisions and shutdown. `read-shared-memory NAME` needs no profile/hardware; the Python reference reader is in `examples/shared_memory_reader.py`. A reader holding the mutex causes publisher skips, not acquisition waits. Do not equate commit sequence/replacements with source loss or confirmed reader delivery. Check age/process state even when an old snapshot says active. Use `tools/verify_shared_memory.py` for native/synthetic checks. Runtime options are not saved profile keys.

Optional C FFI remains roadmap work. Consult the interface document before implementing; each consumer needs an independent bounded handoff, visible loss/age and no sampler blocking. Evaluate whether a C# host needs hardware-connection ownership before choosing FFI. Reference applications' capabilities are not capabilities of this app.

Update operator/developer docs and this skill with public behavior. Source samples must be synthetic/sanitized. Preserve unrelated work and read-only references; package from a clean verified commit. Publish commits only when the session authorizes it; this skill grants no external-write permission. Avoid automatic GitHub Actions spending.
