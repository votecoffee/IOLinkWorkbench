# Developing IOLinkWorkbench

Read [AGENTS.md](../AGENTS.md) and the [operations guide](OPERATIONS.md) before changing a public workflow. The package is Rust with an eframe/glow desktop binary and Clap CLI. Profiles remain version 1; preserve compatibility deliberately rather than silently changing stored meaning.

## Ownership and flow

| Area | Responsibility |
|---|---|
| `src/transport*` | Read-only adapter contracts, ifm HTTP snapshots and synthetic gateway; observed status/identity/quality/freshness |
| `src/iodd*`, `src/decode*` | Bounded local description import, declared metadata and pure raw/typed decoding |
| `src/profile*` | Saved host configuration, imported/manual precedence, validated candidates, identity pins, preferences and offline capture plans |
| `src/capture*` | Cyclic acquisition, receive timestamps, selection, bounded queue, writer, finalized reports and timing/loss accounting |
| `src/local_api*` | Optional read-only loopback server, independent queue/ingest/history, request bounds, typed rendering, session cursors and API loss accounting |
| `src/mqtt*`, `src/telemetry.rs` | Independent latest-state publisher, bounded protocol handling and MQTT accounting; shared pure raw/typed telemetry rendering |
| `src/shared_memory*` | Windows mapping/mutex ownership, explicit ABI, bounded latest-state handoff, native read helper and independent commit/skip accounting |
| `src/archive*` | Offline capture verification, bounded review and re-decode exports |
| `src/parameter_snapshot*`, diagnostics | Explicit acyclic results and offline reopening with embedded context |
| `src/bin/iolink-cli*`, `src/desktop*` | Shared core operations exposed through CLI and GUI; pending GUI drafts guard acquisition |

An applied profile selects a transport and effective mapping. The cyclic worker reads, timestamps and validates an input, accounts for freshness/selection and submits a bounded preallocated queue entry. The independent writer batches CSV, rotates, flushes/syncs and drains on stop. Queue saturation is counted rather than blocking sampling indefinitely. An explicit acyclic query creates its own client/worker. Decoding and offline plans must not open transports. Any future integration gets its own bounded handoff and loss accounting; see [interface criteria](INTERFACES.md).

The [local API](LOCAL_API.md) is optional per acquisition. The sampler offers fixed input frames; separate ingest/HTTP workers own history and serialization. API responses include input duplicates/late replies independently of CSV filtering. Export queue drops, history replacement, source gaps and logger overruns have distinct counters. API report fields are optional so old captures remain readable; archive verification validates recorded API accounting when supplied. The local API suite uses simulator/loopback clients and measures no physical acceptance.

Capture plans describe saved configuration, never observed transport capabilities. Selected field positions are one-based value order; `csv_columns` contains the actual full header including metadata. Rotation follows receive elapsed timestamps, not writer wall time. Preserve raw payloads and the distinction between source gaps, host polls, intentional filtering, reconnect uncertainty and logger drops. Do not infer sensor freshness from equal/different payloads.

`archive::review_interfaces` uses the same single-pass `inspect` path as verification/review/export, retaining the exact parsed profile/report bytes for hashes. It presents optional recorded interface summaries and availability separately from acquisition/CSV data. Offer totals are checked against acquisition events when receive/disconnect counters exist; legacy absence remains unavailable. The GUI's Verify action and optional field-review diagnostics share this interpretation. No live transport or IPC reader belongs in this path. See [offline review](CAPTURE_REVIEW.md).

