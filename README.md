# IOLinkWorkbench

Read and log one IO-Link sensor on one configured port from a portable Windows x64 desktop application or headless CLI. The application opens offline with a synthetic sensor. Its first hardware adapter targets the ifm AL1350 IoT Core HTTP(S) snapshot API.

## Contents

- [Install](#install)
- [Usage](#usage)
- [AL1350 bench setup](#al1350-bench-setup)
- [Capture files and timing](#capture-files-and-timing)
- [Maintainers](#maintainers)
- [Contributing](#contributing)
- [License](#license)

## Install

Extract the Windows x64 ZIP into a writable folder. Open **iolink-workbench.exe** for the desktop app or run **iolink-cli.exe** in PowerShell. The package uses the static MSVC C runtime and Windows system libraries. The GUI uses the machine's OpenGL graphics driver. The preview targets Windows 10/11 x64.

To build from source, install stable x64 MSVC Rust, the Visual Studio C++ Build Tools, and a Windows SDK. Build from the repository root.

~~~powershell
cargo build --locked --release --target x86_64-pc-windows-msvc
~~~

## Usage

In the GUI, use **Setup** to load, edit, and save a named TOML profile. The profile editor accepts the same TOML as the CLI. Choose **Inspect IODD** to list the variants and input layouts, select the matching entries, then use **Apply IODD layout**. The effective field list distinguishes IODD fields, overrides, and manual fields. **Edit as override** copies a field into the editable TOML. **Add manual field** inserts a starter mapping. Apply edits before connecting or saving.

Use **Monitor & CSV** to inspect the configured port or start a capture in a new directory. **Stop and flush** drains the writer. Use **Offline decode** to paste a hexadecimal input and view the same raw and typed fields without a connection.

Run these examples from the extracted package folder.

~~~powershell
New-Item -ItemType Directory -Force .local
.\iolink-cli.exe decode .\samples\simulator.toml FFF9
.\iolink-cli.exe iodd .\samples\sensor.xml
.\iolink-cli.exe import .\samples\sensor.xml .\.local\imported.toml --variant SIM-SENSOR
.\iolink-cli.exe validate .\.local\imported.toml
.\iolink-cli.exe capture .\samples\simulator.toml .\.local\sim-run --seconds 2 --simulate-faults
~~~

**IODD scope.** The importer accepts UTF-8 or BOM-tagged UTF-16 IODD 1.1 XML and ZIP bundles. ZIP imports select a unique main IODD by filename or accept an exact member with **--entry**. Use **--variant** when the IODD lists several product IDs and **--layout** for multiple or conditional input layouts. For conditional physical captures, set **confirm_iodd_condition = true** after verifying the active condition on the sensor. The importer supports scalar signed and unsigned integers, Boolean and Float32, including those fields in an input record. It resolves local datatype references, primary-language names, process-input display scaling, and common standard unit codes. Unsupported fields, unknown unit codes, and conditions appear for review. External language files, profile snippets, arrays, and full schema validation are roadmap items.

**Manual mapping.** Set **input_bytes** to the exact input width. Each **[[fields]]** table describes a value with a name, bit offset, width, kind, byte order, scale, offset, and unit. Bit zero is the least significant bit of the last payload byte for big endian, or the first payload byte for little endian. The decoded value is **raw × scale + offset**. Optional **decimals** controls the decimal places in the typed display and CSV. A field with the same name as an imported field replaces its mapping. Set **logging.fields** to an ordered list of effective names or leave it empty to log all mapped fields. The raw payload is always included in CSV.

## AL1350 bench setup

The adapter follows ifm's AL1350 IoT Core contract for the 3.1.x firmware documentation. It uses a Rust HTTP client and the Windows Schannel TLS backend and requires no ifm SDK, driver, moneo runtime, or application license key for the implemented requests. The AL1350 model and actual firmware are read at connection. This adapter has passed a loopback contract fixture. Hardware and individual firmware revisions await bench qualification. [ifm product data](https://www.ifm.com/gb/en/product/AL1350?tab=documents) · [ifm AL1350 operating instructions, 80284128/03](https://www.quicktimeonline.com/assets/images/pdf/IFM%20Electronic/operating-instruction-AL1350.pdf)

1. Copy [the AL1350 template](samples/ifm-al1350.toml) to **.local/**. Enter the Ethernet IoT endpoint, an already configured IO-Link port, the sensor's input width, and its mapping or imported IODD. Set an exact **ifm_firmware** pin once you know the version.
2. Confirm the port mode in the existing gateway setup. If the endpoint reports a numeric mode, set **ifm_mode** to the confirmed IO-Link mode code. The application checks the mode and reads the connected device's numeric IDs before sampling. Pin the expected IDs, and optionally product name or serial, to reject mismatches.
3. If the installation uses HTTPS, Windows must trust the gateway certificate. For enabled IoT Core authentication, set **IOLINK_IFM_PASSWORD** in the process environment and use an HTTPS origin. The adapter sends the documented administrator authentication object over TLS.
4. Use **status** before capture. If the profile pins an IODD Product ID, **status --verify-product-id** or **Verify Product ID** in the GUI reads ISDU index 19:0 and compares it explicitly. During capture, the gateway's cyclic snapshot endpoints supply Vendor ID, Device ID, product name, and serial where present. Product ID verification and other explicit ISDU reads use a separate client and worker. The gateway or port can still have shared service time.
5. Confirm the sensor's byte order and values against a known observation before relying on typed data. The gateway's reported cycle is preserved raw. Set **ifm_cycle_unit** to **microseconds** or **milliseconds** after confirming the unit for the installed firmware. The cited manual gives conflicting units in its setup and API sections.

The app sends only documented data reads and explicitly requested ISDU reads. Port configuration, device commands, ISDU writes, and process outputs are outside this build.

~~~powershell
.\iolink-cli.exe status .\.local\bench.toml --verify-product-id
.\iolink-cli.exe capture .\.local\bench.toml .\.local\bench-run --seconds 30
~~~

## Capture files and timing

Each run creates a new directory with **profile.toml**, **capture.toml**, and UTF-8 **capture-NNNN.csv** files. Rotation uses the configured row or elapsed-time limit. Each CSV has the UTC origin, monotonic host receive elapsed time, selected sample index, connection generation, row kind, validity, staleness and source metadata where exposed, diagnostic code, raw bytes, and selected typed values. Typed cells are blank for invalid or known-stale rows. A queue overrun drops the newest selected row and leaves a gap in saved sample indexes. Ctrl+C requests a clean drain and returns exit code 130 after the report is saved.

The report accounts for selected rows as **csv_rows_flushed + csv_rows_unconfirmed + counts.logger_overruns**. Unconfirmed rows were accepted by the queue but have no confirmed successful flush. A writer error stops the capture and retains its report and completed CSV parts. Staleness uses both the gateway-reported age and the time since the host last observed an advancing source counter or timestamp. A frozen source therefore becomes stale even when the gateway continues reporting zero age.

- **Every update** writes each observed, verifiably new update when the adapter provides source freshness. Equal payloads with new counters still log. Counter gaps are counted within a connection generation. The first update after reconnect starts a new generation and loss during the disconnect is unknown.
- **On change** compares the whole raw payload, validity, stale flag, and diagnostic code within a generation. It emits the first eligible row in each generation. A full queue leaves the previous queued value as the comparison point, so the next eligible observation can retry a dropped change.
- **AL1350 snapshots** are always labeled **host_poll**. The response's IoT Core status distinguishes success and documented invalid-data errors. The snapshot service exposes no source timestamp or update counter. Input age, source-update loss, and gateway update cadence stay unknown on this path. Repeated successful reads do not establish freshness.
- **Timing** reports p50, p95, p99, longest gap, and mean rate for host poll starts and host receive completions. It separately reports observed new-update receive spacing and gateway timestamp intervals where available. The master-reported IO-Link port cycle is its own field. Host timing includes scheduling and network/API effects. **1 ms** is a target for the eventual sensor and gateway qualification.

See the [roadmap](ROADMAP.md) for physical acceptance.

## Maintainers

[votecoffee](https://github.com/votecoffee)

## Contributing

Build a portable ZIP from a clean checkout with **python tools/package_windows.py .local/dist**. Packaging checks the two PE subsystems and static C runtime imports, includes the dependency notices, and writes the archive SHA-256.

Run **cargo test --locked**, **cargo clippy --locked --all-targets -- -D warnings**, and **python tools/verify_headless.py target/x86_64-pc-windows-msvc/release/iolink-cli.exe** after building. The headless suite covers synthetic source updates and an AL1350 JSON fixture on loopback. It keeps generated profiles, ZIP fixtures, reports and CSV under ignored **.local/**. Keep vendor IODDs, installed configuration, and raw bench captures there too.

Named profiles, the separate GUI and CLI entry points, and the bounded CSV writer follow patterns from the read-only [EIPMaster reference](https://github.com/votecoffee/EIPMaster/tree/593292766a1d927a405d7500aeca0b6583223589). The IODD reader was checked against the [IO-Link Community examples and standard unit definitions](https://io-link.com/downloads).

## License

[MIT](LICENSE). The portable archive includes third-party notices.
