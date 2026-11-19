# GUI workflow

The workspace follows EthernetIP Workbench's Device → Inputs → Logging pattern. Connection, profile and stop controls stay above the tabs. Each page has one job; secondary tabs stay above its scrolling content.

| Tab | Use it for |
|---|---|
| Device | Setup overview, host connection, identity expectations, IODD import and advanced profile TOML, on separate pages |
| Inputs | Current raw input, quality, advancing host ages and a compact field/value/unit table; expand bit layout and scaling when needed |
| Logging | New capture folder, duration, update/on-change selection and CSV values; buffer sizing, rotation and the full plan expand on demand |
| Mappings | Imported/manual fields and overrides; Edit mapping opens a focused editor instead of stacking it below the list |
| Parameters | Explicit IODD reads or documented numeric ISDU reads, on separate pages |
| Integration | Optional local HTTP, MQTT and Windows shared-memory settings, on separate pages |
| Diagnostics | Observed connection/identity, finalized timing and explicit device-event reads, on separate pages |
| Offline | Saved capture review/comparison, process bytes, parameter bytes, saved parameter snapshots and event-code decoding, on separate pages |

## First session

1. The bundled typed simulator is already applied at startup. **Device → Overview** summarizes the applied connection, description/mapping, identity expectations and CSV values, with links to each task. It flags an empty hardware endpoint, IODD review notes and an unconfirmed layout condition without claiming physical readiness.
2. Use **Profiles… → Start a profile** to choose the typed simulator, raw-only simulator or incomplete AL1350 template, then **Use starting profile**. Selecting a choice alone changes nothing. Use replaces settings in memory, clears the old save destination and returns to Device overview. **Open & save** loads a saved TOML or saves/replaces the applied profile at an explicit path. Successful loads also return to the overview; failed loads preserve applied settings. Neither action connects.
3. In **Device**, choose the host connection. Import a local description in **IODD import**, selecting its exact variant and input layout. Review **Identity expectations** separately from observed identity. Language, standard resources and validation options expand when needed.
4. In **Mappings**, review the effective fields and unresolved input. Apply or discard edits before leaving a draft pending. **Profiles… → Open & save** saves the applied profile, including logging selections.
5. Choose **Connect…**, review the actual route, port, poll interval, expected identity and enabled feeds, then **Start monitor**. The app opens **Inputs**. Inspect **Diagnostics → Connection & identity** for observations. Import and configured expectations do not verify the device.
6. To log, finish the current monitor session with **Stop and flush**, then choose **Record CSV…**. Select a new folder, duration and CSV fields in **Logging**, then **Start CSV capture**. Zero duration means manual stop. Recording can also start directly while idle.
7. Switch tabs freely during acquisition. **Stop and flush** remains in the header and ends the IO-Link acquisition session, draining any CSV writer. Wait for completion before starting another session. Unlike EthernetIP Workbench, this application does not attach a CSV recorder to a separately retained connection.
8. Use **Offline → Saved captures** to verify a finalized folder. **Review & export** and **Compare captures** are separate pages; stored endpoints are not followed.

Unapplied changes remain visible in the header even when their page is hidden. **Review pending edits** returns to the correct draft. Setup, mapping and profile-file changes are disabled during acquisition or an active parameter read; integration changes apply to the next session. Independent parameter reads and saved reviews keep their existing background ownership and do not become sampling locks.

The footer shows the latest session message; hover to read it in full. **Diagnostics → Connection & identity** also shows the full message. Quality/loss alerts remain above the tabs. Per-session feed choices are summarized in Integration and the connection review; they are not saved profile keys.

## Selecting files from Explorer

Drop one file anywhere on the window. A **.toml** path opens **Profiles… → Open & save**; review it and choose **Load profile**. An **.xml** or **.zip** path opens **Device → IODD import**; review the import options and choose **Inspect IODD**, then select/apply the matching variant/layout. Extensions are case-insensitive. A drop selects a path only; it does not read, validate, apply, save or connect. Open and Save have separate path fields. Selecting a profile to open does not redirect saving; a successful load sets the save destination to that profile, and a new starter clears both paths. Save / replace always writes the applied profile to its displayed save destination.

Selecting another IODD clears the previous inspection, variant/layout and ZIP-member selection. Language, standard-resource and validation options are retained for review. Applied settings remain unchanged until explicit loading/import. Profile drafts, acquisition, parameter reads, IODD inspection and closing block drops without replacing the selection. Multiple files, unsupported extensions and files without a representable local path are rejected. Use Offline for saved capture folders.

## Verification scope

In-process egui tests check page isolation and toolbar bounds at **790×530** and **1130×790**, review/profile navigation, starter selection and guarded replacement, failed-load and source-file preservation, overview routing, synthetic file-drop routing and explicit load/inspect/apply, Unicode paths, rejected multi-file/unsupported drops, hover geometry, hidden-draft recovery, and stopping an actual simulator capture from another tab with verified CSV drain. Synthetic pointer events stay inside an isolated egui context; no desktop input is sent.

The capability-checked native verifier paints 89 built-in profile/page/dialog and observation cases at **1130×790** into a hidden OpenGL framebuffer. It sends no input and reads no pixels. These checks establish layout geometry, workflow behavior and draw submission. Visible appearance, Explorer/OS drop delivery, window-manager interaction, DPI and physical-device acceptance remain separate. The preview case supplies synthetic hover metadata without desktop input or file access.
