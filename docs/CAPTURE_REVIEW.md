# Offline capture and interface review

Use finalized capture folders containing `capture.toml`, `profile.toml` and every rotated CSV part. Review reads the saved profile, not the current GUI setup. It does not query a master, broker, HTTP service or shared-memory object. Keep private/vendor evidence under ignored `.local/` in a source checkout.

```powershell
.\iolink-cli.exe verify-capture trial
.\iolink-cli.exe review-interfaces trial
.\iolink-cli.exe review-capture trial --field level --buckets 128
```

`verify-capture` retains its existing summary format and checks every CSV row, field, part and saved accounting. `review-interfaces` performs that same verification in one pass, then prints TOML with capture summary, acquisition timing/context, original metadata SHA256 values, exact recorded interface counters and categorized findings. It works for raw-only/unmapped captures, where `review-capture` needs a logged typed field. Review prints to stdout and does not replace source files.

In the GUI's **Review saved capture** section, **Verify capture** now also shows **Recorded interface diagnostics**, sampling timing, logger timing and metadata hashes. The existing optional **Combined CSV** destination still publishes atomically after successful verification and refuses an existing target. **Review field** retains its bounded signal plot and includes the same interface diagnostics when recorded. Verification/review failures clear the previous successful result so stale findings are not presented for another folder.

## Read the domains separately

| Domain | What the review preserves | What it cannot establish |
|---|---|---|
| Source/acquisition | Recorded source gap/loss meaning, receive/disconnect/reconnect counts where present, declared/reported cycle and recorded spacing distributions | New physical timing measurements, loss during disconnects or snapshot source updates without freshness metadata |
| CSV/logger | Verified rows/index gaps, recorded logger overruns and queue/writer/drain timing | Every source update when the gateway offers snapshots; exporter delivery |
| HTTP API | Exact offered/ingested/drop/history/request counters and worker flag | Which input events an individual client observed |
| MQTT | Exact handoff/coalescing/shutdown/socket/broker counters and worker/error data | Broker/subscriber receipt from QoS 0 socket completion; partial-write delivery |
| Shared memory | Exact drops/coalescing/busy skips/commits/replacements/final state and worker data | Which snapshots readers saw, or current liveness from a retained active flag |

There is no combined loss score. Intentional MQTT/shared-memory coalescing is `intentional_selection`; it is separate from `interface_loss`. API history eviction and shared-memory replacement are `retention`, not evidence of individual client loss. Failed/partial writes and missing final inactive commit are `unconfirmed`. Worker failures remain distinct from transport/CSV failures: a bundle can have verified raw CSV while an export worker failed.

Source snapshot loss remains unknown even if all recorded interface queue drops are zero. Requested 1 ms polling, sensor minimum, reported port cycle, gateway cadence, host spacing and logger service time stay separate. Timing values are copied from the acquisition report, not recomputed from the selected CSV rows or measured by the review. Field on-change selection, raw-only mode and exporter coalescing must not be read as source-counter gaps.

## Availability and consistency

Each interface status is `recorded` or `not_recorded`. An absent section means no saved metrics; it cannot distinguish a disabled feature from an older/absent producer record. Missing metrics are not zero counters. A present valid section with zero drops remains recorded zero. Missing acquisition timing/counters remain unavailable. `review-capture` omits its optional `interfaces` table when none were recorded, preserving its previous output for ordinary/older captures.

Existing interface validators check internal queue/commit/publish equations. When acquisition receive and disconnect counts are available, verification also checks each interface's offered total against:

```text
received responses + recorded disconnects + successful reconnects
+ first accepted connection if a last accepted status exists + final capture event
```

The final event is offered even if its interface handoff drops it. Broker reconnects are not source reconnects. A balanced interface report with the wrong offered total is rejected when this comparison is possible. For older data missing the acquisition counters, `offer_counts_checked_against_acquisition` stays false; internal validation still applies. This is saved-record consistency, not authentication of those records or delivery attestation.

