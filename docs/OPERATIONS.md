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

### Compare saved input mappings

After saving both profiles, open **Offline → Process bytes → Compare saved mappings** and choose **Input**. Enter both TOML paths, review the summary, and expand either profile to see its effective definitions in field order.

~~~powershell
.\iolink-cli.exe compare-input-mappings before.toml after.toml
~~~

The comparison checks input byte width and encoding, the saved IODD variant and layout condition, and effective fields by exact name. It lists added and removed fields, changed properties and origins, and a change in the relative order of fields common to both profiles. A renamed field appears as a removal and an addition. Each definition shows whether it comes from the IODD, an override, or a manual field. Both profiles are read offline. The comparison covers saved input definitions and requires a separate connected-device check to establish the actual active layout.

Enter **Recorded payload hex (optional)** to decode the same recorded input with both profiles.

~~~powershell
.\iolink-cli.exe compare-input-mappings before.toml after.toml --payload "03 E9"
~~~

The preview compares decoded fields by exact name and shows changes in values, units, display text, or raw bits. Each profile's result includes its wire encoding warnings and field errors. If one profile expects a different byte width, the other profile's result remains available. Malformed text in one field still allows comparison of other usable fields. The CLI prints the available results and returns 1 when a profile or field cannot decode the payload. Wire encoding warnings by themselves keep exit 0.

### Compare saved output mappings

Choose **Output** in **Offline → Process bytes → Compare saved mappings** to compare the selected IODD output layouts from two TOML profiles. Review wire byte width, declared bit length, encoding, layout selection and condition, and output fields by exact name. A profile without a saved output layout is identified in the result. Expand both sources for the saved definitions and any IODD review notes.

~~~powershell
.\iolink-cli.exe compare-output-mappings before.toml after.toml
.\iolink-cli.exe compare-output-mappings before.toml after.toml --payload "00 05"
~~~

The optional recorded bytes decode against each selected output layout independently. The report retains any successful result when the other profile lacks a layout or expects a different width. The CLI returns 1 after reporting a payload decoding error. Noncanonical Boolean or integer padding produces a warning alongside the decoded value.

### Check a conditional layout

Select the IODD layout in the profile. In **Mappings → Layout condition**, review the selector's ISDU index, subindex, and declared value. Use **Read layout selector** for one acyclic observation.

```powershell
iolink-cli check-layout profile.toml
iolink-cli check-layout profile.toml --read
```

The first command resolves the selector offline. With `--read`, Workbench checks the configured connection and identity pins, reads the selector, and compares its unscaled Boolean or integer value with the IODD value. The report includes the observed port and identity, host read time, and raw reply. A mismatch or undecodable reply returns exit code 1 and keeps the report on stdout.

The read uses the independent acyclic path. For a record or array whose IODD disables individual subindex access, the check reads the whole value at subindex 0 and compares the declared member of that reply. The plan's `subindex` is the selector member and `read_subindex` is the actual ISDU address component. The result's `selector_raw_value` is the value compared, and `reading.raw_hex` contains the complete reply. [IO-Link Interface and System Specification, section 4.4](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)

The selector and its parent must both be readable. An unresolved selector or a prohibited read is reported before transport access. Before cyclic acquisition or live output review, use **Confirmed active condition on the device** after checking the layout on the device. The check works with existing profiles whose stored condition resolves to an imported parameter.

### Acyclic-only IODDs

Select the acyclic-only layout in **Device → IODD import**. The CLI uses the ID of the empty `ProcessData` item with `import --layout ID`. The sole unconditional layout is selected automatically.

The saved profile retains identity, parameter and event definitions with zero input bytes. Use **Parameters** for ISDU reads and snapshots, **Diagnostics → Device events** for diagnostic reads, and **Device → Writes** for reviewed ISDU operations. Process input acquisition and process output operations require corresponding cyclic data. Acyclic-only profiles require v0.83.0 or later.

### Cyclic state names

