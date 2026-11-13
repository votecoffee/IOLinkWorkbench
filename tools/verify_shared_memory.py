"""Native Windows shared-memory qualification with a synthetic gateway and independent reader."""
import argparse
import json
import os
from pathlib import Path
import queue
import runpy
import subprocess
import threading
import time
import tomllib
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]


def start(cli, profile, output, name, seconds, extra=()):
    command = [str(cli), "capture", str(profile), str(output), "--seconds", str(seconds), "--shared-memory", name,
               "--shared-memory-interval-ms", "1", *map(str, extra)]
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8",
                               env={k: v for k, v in os.environ.items() if k.upper() != "IOLINK_IFM_PASSWORD"},
                               creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    stdout, stderr, lines = [], [], queue.Queue()

    def errors():
        for line in process.stderr:
            stderr.append(line)
            lines.put(line)

    threads = [threading.Thread(target=errors, daemon=True), threading.Thread(target=lambda: stdout.append(process.stdout.read(1_048_577)), daemon=True)]
    for thread in threads:
        thread.start()
    return process, stdout, stderr, lines, threads


def finish(run, cli, output, expected=0):
    process, stdout, stderr, _, threads = run
    assert process.wait(timeout=15) == expected, stderr
    for thread in threads:
        thread.join(timeout=2)
    process.stdout.close()
    process.stderr.close()
    assert stdout and len(stdout[0]) < 1_048_576
    if expected:
        return None
    report = tomllib.loads(stdout[0])
    assert report == tomllib.loads((output / "capture.toml").read_text(encoding="utf-8"))
    summary = report["shared_memory"]
    assert summary["events_offered"] == summary["events_dequeued"] + summary["queue_dropped"]
    assert summary["events_dequeued"] == summary["coalesced"] + summary["publish_attempts"]
    assert summary["publish_attempts"] + summary["final_refresh_attempts"] == summary["snapshots_written"] + summary["busy_skips"] + summary["encoding_failures"] + summary["write_failures"]
    assert summary["snapshots_replaced"] == max(0, summary["snapshots_written"] - 1)
    assert not summary["worker_failed"] and not summary["write_failures"] and not summary["encoding_failures"]
    subprocess.run([str(cli), "verify-capture", str(output)], capture_output=True, timeout=10, check=True)
    return report


def open_reader(Reader, name, run):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        try:
            reader = Reader(name)
            try:
                reader.read()
                return reader
            except (ValueError, TimeoutError):
                reader.close()
        except OSError:
            pass
        assert run[0].poll() is None, run[2]
        time.sleep(.005)
    raise AssertionError("publisher did not initialize a readable snapshot")


def api_url(run):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        try:
            line = run[3].get(timeout=.1)
        except queue.Empty:
            continue
        if "Local API listening at " in line:
            return line.split("Local API listening at ", 1)[1].split()[0]
    raise AssertionError("API not ready")


def stop_if_running(run):
    if run[0].poll() is None:
        run[0].kill()
        run[0].wait(timeout=5)
    for thread in run[4]:
        thread.join(timeout=2)
    for stream in (run[0].stdout, run[0].stderr):
        if not stream.closed:
            stream.close()