`review-interfaces` returns no successful report if metadata, CSV values, parts or accounting fail. The hashes identify the exact original bytes of `capture.toml` and `profile.toml` parsed during that verification pass, including comments and line endings. They are not signatures or a complete bundle hash receipt. Originals remain unchanged. Use [re-decode/export verification](../README.md#capture-files-and-timing) for the existing new-bundle workflow and source/file receipts.

## Findings and retained details

Findings have stable component/code/category fields, an optional count and a short meaning. A missing finding is not proof of delivery. Exact saved sections remain available alongside findings, including worker flags, source settings, capacities, session IDs, errors and endpoint/name declarations. Those declarations are displayed as recorded data and never followed.

Useful cases include `mqtt_write_uncertain`, `mqtt_subscriber_delivery_unknown`, `shm_final_state_unconfirmed`, `shm_reader_delivery_unknown` and `api_client_delivery_unknown`. A blocked final shared-memory commit can leave old bytes saying active; this review does not reopen that mapping. A live reader must separately check age and publisher identity/liveness as described in the [shared-memory guide](SHARED_MEMORY.md).

The [HTTP](LOCAL_API.md), [MQTT](MQTT.md) and [shared-memory](SHARED_MEMORY.md) guides define their individual counters. The standalone [operations](OPERATIONS.md) and [development](DEVELOPMENT.md) guides cover acquisition and qualification boundaries.

`tools/verify_interface_review.py` copies explicitly supplied saved fixtures into a new ignored directory and verifies original-file preservation, exact counters/hashes, raw-only/legacy cases, unreachable endpoint and absent mapping metadata, corrupted CSV and balanced-but-wrong offer counts. It opens no live service for this review. Use a fixture with all three interface sections to exercise the complete report. Existing native/loopback suites remain separate from this offline evidence and from physical or visible GUI acceptance.

## Compare two captures

```powershell
.\iolink-cli.exe compare-captures trial-a trial-b
```

GUI **Compare two saved captures offline** accepts independent left/right folders and runs verification in a background worker. Both sides must pass complete metadata/CSV checks before a result appears. Failure or edited folder selections clear the previous comparison. It neither joins nor writes CSV and never opens a transport or IPC object. Raw-only captures are supported.

Output is TOML with both full `review-interfaces` reports, metadata hashes, an explicit settings table and numeric metric rows. The left/right values are recorded statistics or verified CSV counts. Missing numeric fields are omitted and marked `metric_unavailable`, distinct from recorded zero. Each metric keeps its own domain; source gaps, CSV overruns, API drops/retention, MQTT coalescing/uncertain writes and shared-memory skips/replacements are never added together. Original fractional means and complete counters remain in the embedded reviews; the comparison table uses integer counts and microsecond quantiles.

`matching_recorded_context` means the required recorded settings agree. It does not establish identical hardware, observed identity, load, clock, firmware truth or physical provenance. Settings cover the host connection, expected identity, observed master/device context (see identity evidence below), input width, effective fields, IODD variant/layout/communication selection, logging mode/selection/queue/rotation, recorded route/freshness/row kind/source-loss meaning, requested polling, reported port cycle, stale threshold, simulation controls and exporter configurations. Known absent IODD/interface settings can match; absent observed route/freshness/polling or simulation controls remain unavailable. Session IDs, profile display names, UTC starts and metadata comments do not gate deltas. Comments still change the exact metadata hashes.

Timing differences require all those settings to match. Count differences additionally require identical recorded `elapsed_us`, rather than silently comparing unequal exposure or normalizing by an assumed rate. `settings_differ`, `settings_unavailable`, `metric_unavailable` or `capture_or_worker_error` suppress arithmetic deltas but retain both values and full reports. A failed recorded worker or acquisition suppresses all deltas conservatively. All archive operations reject inconsistent spacing statistics; see the shared checks below. A delta, when available, is **right relative to left**, with unsigned magnitude and `increase`, `decrease` or `equal`. Neither direction is automatically good or bad.

This is a descriptive saved-record comparison, not a controlled benchmark. Monotonic clocks are local to each acquisition and are never aligned. The tool performs no interpolation, rate normalization, significance test, combined loss score, source-update reconstruction or automatic 1 ms acceptance. Requested polling, declared sensor minimum, reported port cycle, gateway timestamps, host spacing and logger operations stay separate. Matching simulator metadata does not qualify physical timing; a changed route label does not prove physical origin. Snapshot source-update loss stays unknown even when interface drops are zero.

`tools/verify_comparison.py CLI CAPTURE_A CAPTURE_B` tests exact embedded reports/hashes, comments, missing settings, unequal duration, unreachable declared endpoints, timing/CSV corruption and original-file preservation using private copies. Unit checks also cover raw snapshot/counter distinctions, logger pressure, missing versus zero, selection changes, recorded errors and safe arithmetic. These results remain separate from native/loopback suites and physical acceptance.

## Recorded timing consistency

Every saved-capture path uses the same manifest validation: `verify-capture` (with or without CSV joining), `review-interfaces`, `review-capture`, `compare-captures`, `redecode-capture`, and `verify-export` for its copied source report. GUI actions call those same operations. In v0.36 only comparison checked malformed spacing; ordinary verification could accept reversed percentiles. This inconsistency is fixed. A failed check identifies the affected timing section, produces no successful report and does not publish a joined CSV or re-decoded output. Keep the original files for inspection.

Each present spacing group needs a positive observation count, ordered p50/p95/p99/longest intervals and a finite nonnegative mean frequency. For a positive longest interval, the frequency must fall between `1_000_000 / longest_us` and `count * 1_000_000 / longest_us`, with a small relative rounding allowance. These bounds follow from the reported number of intervals and their maximum; they do not authenticate the measurements. All-zero integer-microsecond durations retain the producer's zero-frequency sentinel, which is not evidence of infinite speed or a source update rate.

When the needed acquisition counters exist, host poll-start interval count must equal `poll_attempts - 1` and receive interval count must equal `received - 1`, saturating at zero. Those host measurements span reconnect boundaries. New-update receive and gateway interval counts cannot exceed `new_updates_observed - 1`; reconnect resets may make the actual number smaller. Gateway interval count cannot exceed recorded new-update receive interval count when both groups exist. Recorded source-clock quantiles must be positive. A snapshot capture cannot claim new-source receive spacing or gateway source cadence, even if edited metadata supplies such tables.

The longest host poll/receive/new-update interval must fit the host acquisition window. Gateway/source timestamps use their own clock domain and may have intervals longer than the host window; no cross-clock bound, synchronization, rate reconstruction or drift conclusion is implied. Port-cycle declarations remain separate.

Absent timing tables or optional older acquisition counters remain unavailable. Their absence does not become zero or cause an invented consistency comparison. The validator never estimates full receive counts or timing from CSV rows: on-change filtering, duplicates, invalid/stale classifications, writer pressure and rotation can separate those populations. Existing valid capture and export formats are unchanged. Contradictory recorded statistics are rejected even when CSV bytes and rehashed export inventories otherwise agree.

`tools/verify_timing.py CLI COUNTER_CAPTURE SNAPSHOT_CAPTURE` tests copied saved evidence across six capture command paths, corrupted aggregates, count mismatches, older missing data, independent clocks, snapshot cadence and failed-output preservation. The first fixture must contain all four timing groups and typed fields; include a real synthetic snapshot bundle as the second fixture. Rust checks also cover local rehashed-export corruption, unsigned extremes and zero-duration statistics. These are consistency tests, separate from simulator timing observations, native IPC checks and physical qualification.

## Recorded identity and reconnects

`review-interfaces`, GUI **Verify capture** and field review include an optional `identity` section when saved identity/status details, configured pins, producer check text or a rejected observation exist. It preserves expected pins, the final accepted status and any rejected status separately. Master model, firmware, serial, port/mode, device IDs, Product ID/name, sensor serial and saved status/diagnostic labels are retained where recorded. Missing values stay unavailable; empty strings remain literal values. The GUI distinguishes absent expectations as **not pinned** and absent observations as **unavailable**.

`accepted_recheck` compares the saved profile's pins against the last accepted saved identity, using the same full/partial/unavailable rules as acquisition. It performs no live master connection, ISDU read or identity query. `producer_check` retains the original report's text separately; that text cannot fill a missing observation or establish a live match. Common manifest validation rejects known accepted-field contradictions against saved pins and accepted Device IDs outside the profile's supported 24-bit range. This applies to verification, review, comparison, re-decoding and local re-decoded bundle verification. Missing old identity data stays accepted as unavailable. Rejecting contradictory metadata is a consistency check, not authentication.

A rejected connection can disagree with pins and remains visible as rejected, even when earlier accepted CSV rows verify. It never replaces the accepted identity. Reports preserve only the final accepted status and one rejected status, not a per-generation connection-status journal. Reconnect counts, CSV generations and equal final identity fields do not reconstruct every prior connection or prove identity continuity. The acquisition guard checks device identity, master model/firmware/serial and freshness on reconnect; an offline review cannot replay those checks or recover absent observations.

Capture comparison now adds `observed_master` and `observed_device` settings. Required master observations are nonempty model/firmware/port-mode strings and a recorded port; device Vendor ID and Device ID must be available. Changed observed model/firmware/master serial/port/mode or any supplied device identity field suppress arithmetic differences. Missing required observations produce `settings_unavailable`. Optional serial numbers remain visible and are compared when supplied; two absent serials do not establish that captures used the same physical unit. Matching observed model IDs also do not establish common hardware load, physical origin or firmware authenticity. Full reviews and metadata hashes remain available alongside the comparison.

The optional review section is additive; capture, CSV, profile and export receipt formats are unchanged. Minimal older records without identity-related evidence or pins omit the section. Existing captures with saved observations can expose them through the new reader, without rewriting originals. Ordinary verification's summary shape is unchanged. Older comparison tools lack these extra observed-context checks and can reach a less conservative delta decision on missing or changed metadata.

`tools/verify_identity.py CLI CAPTURE_A RAW_CAPTURE` verifies exact saved accepted/rejected details and expectations, shared field/interface views, changed observed context, missing observations, accepted-pin contradictions, non-published failed outputs, unreachable declared endpoints and original-file preservation. Its first fixture must have typed fields, an observed device serial and no configured serial pin. Rust checks cover partial/unavailable identity, minimal legacy records, ID bounds and pure GUI rendering. Simulator/native/physical evidence remains separate; this review does not qualify bench identity or timing.
