# IOLinkWorkbench

Read and log one IO-Link sensor on one configured port from a portable Windows x64 desktop application or headless CLI. The application opens offline with a synthetic sensor. Its first hardware adapter targets the ifm AL1350 IoT Core HTTP(S) snapshot API.

## Table of Contents

- [Install](#install)
- [Usage](#usage)
- [AL1350 bench setup](#al1350-bench-setup)
- [Capture files and timing](#capture-files-and-timing)
- [Maintainers](#maintainers)
- [Contributing](#contributing)
- [License](#license)

## Install

Extract the Windows x64 ZIP into a writable folder. Open **iolink-workbench.exe** for the desktop app or run **iolink-cli.exe** in PowerShell. The package uses the static MSVC C runtime and Windows system libraries. The GUI uses the machine's OpenGL graphics driver. The preview targets Windows 10/11 x64.

Optional XSD validation uses the installed Windows PowerShell and .NET Framework **System.Xml** validator. The application locates Windows PowerShell in the Windows system directory. If that component is unavailable or blocked by local policy, an import that requests XSD validation fails with an error. The validator runs without a visible window or user profiles and has a 20-second limit. Logging and decoding run in Rust.

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
.\iolink-cli.exe import .\samples\sensor.xml .\.local\resolved.toml --variant SIM-SENSOR --standard-definitions .\samples\synthetic-standard-definitions.xml
.\iolink-cli.exe validate .\.local\imported.toml
.\iolink-cli.exe parameters .\.local\imported.toml
.\iolink-cli.exe read-parameter .\.local\imported.toml V_SIM_Threshold
.\iolink-cli.exe events .\.local\imported.toml
.\iolink-cli.exe decode-diagnostics .\.local\imported.toml E48DFE000000 --device-status 02
.\iolink-cli.exe read-diagnostics .\.local\imported.toml
.\iolink-cli.exe import .\samples\sensor-record.xml .\.local\record.toml
.\iolink-cli.exe decode-parameter .\.local\record.toml V_SIM_Record FFEC010A54455354
.\iolink-cli.exe read-parameter .\.local\record.toml V_SIM_Record --subindex 1
.\iolink-cli.exe import .\samples\sensor-array.xml .\.local\array.toml
.\iolink-cli.exe decode .\.local\array.toml 02C495F0
.\iolink-cli.exe read-parameter .\.local\array.toml V_SIM_History --subindex 2
.\iolink-cli.exe import .\samples\sensor.xml .\.local\german.toml --variant SIM-SENSOR --language de --unit-definitions .\samples\synthetic-unit-definitions.xml
.\iolink-cli.exe capture .\samples\simulator.toml .\.local\sim-run --seconds 2 --simulate-faults
.\iolink-cli.exe verify-capture .\.local\sim-run --joined-csv .\.local\sim-run-joined.csv
.\iolink-cli.exe review-capture .\.local\sim-run --field level --buckets 128
~~~

**IODD scope.** The importer accepts UTF-8 or BOM-tagged UTF-16 IODD 1.1 XML and ZIP bundles. ZIP imports select a unique main IODD by filename or accept an exact member with **--entry**. Use **--variant** when the IODD lists several product IDs and **--layout** for multiple or conditional input layouts. For conditional physical captures, set **confirm_iodd_condition = true** after verifying the active condition on the sensor. The importer supports scalar signed and unsigned integers, Boolean and Float32, including those fields in an input record or top-level array. Array fields receive stable one-based element suffixes such as **Temperature [2]** and share the array's process-input display metadata. It resolves local datatype references, selected-language names, process-input display scaling, and unit codes. Unit code 1997 explicitly specifies no unit. Unsupported fields, unknown unit codes, and conditions appear for review. Duplicate variant, datatype, language, or text IDs and overlapping supported input fields are rejected. Profile snippets, nested complex layouts, and additional IODD Checker semantic/stamp checks are roadmap items.

**Languages and units.** Add **--language TAG** to **iodd** or **import** to select an embedded **Language** or a companion **ExternalTextDocument**. Without a tag, the importer uses **PrimaryLanguage**. For XML, a requested companion is the adjacent **MAIN-STEM-TAG.xml**. For ZIPs it is the matching **MAIN-STEM-TAG.xml** in the selected main entry's folder; no files are extracted. ZIP catalogs list language tags discovered from companion filenames, and the selected file's declared **xml:lang** must match. **--language-file PATH** selects an explicit language XML and requires a tag. Tags match case-insensitively. Missing translated IDs use the primary text and are listed in **iodd.localization.fallback_text_ids**. Conflicting embedded/external definitions, duplicate IDs, unknown translation IDs, and differing declared release dates cause an error. Ordinary standard-parameter names retain the supplied standard definitions' primary text; supported Direct Parameter Page 2 **TextRedefine** labels are applied.

Add **--unit-definitions PATH** to load abbreviations from a local **IODD-StandardUnitDefinitions1.1.xml**. Supplied codes take precedence, other common codes use the built-in table, and unresolved codes remain visible for review. Abbreviations affect display/CSV units and preserve the mapping's gradient and offset. The full official V1.1.9 table contains 688 entries. The included **synthetic-unit-definitions.xml** is an original minimal fixture. Obtain official files from the [IO-Link Community download area](https://io-link.com/downloads) and keep them in ignored **.local/**.

GUI Setup has the same optional language, explicit language XML, and unit XML inputs. Inspection displays the selected language, available tags, fallback count, and unit source. Saved profiles retain the resolved labels/units, language choices, fallback IDs, and supplied resource filenames/version. Reimporting in another language changes field names and CSV headers; review retained manual fields, overrides, and logging selections before use. Existing profiles retain their saved names and units. XML companion/unit resources receive parser checks; the optional XSD receipt continues to cover only the main XML.

**Optional XSD check.** Obtain the matching IODD 1.1 schema directory from the [IO-Link Community specification package](https://io-link.com/downloads), including its local includes and **xml.xsd**, and keep it under ignored **.local/**. Run **check-iodd XML_OR_ZIP --xsd ROOT_XSD [--entry MEMBER]** for a full machine-readable report. Add **--xsd ROOT_XSD** to **iodd** or **import** to require a validation pass before inspecting or saving a profile. In GUI Setup, enter the optional root XSD before **Inspect IODD**. Inspection runs in a background worker, and schema failures leave the applied profile unchanged.

~~~powershell
.\iolink-cli.exe check-iodd .\.local\vendor-device.zip --xsd .\.local\schemas\IODD1.1.xsd
.\iolink-cli.exe import .\.local\vendor-device.zip .\.local\checked.toml --variant YOUR-PRODUCT-ID --xsd .\.local\schemas\IODD1.1.xsd
~~~

The checker uses [System.Xml XSD 1.0 validation](https://learn.microsoft.com/en-us/dotnet/standard/data/xml/xml-schema-xsd-validation-with-xmlschemaset), including schema identity constraints. Reports include line/column diagnostics and SHA256 hashes of the selected XML's exact bytes and the schema files used. Successful imports save this receipt under **iodd.schema_validation**. The GUI and **validate PROFILE** distinguish a saved main-XML validation pass from parser checks alone. Existing profiles load with validation unperformed unless they contain a receipt. The receipt records the original XML/schema check; profile edits, standard-definition XML, IODD Checker semantic/stamp rules, connected identity, and physical operation have separate checks. A supplied schema's authenticity remains a user choice.

Schema includes/imports are explicitly loaded from within the selected local schema directory. HTTP, UNC/network-drive roots, escaping paths, and child reparse points are rejected. DTDs are prohibited. Document-supplied schema hints and inline schemas are ignored. Limits are 8 MiB per XML/schema file, 64 schema files, 32 MiB of schemas, and 100 diagnostics. Validation warnings also prevent an import pass. The helper is attached to an application-owned Windows job and is terminated if the application exits. Official schemas are supplied locally and are excluded from the portable ZIP. The bundled synthetic IODDs are parser/decoder fixtures with abbreviated headers; use vendor IODDs or official examples for validation against the complete official schema.

**Manual mapping.** Set **input_bytes** to the exact input width. Each **[[fields]]** table describes a value with a name, bit offset, width, kind, byte order, scale, offset, and unit. Bit zero is the least significant bit of the last payload byte for big endian, or the first payload byte for little endian. The decoded value is **raw × scale + offset**. Optional **decimals** controls the decimal places in the typed display and CSV. A field with the same name as an imported field replaces its mapping. Set **logging.fields** to an ordered list of effective names or leave it empty to log all mapped fields. The raw payload is always included in CSV.

**Device diagnostics.** IODD imports save event codes, declared types, localized names, and descriptions. Referenced **StdEventRef** definitions use the optional **--standard-definitions** file; unresolved codes remain listed for review. Use **events PROFILE** to browse the saved catalog. Older profiles can be reimported to add event metadata.

In **Monitor & CSV**, expand **Device diagnostics (explicit ISDU reads)** and choose **Read device diagnostics**, or run **read-diagnostics PROFILE**. This explicitly reads standard Device Status **36:0** and Detailed Device Status **37:0** on independent acyclic clients with the usual identity checks. Each read reports its raw bytes, host UTC completion time, monotonic query duration, and available connection identity. A failed query retains the other query's successful result. Read failures, malformed payload lengths, or differing observed identity fields produce a nonzero CLI exit. The two reads are separate snapshots; they do not establish an atomic device state or a complete event history. Source event loss stays unknown, and cyclic logging does not initiate these reads.

The decoder follows the [IO-Link Interface specification](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf): each Detailed Device Status slot contains one qualifier octet followed by a big-endian two-byte event code. It accepts 1–64 complete slots, counts all-zero empty slots, and preserves raw bytes and slot positions. Qualifier fields show instance, source, type, and mode. Saved device IODD names apply only to device-application events. Reserved fields, unknown codes, unexpected pending-event shapes, and type mismatches receive review notes. An empty slot list does not establish overall device health.

Use **decode-diagnostics PROFILE HEX [--device-status HEX]** or **Offline device diagnostics** in the GUI for a recorded response through the same decoder. The synthetic sensor returns **02** at **36:0** and **E48DFE000000** at **37:0**, containing one synthetic warning and one empty slot. Gateway **device_status**, **port_event**, and **device_event** fields remain raw connection-time snapshots. CSV **diagnostic_code** is the transport response code. Those gateway fields are separate namespaces from these standard ISDU values. Device support and the timing impact of diagnostic queries remain physical bench checks.

**IODD parameters.** Importing an IODD also saves its explicit **Variable** definitions in the profile. The catalog shows parameter IDs, names, indexes, declared access rights, defaults, supported single-value labels, and ranges. In **Monitor & CSV**, expand **IODD parameters**, select an entry and subindex, and choose **Read named parameter**. The CLI provides **parameters PROFILE** for offline browsing and **read-parameter PROFILE ID [--subindex N]** for one explicit ISDU read. Subindex zero selects the whole variable. Older profiles can be reimported to add this metadata.

Named reads use the independent acyclic client and report the raw bytes, supported raw scalar or text value, host read time, and total query duration. Integer, Boolean, Float32, UTF-8, and US-ASCII decoding are supported. Numeric **VariableRef** and **RecordItemRef** menu entries supply gradient, offset, common unit codes, and **Dec.0** through **Dec.12** formatting. Read results retain each supported menu display separately alongside the raw value. Multiple menu displays, missing or unknown units, unsupported formats, and complex types appear for review. Unsupported parameter values remain available as raw hexadecimal. A decoding error retains the returned bytes for review. Write-only named entries are rejected before transport access. IODD defaults and ranges describe the device's declared settings.

**Parameter records.** **RecordT** imports preserve member subindexes, bit offsets, known widths, access restrictions, defaults, choices, and display mappings. Whole-record responses use the declared big-endian layout. Known members are decoded independently, so a malformed text member retains its bytes while other members remain visible. Width mismatches retain the full response and suppress member decoding. Import/profile checks reject duplicate or zero subindexes, overlapping known fields, members outside the record, and increased access rights. Unresolved member widths appear for review because complete overlap checking is unavailable. Named member reads require **subindexAccessSupported** and readable parent/member access. These checks run before opening a transport. Write-only members of a whole-record response retain extracted bytes with a review message. Nested complex layouts still require manual interpretation.

**Arrays.** Cyclic numeric/Boolean arrays expand into fields for the existing typed views, manual overrides, and CSV pipeline. Acyclic **ArrayT** metadata saves the count, element datatype, access, default, and display settings. Whole-array reads and offline decoding report **array_items**; **--subindex N** reads one permitted element. A **VariableRef** display applies to every element. Standard **fixedLengthRestriction** reduces an array's count and recalculates its layout. Sub-byte elements share an octet without crossing its boundary; byte-sized and larger elements require widths that are multiples of eight. The first element occupies the highest used position and the final element starts at bit zero. Padding bits remain visible in the original payload and are ignored by typed fields. Packed signed member bytes can differ from the sign-padded standalone ISDU bytes while decoding to the same value. Numeric and supported text elements decode independently. Octet strings retain their hexadecimal bytes. Unsupported widths/types, oversized layouts, and nested complex elements require review and the raw/manual path. The **sensor-array.xml** fixture defines **67:0** as **02C495F0**, with **67:1** through **67:7** for seven signed four-bit elements. The cyclic and acyclic limits remain 32 and 232 bytes respectively.

Use **decode-parameter PROFILE ID HEX [--subindex N]** or **Offline ISDU parameter decoder** in the GUI to decode a recorded response through the same decoder without opening the configured endpoint. Acyclic payloads accept 1–232 bytes independently of the 32-byte cyclic input limit. The offline CLI prints retained bytes and decoding diagnostics, and exits nonzero when decoding reports an error. The **sensor-record.xml** fixture defines the synthetic eight-byte record at **66:0**, with **66:1** through **66:4** for its numeric, Boolean, enumeration, and text members. ISDU queries remain independent of cyclic logging.

For standard **StdVariableRef** entries, supply a local **IODD-StandardDefinitions1.1.xml** using **--standard-definitions PATH** on **iodd** or **import**, or the optional definitions path in GUI Setup before inspecting the IODD. Obtain the official file from the [IO-Link Community specification package](https://io-link.com/downloads) and keep it in ignored **.local/**. The importer resolves only the referenced variables, records the definitions filename and declared version, applies scalar defaults, text/array length restrictions, supported choice restrictions, and **StdRecordItemRef** defaults/choices, and saves the resolved catalog in the profile. The definitions file is then unnecessary for named reads. Unresolved IDs remain visible for review. The included **synthetic-standard-definitions.xml** is an original test fixture for the synthetic sensors. Official definitions are supplied locally and are not included in the application package.

Display mappings are editable in the profile TOML under **[[iodd.parameters.displays]]** and its **field** table. Preserve the parameter's raw width, kind, bit offset zero, and big-endian byte order while overriding scale, offset, decimals, or unit. Profile validation rejects changes to that raw layout. Older profiles can be reimported to add definitions and display metadata.

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

If a reconnect is rejected, **last_status** retains the last accepted device and its freshness contract. **rejected_status** records the rejected candidate. The report's **format_version = 1** identifies the saved capture format.

Use **verify-capture DIRECTORY** or **Verify capture** in **Review saved capture** under the GUI's **Offline decode** tab to inspect a finalized run using its saved profile. Verification checks the complete numbered part set, headers, final records, UTC origin, sample and generation order, freshness metadata, raw-to-typed decoding, and CSV row accounting. It accepts the earlier capture reports that omit the format version or unconfirmed-row count. A verification error identifies the affected file and data row where available.

Pass **--joined-csv NEW_FILE** to combine the checked parts into one UTF-8 CSV with LF record endings and one header. The new file is published after all checks pass. Existing destination files cause an error. Missing selected sample indexes describe rows absent from CSV, and recorded source-loss fields retain the original report's meaning. The verification summary includes **capture_had_error** when the original acquisition reported a terminal or writer error. Successful verification establishes consistency between the saved files and report.

**Offline signal review.** Run **review-capture DIRECTORY [--field NAME] [--buckets N]** to verify every CSV row and review one logged field in the same streaming pass. The field name must exactly match the saved logging selection; omission selects its first field. In the GUI's **Review saved capture** section, choose **Review field**, then select another saved field and review again if needed. Review uses the saved capture profile and does not open its gateway endpoint or use the current Setup profile. It retains the verification summary, source-loss meaning, generation count, recorded logger overruns, and any original capture-error flag.

Statistics use the saved typed CSV cells, including their scaling and decimal formatting. Blank cells for invalid data, unknown validity, or known staleness are counted separately from non-finite numeric cells; both groups are excluded from numeric statistics. Boolean false/true becomes 0/1. Minimum, maximum, mean, and plotted values use approximate **f64** arithmetic; integers wider than 53 significant bits may lose numeric precision. The first and last nonblank cell texts remain exact, including non-finite representations. The source CSV and raw bytes remain unchanged. These describe saved rows: change-only selection, invalid/stale periods, and missing rows prevent interpreting the mean as a time-weighted or complete source-population mean.

The preview allocates **1–4096** time bins across the recorded host elapsed interval, independently of capture length. Each bin retains finite minimum/maximum and the last finite value, preserving observed extrema rather than selecting occasional rows. Empty bins remain empty. The GUI draws independent range marks and a last-value dot, with amber shading for bins containing blank/non-finite cells or multiple generations. No lines join bins. Bins may group multiple events or generations; use the CSV for exact ordering. All capture parts and fields must pass verification before a review result is returned.

The GUI also shows the acquisition report's recorded port cycle, requested host interval, and separate host poll, host receive, observed new-update, and gateway timestamp spacing summaries. These are retained measurements from the original acquisition, not timing inferred from the reduced CSV row set. Missing legacy measurements remain unavailable. The configured transport, recorded route, and simulation settings remain visible; offline review does not establish physical hardware acceptance.

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