def verify(cli, samples, example, output):
    assert os.name == "nt"
    output.mkdir()
    Reader = runpy.run_path(str(example))["Reader"]
    profile = output / "simulator.toml"
    original = (samples / "simulator.toml").read_bytes()
    profile.write_bytes(original.replace(b"poll_us = 1000", b"poll_us = 4000\nstale_us = 2000"))
    profile_hash = profile.read_bytes()
    name = "verify_" + uuid.uuid4().hex
    run = start(cli, profile, output / "normal", name, 1.7, ["--simulate-faults"])
    observations = []
    try:
        with open_reader(Reader, name, run) as reader:
            session = reader.read()["payload"]["session_id"]
            while run[0].poll() is None:
                observation = reader.read()
                assert observation["header"]["publish_sequence"] == observation["payload"]["snapshot_sequence"]
                assert observation["publisher_process_alive"] is True
                observations.append(observation)
                assert len(observations) < 2000
                time.sleep(.002)
            report = finish(run, cli, output / "normal")
            final = reader.read()
            assert final["payload"]["sampling_active"] is False and final["publisher_process_alive"] is not True
            assert report["shared_memory"]["final_snapshot_written"]
            collision = output / "retained-name-must-not-exist"
            result = subprocess.run([str(cli), "capture", str(profile), str(collision), "--shared-memory", name], capture_output=True, timeout=10)
            assert result.returncode == 1 and not collision.exists()
            assert reader.read()["payload"]["session_id"] == session
        # Closing every retained view/mutex permits a fresh publisher under the same name.
        reused = start(cli, profile, output / "reused", name, .25)
        try:
            with open_reader(Reader, name, reused) as reader:
                assert reader.read()["payload"]["session_id"] != session
            finish(reused, cli, output / "reused")
        finally:
            stop_if_running(reused)
    finally:
        stop_if_running(run)
    inputs = [row["payload"]["input"] for row in observations if row["payload"]["input"]]
    assert any(data["validity"] == "invalid" and data["values"] is None for data in inputs)
    assert any(data["stale"] is True and data["values"] is None for data in inputs)
    assert max(row["payload"]["generation"] for row in observations) >= 2
    assert any(a["raw_hex"] == b["raw_hex"] and a["source_counter"] != b["source_counter"] for a, b in zip(inputs, inputs[1:]))

    Broker = runpy.run_path(str(ROOT / "tools/verify_mqtt.py"))["Broker"]
    with Broker() as broker:
        name = "blocked_" + uuid.uuid4().hex
        blocked = start(cli, profile, output / "blocked-final", name, 1.0, ["--api-listen", "127.0.0.1:0", "--mqtt-broker", broker.address, "--mqtt-interval-ms", "10"])
        try:
            with open_reader(Reader, name, blocked) as reader:
                url = api_url(blocked)
                client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                with reader.lock():
                    frozen = reader.read(max_age_ms=1)
                    time.sleep(.03)
                    aged = reader.read(max_age_ms=1)
                    assert aged["header"]["publish_sequence"] == frozen["header"]["publish_sequence"] and aged["publish_age_exceeds_limit"]
                    denied = subprocess.run([str(cli), "read-shared-memory", name, "--wait-ms", "0"], capture_output=True, timeout=5)
                    assert denied.returncode == 1
                    with client.open(url + "/v1/status", timeout=3) as response:
                        first = json.load(response)["body"]["metrics"]["events_ingested"]
                    before_mqtt = len(broker.messages)
                    time.sleep(.15)
                    with client.open(url + "/v1/status", timeout=3) as response:
                        assert json.load(response)["body"]["metrics"]["events_ingested"] > first
                    mqtt_progress = len(broker.messages) - before_mqtt
                    assert mqtt_progress > 0
                    blocked_report = finish(blocked, cli, output / "blocked-final")
                retained = reader.read(max_age_ms=1)
                assert retained["publisher_process_alive"] is not True
                assert retained["publish_age_exceeds_limit"] and retained["payload"]["sampling_active"] is True
                assert blocked_report["shared_memory"]["busy_skips"] > 0 and not blocked_report["shared_memory"]["final_snapshot_written"]
        finally:
            stop_if_running(blocked)

    snapshots = output / "snapshots.toml"
    snapshots.write_bytes(profile.read_bytes().replace(b"port = 1", b"port = 1\nsimulate_snapshots = true"))
    name = "snapshots_" + uuid.uuid4().hex
    snapshot_run = start(cli, snapshots, output / "snapshot-capture", name, .7, ["--queue-capacity", "2", "--simulate-writer-delay-ms", "20"])
    try:
        with open_reader(Reader, name, snapshot_run) as reader:
            native = subprocess.run([str(cli), "read-shared-memory", name], capture_output=True, text=True, encoding="utf-8", timeout=5, check=True)
            native = json.loads(native.stdout)
            assert native["header"]["qpc_frequency"] == reader.read()["header"]["qpc_frequency"]
            assert native["payload"]["session_id"] == reader.read()["payload"]["session_id"]
            while snapshot_run[0].poll() is None:
                value = reader.read()["payload"]["input"]
                if value:
                    assert value["row_kind"] == "host_poll" and value["source_update_loss"] == "unknown"
                    assert value["source_counter"] is value["source_time_us"] is value["source_age_us"] is value["source_gaps_observed"] is None
                time.sleep(.005)
            snapshot_report = finish(snapshot_run, cli, output / "snapshot-capture")
            assert snapshot_report["counts"]["logger_overruns"] > 0
    finally:
        stop_if_running(snapshot_run)

    name = "crash_" + uuid.uuid4().hex
    crashed = start(cli, profile, output / "expected-incomplete-crash", name, 10)
    try:
        with open_reader(Reader, name, crashed) as reader:
            # Exclude a torn commit before killing this synthetic publisher.
            with reader.lock():
                crashed[0].kill()
                crashed[0].wait(timeout=5)
            retained = reader.read()
            assert retained["publisher_process_alive"] is not True and retained["payload"]["sampling_active"] is True
    finally:
        stop_if_running(crashed)
    assert not (output / "expected-incomplete-crash/capture.toml").exists()
    rejected = output / "must-not-exist"
    for args, expected in [(["--shared-memory-interval-ms", "10"], 2), (["--shared-memory", "Global\\bad"], 1), (["--shared-memory", "bad name"], 1), (["--shared-memory", "bounds", "--shared-memory-queue-capacity", "0"], 1), (["--shared-memory", "bounds", "--shared-memory-interval-ms", "1001"], 1)]:
        result = subprocess.run([str(cli), "capture", str(profile), str(rejected), *args], capture_output=True, timeout=5)
        assert result.returncode == expected and not rejected.exists()
    saved = output / "normal/capture.toml"
    saved_bytes = saved.read_bytes()
    marker = f'busy_skips = {report["shared_memory"]["busy_skips"]}'.encode()
    assert saved_bytes.count(marker) == 1
    try:
        saved.write_bytes(saved_bytes.replace(marker, b"busy_skips = 18446744073709551615"))
        assert subprocess.run([str(cli), "verify-capture", str(saved.parent)], capture_output=True, timeout=10).returncode == 1
    finally:
        saved.write_bytes(saved_bytes)
    baseline_dir = output / "disabled"
    baseline = subprocess.run([str(cli), "capture", str(profile), str(baseline_dir), "--seconds", "1.7", "--simulate-faults"], capture_output=True, text=True, encoding="utf-8", timeout=10, check=True)
    baseline = tomllib.loads(baseline.stdout)
    assert "shared_memory" not in baseline and profile.read_bytes() == profile_hash
    subprocess.run([str(cli), "verify-capture", str(baseline_dir)], capture_output=True, timeout=10, check=True)
    receipt = {"scope": "native Windows IPC, independent Python/Rust readers and synthetic IO-Link only", "version": subprocess.check_output([str(cli), "--version"], text=True).strip(), "fault_capture_rows": report["csv_rows_flushed"], "reader_observations": len(observations), "snapshots_written": report["shared_memory"]["snapshots_written"], "blocked_reader_busy_skips": blocked_report["shared_memory"]["busy_skips"], "blocked_final_commit_reported": not blocked_report["shared_memory"]["final_snapshot_written"], "independent_csv_overruns": snapshot_report["counts"]["logger_overruns"], "snapshot_source_loss": snapshot_report["source_update_loss"], "publisher_exit_and_crash_liveness_checked": True, "retained_mapping_collision_and_reuse_checked": True}
    receipt["mqtt_messages_while_reader_held_mutex"] = mqtt_progress
    keys = ("host_poll_start_spacing", "host_receive_spacing", "gateway_update_spacing", "logger_metrics")
    receipt["timing_observations"] = {name: {key: run.get(key) for key in keys} for name, run in [("disabled_faults", baseline), ("enabled_faults", report), ("reader_holds_mutex_with_api", blocked_report)]}
    (output / "receipt.json").write_bytes((json.dumps(receipt, indent=2) + "\n").encode("utf-8"))
    print(json.dumps({key: value for key, value in receipt.items() if key != "timing_observations"}, indent=2))
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=lambda value: Path(value).resolve())
    parser.add_argument("--samples", type=Path, default=ROOT / "samples")
    parser.add_argument("--example", type=Path, default=ROOT / "examples/shared_memory_reader.py")
    args = parser.parse_args()
    destination = ROOT / ".local" / ("shm-verify-" + uuid.uuid4().hex[:8])
    verify(args.cli, args.samples.resolve(), args.example.resolve(), destination)
    print(f"Evidence directory: {destination}")
