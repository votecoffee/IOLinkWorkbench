# Operating IOLinkWorkbench

The portable ZIP contains `iolink-workbench.exe`, `iolink-cli.exe`, samples, these documents and dependency notices. Extract it to a writable directory on Windows 10/11 x64. The GUI needs the system OpenGL driver; the CLI runs headlessly. Optional XSD checks use Windows PowerShell/System.Xml. See the [README](../README.md) for the detailed command and format reference.

## Start offline

Run these commands from the extracted package directory. Use new output names: profile edit commands and captures refuse to overwrite existing destinations.

```powershell
.\iolink-cli.exe validate samples/simulator.toml
.\iolink-cli.exe decode samples/simulator.toml 01F4
.\iolink-cli.exe timing samples/simulator.toml
.\iolink-cli.exe logging samples/simulator.toml
.\iolink-cli.exe capture samples/simulator.toml trial --seconds 2
.\iolink-cli.exe verify-capture trial
.\iolink-cli.exe review-capture trial
```

These commands use a synthetic device. They establish software behavior, not physical timing or identity. The GUI starts offline; Setup edits the applied host profile, Monitor & CSV starts explicit acquisition, and the review tools reopen saved evidence without connecting.

## Descriptions, identity and manual mapping

Keep vendor XML/ZIP and raw bench captures in ignored `.local/` when using a source checkout. Inspect before importing. For the bundled synthetic description:

```powershell
.\iolink-cli.exe iodd samples/sensor.xml
.\iolink-cli.exe import samples/sensor.xml imported.toml --variant SIM-SENSOR
.\iolink-cli.exe validate imported.toml
```

Use `iodd` output to choose the exact variant and, when ambiguous, `--layout`. Inspect `import --help` for local standard/language/unit resources, ZIP entry selection, stamp requirements and optional XSD checks. Missing resources and unsupported layouts/types need review; importing a description does not observe a device. Check IODD provenance, declared communication limits, input width and selected layout against the actual sensor. CRC/stamp consistency is separate from vendor authenticity and a full IODD Checker pass.

In Setup, select the IODD variant/layout and apply it. Effective fields show IODD, override or manual origin. Use Edit mapping/Add manual field for bit offset, width, signedness/type, byte order, scale, offset, units and decimal precision, then preview a full recorded payload. Bit zero is in the first byte for little endian and the last byte for big endian. Raw bytes are always retained. For a standalone host field:

```powershell
.\iolink-cli.exe set-field samples/simulator.toml mapped.toml signal --bit-offset 0 --bits 16 --kind signed --byte-order big-endian --scale 0.1 --unit mm
.\iolink-cli.exe decode mapped.toml 01F4
.\iolink-cli.exe remove-field mapped.toml restored.toml signal
```

Editing an imported name creates an override; removing it restores the imported definition. Removing a standalone field selected explicitly for CSV is rejected until the saved selection is reset. Do not guess scaling or units from an unfamiliar payload. Save unresolved fields for input and keep raw logging available.

## Prepare the first physical bench

