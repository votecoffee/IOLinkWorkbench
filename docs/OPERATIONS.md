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

These commands use a synthetic device. They establish software behavior, not physical timing or identity. The GUI starts offline with a typed simulator. Device → Overview links the setup tasks; Profiles… separates bundled starting profiles from Open & save. Starting or loading a profile returns to the overview without connecting. Device edits the applied host profile, Inputs displays monitoring, Logging starts CSV acquisition, and the review tools reopen saved evidence without connecting.

Drag one TOML profile or IODD XML/ZIP onto the GUI to select its path; loading or inspection still requires an explicit click. Pending edits and active setup owners block drops. After stopping CSV recording, use **Verify latest recording** to open its folder and check it in Offline. Signal, Exports and Compare captures have their own pages. To join CSV parts, enter a new filename on Exports and choose **Verify and save combined CSV**. See the [GUI workflow](GUI_WORKFLOW.md) for file selection, the tab map, persistent controls, review/profile windows and first session.

## Descriptions, identity and manual mapping

Keep vendor XML/ZIP and raw bench captures in ignored `.local/` when using a source checkout. Inspect before importing. For the bundled synthetic description:

```powershell
.\iolink-cli.exe iodd samples/sensor.xml
.\iolink-cli.exe import samples/sensor.xml imported.toml --variant SIM-SENSOR
.\iolink-cli.exe validate imported.toml
```

Use `iodd` output to choose the exact variant and, when ambiguous, `--layout`. Inspect `import --help` for local standard/language/unit resources, ZIP entry selection, stamp requirements and optional XSD checks. Missing resources and unsupported layouts/types need review; importing a description does not observe a device. Check IODD provenance, declared communication limits, input width and selected layout against the actual sensor. CRC/stamp consistency is separate from vendor authenticity and a full IODD Checker pass.

In Device → IODD import, select the IODD variant/layout and apply it. Effective fields show IODD, override or manual origin. Use Mappings → Edit mapping/Add manual field for bit offset, width, signedness/type, byte order, scale, offset, units and decimal precision, then preview a full recorded payload. Bit zero is in the first byte for little endian and the last byte for big endian. Raw bytes are always retained. For a standalone host field:

```powershell
.\iolink-cli.exe set-field samples/simulator.toml mapped.toml signal --bit-offset 0 --bits 16 --kind signed --byte-order big-endian --scale 0.1 --unit mm
.\iolink-cli.exe decode mapped.toml 01F4
.\iolink-cli.exe remove-field mapped.toml restored.toml signal
```

Editing an imported name creates an override; removing it restores the imported definition. Removing a standalone field selected explicitly for CSV is rejected until the saved selection is reset. Do not guess scaling or units from an unfamiliar payload. Save unresolved fields for input and keep raw logging available.

### Acyclic-only IODDs

Select the acyclic-only layout in **Device → IODD import**. The CLI uses the ID of the empty `ProcessData` item with `import --layout ID`. The sole unconditional layout is selected automatically.

The saved profile retains identity, parameter and event definitions with zero input bytes. Use **Parameters** for ISDU reads and snapshots, **Diagnostics → Device events** for diagnostic reads, and **Device → Writes** for reviewed ISDU operations. Process input acquisition and process output operations require corresponding cyclic data. Acyclic-only profiles require v0.83.0 or later.

### Cyclic octet strings

The importer maps fixed-length `OctetStringT` input and output values, including byte-aligned record members and array elements. Each value preserves wire order and leading, embedded, and trailing zeros. The typed view and each selected CSV column show an ordered list such as `0x00,0x80,0xFF,0x00`. The raw view retains the complete payload. [IO-Link Interface and System Specification, Table A.4 and Annex F.2.7](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)

For a manual input field, choose **Octet string** or use `set-field --kind octet-string`. Specify the offset and width in bits, both multiples of eight, within the 32-byte process image. The big-endian offset counts from the last process byte. Decimal formatting and scaling apply to numeric fields. Use the full expected and desired hex images for a reviewed output operation involving octet strings. Reimport the IODD to add fields from earlier imports, or re-decode saved raw captures with the new mapping. Profiles containing octet fields require v0.92.0 or later.

