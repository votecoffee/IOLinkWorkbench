# GUI operation completion and closing

Use the normal window close action to end a session. Closing requests the cyclic sampler's existing stop token, keeps the event loop responsive, and waits for the capture report after CSV drain/flush and interface shutdown. It also waits for pending acyclic reads, IODD inspection, multi-port profile review and status checks, saved-capture verification/export/comparison and saved-parameter and diagnostic review. A read-only request already in progress finishes under its existing timeout; no device command or port change is issued to cancel it.

During closing, navigation and action controls are disabled and the footer reports pending operations. New cyclic sessions are rejected before the stop token can be reset or a new destination created. Every worker owner is polled each frame, including reviewed writes and saved-parameter and diagnostic review when their tabs are hidden. When all completions have been consumed, the app emits the native close command.

If cyclic capture fails in the completion frame while closing is pending or requested, the close is canceled and the failure stays visible. This includes a failed background task, writer error, or a failed capture/export worker in the final report. Inspect the capture report and any preserved output before retrying. A later explicit close request can leave after the error has been presented. Failures from other read/review/export operations complete their jobs and remain in their respective result panels; waiting for completion does not assert that every operation succeeded. Original files and exclusive/atomic destination rules are unchanged.

Dropping the app also requests the sampler stop token as a fallback. Normal close waits through the UI lifecycle; abrupt process termination, a crash, or OS shutdown that bypasses that lifecycle is outside the clean-close guarantee.

An approved write runs to completion under the existing request timeouts. Normal close waits for it. If the outcome is unconfirmed, the app cancels closing and opens **Device → Writes** with the result. The result retains the target shown at dispatch. An in-progress write blocks new acquisition, live parameter reads, and profile editing. See [reviewed writes](WRITES.md).

## Starting a session

Start monitor and Start CSV capture use one shared availability check. The start method repeats that check before filesystem preparation or state changes: closing, an owned cyclic completion channel, pending profile edits, or unfinished IODD inspection blocks a new session. A completed worker remains owned until its result is consumed, so a second start cannot replace an unread completion or its stop token. Independent acyclic reads and saved reviews remain eligible to run alongside acquisition.

Each accepted session gets a fresh stop token and a fresh bounded GUI display queue. The controller prepares these locally and publishes the new worker/state only after thread launch succeeds. A rejected/failed launch preserves the previous in-memory report, live observations, queued observations and stop state. An accepted worker's subsequent transport/writer failure belongs to the new session and is reported through its normal completion path. Capture destinations must be new; existing files remain protected.

`cargo test --locked --bin iolink-workbench desktop::self_test::start_tests` covers rejected launch with the existing worker-denial guard, pending and completed-but-unconsumed ownership, pending drafts/inspection, and two consecutive GUI-controlled simulator CSV captures. The positive test keeps acyclic/review completion channels pending, verifies both captures, checks fresh sample indexes and stop/queue isolation, rejects destination reuse and preserves the first capture's bytes. Held channels model unrelated work; only the cyclic simulator capture actually runs. No desktop input or native window is used, and these checks do not establish visible interaction or physical gateway concurrency.

The **Multiple ports** page shares the cyclic acquisition owner with single-port recording and monitoring. Close requests one stop for the set and waits for each port writer to finish. A partial failure cancels close and opens **Logging → Multiple ports** with the per-port results. The two modes use separate input views and preserve previous recording paths.

## Automated evidence

```powershell
cargo test --locked --bin iolink-workbench desktop::shutdown
python tools/verify_gui.py target/x86_64-pc-windows-msvc/release/iolink-workbench.exe --output .local/gui-check-NEW.json
```

The shutdown tests use internal logical close requests, channel gates and egui output commands. They cover hidden-tab completion, lost worker senders, refusal of new sessions, capture-error cancellation and app-drop stop signaling. A real simulator session is started through the GUI controller, stopped, drained, and its rotated saved CSV bundle is verified by the common archive reader. Tests serialize their real GUI-worker dispatch with the self-test worker-denial guard. They send no desktop keyboard/pointer input, create no native test window and take no screenshots.

A separate loopback write test holds a reply during close and verifies that an unconfirmed result reopens the Writes page. Tests in `desktop::port_capture` check simulator capture and status completion on hidden tabs, per-port CSV verification, applied-profile preservation and partial failures. The native verifier paints 119 profile, page, dialog and [synthetic observation views](LIVE_VIEW.md) in a hidden OpenGL context. It denies background dispatch and performs no input injection or pixel readback. Neither suite establishes native window-manager close routing, visible interactions, DPI/multi-monitor behavior, file-entry usability or physical IO-Link operation.

Route checks precede output-folder preparation and worker launch. They share the transport's ifm origin and HTTPS rules and capture's physical IODD condition check. A local rejection leaves the prior observation, report and stop token available. Resolve issues through Device → Connection or Mappings. Reachability and observed identity are checked in the worker.

Enabled feeds are validated before output-folder preparation and dispatch. Connection review, Logging and Start use the same configuration construction and the services' local validators. Binding, broker reachability and mapping creation are tested when the services run.

Logging's Check folder action checks the resolved parent and target on request. Start repeats the check before dispatch. The check creates no directories. A missing default `.local` parent is prepared by the worker, and the writer creates the capture directory exclusively. A collision after checking still fails at creation.

## Operator acceptance checklist

These are pending visible checks, not results claimed by the automated suites. Start with the bundled **Simulator** profile and a new output directory; select bench hardware only under the session's authorized read-only scope.

1. Open the GUI normally. Check Device, Inputs, Logging, Mappings, Parameters, Integration, Diagnostics and Offline at the default size, minimum size and Windows display scaling used on the workstation. Check scroll access and long-path/Unicode text readability.
2. Enter or paste a new profile/capture path. Check empty and existing destinations, source preservation, save/reload and pending-edit guards. A new output must never silently replace an existing file or folder.
3. Start a manual-duration simulator CSV capture. Use **Stop and flush**, then verify the finalized directory with `verify-capture`. Repeat using the window close action while sampling and confirm the resulting bundle verifies after the window leaves.
4. Begin a saved-capture review/export or saved-parameter review, switch tabs and request close while it is pending. Confirm the window remains responsive until completion and any published output verifies. Repeat with a configured acyclic read only after bench authorization; check that waiting is separate from cyclic sampling.
5. Review error behavior with a controlled disposable output location. A capture completion/flush error during closing must leave the window open with its error. Preserve the affected output for inspection; do not treat an incomplete bundle as verified.

Record visible GUI observations separately from the simulator, loopback/native IPC, offline consistency and physical timing receipts. Hardware selection and physical 1 ms/source-update qualification remain open.
