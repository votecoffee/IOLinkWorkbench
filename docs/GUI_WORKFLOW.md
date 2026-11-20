# GUI workflow

The workspace follows EthernetIP Workbench's Device → Inputs → Logging pattern. Connection, profile and stop controls stay above the tabs. Each page has one job; secondary tabs stay above its scrolling content.

| Tab | Use it for |
|---|---|
| Device | Setup overview, host connection, identity expectations, IODD import and advanced profile TOML, on separate pages |
| Inputs | Current raw input, quality, advancing host ages and a compact field/value/unit table; expand bit layout and scaling when needed |
| Logging | New capture folder, duration, update/on-change selection and CSV values; buffer sizing, rotation and the full plan expand on demand |
| Mappings | Imported/manual fields and overrides; Edit mapping opens a focused editor instead of stacking it below the list |
| Parameters | Search declared IODD parameters or read a documented numeric ISDU address |
| Integration | Optional local HTTP, MQTT and Windows shared-memory settings, on separate pages |
| Diagnostics | Observed connection/identity, finalized timing and explicit device-event reads, on separate pages |
| Offline | Saved capture review/comparison, process bytes, parameter bytes, saved parameter snapshots and event-code decoding, on separate pages |

## First session

1. The bundled typed simulator is already applied at startup. **Device → Overview** summarizes the applied connection, description/mapping, identity expectations and CSV values, with links to each task. It flags an empty hardware endpoint, IODD review notes and an unconfirmed layout condition without claiming physical readiness.
2. Use **Profiles… → Start a profile** to choose the typed simulator, raw-only simulator or incomplete AL1350 template, then **Use starting profile**. Selecting a choice alone changes nothing. Use replaces settings in memory, clears the old save destination and returns to Device overview. **Open & save** loads a saved TOML or saves/replaces the applied profile at an explicit path. Successful loads also return to the overview; failed loads preserve applied settings. Neither action connects.
3. In **Device**, choose the host connection. Import a local description in **IODD import**, selecting its exact variant and input layout. Review **Identity expectations** separately from observed identity. Language, standard resources and validation options expand when needed.
4. In **Mappings**, review the effective fields and unresolved input. Apply or discard edits before leaving a draft pending. **Profiles… → Open & save** saves the applied profile, including logging selections.
5. Choose **Connect…**, review the actual route, port, poll interval, expected identity and enabled feeds, then **Start monitor**. The app opens **Inputs**. Inspect **Diagnostics → Connection & identity** for observations. Import and configured expectations do not verify the device.
6. To log, finish the current monitor session with **Stop and flush**, then choose **Record CSV…**. Select a new folder, duration and CSV fields in **Logging**, then **Start CSV capture**. A timed run counts from local setup completion and includes gateway connection time. Zero duration means manual stop. Recording can also start directly while idle.
7. Switch tabs freely during acquisition. **Stop and flush** remains in the header and ends the IO-Link acquisition session, draining any CSV writer. Wait for completion before starting another session. Unlike EthernetIP Workbench, this application does not attach a CSV recorder to a separately retained connection.
8. Choose **Verify latest recording** in **Logging** or **Offline → Saved captures**. This opens the Verify page and checks the last finalized CSV folder from this app session. **Choose a signal** opens the Signal page after verification. Select a saved field and choose **Review field**. For an earlier recording, enter its folder and choose **Verify capture**.

Unapplied changes remain visible in the header even when their page is hidden. **Review pending edits** returns to the correct draft. Setup, mapping and profile-file changes are disabled during acquisition or an active parameter read; integration changes apply to the next session. Independent parameter reads and saved reviews keep their existing background ownership and do not become sampling locks.

The footer shows the latest session message; hover to read it in full. **Diagnostics → Connection & identity** also shows the full message. Quality/loss alerts remain above the tabs. Per-session feed choices are summarized in Integration and the connection review; they are not saved profile keys.

## Inspect a live payload offline

In **Inputs**, choose **Inspect bytes offline** beside the raw input. **Offline → Process bytes → From Inputs** shows one fixed copy of the bytes, receipt quality, and the mapping used at copy time. You can inspect held or invalid bytes and see their original Inputs state. For host polls, source age and update count remain unknown.

Switch to **Pasted bytes** to keep working with your existing entry under the current mapping. A new copy replaces only the **From Inputs** sample. The copied sample stays in memory for this app session. Monitoring or CSV recording continues while you inspect it.

## Review the route before starting

**Connect…** opens the route review. Both this dialog and **Logging** check the configured AL1350 endpoint, the HTTPS requirement for configured credentials, and confirmation of any conditional physical IODD layout. If a check fails, use **Go to connection settings** or **Review IODD layout** to resolve it. You can continue preparing the CSV options.

These checks run locally. The read verifies reachability and observes master and device identity after startup. Confirm a conditional layout only after checking which condition is active on the device.

## Review enabled software feeds

The connection review lists the HTTP status URL, MQTT broker, exact topic and interval, and Windows mapping name and interval for enabled feeds. **Logging → Software feeds for this recording** shows the same destinations before capture. The settings apply to the next session.

Invalid enabled settings block Start and offer a link to the relevant **Integration** page. The selected Integration page displays its validation error as you edit. You can still prepare CSV settings. The application checks port availability, broker connections and Windows object creation during startup or operation.

## Finding a recording