### Standalone Boolean input

An IODD input consisting of a single `BooleanT` uses one wire byte. Workbench decodes `00` as false and any nonzero byte as true. Inputs, Offline → Process bytes and CLI `decode` flag values from `01` through `FE` as noncanonical. Packed record and array Booleans still use one bit per field. Manual Boolean mappings can specify either one bit or eight bits. [IO-Link Interface and System Specification, Annex F.2.2](https://io-link-old.io-link.com/share/Downloads/Package-2024-released/IOL-Interface-Spec_10002_V114_Jun24.pdf)

Reimport older standalone Boolean IODDs to get the eight-bit mapping. Older profiles and captures keep their saved bit interpretation. Profiles with eight-bit Boolean fields require v0.91.0 or later.

### Standalone integer input

For a single IODD signed or unsigned integer, the import preview shows the wire byte count. When the input declaration specifies the logical width or its smallest fitting standard container, the saved profile records the declared length under `iodd.padded_input_bit_length`. For example, a 17-bit value uses a four-byte wire container and retains a 17-bit mapping. Sign-extended or zero-padded high bits do not enter the decoded value. [IO-Link Interface and System Specification, Annex F.2.3–F.2.4](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)

Inputs and Offline → Process bytes show a warning when the high bits disagree with the IODD encoding. The CLI `decode` reports the same warning on stderr and still prints the typed value. Width checks, raw bytes, validity and CSV column definitions follow the existing capture rules. `verify-capture` counts noncanonical padding among saved valid rows. Padding checks apply when the configured input width matches the saved IODD wire width. Older profiles keep their saved width and mapping. Re-import to select the new geometry. Profiles with the padding metadata require v0.90.0 or later.

## Prepare the first physical bench

No master, sensor or signal has been chosen for this project yet. Before physical acceptance, record master model/firmware, supported host interface and license/SDK requirements, endpoint or USB connection, configured port/mode, sensor model, IODD revision/variant and desired signal/unit. The AL1350 HTTP(S) adapter targets documented IoT Core 3.1.x behavior. Its [bench reference](../README.md#al1350-bench-setup) records the API, Windows requirements and snapshot limitations; it is not a purchase recommendation or qualification of other firmware.

Use an existing configured IO-Link port. Host connection settings edit the local profile only. `set-connection` saves a new host profile; `set-identity` saves expected pins. Use each command's `--help` for explicit values and clears. `set-connection --expected-mode` saves the `connection.ifm_mode` expectation; it checks the observed mode and never sets it. Confirm device identity where exposed with `status PROFILE`; expected pins and selected IODD identity remain separate from observed matches. A Product ID difference requires review. Exact text pins distinguish unset, empty and spaces.

With explicit profiles for several ports on the same AL1350, use `status-ports PROFILE1 PROFILE2 ...` to check up to four distinct ports concurrently. Review each result independently. A mode or identity failure can coexist with successful results on the other ports. Use a separate approved `status PROFILE --verify-product-id` for an ISDU-based Product ID check.

If the selected IODD input layout has a paired output declaration, the applied profile retains it separately from input mappings. Use Device → Writes → Process output for a byte-for-byte preview with named expected and proposed values, declared width, and unmapped bit changes. **Read current value** can interpret the observed output before you use it as the expected image. **Edit one field** builds the proposed full image from the expected bytes and one unscaled value for a supported IODD field. A standalone Boolean uses `00` or `FF` for reviewed writes. Reads flag other bytes and retain their exact hex. Standalone padded integers write sign or zero padding for the applicable wire width and flag noncanonical observed padding. Re-import the relevant IODD to enable this coding. For saved bytes use **Offline → Process bytes → Output bytes** or `decode-output PROFILE HEX`. Choose an output-only layout by ID in Device → IODD import or with `import --layout ID`. It saves zero input bytes and retains the selected identity, output and parameter mappings. For a conditional layout, confirm the active condition in Mappings before live output interpretation or preview. Use CLI `status` or the explicit read controls for the configured port. Input monitoring, logging and live feeds require an input layout.

An explicit parameter or diagnostic read uses its own client, apart from the cyclic loop. Save named reads with `read-parameter --snapshot` and use `compare-parameters LEFT.json RIGHT.json` to compare exact bytes and both connection observations offline. The reads must share an ISDU address. Measure gateway-side contention on hardware. For authorized one-shot AL1350 mode, parameter, SystemCommand and process output writes, use [Device → Writes or the CLI preview](WRITES.md).

### Save a diagnostic read

```powershell
.\iolink-cli.exe read-diagnostics PROFILE.toml --snapshot .\.local\diagnostics.json
.\iolink-cli.exe review-diagnostics .\.local\diagnostics.json
```

Use a new file in an existing parent folder. The two independent 36:0 and 37:0 queries retain their own raw reply or error, host UTC and monotonic duration, and any accepted connection observation. The saved profile supplies event definitions offline. A partially successful read or a change in master or device identity remains reviewable. Exit 1 indicates a read, decode, context or save failure. Continue to treat source event history and loss as unknown.

### Compare diagnostic snapshots

```powershell
.\iolink-cli.exe compare-diagnostics .\.local\before-diagnostics.json .\.local\after-diagnostics.json
```

Use **Offline → Diagnostic snapshots → Compare two snapshots** in the GUI. The comparison reopens both files with their embedded event definitions and shows exact byte equality separately for **36:0** and **37:0** when both replies exist. Each snapshot's full review remains available. Files and endpoints are left as recorded.

Values and changed slot positions require two valid replies for that query and matching recorded gateway route, master model and firmware, port and mode, and observed Vendor ID, Device ID and device serial. AL1350 comparisons also require matching recorded master serials. Other observed identity fields must agree where both files supply them. A discrepancy between the two reads within either file suppresses interpreted differences across the comparison. A failed query can still leave the other query eligible.

The **changed_slots** list compares positions in the two saved **37:0** payloads. An omitted slot value denotes an absent position and **000000** denotes a present, empty slot. An empty list denotes matching slots. An omitted list means the comparison is unavailable and includes a reason in **detailed_status_reply**. Slot movement and byte changes do not establish event transitions or event-loss counts. Exit 1 flags an incomplete, malformed or internally inconsistent snapshot. Two valid files with differing recorded targets can still report raw equality and exit 0.

## Choose logging and inspect its plan

```powershell
.\iolink-cli.exe set-logging mapped.toml logging.toml --mode every-update --queue-capacity 8192 --rotate-rows 100000 --rotate-seconds 0 --field signal
.\iolink-cli.exe logging logging.toml
```

`set-logging INPUT OUTPUT` preserves unspecified settings and saves a new file. Choose one selection option: repeated `--field NAME` in CSV order, `--all-fields`, or `--raw-only`. Modes are `every-update` and `on-change`; zero disables each rotation limit. Queue capacity is 1–1,000,000 rows. Edited rotation limits must fit TOML's signed 64-bit integer range. GUI controls share candidate validation; apply/discard pending profile drafts before acquiring or saving.

`logging PROFILE` is an offline plan with configured row semantics, selected field mappings/origins and exact CSV headers. It contacts no gateway and verifies no rate/freshness. In Logging, expand Configured capture plan while idle for the summary. Every-update retains equal consecutive payloads when a new source update is verifiable. Snapshot adapters instead record host polls. On-change selects the first eligible row per connection generation and changes in the complete raw payload, validity, stale flag or diagnostic code; field selection/scaling does not change that decision.

Rotation is checked before the next row. Time rotation uses receive elapsed timestamps from the first row in each part; either enabled threshold rotates. Silence creates no empty timed files. Stop and flush the capture before treating its report as finalized.

## Capture several configured ports

Use one saved profile per distinct port on the same AL1350 origin or simulator route. Select the active input layout and confirm any required IODD condition on each physical profile. Give every port its own mapping and logging settings. For a simulator example, select the two bundled profiles in **Logging → Multiple ports** or pass them to the CLI:

```powershell
New-Item -ItemType Directory -Force .\.local | Out-Null
.\iolink-cli.exe capture-ports .\.local\sim-port-set .\samples\simulator.toml .\samples\simulator-port2.toml --seconds 2
.\iolink-cli.exe verify-capture .\.local\sim-port-set\port-2
```

The parent of OUTPUT must exist and OUTPUT must be new. The command validates the entire selection before opening a gateway. At most four ports run independently. All AL1350 profiles must use the same configured origin and compatible firmware pins. The sessions read the existing port configuration and input data. They have separate receive clocks and acquisition windows, so use each port report for timing, source gaps and overruns. AL1350 input remains a host poll with unknown source-update loss.

A separate folder per port contains the normal `profile.toml`, CSV parts and `capture.toml` report. When the workers and CSV writers finish, `capture-set.toml` records each port in selection order, its relative directory, whether the report was saved, and the outcome. The summary shows the first error per failed port. The port's `capture.toml`, when saved, has the detailed acquisition and writer results. When observed master identifiers disagree, the batch flags the conflict and returns exit 1 while retaining individual results. A failed port leaves other ports running through their own sampling and flush. Choose a new output folder to retry.

Ctrl+C or Ctrl+Break requests stop for all selected ports. A successful drain returns 130 on interruption or 0 at the end of the time limit. A port failure or conflicting master observations return 1, including on an interrupted run. Detailed guarantees and test scope are in [CLI interruption](CLI_LIFECYCLE.md). Use `capture` for single-port integration options. See [record several ports](GUI_WORKFLOW.md#record-several-ports) for the GUI workflow.

## Timing, loss and saved evidence

A timed run starts its requested duration once CSV and optional feed setup are ready. The duration includes gateway connection attempts. CSV, feeds and the report share the start time recorded before local setup. `local_setup_us` measures from that origin to acquisition start. `elapsed_us` measures from the same origin through acquisition end, before final drain. Timing views show the difference as the acquisition window. Saved captures without a setup measurement show that breakdown as unavailable.

Final-run timing and saved review retain [terminal host receive age](CAPTURE_REVIEW.md#terminal-host-receive-age): the interval from the final accepted receive to acquisition end, before writer drain. It is separate from source age and current GUI cache age. Missing measurements stay unavailable. Exact last-row checks require complete received-input coverage; filtered/dropped CSV does not identify the final receive.

The [live observation guide](LIVE_VIEW.md) distinguishes receipt quality, advancing host ages and the GUI display-age budget. Typed values hide for held/stopped/disconnected or unusable observations; raw input remains. The budget includes GUI handoff delay and does not reclassify saved data or establish freshness for snapshots. Use offline decode/review for historical typed values.

CLI Ctrl+C/Windows Ctrl+Break requests stop and drain; a successful interrupted capture saves/prints its report and returns 130. A capture/writer/integration failure takes precedence and returns 1. Wait for completion, then verify the saved directory. An in-flight gateway read finishes under its timeout. Repeated interruption does not force termination. See [CLI lifecycle and test scope](CLI_LIFECYCLE.md); closing the console or forcing process termination is outside the clean-drain guarantee.

Normal GUI close waits for [pending operations and capture flush](GUI_LIFECYCLE.md). Controls are disabled while closing. If capture completion fails during closing, the window stays open with the error; inspect the preserved output before retrying. Hidden-tab reviews are polled globally and cannot strand close waiting on an unread result. The linked operator checklist covers visible acceptance still to run.

GUI session starts also enforce [single ownership and launch completion](GUI_LIFECYCLE.md#starting-a-session). Wait for the previous cyclic result and apply/discard pending profile edits before starting. Each new run uses fresh display/stop state; failed thread launch preserves the previous report and observations. Independent parameter reads and saved reviews can remain pending during acquisition.

Saved verification also applies [quality consistency checks](CAPTURE_REVIEW.md#validity-staleness-and-classification-consistency). Counts describe received responses, not necessarily selected CSV rows. Invalidity and staleness can overlap. A new mapping does not replace the saved acquisition stale policy. Missing metrics or age data are not filled with zero.

All offline capture paths also share [selection/reconnect/source-gap accounting checks](CAPTURE_REVIEW.md#selection-reconnect-and-source-gap-accounting). Preserve a contradictory bundle for inspection. Missing legacy counters are not zero, and selected-row counter jumps are not a source-loss measurement when filtering or writer drops occurred.

Saved reviews also expose [recorded identity evidence](CAPTURE_REVIEW.md#recorded-identity-and-reconnects). Read configured expectations, last accepted status and rejected connections separately. The offline recheck uses only saved observed fields; producer text is a separate claim. Preserve a contradictory bundle for inspection. No live identity or ISDU query is issued by review.

All saved capture operations share [recorded timing consistency checks](CAPTURE_REVIEW.md#recorded-timing-consistency). A failure identifies the affected timing domain; preserve the original bundle for inspection rather than editing it to pass. Missing old measurements remain unavailable. Verification checks recorded consistency and does not remeasure device timing.

Use `compare-captures LEFT RIGHT` or **Offline → Saved captures → Compare captures** to verify and compare finalized folders. Read the [comparison policy](CAPTURE_REVIEW.md#compare-two-captures): required settings must match before arithmetic differences appear, and counts and terminal receive-age deltas additionally require equal recorded duration. Both complete reviews remain available even when deltas are suppressed. This never remeasures timing or follows saved endpoints.

Use `review-interfaces DIRECTORY` for a verified offline view of all recorded interface counters, including raw-only captures. The GUI's Verify capture shows the same diagnostics and timing. Read the [capture review guide](CAPTURE_REVIEW.md) before treating coalescing, retention or socket/commit counts as delivery or source loss. No gateway, broker or mapping is opened by review.

Keep the requested host interval, declared sensor minimum, reported port cycle, gateway update rate, source cadence, host poll/receive spacing and queue/writer timings separate. Treat 1 ms as a target to evaluate. AL1350 snapshots have no verifiable source update counter/timestamp in the supported path: source loss, source age and gateway update cadence are unknown. Host receive age is not sensor age. Simulator counters/timestamps are synthetic. Within-generation counter gaps cannot measure loss during disconnects.

Captures retain UTC start, monotonic host elapsed times, sample indexes, reconnect generations, raw input, validity, staleness, diagnostics, selected typed cells and finalized timing/accounting reports. Inspect p50/p95/p99/longest gaps, invalid/stale/unknown input and logger overruns. Review queue high water, queue wait, service/flush/sync and drain separately from source loss. On-change filtering is intentional selection, not a source gap. Invalid/stale values stay blank in typed cells.

`verify-capture DIRECTORY` verifies all parts. `review-capture DIRECTORY --field NAME` produces bounded offline signal summaries. `redecode-capture DIRECTORY MAPPING OUTPUT` exports a new verified bundle without replacing the original; `verify-export OUTPUT --source DIRECTORY` optionally verifies the explicitly supplied source. Parameter workflows use `parameters`, `defaults`, `decode-parameter`, `read-parameter --snapshot`, and `review-parameter`; a declared default is metadata, not an observed value or write. Saved parameter review uses embedded context without following stored endpoints or file paths. See the [README](../README.md) for their full examples.

## Integrations and support evidence

Integration includes CLI, saved TOML/JSON/CSV and an opt-in [read-only local HTTP API](LOCAL_API.md). Enable it per acquisition in Integration or use capture `--api-listen 127.0.0.1:8765`; it stops with acquisition. Optional [MQTT publishing](MQTT.md) runs on its own bounded handoff with intentional latest-state coalescing and separate loss accounting. Optional [Windows shared-memory snapshots](SHARED_MEMORY.md) run independently with bounded queues and a named mutex; CLI `read-shared-memory` reads without a gateway. C FFI remains [future work](INTERFACES.md). Keep simulation/loopback reports apart from physical results. For a bench issue, retain firmware, identity, profile, capture report and raw bytes in private storage; publish only sanitized reproductions.
