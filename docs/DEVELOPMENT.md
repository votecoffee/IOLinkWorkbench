# Developing IOLinkWorkbench

Read [AGENTS.md](../AGENTS.md) and the [operations guide](OPERATIONS.md) before changing a public workflow. The package is Rust with an eframe/glow desktop binary and Clap CLI. Profiles remain version 1; preserve compatibility deliberately rather than silently changing stored meaning.

## Ownership and flow

| Area | Responsibility |
|---|---|
| `src/transport*` | Read-only adapter contracts, ifm HTTP snapshots and synthetic gateway; observed status/identity/quality/freshness |
| `src/iodd*`, `src/decode*` | Bounded local description import, declared metadata and pure raw/typed decoding |
| `src/profile*` | Saved host configuration, imported/manual precedence, validated candidates, identity pins, preferences and offline capture plans |
| `src/capture*` | Cyclic acquisition, receive timestamps, selection, bounded queue, writer, finalized reports and timing/loss accounting |
| `src/archive*` | Offline capture verification, bounded review and re-decode exports |
| `src/parameter_snapshot*`, diagnostics | Explicit acyclic results and offline reopening with embedded context |
| `src/bin/iolink-cli*`, `src/desktop*` | Shared core operations exposed through CLI and GUI; pending GUI drafts guard acquisition |

An applied profile selects a transport and effective mapping. The cyclic worker reads, timestamps and validates an input, accounts for freshness/selection and submits a bounded preallocated queue entry. The independent writer batches CSV, rotates, flushes/syncs and drains on stop. Queue saturation is counted rather than blocking sampling indefinitely. An explicit acyclic query creates its own client/worker. Decoding and offline plans must not open transports. Any future integration gets its own bounded handoff and loss accounting; see [interface criteria](INTERFACES.md).

Capture plans describe saved configuration, never observed transport capabilities. Selected field positions are one-based value order; `csv_columns` contains the actual full header including metadata. Rotation follows receive elapsed timestamps, not writer wall time. Preserve raw payloads and the distinction between source gaps, host polls, intentional filtering, reconnect uncertainty and logger drops. Do not infer sensor freshness from equal/different payloads.

## Local validation

Use a stable x64 MSVC Rust toolchain with Visual Studio C++ Build Tools and Windows SDK. The repository pins dependency resolution through Cargo.lock and static CRT through `.cargo/config.toml`. Run from the checkout root:

```powershell
cargo fmt --all -- --check
cargo test --locked --quiet
cargo clippy --locked --all-targets -- -D warnings
cargo check --locked --no-default-features --bin iolink-cli
cargo build --locked --release --target x86_64-pc-windows-msvc --bins
python tools/verify_headless.py target/x86_64-pc-windows-msvc/release/iolink-cli.exe
python tools/verify_gui.py target/x86_64-pc-windows-msvc/release/iolink-workbench.exe --output .local/gui-check-NEW.json
```

Use a new GUI receipt path. The headless suite creates a fresh ignored directory and exercises synthetic IODDs, recorded decoding, localhost HTTP responses, stale/invalid data, reconnects, independent held acyclic reads, writer backpressure, rotations and offline verification. It does not contact a physical bench. New command tests should verify observable preservation/rejection/CSV behavior rather than mirror implementation text.

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
