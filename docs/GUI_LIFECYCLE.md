# GUI operation completion and closing

Use the normal window close action to end a session. Closing requests the cyclic sampler's existing stop token, keeps the event loop responsive, and waits for the capture report after CSV drain/flush and interface shutdown. It also waits for pending acyclic reads, IODD inspection, saved-capture verification/export/comparison and saved-parameter review. A read-only request already in progress finishes under its existing timeout; no device command or port change is issued to cancel it.

During closing, navigation and action controls are disabled and the footer reports pending operations. New cyclic sessions are rejected before the stop token can be reset or a new destination created. Every worker owner is polled each frame, including saved-parameter review when its tab is hidden. When all completions have been consumed, the app emits the native close command.

If cyclic capture fails in the completion frame while closing is pending or requested, the close is canceled and the failure stays visible. This includes a failed background task, writer error, or a failed capture/export worker in the final report. Inspect the capture report and any preserved output before retrying. A later explicit close request can leave after the error has been presented. Failures from other read/review/export operations complete their jobs and remain in their respective result panels; waiting for completion does not assert that every operation succeeded. Original files and exclusive/atomic destination rules are unchanged.

Dropping the app also requests the sampler stop token as a fallback. Normal close waits through the UI lifecycle; abrupt process termination, a crash, or OS shutdown that bypasses that lifecycle is outside the clean-close guarantee.

## Automated evidence

```powershell
cargo test --locked --bin iolink-workbench desktop::shutdown
python tools/verify_gui.py target/x86_64-pc-windows-msvc/release/iolink-workbench.exe --output .local/gui-check-NEW.json
```

The shutdown tests use internal logical close requests, channel gates and egui output commands. They cover all six worker domains, hidden-tab completion, lost worker senders, refusal of new sessions, capture-error cancellation and app-drop stop signaling. A real simulator session is started through the GUI controller, stopped, drained, and its rotated saved CSV bundle is verified by the common archive reader. Tests serialize their real GUI-worker dispatch with the self-test worker-denial guard. They send no desktop keyboard/pointer input, create no native test window and take no screenshots.

The separate native verifier still checks hidden OpenGL initialization and offscreen painting of 24 built-in views. It denies background dispatch and performs no input injection or pixel readback. Neither suite establishes native window-manager close routing, visible interactions, DPI/multi-monitor behavior, file-entry usability or physical IO-Link operation.

## Operator acceptance checklist

These are pending visible checks, not results claimed by the automated suites. Start with the bundled **Simulator** profile and a new output directory; select bench hardware only under the session's authorized read-only scope.

1. Open the GUI normally. Check Setup, Monitor & CSV and Offline decode at the default size, minimum size and Windows display scaling used on the workstation. Check scroll access and long-path/Unicode text readability.
2. Enter or paste a new profile/capture path. Check empty and existing destinations, source preservation, save/reload and pending-edit guards. A new output must never silently replace an existing file or folder.
3. Start a manual-duration simulator CSV capture. Use **Stop and flush**, then verify the finalized directory with `verify-capture`. Repeat using the window close action while sampling and confirm the resulting bundle verifies after the window leaves.
4. Begin a saved-capture review/export or saved-parameter review, switch tabs and request close while it is pending. Confirm the window remains responsive until completion and any published output verifies. Repeat with a configured acyclic read only after bench authorization; check that waiting is separate from cyclic sampling.
5. Review error behavior with a controlled disposable output location. A capture completion/flush error during closing must leave the window open with its error. Preserve the affected output for inspection; do not treat an incomplete bundle as verified.

Record visible GUI observations separately from the simulator, loopback/native IPC, offline consistency and physical timing receipts. Hardware selection and physical 1 ms/source-update qualification remain open.