Import an IODD to show its localized `SingleValue` names for Boolean, signed or unsigned integer, and finite Float32 process inputs and outputs. The same matching applies to packed record members and array elements. Workbench compares the exact unscaled value and displays the name alongside the regular value. Float32 names match the declared value rounded to binary32, before engineering scaling or decimal display rounding. Both signs of zero share the same IODD state name and retain their original raw value and bits. Unnamed values, duplicate declarations for a value, nonfinite declarations, and unresolved names display the regular value. [IO Device Description V1.1.5 schemas, Package 2025](https://io-link.com/downloads)

Named integer and Float32 `ValueRange` declarations include both boundaries. Workbench keeps a range label when the range is valid and disjoint from the other valid ranges and exact values, including unnamed declarations. Overlapping ranges fall back to the normal value display. The label describes the IODD range and acquisition validity remains a separate observation.

See the name in **Inputs**, the mapping preview, **Offline → Process bytes**, or the reviewed process output view. CLI `decode` prints the name alongside the value. In `decode-output` JSON, the decoded field's `value_name` accompanies its numerical or Boolean value and raw bits. CSV records the usual engineering or Boolean value. A unit or scaling override retains the name lookup on the original raw field. Changing the width, offset, byte order or type through the mapping editor or `set-field` clears the names for that override. Removing the override restores the imported field and names.

Reimport earlier IODDs to save their names. The existing `value_names` mapping was added in v0.98.0. Importing Float32 names and applying the signed-zero lookup require v0.108.0 or later. Profiles and captures containing `value_ranges` require v0.109.0 or later. Existing profiles can be reopened before reimporting to add range labels.

### Declared device capabilities

Open **IODD declared capabilities** in the **Device → IODD import** preview or in **Mappings** after applying the IODD. The section shows block parameter transfer, device Data Storage support, the four declared access lock capabilities, and the profile identifiers in decimal and hexadecimal. Missing optional declarations appear as **Undeclared**. The CLI's `iodd FILE` output includes a `[features]` table. `validate PROFILE` shows the saved declarations. [IODD V1.1.5, section 7.5.1](https://io-link.com/downloads)

These entries describe the device capabilities in the IODD. Actual lock state, master Data Storage configuration, and the connected device's profile IDs are separate observations. Reimport an existing IODD to save its capabilities. Saved profiles and captures containing `iodd.features` require v0.111.0 or later.

For a recorded whole **ProfileCharacteristic** reply at index 13, subindex 0, open **Offline → Parameter bytes → Device profile IDs (13:0)** and paste the hex. Workbench shows the raw reply and the 16-bit identifiers in wire order. With a saved IODD declaration it reports the missing and unexpected identifiers. It flags incomplete words, repeated IDs, and replies out of ascending order. A missing declaration still allows decoding. [IO-Link Common Profile V1.2.1, Annex B.4](https://io-link.com/fileadmin/user_upload/Downloads/Package_2025/IO-Link_Common_Profile_Specification_10.072_V1.2.1_Oct2025.zip)

```powershell
.\iolink-cli.exe decode-profile-characteristic "4000 8000" --profile device.toml
```

Omit `--profile` to decode the reply by itself. The CLI returns 1 for a malformed list or a difference against the declared IDs and prints the available evidence. Both views use the supplied bytes offline.

### Device Access Locks

In **Diagnostics → Device profile → Device access locks (12:0)**, choose **Read access locks** for the configured port or **Read and save lock snapshot** to retain the query evidence. Open the JSON under **Offline → Diagnostic snapshots → Access locks snapshots**. To decode a recorded reply directly, paste it under **Offline → Parameter bytes → Device access locks (12:0)**. [IO-Link Interface and System V1.1.5, B.2.4](https://io-link.com/fileadmin/user_upload/Downloads/Package_2025/IOL-Interface-Spec_10002_V1.1.5_Oct2025.zip)

```powershell
.\iolink-cli.exe read-device-access-locks device.toml --snapshot locks.json
.\iolink-cli.exe review-device-access-locks locks.json
.\iolink-cli.exe decode-device-access-locks "00 0C" --profile device.toml
```

The standard reply has two bytes. Bits 0 and 1 are the legacy parameter-write and Data Storage lock flags. Bits 2 and 3 report local parameterization and local user interface locks. The view shows each observed flag and the saved IODD's support declaration when present. An unavailable read keeps its error and available connection context. The CLI returns 1 for read failures, invalid widths, set reserved bits, or a set flag the IODD marks unsupported. This is the device's recorded lock word. Master Data Storage status and write permission have their own controls and evidence.

#### Compare saved access locks

Open **Offline → Diagnostic snapshots → Access locks snapshots → Compare two access locks snapshots**. Enter the first snapshot above the comparison and the second path inside it. Expand the left and right source reviews for the individual replies, read errors, IODD declarations, and recorded identity.

~~~powershell
.\iolink-cli.exe compare-access-locks before.json after.json
~~~

The comparison reports exact wire equality when both replies exist and a 16-bit XOR mask when both are two bytes. It lists named set/clear changes when the replies pass lock decoding review, the recorded master, port, and device context matches, and both profiles agree on IODD lock support, including when both leave support undeclared. If this interpretation is unavailable, the CLI returns 1 and prints both reviews with any available wire comparison. Exit 0 means the flag comparison was available, including when flags changed. This is a comparison of two saved observations.

### Acyclic process data readback

In **Diagnostics → Device profile → Acyclic process data readback**, select **Input (40:0)** or **Output (41:0)** and choose **Read process data**. These optional read-only objects return the device’s last valid process data for the selected direction. The displayed host time records when the read completed. The device sample time and current process-data validity remain unknown. [IO-Link Interface and System V1.1.5, B.2.23–24](https://io-link.com/fileadmin/user_upload/Downloads/Package_2025/IOL-Interface-Spec_10002_V1.1.5_Oct2025.zip)

```powershell
.\iolink-cli.exe read-process-readback device.toml 40 --snapshot process-input.json
.\iolink-cli.exe read-process-readback device.toml 41 --snapshot process-output.json
.\iolink-cli.exe review-process-readback process-input.json
.\iolink-cli.exe decode-process-readback 40 "03 E9" --profile device.toml
```

Input uses the saved profile’s effective mapping and output uses the selected IODD output layout. Review preserves the raw reply when a mapping is missing, a layout condition needs confirmation, or the width or encoding needs review. A mismatch or failed request returns exit 1 with available evidence. A valid raw reply without a usable mapping remains available. Use **Offline → Diagnostic snapshots → Process readback snapshots** to reopen an explicitly saved read with its embedded profile. Cyclic sampling and CSV capture continue independently.

#### Compare saved process readbacks

Open **Offline → Diagnostic snapshots → Process readback snapshots → Compare two process readback snapshots**. Enter the first snapshot above the compare section and the second path inside it. Expand each source review to see the original read and mapping.

```powershell
.\iolink-cli.exe compare-process-readbacks before.json after.json
```

The report lists changed bytes by zero-based wire offset, with an XOR mask for byte positions present in both replies. Typed changes require matching readback directions, recorded gateway and device context, and effective field mappings. A field change shows the original and revised raw bits and displayed values. The host observation times belong to the independent reads. Each source process sample time remains unknown. CLI exit 0 means the typed comparison ran, including when values differ. Exit 1 retains the available raw comparison and both source reviews when typed comparison needs review.

### Device revisions

Open **Diagnostics → Connection & identity → Device revisions (22:0 / 23:0)** to read the connected sensor’s hardware and firmware strings. The master firmware appears separately in the connection details. Choose **Read and save device revisions snapshot** to keep each raw reply or read error, its observation time, and its master, port, and device identity. Reopen the file under **Offline → Diagnostic snapshots → Device revision snapshots**. [IO-Link Interface and System V1.1.5, Table B.8 and B.2.14–15](https://io-link.com/fileadmin/user_upload/Downloads/Package_2025/IOL-Interface-Spec_10002_V1.1.5_Oct2025.zip)

```powershell
.\iolink-cli.exe read-device-revisions device.toml --snapshot revisions.json
.\iolink-cli.exe review-device-revisions revisions.json
.\iolink-cli.exe decode-device-revision 23 "46 57 2D 31 2E 32"
```

For raw replies use **Offline → Parameter bytes → Device revisions**, or pass index 22 for hardware and 23 for firmware to the CLI decoder. Each optional UTF-8 reply can contain up to 64 bytes. Condensed text and trailing NUL padding are accepted and original bytes are retained. The reads use independent acyclic clients. The CLI exits 1 for a failed, blank, or malformed reply or when the recorded contexts conflict. If a context field is missing, the report marks the pairing as unavailable and still shows each valid string.

#### Compare two saved device revision snapshots

Open **Offline → Diagnostic snapshots → Device revision snapshots → Compare two device revision snapshots**. Select the earlier snapshot in the first path and the later snapshot in the second path. Expand either source review to see the original bytes, observations, and errors.

```powershell
.\iolink-cli.exe compare-device-revisions before.json after.json
```

Workbench checks raw-byte equality for each available pair of replies. It compares the decoded revision strings only when the recorded endpoints and per-query master, port, and device identity match, and neither snapshot has an internal context conflict. A change in NUL padding can change the raw bytes while leaving the decoded string equal. Revision strings compare exactly, including spaces and letter case. Both source reviews remain available if either comparison needs review. CLI exit 0 means both fields were comparable and includes genuine revision changes. Exit 1 retains the available evidence when one field or context could not be compared.

### Device error count

In **Diagnostics → Device events → Device error count (32:0)**, choose **Read error count** or **Read and save error count snapshot**. Review a saved reply under **Offline → Diagnostic snapshots → Error count snapshots**. To decode recorded bytes directly, use **Offline → Event codes → Device error count (32:0)**. [IO-Link Interface and System V1.1.5, B.2.20](https://io-link.com/fileadmin/user_upload/Downloads/Package_2025/IOL-Interface-Spec_10002_V1.1.5_Oct2025.zip)

```powershell
.\iolink-cli.exe read-error-count device.toml --snapshot device-errors.json
.\iolink-cli.exe review-error-count device-errors.json
.\iolink-cli.exe decode-error-count "00 05"
```

The optional reply is a two-byte unsigned value. It describes device application errors since power-on or reset according to the vendor's rules. Gateway errors, pending events, and capture loss have their own evidence. The explicit read uses an independent acyclic client. A snapshot preserves the query, raw reply or error, and any observed master and device context. The CLI exits 1 for an unavailable or malformed reply and prints the available evidence. The checked AL1350 write planner rejects writes to index 32.

#### Compare saved error counts

Open **Offline → Diagnostic snapshots → Error count snapshots → Compare two error count snapshots**. Enter the first file above the comparison and the second file inside it. Expand both source reviews to inspect host read times, recorded identities, and the original replies or errors.

~~~powershell
.\iolink-cli.exe compare-error-counts before.json after.json
~~~

Wire equality is available when both replies exist. For two decodable replies with matching recorded master, port, and device context, the comparison reports each count and the signed arithmetic difference in file order. Counter history between the reads is unknown, including any resets or rollovers. The CLI returns 0 when the counts are comparable, including when they differ. It returns 1 with both reviews and any available wire comparison when the count comparison is unavailable.

### Direct Parameter Page 1

Open **Offline → Parameter bytes → Direct Parameter Page 1 (0:0)** and paste the 16 recorded wired IO-Link bytes in ascending address order, from 0x00 to 0x0F. The decoder retains the complete page and interprets wired protocol revision 1.1. It displays the recorded master cycle setting, the device's reported minimum cycle, ISDU and SIO capability, M-sequence codes, process-data lengths, and the 16-bit Vendor ID and 24-bit Device ID. [IO-Link Interface and System V1.1.5, B.1.1 and B.1.3–9](https://io-link.com/fileadmin/user_upload/Downloads/Package_2025/IOL-Interface-Spec_10002_V1.1.5_Oct2025.zip)

```powershell
.\iolink-cli.exe decode-direct-page "00 0A 04 1B 11 50 82 FF FF 00 00 01 00 00 00 00" --profile device.toml
```

The active GUI profile or optional CLI profile supplies expected Vendor and Device IDs. Zero device minimum means the page supplied no minimum. Compare the reported durations separately from measured receive spacing and gateway update rate. M-sequence codes are shown as recorded. Command slots stay in the raw hex because reads of those slots return unspecified values. The CLI returns 1 with retained evidence for malformed length, unsupported protocol revision, reserved encodings and bits in the decoded fields, a zero Device ID, or mismatched identity pins. This workflow only interprets supplied bytes.

Expand **IODD comparison** or read the CLI's `iodd_comparison` section to compare the reported device minimum cycle, SIO flag, M-sequence capability byte, and selected IODD payload widths. Both numeric profile ID pins must match the recorded page before the comparison can use the saved declarations. An unconfirmed layout condition withholds the width checks. The width comparison uses the imported payload bytes, including standalone integer padding, and compares an output width when the profile contains an IODD output layout. One to eight recorded bits occupy one payload byte. A zero minimum indicates an unavailable value. The CLI returns 1 for compared differences while retaining both values. An unavailable comparison by itself is advisory. [IO Device Description Specification V1.1.5, PhysicalLayer](https://io-link.com/fileadmin/user_upload/Downloads/Package_2025/IO-Device-Description_Specification_10.012_V1.1.5_Oct2025.zip)

### Data Storage metadata

Use **Offline → Parameter bytes → Data Storage metadata (3:2–5)** to select the field and paste a recorded reply. The IO-Link specification reserves this index for private exchanges between the master and device. Workbench decodes these bytes offline and rejects index 3 in AL1350 write plans. [IO-Link Interface and System V1.1.5, B.2.3](https://io-link.com/fileadmin/user_upload/Downloads/Package_2025/IOL-Interface-Spec_10002_V1.1.5_Oct2025.zip)

```powershell
.\iolink-cli.exe decode-data-storage 2 "82"
.\iolink-cli.exe decode-data-storage 3 "00 00 08 00"
.\iolink-cli.exe decode-data-storage 4 "FE DC BA 98"
.\iolink-cli.exe decode-data-storage 5 "00 40 01 00 00"
```

Subindex 2 shows the transfer state and pending upload flag. Subindex 3 gives the required storage size. Subindex 4 shows the parameter-set revision marker as supplied by the device. Subindex 5 lists the index and subindex pairs on one page. A zero marker ends the list. A nonzero marker gives the index of the next page, which you can decode separately if you have its bytes. The report retains raw hex. The CLI exits 1 for a malformed reply, set reserved state bits, or a storage size above 2048 bytes.

### Device profile snapshots

Open **Diagnostics → Device profile** to explicitly read **ProfileCharacteristic (13:0)**, **PDInputDescriptor (14:0)**, and **PDOutputDescriptor (15:0)** from the configured port. Choose **Read and save device profile snapshot** to retain all three query outcomes. Use **Offline → Diagnostic snapshots → Device profile snapshots** to reopen the JSON. [IO-Link Common Profile V1.2.1, Annex B](https://io-link.com/fileadmin/user_upload/Downloads/Package_2025/IO-Link_Common_Profile_Specification_10.072_V1.2.1_Oct2025.zip)

```powershell
.\iolink-cli.exe read-device-profile device.toml --snapshot device-profile.json
.\iolink-cli.exe review-device-profile device-profile.json
```

Each query captures its own connection observation, raw reply or read error, host UTC, and elapsed time. The three requests run sequentially through the independent acyclic path while cyclic sampling can continue. The report retains partial successes. Conflicting recorded master, port, or device context withholds comparison against the IODD while preserving each decoded reply. A successful zero-byte descriptor represents an empty layout. An unavailable service appears as a read error. CLI exit 1 signals an incomplete, conflicting, malformed, or differing result with the available evidence on stdout. Saved evidence uses a new path.

#### Compare two saved device profiles

Open **Offline → Diagnostic snapshots → Device profile snapshots → Compare two device profile snapshots** and select the first and second files. The report shows byte equality for each available reply and keeps both full reviews. When the replies decode and their saved endpoint, gateway, port, mode, master and device evidence match, it lists added or removed profile IDs and changes to descriptor datatypes and widths by starting bit offset. A zero-byte descriptor is an observed empty layout. If a reply is missing, malformed, or has conflicting or incomplete identity context, the affected comparison reports the reason alongside the available wire evidence.

```powershell
.\iolink-cli.exe compare-device-profiles earlier.json later.json
```

Exit 0 means all three comparisons were available, including cases with differences. Exit 1 means at least one comparison needs review. The command uses the saved files offline.

### Process data descriptors

In **Offline → Parameter bytes → Process data descriptors**, choose **Input (14:0)** or **Output (15:0)** and paste the complete recorded reply. Select **Empty reply** for a successful zero-byte reply. Each three-byte descriptor contains a datatype code, bit length and offset. A zero length byte represents 256 bits. The decoder retains the raw reply and flags incomplete, overlapping, out-of-order or out-of-range entries. [IO-Link Common Profile V1.2.1, Annex B.5](https://io-link.com/fileadmin/user_upload/Downloads/Package_2025/IO-Link_Common_Profile_Specification_10.072_V1.2.1_Oct2025.zip)

```powershell
.\iolink-cli.exe decode-process-descriptor input "01 01 00 03 0F 01" --profile device.toml
```

Use `output` for index 15 and `""` for an empty reply. Omit `--profile` to decode without an IODD comparison. The comparison uses the imported fields for the selected layout, including its declared condition when present. It checks mapped datatypes and bit ranges, allows grouped packed Booleans, and identifies entries that cover unmapped bits or extend past the payload. Reserved and profile-specific datatypes and standalone Boolean width cases can require manual review. The CLI returns 1 for differences, partial comparisons or malformed replies. Device identity, active layout condition, and engineering interpretation require their own evidence.

#### Suggest manual input mappings

Paste a complete recorded **Input (14:0)** descriptor, then expand **Suggest manual input fields**. Review the proposed types and ranges. **Add manual fields** applies them to the active host profile and opens Mappings. Save the profile when ready. In the CLI, preview the same candidates and optionally save them to a new profile.

```powershell
.\iolink-cli.exe plan-input-descriptor device.toml "01 02 00 03 0E 02"
.\iolink-cli.exe plan-input-descriptor device.toml "01 02 00 03 0E 02" --save-profile with-fields.toml
```

Use a profile with a known input width. Suggestions use big-endian bit offsets and the standard descriptor type. Boolean sets expand to individual one-bit fields. Scale defaults to 1, offset to 0, and units are blank. Overlapping entries, out-of-width entries, StringT without an encoding, and unsupported types remain listed for review. All-field CSV selection includes the added fields. Explicit CSV selection logs the field names listed in the profile. A saved output requires at least one addition and a new destination path.

### Cyclic display formats

IODD `ProcessDataInfo` and `ProcessDataRecordItemInfo` can render integers as `Bin` or `Hex` and Booleans as `Dec` (0/1). Negative integers use two’s complement. A 5-bit signed field containing -4 appears as `1111 1100b` in binary or `FCh` in hexadecimal. The GUI and CLI keep the raw bits and can show a state name alongside the formatted value. The CLI's decoded output JSON carries the optional `formatted_value` beside `raw` and `value`. The CSV column uses the underlying engineering or Boolean value. [IODD V1.1.5, sections 7.5.8.1 and 7.5.8.4](https://io-link.com/downloads)

An incompatible display format adds a note for mapping review and keeps the field wherever its remaining metadata is usable. Applying an override with incompatible scaling, unit, precision, or datatype returns that field to its ordinary display. Removing the override reveals the imported display again.

Reimport to save cyclic display preferences. Saved profiles and captures containing `display_format` require v0.110.0 or later.

### Cyclic TimeT and TimeSpanT

The importer maps `TimeT` and `TimeSpanT` as eight-byte big-endian fields, including byte-aligned record members and array elements. The live and offline decoders show UTC or a signed duration, with the wire hex and exact tick count. `TimeT` uses the IO-Link era rule at seconds word `0x9DFF4400`. The display truncates to milliseconds and a negative sub-millisecond duration keeps its negative sign. Device clock validity and further era rollovers require external context. [IO-Link Interface and System Specification, Annex F.2.8–F.2.9](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)

CSV stores exact decimal ticks at 4,294,967,296 ticks per second for both types. These are payload values. Host receive timestamps, gateway source metadata and timing reports follow their separate acquisition rules. For manual input use **TimeT** or **TimeSpanT**, or `set-field --kind time` or `--kind time-span` with 64 bits and a byte-aligned offset. Reviewed writes to temporal outputs use complete hex images. Reimport earlier IODDs to add these mappings. Temporal field profiles require v0.94.0 or later.

### Cyclic text

Fixed-length `StringT` input and output fields use the IODD encoding and byte-aligned layout. The supported encodings are UTF-8 and US-ASCII, including within records and arrays. Workbench strips trailing NUL padding from the typed text and retains all original bytes. A nonzero byte after the first NUL or a sequence outside the specified encoding produces an error for that field. [IO-Link Interface and System Specification, Annex F.2.6](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)

Typed views use JSON-quoted text. For example, spare bytes after a short `"Café"` do not enter the rendered text. An all-zero field displays `""` and control characters use escape sequences. A malformed field displays `Invalid StringT` with the reason. Neighboring values and gateway validity remain available. See [text CSV columns](CAPTURE_REVIEW.md#text-columns) for their recorded representation.

For manual input, choose **UTF-8 string** or **US-ASCII string**, or use `set-field --kind utf8-string` or `--kind ascii-string`. Offset and width are in bits and must use whole bytes within the 32-byte process image. Reviewed output edits use full expected and desired hex images. Reimport an IODD to add text mappings to a prior profile. Profiles with these fields require v0.93.0 or later.

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

Before physical acceptance, record master model and firmware, supported host interface and license or SDK requirements, endpoint or USB connection, configured port and mode, sensor model, IODD revision and variant, and desired signal and unit. The four ifm routes target documented IoT Core 3.1.x behavior. [AL1350 bench setup](../README.md#al1350-bench-setup) · [Read-only AL1351, AL1352 and AL1353 setup](../README.md#al1351-al1352-and-al1353-read-only-setup)

Use an existing configured IO-Link port. Host connection settings edit the local profile only. `set-connection` saves a new host profile; `set-identity` saves expected pins. Use each command's `--help` for explicit values and clears. `set-connection --expected-mode` saves the `connection.ifm_mode` expectation; it checks the observed mode and never sets it. Confirm device identity where exposed with `status PROFILE`; expected pins and selected IODD identity remain separate from observed matches. A Product ID difference requires review. Exact text pins distinguish unset, empty and spaces.

Use `status-ports PROFILE1 PROFILE2 ...` for concurrent reads from distinct ports. The simulator, AL1350 and AL1351 support up to four profiles per set. The AL1352 and AL1353 support up to eight. In **Logging → Multiple ports**, choose **Review profiles** and then **Read port status**. Review each result independently. Status works with any valid layout and needs no CSV destination. A mode or identity failure can coexist with successful results on the other ports. Use `status PROFILE --verify-product-id` for an explicit ISDU-based Product ID check.

If the selected IODD input layout has a paired output declaration, the applied profile retains it separately from input mappings. Use Device → Writes → Process output for a byte-for-byte preview with named expected and proposed values, declared width, and unmapped bit changes. **Read current value** can interpret the observed output before you use it as the expected image. **Edit one field** builds the proposed full image from the expected bytes and one unscaled value for a supported IODD field. A standalone Boolean uses `00` or `FF` for reviewed writes. Reads flag other bytes and retain their exact hex. Standalone padded integers write sign or zero padding for the applicable wire width and flag noncanonical observed padding. Re-import the relevant IODD to enable this coding. For saved bytes use **Offline → Process bytes → Output bytes** or `decode-output PROFILE HEX`. Choose an output-only layout by ID in Device → IODD import or with `import --layout ID`. It saves zero input bytes and retains the selected identity, output and parameter mappings. For a conditional layout, confirm the active condition in Mappings before live output interpretation or preview. Use CLI `status` or the explicit read controls for the configured port. Input monitoring, logging and live feeds require an input layout.

An explicit parameter or diagnostic read uses its own client, apart from the cyclic loop. Save named reads with `read-parameter --snapshot` and use `compare-parameters LEFT.json RIGHT.json` to compare exact bytes and both connection observations offline. The reads must share an ISDU address. Measure gateway-side contention on hardware. For authorized one-shot AL1350 mode, parameter, SystemCommand and process output writes, use [Device → Writes or the CLI preview](WRITES.md).

### Compare saved parameter catalogs

Open **Offline → Parameter bytes → Compare saved parameter catalogs** and select two saved TOML profiles. The report pairs declarations by ISDU index and record members by subindex. It shows added and removed entries, changed properties such as access and defaults, and exact definitions for the affected addresses. Array length and access changes appear at the parent index. Changes to the shared element definition appear once for the array.

~~~powershell
.\iolink-cli.exe compare-parameter-catalogs before.toml after.toml
~~~

The report lists unresolved standard-reference changes separately. A saved IODD with zero declarations is a valid empty catalog. If either profile lacks an IODD, the CLI returns 1 with available context. The comparison reports declared access and defaults. Use explicit reads to check current device values.

Expand **Decode a recorded ISDU reply** and choose **Hex bytes** to enter an index, subindex, and hexadecimal payload. Leave the payload blank in that mode for a catalog-only comparison.

~~~powershell
.\iolink-cli.exe compare-parameter-catalogs before.toml after.toml --index 66 --subindex 1 --payload "FF EC"
~~~

The preview uses the same address and bytes on both profiles. It compares the unscaled value, named value, assessment, and display values. A whole record or array compares usable members by subindex within the same datatype. Each side retains its declared access, decoded result, and any errors. If a declaration is absent or a reply needs decoding review, the CLI prints the available results and returns 1. Omit subindex for a whole reply at subindex 0.

Choose **Saved snapshot** to use the address and bytes from a previously saved parameter read. The report includes the snapshot's host read time, recorded master and port, and observed device identity. It checks each profile's configured identity against that observation. A mismatch remains visible alongside the offline decode.

~~~powershell
.\iolink-cli.exe compare-parameter-catalogs before.toml after.toml --reply-snapshot .\.local\recorded-parameter.json
~~~

#### Use the snapshot's embedded catalog as the baseline

Open **Offline → Parameter snapshot**, enter the snapshot path, and expand **Compare with revised profile**. Select the revised TOML profile. The comparison uses the snapshot's embedded catalog as the baseline and decodes the same recorded reply with both catalogs. The result includes the full catalog differences and the source snapshot context.

~~~powershell
.\iolink-cli.exe compare-parameter-snapshot .\.local\recorded-parameter.json revised.toml
~~~

### Save a diagnostic read

```powershell
.\iolink-cli.exe read-diagnostics PROFILE.toml --snapshot .\.local\diagnostics.json
.\iolink-cli.exe review-diagnostics .\.local\diagnostics.json
```

Use a new file in an existing parent folder. The two independent 36:0 and 37:0 queries retain their own raw reply or error, host UTC and monotonic duration, and any accepted connection observation. The saved profile supplies event definitions offline. A partially successful read or a change in master or device identity remains reviewable. Exit 1 indicates a read, decode, context or save failure. Continue to treat source event history and loss as unknown.

### Inspect a configured master

Apply a supported ifm profile with the master endpoint. Open **Diagnostics → Connection & identity → Master and ports → Inspect master**, or run:

```powershell
.\iolink-cli.exe inspect-master .\.local\bench.toml
```

The first request reads master metadata and checks the model and saved firmware pin. The second request reads the master diagnostics and every port, then Workbench checks the returned master identifiers again. The `master_diagnostics` table reports temperature in °C, supply voltage in mV, and sensor-supply current in mA. The supply status is shown as returned by IoT Core without interpreting its code. Each port shows its raw mode, COM speed and cycle, with cycle conversion when the profile specifies the unit. Available device IDs, name, serial and diagnostic points appear alongside the port. Missing values and IoT Core response codes remain visible. An unavailable device identity can have several causes, including a disabled port. The requests use the documented read-only `getdatamulti` service with sequential data-point reads. [AL1350 operating instructions](https://www.quicktimeonline.com/assets/images/pdf/IFM%20Electronic/operating-instruction-AL1350.pdf) · [AL1351 operating instructions](https://www.quicktimeonline.com/assets/images/pdf/IFM%20Electronic/operating-instruction-AL1351.pdf) · [AL1352 operating instructions](https://www.quicktimeonline.com/assets/images/pdf/IFM%20Electronic/operating-instruction-AL1352.pdf) · [AL1353 operating instructions](https://www.quicktimeonline.com/assets/images/pdf/IFM%20Electronic/operating-instruction-AL1353.pdf)

Expand **Port configuration** under the result to review the master settings for each port. The CLI includes them under `ports.configuration`. A zero cycle preset means automatic selection. A positive preset is converted to microseconds only when the profile specifies a confirmed cycle unit. The configured Vendor ID and Device ID are the validation settings. The connected identity appears in the main port observation. The report preserves the raw validation/Data Storage mode, plus unavailable fields and service codes. [ifm AL1352 operating instructions, sections 9.1.6, 9.2.8 and 9.2.9](https://www.quicktimeonline.com/assets/images/pdf/IFM%20Electronic/operating-instruction-AL1352.pdf)

Keep this inventory separate from a sampling session or a checked read of a selected device. Use **status** or **Read port status** to check the sensor and mode pins for acquisition. A model or firmware mismatch or a failed port batch returns exit 1 and retains any master metadata already collected. Individual unavailable data points remain in a successful report, including measurements with a successful service code and an unusable value. Master diagnostics apply to the inspected endpoint as a whole.

To keep a bench baseline, expand **Read and save master snapshot** and choose a new JSON path, or use the CLI:

```powershell
.\iolink-cli.exe inspect-master .\.local\bench.toml --snapshot .\.local\master.json
.\iolink-cli.exe review-master .\.local\master.json
```

**Offline → Diagnostic snapshots → Master inspection snapshots** opens the same file without a profile or gateway connection. The file stores the original report, configured endpoint, firmware pin, cycle unit, profile name and producing Workbench version. Keep it under ignored `.local/` in a source checkout. A failure after the initial master reply can be saved with its error and initial observations. The CLI prints the available report and reports exit 1 for that outcome when inspecting or reopening it. If the initial request fails, Workbench has no inspection report to save. The save checks for an existing destination before the read and publishes a new file atomically. Offline review checks the saved format and consistency of the reported context, port list and Device ID widths. It reflects the recorded inspection.

Expand **Compare two master snapshots** in **Master inspection snapshots** to select a second file. The CLI uses `compare-master LEFT.json RIGHT.json`. The report lists recorded context changes and groups observed differences by master supply, port link and diagnostics, connected identity, and configured settings. Expand either original snapshot in the GUI to review its evidence.

Per-port and supply differences require complete inspections, matching configured routes, matching recorded master serial and firmware, and no conflicting observed hardware revision. Missing readings and changes in returned service codes remain visible. Converted cycle values are compared only with matching saved cycle units. If the master context prevents comparison, the CLI still prints both snapshots and returns exit 1. Review and comparison use only the supplied files.

### Compare diagnostic snapshots

```powershell
.\iolink-cli.exe compare-diagnostics .\.local\before-diagnostics.json .\.local\after-diagnostics.json
```

Use **Offline → Diagnostic snapshots → Compare two snapshots** in the GUI. The comparison reopens both files with their embedded event definitions and shows exact byte equality separately for **36:0** and **37:0** when both replies exist. Each snapshot's full review remains available. Files and endpoints are left as recorded.

Device Status, pending-event counts and changed slot positions require two valid-length replies for the applicable query and matching recorded gateway route, master model and firmware, port and mode, and observed Vendor ID, Device ID and device serial. Comparisons for either ifm model also require matching recorded master serials. Other observed identity fields must agree where both files supply them. A discrepancy between the two reads within either file suppresses interpreted differences across the comparison. A failed query can still leave the other query eligible.

The **pending_events** result compares occurrences of each exact qualifier/code pair among pending device-application warnings and errors in the two **37:0** replies. An empty **changed_counts** list means the comparable counts match, even if the entries moved. Each change reports the observed severity, left and right counts, and available names from each file's embedded catalog. Other nonempty entries are counted separately and remain in the full reviews and slot comparison. These are counts in two saved replies. Event transitions, order and loss remain unknown. [IO-Link Interface and System Specification, Annex B.2.22](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)

Expand **Wire slot positions** in the GUI or inspect **changed_slots** in CLI output to compare positions in the two payloads. An omitted slot value denotes an absent position and **000000** denotes a present, empty slot. An empty list denotes matching slots. If **pending_events** and **changed_slots** are absent, **detailed_status_reply** gives the reason. Exit 1 flags an incomplete, malformed or internally inconsistent snapshot. Two valid files with differing recorded targets can still report raw equality and exit 0.

### Decode an ISDU ErrorType

Use the two ErrorType bytes from a recorded negative ISDU reply. Paste them into **Offline → Event codes → ISDU ErrorType** or run the CLI lookup. The bytes are the ErrorCode followed by the AdditionalCode. The built-in table covers standard device application codes, and unsupported values retain the exact raw bytes. Gateway HTTP and IoT Core status codes have their own formats. [IO-Link Interface and System Specification, Annex C.2](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)

```powershell
.\iolink-cli.exe decode-error-type "80 11"
.\iolink-cli.exe decode-error-type "81 11" --profile .\sensor.toml
```

Import the IODD to save its vendor ErrorType names and descriptions in the selected language. Supply `--standard-definitions` and the optional `--standard-language-file` at import to resolve and localize standard references. The lookup uses the applied GUI profile or the profile supplied on the CLI. Unresolved standard references remain visible alongside any built-in meaning. This is an offline lookup. Profile files containing the imported ErrorType catalog or its references require v0.100.0 or later.

## Choose logging and inspect its plan

```powershell
.\iolink-cli.exe set-logging mapped.toml logging.toml --mode every-update --queue-capacity 8192 --rotate-rows 100000 --rotate-seconds 0 --field signal
.\iolink-cli.exe logging logging.toml
```

`set-logging INPUT OUTPUT` preserves unspecified settings and saves a new file. Choose one selection option: repeated `--field NAME` in CSV order, `--all-fields`, or `--raw-only`. Modes are `every-update` and `on-change`; zero disables each rotation limit. Queue capacity is 1–1,000,000 rows. Edited rotation limits must fit TOML's signed 64-bit integer range. GUI controls share candidate validation; apply/discard pending profile drafts before acquiring or saving.

`logging PROFILE` is an offline plan with configured row semantics, selected field mappings/origins and exact CSV headers. It contacts no gateway and verifies no rate/freshness. In Logging, expand Configured capture plan while idle for the summary. Every-update retains equal consecutive payloads when a new source update is verifiable. Snapshot adapters instead record host polls. On-change selects the first eligible row per connection generation and changes in the complete raw payload, validity, stale flag or diagnostic code; field selection/scaling does not change that decision.

Rotation is checked before the next row. Time rotation uses receive elapsed timestamps from the first row in each part; either enabled threshold rotates. Silence creates no empty timed files. Stop and flush the capture before treating its report as finalized.

## Capture several configured ports

Use one saved profile per distinct port on the same ifm model and origin or simulator route. Select the active input layout and confirm any required IODD condition on each physical profile. Give every port its own mapping and logging settings. For a simulator example, select the two bundled profiles in **Logging → Multiple ports** or pass them to the CLI:

```powershell
New-Item -ItemType Directory -Force .\.local | Out-Null
.\iolink-cli.exe capture-ports .\.local\sim-port-set .\samples\simulator.toml .\samples\simulator-port2.toml --seconds 2
.\iolink-cli.exe verify-capture .\.local\sim-port-set\port-2
```

The parent of OUTPUT must exist and OUTPUT must be new. The command validates the entire selection before opening a gateway. Up to four AL1350, AL1351 or simulator ports, or eight AL1352 or AL1353 ports, run independently. All ifm profiles in a set use the same model, configured origin and compatible firmware pins. The sessions read the existing port configuration and input data. They have separate receive clocks and acquisition windows, so use each port report for timing, source gaps and overruns. All ifm routes record host polls with unknown source-update loss.

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

Keep the requested host interval, declared sensor minimum, reported port cycle, gateway update rate, source cadence, host poll/receive spacing and queue/writer timings separate. Treat 1 ms as a target to evaluate. The supported ifm snapshots have no verifiable source update counter/timestamp in the supported path: source loss, source age and gateway update cadence are unknown. Host receive age is not sensor age. Simulator counters/timestamps are synthetic. Within-generation counter gaps cannot measure loss during disconnects.

Captures retain UTC start, monotonic host elapsed times, sample indexes, reconnect generations, raw input, validity, staleness, diagnostics, selected typed cells and finalized timing/accounting reports. Inspect p50/p95/p99/longest gaps, invalid/stale/unknown input and logger overruns. Review queue high water, queue wait, service/flush/sync and drain separately from source loss. On-change filtering is intentional selection, not a source gap. Invalid/stale values stay blank in typed cells.

`verify-capture DIRECTORY` verifies all parts. `review-capture DIRECTORY --field NAME` produces bounded offline signal summaries. `redecode-capture DIRECTORY MAPPING OUTPUT` exports a new verified bundle without replacing the original; `verify-export OUTPUT --source DIRECTORY` optionally verifies the explicitly supplied source. Parameter workflows use `parameters`, `defaults`, `decode-parameter`, `read-parameter --snapshot`, and `review-parameter`; a declared default is metadata, not an observed value or write. Saved parameter review uses embedded context without following stored endpoints or file paths. See the [README](../README.md) for their full examples.

## Integrations and support evidence

Integration includes CLI, saved TOML/JSON/CSV and an opt-in [read-only local HTTP API](LOCAL_API.md). Enable it per acquisition in Integration or use capture `--api-listen 127.0.0.1:8765`; it stops with acquisition. Optional [MQTT publishing](MQTT.md) runs on its own bounded handoff with intentional latest-state coalescing and separate loss accounting. Optional [Windows shared-memory snapshots](SHARED_MEMORY.md) run independently with bounded queues and a named mutex; CLI `read-shared-memory` reads without a gateway. C FFI remains [future work](INTERFACES.md). Keep simulation/loopback reports apart from physical results. For a bench issue, retain firmware, identity, profile, capture report and raw bytes in private storage; publish only sanitized reproductions.
