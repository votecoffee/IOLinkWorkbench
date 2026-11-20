# Reviewed AL1350 writes

The GUI and CLI support four one-shot operations for the AL1350 IoT Core 3.1.x interface. The [ifm operating instructions](https://www.quicktimeonline.com/assets/images/pdf/IFM%20Electronic/operating-instruction-AL1350.pdf) document port `mode` in section 9.2.8, `pdout/setdata` in section 9.2.11, and `iolwriteacyclic` in section 9.2.13. The [IO-Link system specification](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf) defines the SystemCommand address and command classes in Annex B.2.2.

| CLI operation | Review inputs | Live check before dispatch |
|---|---|---|
| `port-mode` | `--expected` and `--desired` numeric codes confirmed for the exact firmware | Master identity and current numeric mode. A port can be configured without a connected sensor. |
| `isdu` | `--index`, optional `--subindex`, and `--expected` and `--desired` raw hex values | Master and device identity, IO-Link mode, and the exact current parameter bytes. When the profile has the address in its IODD, access must be declared read/write and the selected subindex must be enabled. |
| `device-command` | Decimal `--code` and `--meaning` from the device's documentation | Master and device identity and IO-Link mode. Sends a single SystemCommand byte to index 2, subindex 0. The supported code classes are profile-defined, the four standard reset commands, and vendor-defined. |
| `process-output` | `--expected` and `--desired` complete hex images of equal width, up to 32 bytes | Master and device identity, IO-Link mode, and the entire gateway `pdout` value. |

## GUI

1. Apply an AL1350 profile for the approved bench endpoint and port. Under **Device → Writes**, enter the expected master serial and choose **Port mode**, **ISDU parameter**, **Device command**, or **Process output**.
2. For mode, ISDU or process output, choose **Read current value** to check the target and see its reported value. **Use as expected value** copies it into the draft for that master and point. This uses an independent read and can run during monitoring or capture. Enter the desired value, then choose **Preview write offline**. Review the endpoint, firmware, port, identity pins, expected value, service address and payload. **Edit write** returns to the draft and clears approval. Applying another profile or editing the applied connection or identity also clears the draft and approval.
3. Select **I approve this exact write** and choose **Apply reviewed write**. The write worker blocks new acquisition and acyclic reads until it finishes. **Last write result** shows the dispatched target, outcome and exact JSON receipt when available. If normal window close is pending when the outcome becomes unconfirmed, the app returns to Writes so the result can be reviewed.

## CLI

Use a profile for the approved bench endpoint and port with `ifm_firmware` pinned to its exact 3.1.x value. `--master-serial` pins the master. Device operations require nonzero Vendor ID and Device ID pins from the profile. Every optional configured identity pin must match. A Product ID pin is checked by ISDU and a missing gateway serial can be checked through the device Serial Number ISDU. If the gateway reports a numeric IO-Link mode, use the existing `ifm_mode` profile setting for its confirmed code. Authenticated requests use the existing HTTPS and `IOLINK_IFM_PASSWORD` setting.

Preview runs offline and prints JSON with the destination, wire address, payload, expected value and confirmation. For example, the following previews an illustrative one-byte parameter change using your bench profile and master serial.

```powershell
.\iolink-cli.exe write --help
.\iolink-cli.exe write $profile --master-serial $serial isdu --index 65 --expected 01 --desired 02
```

`read-current` uses the same target checks as write preflight. It returns JSON with the master, port, point, UTC observation time and `value`. The value is a number for a port mode and uppercase hex for ISDU or process output. Use the observed value as `--expected` when preparing the separate write. Execution checks the value again.

```powershell
.\iolink-cli.exe read-current $profile --master-serial $serial port-mode
.\iolink-cli.exe read-current $profile --master-serial $serial isdu --index 65
.\iolink-cli.exe read-current $profile --master-serial $serial process-output
```

Review the exact operation against the device and machine state. To execute an approved operation, repeat the command with `--confirm CODE` immediately before the operation name, using the `confirmation` from its preview. The code binds the reviewed destination, operation, identity pins, expected value and payload. The preflight reads and the write are separate requests. Coordinate ownership of the target port and device during the write. The gateway can enforce its own write restrictions, including the documented PLC-running restriction for mode and process outputs.

The CLI dispatches at most one write for each invocation. It makes a fresh readback for mode, readable ISDU parameters, and process outputs. The JSON receipt reports one of these outcomes.

- `readback_matched` means the gateway reported the requested value after acknowledging the write. Process output readback describes the gateway image. Physical effect and persistence require separate bench evidence.
- `gateway_acknowledged` records the SystemCommand acknowledgement. Verify the command's effect according to the device documentation.
- `unconfirmed` includes a gateway error, lost or malformed reply, or failed or different readback. The CLI returns exit 1 after printing the receipt. Inspect the current state before deciding on any further action.

Monitoring, capture and optional data feeds use read-only paths. The reviewed write workflow covers mode changes, individual readable ISDU parameters, one-shot documented SystemCommands, and a single full output image. Cyclic output ownership, write-only vendor parameters, typed output encoding, and Data Storage workflows remain follow-on work.

## Verification

`python tools/verify_writes.py PATH_TO_CLI` uses an isolated loopback peer. It checks all three current-value reads, all four write paths, offline confirmation, identity and value mismatches, readback, and a lost write reply with one dispatch. `cargo test --locked --bin iolink-workbench desktop::writes::tests` exercises offline GUI drafts, a current read alongside a held acquisition owner, approval guards, and close behavior with a held loopback write reply. Real firmware, machine conditions and actuator effects await physical qualification.