`archive::compare_captures` uses one complete inspection per side and only then constructs descriptive setting/metric comparisons. Required recorded settings include host connection, identity expectations, input width, effective mapping/IODD selection, logging, freshness/route, simulation controls and exporter configuration. Do not add clock alignment, source provenance inference, automatic benchmark verdicts, or loss-domain aggregation. Missing metrics remain unavailable and elapsed duration gates count differences. Shared `check_manifest` invokes `archive::timing_validation` for every archive entry point, including local re-decoded bundle verification. `Spacing::validate` checks aggregate ordering and finite/range-consistent frequency. Optional acquisition counters gate interval-count crosschecks; absent legacy counters stay unknown. Host intervals fit the acquisition window, while source clocks are independent. Do not reconstruct these aggregates from selected CSV rows. Run `python tools/verify_timing.py CLI COUNTER_CAPTURE SNAPSHOT_CAPTURE` on explicit saved fixtures; its first bundle needs all four timing domains and typed fields. See the [comparison policy](CAPTURE_REVIEW.md#compare-two-captures). Run `python tools/verify_comparison.py CLI CAPTURE_A CAPTURE_B` with explicitly chosen synthetic/recorded bundles; this suite opens no live service and preserves originals.

`archive::identity` parses optional saved master/device observations without opening a transport. Common manifest validation rejects known accepted identity/pin contradictions; rejected statuses may disagree and remain separate. Interface and field review share one optional identity result. Comparison requires observed model/firmware/port/mode plus vendor/device IDs, compares other supplied identity fields too, and leaves missing serials explicitly unresolved. No per-generation identity journal is inferred. Run `python tools/verify_identity.py CLI CAPTURE_A RAW_CAPTURE` on explicit saved fixtures; the first needs typed fields and an observed serial without a configured serial pin.

`archive::accounting::validate` checks recorded lifecycle and input-selection populations in the common manifest path. Its constant-memory `Rows` accumulator runs after row/value validation in both original inspection and local re-decoded bundle verification. It only compares counter-gap sums when source mode, every-update selection, zero CSV overruns and matching new-update/selected populations establish complete observed coverage. Never compare across generation boundaries or normalize filtered gaps into source loss. Run `python tools/verify_accounting.py CLI COMPLETE_COUNTER ON_CHANGE DROPPED_ROWS SNAPSHOTS` on explicit saved synthetic fixtures; the first needs typed fields and complete coverage.

`archive::quality` validates optional received-input quality/classification counters and the captured source stale threshold. Its constant-memory row accumulator is composed into `accounting::Rows`, so original inspection and local export verification share subset/equality rules. Never add validity and staleness together: they overlap. Equality requires zero logger overruns and recorded received count equal to selected count. Apply source age/threshold checks using the saved source profile, not revised mapping preferences. Run `python tools/verify_quality.py CLI FAULT_CAPTURE FILTERED DROPPED SNAPSHOTS`; the first needs typed fields, invalid rows and an explicitly aged valid/not-stale row.

`desktop::shutdown` counts every GUI worker domain and models idle/wait/cancel/close decisions before controls render. The normal-close path requests cyclic stop, polls all owners even with their tabs hidden, and emits close only after completion. Capture failures during close cancel exit for error inspection. `parameters::Saved` completion belongs in the frame-wide update path, not only its view. Run `cargo test --locked --bin iolink-workbench desktop::shutdown`; tests use logical requests and empty egui input, plus an actual simulator capture/flush. The GUI-worker test mutex isolates real test dispatch from the self-test denial guard. See [scope and operator checks](GUI_LIFECYCLE.md); visible/native close routing remains unverified.

`desktop::observation` assesses the held GUI observation using a supplied monotonic instant. Keep receipt quality/age unchanged, advance host timers, and include both publication/repaint intervals in the display-age allowance so fast source thresholds do not create false UI silence. `capture::LIVE_REFRESH_INTERVAL` preserves the existing 80 ms publication cadence and supplies the UI repaint interval. Stopped/held/unusable observations retain raw bytes while hiding typed values; snapshots never acquire an invented source timer. Five deterministic tests and six synthetic text/geometry cases cover these decisions; see [live view](LIVE_VIEW.md).

`desktop::workspace` owns persistent actions, focused page navigation and profile/connection-review windows. It polls workers before all views and preserves per-page scroll positions. `desktop::workflows` renders live input, logging, diagnostics and explicit parameter tools without changing transport/capture ownership. See [GUI workflow](GUI_WORKFLOW.md). The workspace tests check normal/minimum-size isolation, overview routing, starter/file replacement guards and source preservation, navigation and actual cross-tab simulator flush with offline bundle verification. `desktop::setup::profiles` owns explicit starter replacement and file controls; `desktop::setup::overview` only routes host setup tasks and must never dispatch reads or claim connected readiness.

`desktop::file_drop` consumes egui drop metadata after global completion polling. It only stages a profile/IODD path (Open and Save destinations remain separate), reusing acquisition admission plus the independent parameter-read fence; it must never read files or dispatch workers. A new IODD path invalidates the old inspection and ZIP selection without applying a profile. Synthetic egui tests cover routing, source preservation, explicit load/inspect/apply, owner/draft rejection and hover bounds. Native offscreen painting covers the hover preview; actual Explorer/OS routing remains a visible acceptance check.

The GUI tracks the absolute output path of an owned recording and keeps the last published recording available independently of the live session report. Promote the path only when the recording returns a report after publishing `capture.toml`. The review shortcut stages the directory without dispatching work. Switching sources clears the previous combined CSV and re-decode destinations.

Saved capture Verify, Signal and Exports each hold their own result and share the source folder and the file-review admission check. Compare holds its own inputs and worker. Poll all completions before rendering any page. Changing the shared source invalidates dependent results and destination selections. Each page has a separate scroll ID.

The named-parameter picker shares search and selection logic between live reads and offline decoding. Each view holds its own query and selection. Search is a view filter and keeps the chosen parameter and subindex until the operator selects a different parameter. Rendering details and searching must stay free of transport calls.

The GUI has one acyclic worker with an immutable query label and a separate last result for each action. Poll completion globally so switching pages does not affect ownership. Profile replacement clears the results. Keep acyclic admission independent of the cyclic sampler.

Connection review and Logging use the local acquisition route check before presenting Start. The entry point repeats the check before preparing output paths or dispatching a worker. The origin and HTTPS rules are shared with the AL1350 client, and the IODD confirmation check is shared with the capture worker. Keep stored templates loadable while their route is incomplete.

The GUI feed review builds enabled HTTP, MQTT and shared-memory configurations using the services' pure validators. Start transfers those configurations to the worker after the route and feed checks pass. Keep local review free of socket binding, broker connections and mapping creation.

Capture verification keeps the saved typed field definitions from its checked profile for the GUI picker. The list follows saved CSV column order. Its in-memory accessor leaves the serialized review format intact. A source change or failed verification withdraws the prior field list, and a signal review verifies the capture as it plots the selected field.

In the saved capture workspace, a drop of capture.toml or receipt.toml stages the file's containing directory. This selection uses the saved review worker's admission check and may run during unrelated acquisition or profile editing. It does no file access. Receipt drops select local bundle verification and clear the prior optional original-source selection.

GUI destination checks run on Check folder or Start, using the same resolved path for the status and writer. Keep filesystem inspection out of ordinary rendering. Defer creation of the default parent to the worker and preserve the writer's exclusive directory creation for collisions after the check.

Inspect bytes offline copies the same observation and applied profile used by the rendered Inputs frame. Its state is independent of the sampler, live readout and pasted payload draft. Preserve receipt metadata and label copy-time age separately from current live age.

## Local validation

GUI `start_block_reason` supplies both button availability and the start method's preflight check. Keep owned cyclic results (including ready but unread completions), closing and pending profile work fenced before state/output preparation. Prepare fresh stop/display queues locally and commit them only after worker launch succeeds; independent acyclic/review jobs must not become sampler admission locks. Run `desktop::self_test::start_tests` for guard-denied dispatch, completion ownership, draft/inspection guards and consecutive simulator capture/byte-preservation checks. See [session startup](GUI_LIFECYCLE.md#starting-a-session).

`Manifest.time_since_last_receive_us` retains the existing producer field as optional. Common timing validation checks host-window/zero-receive contradictions; row validation applies the final-receive upper bound. `accounting::Rows` retains the final host row time and invokes the timing coverage check for complete received populations, including local exports. Preserve the one-microsecond truncation allowance and avoid exact reconstruction from filtered/dropped/legacy populations. Review exposes the optional age; its comparison metric uses duration-gated context. Run `tools/verify_terminal_age.py CLI COMPLETE FILTERED DROPPED SNAPSHOT` with explicit saved fixtures; see [terminal receive age](CAPTURE_REVIEW.md#terminal-host-receive-age).

Use a stable x64 MSVC Rust toolchain with Visual Studio C++ Build Tools and Windows SDK. The repository pins dependency resolution through Cargo.lock and static CRT through `.cargo/config.toml`. Run from the checkout root:

```powershell
cargo fmt --all -- --check
cargo test --locked --quiet
cargo clippy --locked --all-targets -- -D warnings
cargo check --locked --no-default-features --bin iolink-cli
cargo build --locked --release --target x86_64-pc-windows-msvc --bins
python tools/verify_headless.py target/x86_64-pc-windows-msvc/release/iolink-cli.exe
python tools/verify_writes.py target/x86_64-pc-windows-msvc/release/iolink-cli.exe
python tools/verify_local_api.py target/x86_64-pc-windows-msvc/release/iolink-cli.exe
python tools/verify_mqtt.py target/x86_64-pc-windows-msvc/release/iolink-cli.exe
python tools/verify_shared_memory.py target/x86_64-pc-windows-msvc/release/iolink-cli.exe
python tools/verify_cli_shutdown.py target/x86_64-pc-windows-msvc/release/iolink-cli.exe
python tools/verify_gui.py target/x86_64-pc-windows-msvc/release/iolink-workbench.exe --output .local/gui-check-NEW.json
```

Use a new GUI receipt path. The headless suite creates a fresh ignored directory and exercises synthetic IODDs, recorded decoding, standalone Boolean bytes, integer padding, localized cyclic names with Boolean CSV values, 32-byte cyclic octet strings, cyclic UTF-8 and US-ASCII fields, exact cyclic temporal ticks, conditional layout plans and held or whole-value selector reads, localhost HTTP responses, stale/invalid data, reconnects, independent held acyclic reads, offline diagnostic snapshot review and comparison, concurrent cyclic port reads, writer backpressure, rotations and offline verification. It does not contact a physical bench. New command tests should verify observable preservation/rejection/CSV behavior rather than mirror implementation text.

The [CLI interruption suite](CLI_LIFECYCLE.md) creates a new hidden Windows console and contains its worker/CLI descendants in a kill-on-close job before allowing test dispatch. Exact owned-PID and hidden-window checks precede each console event. It verifies actual Ctrl+C and repeated Ctrl+Break, rotated/pressured writer drain, an in-flight loopback timeout, concurrent port drains, and failure exit precedence. It sends no desktop input or event to an existing console. A passing private receipt requires normal process exits, report/CSV accounting and original-profile/destination preservation. Terminal keyboard routing, console close, shutdown, crash and physical acceptance remain separate.

The multi-port GUI controller shares the CLI's capture engine and port status path. Offline review checks the saved routes and reports capture-specific layout requirements separately. The status worker uses the reviewed profiles and its own clients. The capture controller keeps one replaceable live observation per port outside the CSV queues. `cargo test --locked --bin iolink-workbench desktop::port_capture` checks independent simulator sessions, partial status and capture outcomes, status on an acyclic-only layout, stop/flush and hidden-tab completion.

Diagnostic comparison tests verify pending-event counts for moved and repeated qualifier/code pairs, exclusion of other entries, and unavailable comparisons for incompatible contexts. The headless suite runs these comparisons on saved simulator and loopback replies, with original snapshot bytes checked after review.

The GUI verifier checks a capability marker before launching, then creates a hidden native GL context and paints built-in profile/tab views offscreen. It verifies its own HWND stays hidden, worker denial, framebuffer/GL state and a non-overwriting receipt. It sends no input, takes no screenshot and reads no pixels. It does not establish visible interaction, DPI, presentation or hardware acceptance. Older GUI binaries may ignore arguments; never launch them to probe self-test support.

Keep private/vendor material and evidence under ignored `.local/`. Public samples are original synthetic descriptions. Optional XSD tests validate local schema closures with bounded System.Xml execution; CRC receipts do not prove authenticity or full Checker compliance. Retain old profiles and capture/snapshot fixtures when testing compatible schema evolution. Avoid desktop automation without an explicit request and keep GitHub Actions manual-only if added.

## Documentation and skill maintenance

Update README commands, operations, roadmap and the repository skill when public behavior changes. The maintained skill is [.agents/skills/iolink-workbench/SKILL.md](../.agents/skills/iolink-workbench/SKILL.md). To make it discoverable on another workstation, copy its folder to `$CODEX_HOME/skills` (normally `~/.codex/skills`) only when that destination does not already exist; inspect and reconcile an existing copy. Keep normal automatic discovery enabled. The skill supports host workflows and does not authorize hardware mutations or publication on its own.

Use EIPMaster and EthernetIPWorkbench only as read-only architectural references. Follow this application's schemas and IO-Link semantics; their interface implementations do not establish capability here. Never modify another checkout as part of this task.

## Portable release

Finish appropriate local checks, inspect named staged paths and publish only when the session authorizes it. Make cohesive verified commits; do not mix private evidence with source. Bump Cargo.toml and the package entry in Cargo.lock together, preserving the manifest's existing byte format. Verify main and the intended origin commit. Package from a clean checkout:

```powershell
python tools/package_windows.py C:/path/to/new-output-directory
```

The packager builds Windows x64, checks PE architecture/subsystems/static CRT imports, includes samples/docs/skill and dependency notices, and writes build provenance plus ZIP SHA256. Test extracted executables and included documentation, run capture/snapshot compatibility checks with the previous CLI, and verify hashes/source preservation. Never use old GUI binaries for compatibility probes. Keep release receipts private and report simulation/loopback/native-offscreen evidence separately from unperformed physical/visible-GUI checks.

## Artifact retention and cleanup

Use the [research checkpoint](RESEARCH.md) instead of repeating research dumps. Keep source/samples, read-only references, original vendor files and the local Rust toolchain/dependency sources. Retain the newest portable release plus one rollback/previous reader, their receipts and representative regression evidence. Keep legacy-format fixtures in a small explicitly indexed private set.

`target/` is rebuildable Cargo output; `cargo clean` from this checkout removes it. Before cleanup, check active process ownership and verify resolved paths remain inside the intended project/workspace. Never stop an unrelated process or delete an uncertain owner to reclaim space. Do not rebuild merely to verify documentation-only cleanup.

Compress older unique captures, profiles, snapshots, audits and notes with per-file hashes before removing loose copies. Verify archive CRC and each restored byte hash. Only omit an old extracted distribution when its files match its original delivered ZIP; preserve any modified or additional files as evidence. Record deliberately removed build/installer caches and their provenance. Keep the current portable package's original bytes and checksum.

Use maintained verifiers with explicit fixture paths for future work. Avoid new version-specific copies of private release scripts and long chains of historical absolute paths. Historical archives are evidence, not an assertion that their one-off scripts still run without restoring inputs. Private cleanup manifests, source paths, vendor bytes and raw captures stay in ignored `.local/`.
