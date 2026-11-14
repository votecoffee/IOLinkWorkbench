# CLI interruption and finalized captures

During `capture`, Ctrl+C or Windows Ctrl+Break requests the existing stop token. The next sampler stop check ends acquisition; the writer then drains its accepted queue, flushes/syncs CSV parts, and the independent integration workers finish. The CLI saves `capture.toml`, prints the same report to stdout, and returns **130** when interruption completed without a transport-terminal, writer or integration-worker failure. Repeated interrupts request the same stop; they do not implement a force-stop command.

A failed final report returns **1**, including a writer failure during an interrupted drain. Inspect `writer_error`, `terminal_error`, integration `worker_failed`, `csv_rows_unconfirmed` and logger overruns. Do not interpret 130 as zero source loss: known queue overruns may already have occurred, and snapshot source loss remains unknown. Failure before a report can be saved may leave a partial directory and an error on stderr. Preserve that evidence rather than treating it as a finalized capture.

The stop token does not abort an in-flight blocking gateway request. A completed response may still be accounted before the next stop check; an unresponsive request waits for its configured timeout. Acquisition elapsed time and host receive statistics exclude the later writer drain; `logger_metrics.final_drain_us` records that separately. Source-update, host receive and writer timing retain their existing meanings.

Let the process finish before reviewing its output. For a normal interrupted capture, use `verify-capture DIRECTORY` to check the complete saved bundle. That checks saved consistency, not physical identity, source completeness or hardware performance. Do not overwrite or repair a failed bundle in place.

This behavior applies after capture's interrupt handler has been registered. Console-window close, logoff/OS shutdown, forced termination, a crash, and interruption before handler registration are outside the clean-drain guarantee. A terminal or launcher can also change Ctrl+C routing or inheritance. The native checks below exercise Windows control events, not keyboard or terminal interaction.

## Developer verification

From a Windows source checkout with Python 3.11+ and a built CLI:

```powershell
python tools/verify_cli_shutdown.py target/x86_64-pc-windows-msvc/release/iolink-cli.exe
```

An optional `--output NEW_DIRECTORY` chooses a new evidence location; the default is under ignored `.local/`. The executable argument can also point to an extracted portable CLI. The helper uses original synthetic profiles/replies and no vendor files or physical endpoints. It preserves input profiles and existing rotation destinations. Failed runs retain private files for inspection and do not publish a passing receipt.

Four process-level cases cover:

- Ctrl+C with row rotation: exit 130, identical stdout/saved report, zero unconfirmed rows, and complete offline CSV verification.
- Repeated Ctrl+Break with a deliberately slow writer and full bounded queue: exit 130, counted overruns, accepted-row drain and verified rotated CSV.
- Ctrl+Break during an unanswered loopback HTTP poll: the request reaches its configured timeout, the report retains the disconnect/error, and the empty finalized CSV verifies. Snapshot source loss remains unknown.
- Ctrl+Break before an injected rotation failure during drain: exit 1 takes precedence over interruption, flushed/unconfirmed/overrun accounting balances, and the existing destination stays unchanged. This failing fixture is not labeled a verified capture.

The controller creates a separate console with hidden startup settings. The worker checks that console contains only itself before launch and exactly itself plus the test CLI before each event, and checks that its console window is hidden. Ctrl+C broadcasts only within that isolated console; Ctrl+Break addresses a newly created CLI process group. A kill-on-close Windows job contains the worker and its children, including timeout/failure cleanup. Only successful normal process completion counts as a passing case. No desktop input, screenshots, pixel readback or existing console attachment is used.

The receipt records executable/report/profile hashes, exits, flushed rows, parts, overruns and unconfirmed rows. These are synthetic/native process results. They do not establish terminal keyboard routing, console-window close or OS shutdown behavior, crash recovery, visible GUI acceptance, or physical 1 ms acquisition.

Microsoft's API contracts distinguish [Ctrl+C broadcast and Ctrl+Break process groups](https://learn.microsoft.com/en-us/windows/console/generateconsolectrlevent), [console membership](https://learn.microsoft.com/en-us/windows/console/getconsoleprocesslist), and [handler/ignore-attribute inheritance](https://learn.microsoft.com/en-us/windows/console/setconsolectrlhandler). The application's pinned `ctrlc` dependency handles both interruption events; no new device operation or transport cancellation is introduced here.
