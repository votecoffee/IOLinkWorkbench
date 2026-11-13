# Windows shared-memory snapshots

An optional Windows interface exports the latest acquired state to local readers. It is disabled by default, runs only during one monitor/capture, and uses its own bounded queue and worker. It adds no device connection, acyclic queries or command/output path. These are runtime options, not saved profile keys. This is IPC for another process; a C-compatible embedding library remains future work.

## Enable and read

In Monitor & CSV, enable **Windows shared-memory snapshot**, choose a name and requested interval. From the portable package:

```powershell
.\iolink-cli.exe capture samples/simulator.toml trial-shm --seconds 30 --shared-memory input
# In another PowerShell window during capture:
.\iolink-cli.exe read-shared-memory input
```

`--shared-memory NAME` opts in. Names are 1–64 ASCII letters/digits/underscore/dash/dot; full Windows paths and namespace prefixes are rejected. `--shared-memory-interval-ms N` requests 1–1000 ms, default 10. `--shared-memory-queue-capacity N` is 1–16384 events, default 256. Additional options require the name. The two object names are:

```text
Local\IOLinkWorkbench.Snapshot.NAME
Local\IOLinkWorkbench.Snapshot.NAME.Mutex
```

Objects live in the current Windows session. Existing mutex/mapping names cause startup failure before CSV creation; existing bytes are preserved. Readers holding handles/views can keep objects alive after publisher exit. Close those readers before reusing a name, or choose a different name.

Objects have a protected DACL granting access to the creator's current user and SYSTEM. The application and [Python reference reader](../examples/shared_memory_reader.py) open `FILE_MAP_READ` views. Their mutex handles request synchronize/release rights. Same-account code can separately request write access, and privileged code can act beyond this policy; this is not data authentication. Different Windows sessions and elevated/service deployments need their own qualification. There is no Global namespace or remote transport.

`read-shared-memory NAME --wait-ms 50 --max-age-ms 1000` copies one consistent snapshot, prints JSON and closes its handles. Wait is bounded to 0–1000 ms; age limit is 1–60000 ms. Busy/uninitialized/unsupported/abandoned objects fail with a nonzero exit. Valid but old/stopped data is returned with flags for the caller to inspect. Reading never loads a profile or contacts hardware. Python is optional; the packaged CLI needs no Python runtime. For another language, follow the ABI below or use the reference reader, which also supports bounded repeated observations through `--count`.

## ABI version 1

The paging-file-backed mapping is exactly 1,048,576 bytes. Every header integer is unsigned little endian; there is no Rust/C struct padding. The 64-byte header precedes UTF-8 JSON without a NUL terminator.

| Offset | Bytes | Meaning |
|---|---|---|
| 0 | 8 | Magic `IOLWSHM` followed by NUL |
| 8 | 4 | ABI version, 1 |
| 12 | 4 | Header size, 64 |
| 16 | 4 | Mapping size, 1048576 |
| 20 | 4 | JSON byte count, 1–1048512 |
| 24 | 8 | Successful snapshot commit sequence, starts at 1 |
| 32 | 4 | Publisher process ID |
| 36 | 4 | Flags: bit 0 sampling active; bit 1 gateway connected last observed; other bits zero |
| 40 | 8 | Publication QueryPerformanceCounter ticks |
| 48 | 8 | QueryPerformanceFrequency ticks per second |
| 56 | 8 | Publisher process creation FILETIME, for PID reuse detection |
| 64 | variable | Exactly the declared UTF-8 JSON bytes; remaining mapping bytes are not data |

Open the named mutex and read-only mapping. Acquire the mutex with a bounded wait, copy and validate the header, then copy exactly the bounded JSON length. Release the mutex before decoding or doing other work. Reject an abandoned mutex observation and release its ownership; do not accept a possibly interrupted copy. The publisher can recover by overwriting the complete header/payload under that lock. Never read bytes without the mutex or assume a changing sequence is a substitute for synchronization.

Validate magic/version/sizes, nonzero sequence/PID/frequency/creation time, known flags and payload bounds before allocating/copying. Decode JSON and require its version, snapshot sequence and active/connected flags to match the header. The supplied readers implement these checks. Read-only mapped access and named-object synchronization follow Microsoft's [file-mapping access](https://learn.microsoft.com/en-us/windows/win32/memory/file-mapping-security-and-access-rights) and [memory ordering](https://learn.microsoft.com/en-us/windows/win32/sync/synchronization-and-multiprocessor-issues) contracts.

## Payload and freshness

