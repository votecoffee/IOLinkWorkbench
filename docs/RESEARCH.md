# Research checkpoint

This is the compact index for the research behind Windows preview **0.45.0**, built from commit `36ae4fe0d7874f0f141215ce7b23e4a6c4e718c1`. Detailed commands and implementation contracts remain in the maintained guides. These conclusions describe saved research and verified software behavior; vendor documentation and interface/licensing conditions must be checked against the hardware eventually selected.

## Decisions and remaining inputs

| Topic | Current conclusion | Maintained detail |
|---|---|---|
| Bench hardware | Master, firmware, configured port, sensor, IODD and signal remain unselected. | [First bench](../ROADMAP.md#first-physical-bench) |
| Initial adapter | ifm AL1350 IoT Core 3.1.x HTTP(S) is a documented target and loopback fixture, with no vendor USB SDK added. It is not selected or physically qualified bench hardware. | [AL1350 reference](../README.md#al1350-bench-setup), `src/transport/ifm.rs` |
| Writes | CLI supports offline review and one-shot mode, readable ISDU, SystemCommand and process image writes. Physical behavior awaits bench qualification. | [AL1350 writes](WRITES.md) |
| Freshness | The supported AL1350 path returns host snapshots. Source update loss, cadence and age remain unknown without source metadata; value changes cannot supply it. | [Operations](OPERATIONS.md#timing-loss-and-saved-evidence) |
| Architecture | Transport, pure decoding, cyclic sampling, bounded CSV writer and independent acyclic clients remain separate. | [Development](DEVELOPMENT.md#ownership-and-flow) |
| Descriptions | IODD XML/ZIP selection, local standard/language/unit resources, CRC receipts and optional local XSD checks are supported. Full IODD Checker/profile-snippet conformance remains separate. | [Description workflows](OPERATIONS.md#descriptions-identity-and-manual-mapping), [roadmap](../ROADMAP.md#follow-on-work) |
| Typed input | IODD/manual mappings retain bit layout, signedness, byte order, scaling and units. Unknown or unsupported interpretation stays visible for review. | [README](../README.md), [live view](LIVE_VIEW.md) |
| Logging and review | Equal new source updates, on-change selection, host-poll uncertainty, rotations, drain/flush and saved consistency checks have distinct meanings. | [Capture review](CAPTURE_REVIEW.md), [CLI lifecycle](CLI_LIFECYCLE.md), [GUI lifecycle](GUI_LIFECYCLE.md) |
| Integration | CLI/files, opt-in loopback HTTP, latest-state MQTT and Windows shared memory are implemented. C FFI and production MQTT TLS/authentication remain follow-on work. | [Interfaces](INTERFACES.md) |
| Licensing | Project source is MIT and portable builds contain dependency notices. No proprietary SDK license or selected hardware/API entitlement is inferred. | [License](../LICENSE), [bench preparation](OPERATIONS.md#prepare-the-first-physical-bench) |

## Saved primary material

Original vendor/standard files remain private and outside Git. The cleanup ledger records their exact local locations, lengths and SHA256 values.

- The saved ifm AL1350 manual is `ifm-al1350.pdf`; the README links its product/document reference. The small `ifm-80284128.pdf` download was HTML rather than a PDF and is retained only in the historical archive.
- The saved IODD specification and guideline are the October 2025 V1.1.5 ZIPs, `iodd-spec.zip` and `iodd-guide.zip`, with their extracted examples/resources preserved. The local schema closure remains under `.local/schema-official-2025/`.
- `.local/vendor/` retains the 2025 system specification package, Interface V1.1.5 PDF and saved time-coding notes. Older interface material and intermediate audits are available in the historical archive.
- The EIPMaster reference stays read-only at pinned commit `593292766a1d927a405d7500aeca0b6583223589`. Its local checkout and the installed Rust toolchain/dependency sources are preserved. Reference-application capabilities are not capabilities of this application.

## Evidence and recovery

The v0.45.0 checkpoint passed **303 Rust tests**, headless simulator/loopback regression, 30 native offscreen GUI views and extracted-package/previous-reader checks. The private release receipt separates those results from physical operation. Visible interaction, native window-manager/DPI acceptance, selected-device identity/scaling and physical 1 ms/sustainable-rate qualification remain pending.

The 2026-11-14 cleanup retains the v0.45.0 and v0.44.0 portable distributions, their current evidence and receipts, and a directly usable legacy capture at `.local/compatibility/0.17-normal/`. Older captures, profiles, snapshots, research notes, audits and one-off scripts are compressed without byte conversion in `.local/cleanup/2026-11-14/historical-evidence.zip`. Its `plan.json` records archive-member paths and hashes, retained inputs, removed duplicate distributions and original build provenance.

To recover an older input, locate its original path in `historical_files`, then extract that record's exact `member` to a new private destination and verify `sha256`. The archive uses `repo-local/` and `workspace-work/` prefixes. Old distribution bytes deliberately pruned as rebuildable copies have provenance/hash records instead of archive members. Historical scripts may contain old absolute paths; use maintained `tools/verify_*.py` commands with explicit fixtures for new verification.

Do not recreate full research dumps or a copied private release script for every increment. Update the maintained contract that changed, retain a compact receipt and representative fixtures, and archive historical material after verification. See the [artifact retention policy](DEVELOPMENT.md#artifact-retention-and-cleanup).
