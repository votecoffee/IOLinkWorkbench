# GUI workflow

The workspace follows EthernetIP Workbench's Device → Inputs → Logging pattern. Connection, profile and stop controls stay above the tabs. Each page has one job; secondary tabs stay above its scrolling content.

| Tab | Use it for |
|---|---|
| Device | Host connection, identity expectations, IODD import and advanced profile TOML, on separate pages |
| Inputs | Current raw input, quality, advancing host ages and a compact field/value/unit table; expand bit layout and scaling when needed |
| Logging | New capture folder, duration, update/on-change selection and CSV values; buffer sizing, rotation and the full plan expand on demand |
| Mappings | Imported/manual fields and overrides; Edit mapping opens a focused editor instead of stacking it below the list |
| Parameters | Explicit IODD reads or documented numeric ISDU reads, on separate pages |
| Integration | Optional local HTTP, MQTT and Windows shared-memory settings, on separate pages |
| Diagnostics | Observed connection/identity, finalized timing and explicit device-event reads, on separate pages |
| Offline | Saved capture review/comparison, process bytes, parameter bytes, saved parameter snapshots and event-code decoding, on separate pages |

## First session

1. Use **Profiles…** to load a saved TOML profile. The bundled simulator is already applied at startup. Loading or applying host settings does not connect.
2. In **Device**, choose the host connection. Import a local description in **IODD import**, selecting its exact variant and input layout. Review **Identity expectations** separately from observed identity. Language, standard resources and validation options expand when needed.
3. In **Mappings**, review the effective fields and unresolved input. Apply or discard edits before leaving a draft pending. **Profiles…** saves the applied profile, including logging selections.
4. Choose **Connect…**, review the actual route, port, poll interval, expected identity and enabled feeds, then **Start monitor**. The app opens **Inputs**. Inspect **Diagnostics → Connection & identity** for observations. Import and configured expectations do not verify the device.
5. To log, finish the current monitor session with **Stop and flush**, then choose **Record CSV…**. Select a new folder, duration and CSV fields in **Logging**, then **Start CSV capture**. Zero duration means manual stop. Recording can also start directly while idle.
6. Switch tabs freely during acquisition. **Stop and flush** remains in the header and ends the IO-Link acquisition session, draining any CSV writer. Wait for completion before starting another session. Unlike EthernetIP Workbench, this application does not attach a CSV recorder to a separately retained connection.
7. Use **Offline → Saved captures** to verify a finalized folder. **Review & export** and **Compare captures** are separate pages; stored endpoints are not followed.

Unapplied changes remain visible in the header even when their page is hidden. **Review pending edits** returns to the correct draft. Setup, mapping and profile-file changes are disabled during acquisition or an active parameter read; integration changes apply to the next session. Independent parameter reads and saved reviews keep their existing background ownership and do not become sampling locks.

The footer shows the latest session message; hover to read it in full. **Diagnostics → Connection & identity** also shows the full message. Quality/loss alerts remain above the tabs. Per-session feed choices are summarized in Integration and the connection review; they are not saved profile keys.

## Verification scope

In-process egui tests check page isolation and toolbar bounds at **790×530** and **1130×790**, review/profile navigation, hidden-draft recovery, and stopping an actual simulator capture from another tab with verified CSV drain. Synthetic pointer events stay inside an isolated egui context; no desktop input is sent.

The capability-checked native verifier paints 86 built-in profile/page/dialog and observation cases at **1130×790** into a hidden OpenGL framebuffer. It sends no input and reads no pixels. These checks establish layout geometry, workflow behavior and draw submission. Visible appearance, window-manager interaction, DPI and physical-device acceptance remain separate.