JSON contains Workbench/format version, opaque `session_id`, UTC capture start, capture-relative monotonic host elapsed time, `snapshot_sequence`, newest ingested `event_sequence`, its elapsed time, source `generation`, `sampling_active` and `gateway_connected_last_observed`. Accepted master/firmware/port/identity is supplied only for the matching generation; bounded text has a truncation flag. A dropped connection event can leave identity unavailable. Input is cleared on accepted reconnect/disconnect events; a finished snapshot can retain its last input with active=false.

`input` uses the shared raw/typed decoder: response index/classification, raw hex, validity, stale/unknown status, source counter/time/age and gaps where measurable, diagnostics, typed values/origins/units, decode errors and optional CSV attempt/index. Invalid or known-stale inputs have no typed values. `fields` supplies effective bit/type/endian/scaling/unit/precision mappings. Numeric decoded values remain strings for exact wide values. Export permits at most 128 effective fields with names/units up to 256 UTF-8 bytes. Master text is capped at 256 bytes on this export without changing original identity checks.

Equal payloads can produce new commits; updates can also be skipped through queue drops, interval coalescing or a busy mutex. Snapshot sequence counts successful commits, not source updates or CSV rows. A reader can compare sequence differences within a session to find commits it did not observe, but cannot recover them. `snapshots_replaced` counts commits after the first; it does not prove any reader lost data. A new session requires resetting reader cursors.

Native readers calculate `publish_age_us` from host QPC ticks/frequency and compare it with their age limit. QPC is a same-host monotonic clock, separate from UTC and sensor time; see [Microsoft's timestamp guidance](https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps). `input_host_age_us_at_encoding` is input age when JSON was constructed, not the current reader-time age; publication and encoding are distinct instants. Source/sensor age remains separate. Readers also check whether the PID is still running with the recorded creation time; inability to query it is unknown, never proof of liveness.

Active/connected flags are last-written observations. They cannot establish current liveness alone. Check publication age, session, publisher process state and input quality together. If a reader holds the mutex across shutdown, the final inactive commit can be skipped, leaving active=true in an old mapping. Publisher crash can do the same. A false/unknown process state, high publication age or inactive flag must not be treated as current input. A live GUI process alone does not prove its capture worker is running.

Snapshots from the supported AL1350 route remain host polls: source update loss, source age and gateway update cadence are unknown. Counter gaps apply only within observed source generations; disconnect loss is unknown. Shared-memory skips do not change those meanings. Requested publish spacing, host polling, sensor minimum, reported port cycle and measured source/gateway/host spacing remain separate. A 1 ms request is an evaluation target, not a hardware or IPC guarantee.

## Backpressure, shutdown and evidence

Acquisition offers fixed-size frames to an independent preallocated queue without taking the shared-memory mutex. The worker coalesces pending events and encodes bounded JSON away from sampling. It tries the mutex with zero wait; a reader holding it produces `busy_skips`, without waiting in acquisition or delaying CSV/API/MQTT on that lock. Mapping allocation and permissions happen once at startup.

On stop, queued events drain and the worker makes a bounded best-effort inactive commit, including a separate final refresh if needed. It never waits for readers. `final_snapshot_written` reports whether an inactive state was committed, not whether readers saw it. All producer handles/views close; reader handles can retain the last bytes. The interface is neither durable storage nor a history service.

Final `capture.toml` has an optional `shared_memory` section. Normal accounting is:

```text
events_offered = events_dequeued + queue_dropped
events_dequeued = coalesced + publish_attempts
publish_attempts + final_refresh_attempts = snapshots_written + busy_skips + encoding_failures + write_failures
snapshots_replaced = max(0, snapshots_written - 1)
```

Final refresh is at most one extra lifecycle attempt. Abandoned-lock recoveries, final state, worker failures and bounded error text are separate diagnostics. Payload counters describe concurrent pre-commit observations and need not be an atomic equation; `snapshots_written_before_this` excludes the current commit. Capture verification checks saved accounting, not reader delivery. API drops/history eviction, MQTT coalescing, source gaps and logger overruns remain independent.

`tools/verify_shared_memory.py` exercises native IPC through independent Python and Rust readers with synthetic inputs: quality, equal values, reconnects, slow readers, blocked final commit, collision/preservation/reuse, exit/crash detection, snapshot ambiguity, API/CSV independence and corrupt reports. Rust tests additionally inspect protected ACLs, denied write views, abandoned mutex recovery and ABI bounds. Results establish offline/native software behavior, separate from physical IO-Link timing and visible GUI acceptance.