No master, sensor or signal has been chosen for this project yet. Before physical acceptance, record master model/firmware, supported host interface and license/SDK requirements, endpoint or USB connection, configured port/mode, sensor model, IODD revision/variant and desired signal/unit. The AL1350 HTTP(S) adapter targets documented IoT Core 3.1.x behavior. Its [bench reference](../README.md#al1350-bench-setup) records the API, Windows requirements and snapshot limitations; it is not a purchase recommendation or qualification of other firmware.

Use an existing configured IO-Link port. Host connection settings edit the local profile only. `set-connection` saves a new host profile; `set-identity` saves expected pins. Use each command's `--help` for explicit values and clears. `set-connection --expected-mode` saves the `connection.ifm_mode` expectation; it checks the observed mode and never sets it. Confirm device identity where exposed with `status PROFILE`; expected pins and selected IODD identity remain separate from observed matches. A Product ID difference requires review. Exact text pins distinguish unset, empty and spaces.

Changing port mode/configuration, device commands, ISDU writes or process outputs requires explicit approval for that operation. The application implements no such write path. An explicit parameter/diagnostic read uses its own client and is not part of the cyclic loop; gateway-side contention must still be measured on hardware.

## Choose logging and inspect its plan

```powershell
.\iolink-cli.exe set-logging mapped.toml logging.toml --mode every-update --queue-capacity 8192 --rotate-rows 100000 --rotate-seconds 0 --field signal
.\iolink-cli.exe logging logging.toml
```

`set-logging INPUT OUTPUT` preserves unspecified settings and saves a new file. Choose one selection option: repeated `--field NAME` in CSV order, `--all-fields`, or `--raw-only`. Modes are `every-update` and `on-change`; zero disables each rotation limit. Queue capacity is 1–1,000,000 rows. Edited rotation limits must fit TOML's signed 64-bit integer range. GUI controls share candidate validation; apply/discard pending Setup drafts before acquiring or saving.

`logging PROFILE` is an offline plan with configured row semantics, selected field mappings/origins and exact CSV headers. It contacts no gateway and verifies no rate/freshness. In Monitor & CSV, expand Configured capture plan while idle for the summary. Every-update retains equal consecutive payloads when a new source update is verifiable. Snapshot adapters instead record host polls. On-change selects the first eligible row per connection generation and changes in the complete raw payload, validity, stale flag or diagnostic code; field selection/scaling does not change that decision.

Rotation is checked before the next row. Time rotation uses receive elapsed timestamps from the first row in each part; either enabled threshold rotates. Silence creates no empty timed files. Stop and flush the capture before treating its report as finalized.

## Timing, loss and saved evidence

Keep the requested host interval, declared sensor minimum, reported port cycle, gateway update rate, source cadence, host poll/receive spacing and queue/writer timings separate. Treat 1 ms as a target to evaluate. AL1350 snapshots have no verifiable source update counter/timestamp in the supported path: source loss, source age and gateway update cadence are unknown. Host receive age is not sensor age. Simulator counters/timestamps are synthetic. Within-generation counter gaps cannot measure loss during disconnects.

Captures retain UTC start, monotonic host elapsed times, sample indexes, reconnect generations, raw input, validity, staleness, diagnostics, selected typed cells and finalized timing/accounting reports. Inspect p50/p95/p99/longest gaps, invalid/stale/unknown input and logger overruns. Review queue high water, queue wait, service/flush/sync and drain separately from source loss. On-change filtering is intentional selection, not a source gap. Invalid/stale values stay blank in typed cells.

`verify-capture DIRECTORY` verifies all parts. `review-capture DIRECTORY --field NAME` produces bounded offline signal summaries. `redecode-capture DIRECTORY MAPPING OUTPUT` exports a new verified bundle without replacing the original; `verify-export OUTPUT --source DIRECTORY` optionally verifies the explicitly supplied source. Parameter workflows use `parameters`, `defaults`, `decode-parameter`, `read-parameter --snapshot`, and `review-parameter`; a declared default is metadata, not an observed value or write. Saved parameter review uses embedded context without following stored endpoints or file paths. See the [README](../README.md) for their full examples.

## Integrations and support evidence

Integration includes CLI, saved TOML/JSON/CSV and an opt-in [read-only local HTTP API](LOCAL_API.md). Enable it per acquisition in Monitor & CSV or use capture `--api-listen 127.0.0.1:8765`; it stops with acquisition. Optional [MQTT publishing](MQTT.md) runs on its own bounded handoff with intentional latest-state coalescing and separate loss accounting. Windows shared memory and C FFI remain [future work](INTERFACES.md). Keep simulation/loopback reports apart from physical results. For a bench issue, retain firmware, identity, profile, capture report and raw bytes in private storage; publish only sanitized reproductions.