Logging shows the resolved output folder during a capture. **New folder name** creates a timestamped name in the current parent folder. **Check folder** shows the resolved path and checks for an existing target or unusable parent. The check only inspects the filesystem. Start repeats it and creates the output as a new folder. For the default destination, the worker creates `.local` if needed. A custom destination needs an existing parent folder.

The latest recording stays available after switching profiles or running a monitor session. A finalized run with a recorded error is labeled and can be reviewed. If the worker fails before returning a finalized report, the previous recording remains available. Selecting a different recording clears the previous field selection and export destinations for that source. An active review or export must finish before the shortcut can select a folder.

## Find and read a named parameter

In **Parameters → IODD reads**, search by name, IODD ID, or exact index. Indexes accept decimal or hexadecimal with an **0x** prefix. Select a match, then select a subindex for a record or array. **Read named parameter** issues the explicit read. Expand **Read and save a snapshot** to save the response and its profile to a new JSON file. **IODD details and display preference** holds the declaration, choices, ranges, default, and saved host display selection.

**Offline → Parameter bytes** has its own search and selection for decoding recorded bytes. Filtering narrows the picker and keeps the current parameter selected until you choose another one.

## Read results and device diagnostics

IODD reads, Raw ISDU, Verify Product ID and Device events each keep their own last result. Every result shows the original query. Changing the selected parameter or address leaves the previous result labeled and points out the difference. Applying a new profile clears the results.

One acyclic read can run at a time while cyclic acquisition continues. Switch pages while it runs and return to its page to review the result. **Diagnostics → Device events** shows the diagnostic reply above the expandable **IODD event definitions**.

## Saved capture pages

| Page | Use it for |
|---|---|
| Verify | Check the saved files and inspect recorded timing, identity and interface metrics. |
| Signal | Review a logged value with its plot and statistics. The picker lists verified fields and units in CSV order. |
| Exports | Combine verified CSV parts into one new file, re-decode with the applied profile, or verify an existing re-decoded bundle. |
| Compare captures | Verify two independently selected folders and compare their recorded context and metrics. |

The field picker starts with the first logged column. Before verification, type a field name or leave the field blank to use that first column. Signal review checks the capture again as it builds the plot. A verified capture without typed columns offers **Open re-decode options**.

Verify, Signal and Exports share the capture folder. Each keeps its results when you switch pages. Editing the source folder clears its earlier results and source-specific export destinations. Compare retains its own two folders. Navigation remains available during background work, and completed results appear on their own pages.

To compare a new recording against a baseline, verify the baseline folder and choose **Compare as baseline**. This opens **Compare captures** and fills the left folder. Choose **Use latest recording** to fill the right folder with the latest finalized recording from this app session. You can also edit either folder. Choose **Verify and compare** to check both captures and show the results. A changed folder clears the prior comparison.

## Selecting files from Explorer

On **Compare captures**, drop one **capture.toml** and choose **Use as baseline** or **Use on the right**. **Cancel drop** keeps the current pair and result. Assign or cancel the drop to continue editing or checking the pair. Choose **Verify and compare** once both folders are set.

On the other **Offline → Saved captures** pages, drop **capture.toml** to select its folder on **Verify**. Dropping a re-decoded bundle's **receipt.toml** selects its folder on **Exports**. Choose **Verify capture** or **Verify bundle** explicitly. Bundle verification starts with local checks. Select **Also reverify the original Capture folder above** when you intend to include the original source.

Saved file drops work during acquisition and profile editing. Wait for an active review or export before replacing its folder selection. The comparison folders are independent. You can still enter any folder path manually.

Drop one file anywhere on the window. A **.toml** path opens **Profiles… → Open & save**; review it and choose **Load profile**. An **.xml** or **.zip** path opens **Device → IODD import**; review the import options and choose **Inspect IODD**, then select/apply the matching variant/layout. Extensions are case-insensitive. A drop selects a path only; it does not read, validate, apply, save or connect. Open and Save have separate path fields. Selecting a profile to open does not redirect saving; a successful load sets the save destination to that profile, and a new starter clears both paths. Save / replace always writes the applied profile to its displayed save destination.

Selecting another IODD clears the previous inspection, variant/layout and ZIP-member selection. Language, standard-resource and validation options are retained for review. Applied settings remain unchanged until explicit loading/import. Profile and IODD drops are blocked during acquisition, parameter reads, IODD inspection, pending profile edits, and closing. Multiple files, unsupported extensions and files without a representable local path are rejected. In the Profiles window, TOML drops select the Open path.

## Verification scope

In-process egui tests check page isolation and toolbar bounds at **790×530** and **1130×790**, review/profile navigation, starter selection and guarded replacement, failed-load and source-file preservation, overview routing, synthetic file-drop routing and explicit load/inspect/apply, Unicode paths, rejected multi-file/unsupported drops, hover geometry, hidden-draft recovery, and acyclic result attribution after selection and page changes. Simulator capture tests cover stopping from another tab, verification with a pending export path, signal review, combining CSV parts, re-decoding, and staging and comparing a baseline and latest recording across page changes. Synthetic pointer events stay inside an isolated egui context.

The capability-checked native verifier paints 98 built-in profile/page/dialog and observation cases at **1130×790** into a hidden OpenGL framebuffer. It sends no input and reads no pixels. These checks establish layout geometry, workflow behavior and draw submission. Visible appearance, Explorer/OS drop delivery, window-manager interaction, DPI and physical-device acceptance remain separate. The preview case supplies synthetic hover metadata without desktop input or file access.
