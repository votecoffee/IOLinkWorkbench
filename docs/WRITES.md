# Reviewed AL1350 writes

The GUI and CLI support four one-shot operations for the AL1350 IoT Core 3.1.x interface. The [ifm operating instructions](https://www.quicktimeonline.com/assets/images/pdf/IFM%20Electronic/operating-instruction-AL1350.pdf) document port `mode` in section 9.2.8, `pdout/setdata` in section 9.2.11, and `iolwriteacyclic` in section 9.2.13. The [IO-Link system specification](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf) defines the SystemCommand address and command classes in Annex B.2.2.

| CLI operation | Review inputs | Live check before dispatch |
|---|---|---|
| `port-mode` | `--expected` and `--desired` numeric codes confirmed for the exact firmware | Master identity and current numeric mode. A port can be configured without a connected sensor. |
| `isdu` | `--index` or `--parameter-id`, optional `--subindex`, `--expected` hex, and either `--desired` hex or `--desired-value` for a supported IODD datatype | Master and device identity, IO-Link mode, and the exact current parameter bytes. When the profile has the address in its IODD, access must be declared read/write and the selected subindex must be enabled. |
| `device-command` | Decimal `--code` and `--meaning` from the device's documentation | Master and device identity and IO-Link mode. Sends a single SystemCommand byte to index 2, subindex 0. The supported code classes are profile-defined, the four standard reset commands, and vendor-defined. |
| `process-output` | `--expected` and `--desired` complete hex images of equal width, up to 32 bytes. For an IODD field edit, use the complete expected image, exact field name, and unscaled value. | Master and device identity, IO-Link mode, and the entire gateway `pdout` value. |

## GUI

1. Apply an AL1350 profile for the approved bench endpoint and port. Under **Device → Writes**, enter the expected master serial and choose **Port mode**, **ISDU parameter**, **Device command**, or **Process output**.
2. For ISDU, expand **Choose from IODD** to search by name, ID, or index. Select an eligible parameter or subindex, then choose **Use selected address** to fill the numeric fields. The Index and Subindex fields also accept manual entry. Changing the address clears the old expected and desired values. Browsing and applying an address take place offline.
3. For mode, ISDU or process output, choose **Read current value** to check the target and see its reported value. For a matching ISDU definition, **Observed IODD details** shows the decoded value, available displays and any interpretation errors. **Use as expected value** copies the exact observed value into the draft for that master and point. The independent read can run during monitoring or capture. Enter the desired value, then choose **Preview write offline**. Review the endpoint, firmware, port, identity pins, expected value, service address and payload. An ISDU preview also shows the exact index and subindex. **Edit write** returns to the draft and clears approval. Applying another profile or editing the applied connection or identity also clears the draft and approval.
4. Select **I approve this exact write** and choose **Apply reviewed write**. The write worker blocks new acquisition and acyclic reads until it finishes. **Last write result** shows the dispatched target, outcome and exact JSON receipt when available. If normal window close is pending when the outcome becomes unconfirmed, the app returns to Writes so the result can be reviewed.

## CLI

Use a profile for the approved bench endpoint and port with `ifm_firmware` pinned to its exact 3.1.x value. `--master-serial` pins the master. Device operations require nonzero Vendor ID and Device ID pins from the profile. Every optional configured identity pin must match. A Product ID pin is checked by ISDU and a missing gateway serial can be checked through the device Serial Number ISDU. If the gateway reports a numeric IO-Link mode, use the existing `ifm_mode` profile setting for its confirmed code. Authenticated requests use the existing HTTPS and `IOLINK_IFM_PASSWORD` setting.

Preview runs offline and prints JSON with the destination, wire address, payload, expected value and confirmation. For example, the following previews an illustrative one-byte parameter change using your bench profile and master serial.

```powershell
.\iolink-cli.exe write --help
.\iolink-cli.exe write $profile --master-serial $serial isdu --index 65 --expected 01 --desired 02
```

Both `write … isdu` and `read-current … isdu` accept `--parameter-id` in place of `--index`. Use the exact case-sensitive ID in the profile's imported IODD. `parameters PROFILE` lists those IDs. Use `--subindex` to select a record or array member when the IODD enables it. The commands resolve the ID locally and apply the same eligibility checks as a numeric address. These examples use an ID from the synthetic sample IODD.

```powershell
.\iolink-cli.exe read-current $profile --master-serial $serial isdu --parameter-id V_SIM_Threshold
.\iolink-cli.exe write $profile --master-serial $serial isdu --parameter-id V_SIM_Threshold --expected 0001 --desired-value -2
```

An ID and its numeric index produce identical previews and confirmation codes for the same request. If the ID resolves to a different address after a profile edit, preview and approve the new address before applying.

For a supported read/write parameter or enabled subindex, choose **IODD value** in the ISDU form or use `--desired-value` in the CLI. **Use as expected value** still copies the exact current hex.

- For Boolean parameters, enter `true` or `false`. Typed ISDU entry encodes these as `FF` and `00`. Generate a fresh preview for a typed `true` operation reviewed before v0.73.0.
- For signed and unsigned integers of 2 through 64 bits, enter an unscaled decimal number. Workbench checks the declared numeric bounds and uses the smallest applicable 1-, 2-, 4- or 8-byte container. It sign-extends signed values and zero-pads unsigned values. [IO-Link interface specification, Annex F.2.2–F.2.4](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)
- For `Float32T`, enter an unscaled decimal number or exponent form, such as `-2.5` or `2.5e-1`. Explicit `INF` and `-INF` are supported, subject to the same IODD checks. Use `--desired-value=-INF` for the negative literal in the CLI. The preview shows the resulting 32-bit encoding.
- For `TimeT`, enter `yyyy-mm-ddThh:mm:ss[.fff]Z`, such as `1984-01-01T00:00:00.001Z`. Typed entry uses UTC and the default era window from `1984-01-01T00:00:00Z` inclusive to `2120-02-07T06:28:16Z` exclusive.
- For `TimeSpanT`, enter signed seconds as `[+-]PTseconds[.fff]S`, such as `PT2.75S` or `-PT0.001S`. In the CLI, pass a negative duration as `--desired-value=-PT0.001S`.
- For `StringT`, enter the exact text, including any leading or trailing spaces. The IODD must specify UTF-8 or US-ASCII and a fixed byte length. Workbench zero-pads to that length. To clear a text value, select **Write empty text** in the GUI or pass `--desired-value=` in the CLI.
- For `OctetStringT`, enter comma-separated `0xNN` entries, such as `0x00,0x80,0xFF,0x2C`. Supply exactly the number of octets declared in the IODD.

Typed timestamps and durations accept up to three fractional digits. A fractional component that falls between IO-Link ticks rounds away from zero by less than one `2^-32`-second tick, so the decoded millisecond display preserves the entered value. The preview shows the exact eight bytes, ticks, and decoded value. Readback still requires an exact byte match for these datatypes. [IO-Link interface specification, Annex F.2.8–F.2.9](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)

If index 64 is a signed 16-bit parameter and its current bytes are `0001`, entering `-2` produces `FFFE` in the preview.

```powershell
.\iolink-cli.exe write $profile --master-serial $serial isdu --index 64 --expected 0001 --desired-value -2
```

The typed and raw forms produce the same confirmation when the reviewed bytes and target match. Use raw hex or `--desired` for other widths, unsupported datatypes, and whole compound images.

Integer decoding also accepts the existing compact representation used in saved values and packed members. A longer wire container has to contain valid sign or zero padding. Preflight and readback compare exact bytes for numeric values.

For a single `StringT` value with a supported IODD definition, the preview includes a `text_readback` policy with the declared encoding and byte length. This covers an addressed scalar or one enabled record or array member. The live preflight still requires the exact expected bytes. After the gateway acknowledges the write, Workbench accepts different trailing zero padding if both byte strings decode to identical text under the approved policy. The confirmation includes the policy. Generate a new preview for text operations reviewed before v0.71.0. [IO-Link interface specification, Annex F.2.6](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)

The ISDU preview includes `isdu_value_review`. In the GUI, the parameter name, datatype, and available expected and prospective values appear beneath the wire request. Expand **IODD value details** for both interpretations, named choices, range assessment and any engineering displays. For supported scalar, text, octet and time datatypes, preview rejects proposed bytes that the declared datatype cannot decode. For supported numeric choices and ranges, it also rejects proposed values outside well-formed definitions and values that cannot be compared with them. Expected bytes are interpreted for review so an out-of-definition current value can still be corrected. Dispatch compares the exact expected bytes with a fresh read.

For a whole record or array at subindex 0, supply complete expected and proposed images of the declared width. **IODD value details** shows decoded members and their bit offsets. Workbench checks writable members with the same rules as scalar writes and requires read-only members and bits outside writable fields to match the expected image. Read-only subindexes are listed in the review. A whole-record write is refused if it contains a write-only member or a member with unknown width. Get the expected image with **Read current value** or `read-current` and keep the protected bytes from that image. The live preflight and readback still compare the complete byte images. The compound layouts follow the [IO-Link interface specification, Annex F.3](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf).

To build the proposed image for one edit, choose **Edit one member** at subindex 0 in **Device → Writes**, or pass `--member-subindex` and `--desired-value` in the CLI. Supply the complete expected image. Workbench encodes the selected writable member into that image and preserves every other bit. This also works when the IODD disables direct subindex access. The same whole-image checks apply to the finished draft, including validation of other writable members. The preview and approval cover one complete write at subindex 0.

```powershell
.\iolink-cli.exe write $profile --master-serial $serial isdu --parameter-id V_SIM_Record --expected FFEC010A41000000 --member-subindex 2 --desired-value false
```

**Changed IODD members** lists the subindexes, bit offsets, and extracted before-and-after hex for members that change. The CLI includes the expected interpretation in `isdu_comparison.expected` and compound changes in `isdu_comparison.changed_members`. These fields also appear in the receipt's preview. The comparison uses supplied expected bytes and the proposed payload. The existing confirmation continues to bind both exact byte images.

Unsupported datatypes and ambiguous choices or ranges call for manual review. A missing IODD parameter is identified as `no_iodd_parameter`, with raw-byte review available. Selected record and array members use their own declarations when the IODD allows subindex access. The ISDU confirmation code includes this review.

`read-current` uses the same target checks as write preflight. It returns JSON with the master, port, point, UTC observation time and `value`. The value is a number for a port mode and uppercase hex for ISDU or process output. A matching ISDU definition adds `iodd_value` with the IODD parameter ID, name, datatype and interpretation of the same returned bytes. An out-of-range value or a decoding error appears in the interpretation alongside the original hex. Use `value` as `--expected` when preparing the separate write. Execution checks those bytes again.

```powershell
.\iolink-cli.exe read-current $profile --master-serial $serial port-mode
.\iolink-cli.exe read-current $profile --master-serial $serial isdu --index 65
.\iolink-cli.exe read-current $profile --master-serial $serial process-output
```

Review the exact operation against the device and machine state. To execute an approved operation, repeat the command with `--confirm CODE` immediately before the operation name, using the `confirmation` from its preview. The code binds the reviewed destination, operation, identity pins, expected value and payload. The preflight reads and the write are separate requests. Coordinate ownership of the target port and device during the write. The gateway can enforce its own write restrictions, including the documented PLC-running restriction for mode and process outputs.

The CLI dispatches at most one write for each invocation. It makes a fresh readback for mode, readable ISDU parameters, and process outputs. The JSON receipt reports one of these outcomes.

- `readback_matched` means the gateway reported the exact requested value after acknowledging the write. Process output readback describes the gateway image. Physical effect and persistence require separate bench evidence.
- `text_readback_matched` means one IODD `StringT` value decoded to the requested text with different zero padding. The receipt retains both the requested and observed hex. Changed characters, whitespace, malformed data, and responses beyond the declared width leave the result unconfirmed. Whole records and arrays and raw addresses without an IODD retain exact readback comparison.
- `gateway_acknowledged` records the SystemCommand acknowledgement. Verify the command's effect according to the device documentation.
- `unconfirmed` includes a gateway error, lost or malformed reply, or failed or different readback. The CLI returns exit 1 after printing the receipt. Inspect the current state before deciding on any further action.

Monitoring, capture and optional data feeds use read-only paths. The reviewed write workflow covers mode changes, individual readable ISDU parameters, one-shot documented SystemCommands, and a single full output image. Cyclic output ownership, write-only vendor parameters, and Data Storage workflows remain follow-on work.

## Review process outputs

Use **Device → Writes → Process output** or the CLI to review two complete gateway images. Read the current image with **Read current value** or `read-current … process-output`, then use those exact bytes as the expected image. Expand **IODD output fields** to inspect the observed image before preparing a change. CLI output reads retain exact `value` hex and add `iodd_output` when the profile has a paired output layout. An unexpected width or an unconfirmed conditional layout shows the raw image and the reason decoding was skipped.

Workbench saves an output declaration from a paired IODD `ProcessData` entry or a selected output-only layout. In the preview, expand **IODD output fields** for names, offsets, widths, signedness, byte order, raw values and scaled displays. CLI preview and receipt JSON include `target_and_request.process_output_review`. Changes to bits without a typed mapping are listed for manual review. Both images must match the declared byte width. Bit-mapped layouts preserve padding outside the declared bit length.

To change one field, select **Edit one field** and choose its IODD name. Keep the complete current image as **Expected output hex**. Enter an unscaled decimal integer, a finite Float32 value, or a Boolean such as `true` or `false`. For a packed record or array, Workbench replaces the selected bits and preserves the remaining bits. Field editing also supports full-width standalone numbers and padded standalone integers. Review the resulting full request. A standalone Boolean occupies one complete wire byte. Both typed entry and reviewed raw writes use `00` for false and `FF` for true. [IO-Link Interface and System Specification, Annex F](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)

Checked reads and offline decoding preserve any noncanonical standalone Boolean byte from `01` through `FE`, show it as true and flag it for review. The exact byte remains the preflight target. Readback still requires an exact match, including when the observed byte also represents true. Re-import an older IODD to enable standalone Boolean coding. Profiles saved with this coding require v0.84.0 or later.

For a standalone integer, use the declared signed or unsigned width when entering an unscaled value. Workbench uses the applicable 1-, 2-, 4- or 8-byte wire container, extends signed values with the sign bit and extends unsigned values with zeros. Both typed and raw proposed images must use canonical padding. Reads flag noncanonical padding and show the low declared bits with the exact observed hex. Preflight and readback check the complete image. Re-import the IODD to add this mapping to an older profile. Profiles with this new mapping require v0.85.0 or later. [IO-Link Interface and System Specification, Annex F.2.3–F.2.4](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf)

The CLI uses `--field` and `--desired-value` for one field edit. Use the exact mapped name from the profile or the output review, including any displayed subindex suffix. Check the device documentation for operating limits and interlocks before approving the image.

```powershell
.\iolink-cli.exe write $profile --master-serial $serial process-output --expected A7F1 --field "level [1]" --desired-value 2
```

For a conditional layout, confirm the active condition against the device and record confirmation in Mappings or set `confirm_iodd_condition = true` in the saved profile. Preview checks the recorded confirmation. The live check compares the exact gateway image and verifies the existing identity and mode pins.

The confirmation binds the full expected and desired images and the available typed review. Profiles without output metadata still use raw review. Earlier Workbench releases can read unchanged input-only profiles. Profiles with `iodd.output` metadata require v0.79.0 or later. Select an output-only layout in the IODD import dialog or pass its ID to `import --layout`. Output-only profiles require v0.82.0 or later. Acyclic-only profiles support reviewed ISDU operations and require v0.83.0 or later. Re-import the IODD to enable field editing for packed outputs in a profile created by v0.79.0. Newly imported packed output profiles require v0.80.0 or later. [IO Device Description Specification and Schemas V1.1.5, released package](https://io-link.com/downloads)

```powershell
.\iolink-cli.exe write $profile --master-serial $serial process-output --expected 0102 --desired 0304
```

## Inspect saved output bytes

Use **Offline → Process bytes → Output bytes** and paste the full image, or run the offline decoder with a profile containing the matching IODD layout. The output entry is separate from pasted inputs and the copied Inputs observation. The CLI prints `raw_hex` and any `iodd_output` interpretation. A width mismatch remains visible alongside the raw bytes. For a conditional IODD, offline interpretation uses the selected layout and reports its condition.

```powershell
.\iolink-cli.exe decode-output $profile A7F1
```

## Verification

`python tools/verify_writes.py PATH_TO_CLI` uses an isolated loopback peer. It checks all three current-value reads, ISDU addresses by index and IODD ID, interpretation with raw-byte retention, all four write paths, offline typed and raw ISDU previews including temporal values, whole-image member edits, paired output import and review, checked and offline output decoding, output-only and acyclic-only profiles, acyclic snapshots and diagnostics, cyclic operation guards, typed field edits with exact preservation, standalone Boolean and padded integer encoding and exact readback, output width and padding checks, identity and value mismatches, exact and padding-aware readback, and a lost write reply with one dispatch. `cargo test --locked --bin iolink-workbench desktop::writes::tests` exercises the filtered IODD picker, address changes, offline GUI drafts, an interpreted current read alongside a held acquisition owner, approval guards, and close behavior with a held loopback write reply. Real firmware, machine conditions and actuator effects await physical qualification.
