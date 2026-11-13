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

`matching_recorded_context` means the required recorded settings agree. It does not establish identical hardware, observed identity, load, clock, firmware truth or physical provenance. Settings cover the host connection, expected identity, input width, effective fields, IODD variant/layout/communication selection, logging mode/selection/queue/rotation, recorded route/freshness/row kind/source-loss meaning, requested polling, reported port cycle, stale threshold, simulation controls and exporter configurations. Known absent IODD/interface settings can match; absent observed route/freshness/polling or simulation controls remain unavailable. Session IDs, profile display names, UTC starts and metadata comments do not gate deltas. Comments still change the exact metadata hashes.

Timing differences require all those settings to match. Count differences additionally require identical recorded `elapsed_us`, rather than silently comparing unequal exposure or normalizing by an assumed rate. `settings_differ`, `settings_unavailable`, `metric_unavailable` or `capture_or_worker_error` suppress arithmetic deltas but retain both values and full reports. A failed recorded worker or acquisition suppresses all deltas conservatively. Inconsistent spacing statistics (zero observations, reversed quantiles or invalid frequency) reject comparison. A delta, when available, is **right relative to left**, with unsigned magnitude and `increase`, `decrease` or `equal`. Neither direction is automatically good or bad.

This is a descriptive saved-record comparison, not a controlled benchmark. Monotonic clocks are local to each acquisition and are never aligned. The tool performs no interpolation, rate normalization, significance test, combined loss score, source-update reconstruction or automatic 1 ms acceptance. Requested polling, declared sensor minimum, reported port cycle, gateway timestamps, host spacing and logger operations stay separate. Matching simulator metadata does not qualify physical timing; a changed route label does not prove physical origin. Snapshot source-update loss stays unknown even when interface drops are zero.

`tools/verify_comparison.py CLI CAPTURE_A CAPTURE_B` tests exact embedded reports/hashes, comments, missing settings, unequal duration, unreachable declared endpoints, timing/CSV corruption and original-file preservation using private copies. Unit checks also cover raw snapshot/counter distinctions, logger pressure, missing versus zero, selection changes, recorded errors and safe arithmetic. These results remain separate from native/loopback suites and physical acceptance.
