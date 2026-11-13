"""Headless read-only loopback API acceptance; no physical hardware or desktop input."""
import argparse
import csv
import json
import os
from pathlib import Path
import queue
import re
import socket
import subprocess
import threading
import time
import tomllib
import urllib.error
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]
CLIENT = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def request(url, path, expected=200, method="GET", headers=None):
    query = urllib.request.Request(url + path, method=method, headers=headers or {})
    try:
        response = CLIENT.open(query, timeout=3)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        assert response.status == expected, (path, response.status, expected)
        assert response.headers["Cache-Control"] == "no-store"
        body = response.read(1_048_577)
        assert len(body) <= 1_048_576
        return json.loads(body)


def capture(cli, profile, output, *options, history=4096, seconds=1.3, exercise=True):
    original = profile.read_bytes()
    process = subprocess.Popen([str(cli), "capture", str(profile), str(output), "--seconds", str(seconds),
                               "--api-listen", "127.0.0.1:0", "--api-history-capacity", str(history), *map(str, options)],
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8",
                              env={k: v for k, v in os.environ.items() if k.upper() != "IOLINK_IFM_PASSWORD"},
                              creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    lines = queue.Queue()
    errors = []

    def read_errors():
        for line in process.stderr:
            errors.append(line)
            lines.put(line)

    reader = threading.Thread(target=read_errors, daemon=True)
    reader.start()
    try:
        deadline = time.monotonic() + 10
        url = None
        while time.monotonic() < deadline and url is None:
            try:
                line = lines.get(timeout=.1)
            except queue.Empty:
                if process.poll() is not None:
                    break
                continue
            match = re.search(r"Local API listening at (http://127\.0\.0\.1:\d+)", line)
            if match:
                url = match[1]
        assert url, errors
        schema = request(url, "/v1/schema")
        session = schema["session_id"]
        assert schema["format_version"] == 1 and schema["body"]["schema"]["application"] == "IOLinkWorkbench"
        assert "endpoint" not in json.dumps(schema) and "password" not in json.dumps(schema).lower()
        assert schema["body"]["schema"]["input_bytes"] == 2
        if exercise:
            request(url, "/v1/status", 405, "POST")
            request(url, "/v1/status", 403, headers={"Origin": "http://example.invalid"})
            request(url, "/v1/status", 403, headers={"Host": "example.invalid"})
            request(url, "/v1/history?after=0&session=wrong", 409)
            for path in ["/v1/history?after=0", "/v1/history?limit=129", "/v1/history?limit=0", "/v1/history?limit=1&limit=2", "/v1/history?write=1"]:
                request(url, path, 400)
            request(url, "/unknown", 404)
        events = []
        after = 0
        status = None
        until = time.monotonic() + seconds - .45
        while time.monotonic() < until and process.poll() is None:
            page = request(url, f"/v1/history?after={after}&session={session}&limit=128")
            assert page["session_id"] == session
            rows = page["body"]["events"]
            assert all(row["sequence"] > after for row in rows)
            events.extend(rows)
            after = page["body"]["next_after"]
            status = request(url, "/v1/status")
            time.sleep(.008)
        if exercise:
            # A stalled partial request occupies HTTP only; ingestion continues.
            port = int(url.rsplit(":", 1)[1])
            before = status["body"]["metrics"]["events_ingested"]
            with socket.create_connection(("127.0.0.1", port), timeout=3) as slow:
                slow.sendall(b"G")
                time.sleep(.10)
                later = request(url, "/v1/status")
                assert later["body"]["metrics"]["events_ingested"] > before
        assert process.wait(timeout=10) == 0, errors
        stdout = process.stdout.read()
        reader.join(timeout=2)
        report = tomllib.loads(stdout)
        assert report == tomllib.loads((output / "capture.toml").read_text(encoding="utf-8"))
        metrics = report["local_api"]
        assert metrics["url"] == url and metrics["session_id"] == session
        assert metrics["events_offered"] == metrics["events_ingested"] + metrics["queue_dropped"]
        assert metrics["events_offered"] == report["counts"]["received"] + report["counts"]["reconnects"] + report["counts"]["disconnects"] + 2
        assert metrics["history_evicted"] == max(0, metrics["events_ingested"] - history)
        assert not metrics["worker_failed"] and report["csv_rows_unconfirmed"] == 0
        assert events and len({e["sequence"] for e in events}) == len(events)
        for previous, current in zip(events, events[1:]):
            assert previous["sequence"] < current["sequence"]
            assert previous["host_elapsed_us"] <= current["host_elapsed_us"]
        checked = subprocess.run([str(cli), "verify-capture", str(output)], capture_output=True, text=True, encoding="utf-8", timeout=10, check=True)
        assert tomllib.loads(checked.stdout)["rows_checked"] == report["csv_rows_flushed"]
        rows = []
        for part in sorted(output.glob("capture-*.csv")):
            with part.open(encoding="utf-8", newline="") as stream:
                rows.extend(csv.DictReader(stream))
        assert len(rows) == report["csv_rows_flushed"]
        assert profile.read_bytes() == original
        with socket.socket() as probe:
            probe.settimeout(.2)
            assert probe.connect_ex(("127.0.0.1", int(url.rsplit(":", 1)[1]))) != 0
        return report, events
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        process.stdout.close()
        reader.join(timeout=2)
        process.stderr.close()


def verify(cli, samples, output):
    output.mkdir()
    profile = output / "simulator.toml"
    original = (samples / "simulator.toml").read_bytes()
    profile.write_bytes(original.replace(b'poll_us = 1000', b'poll_us = 1000\nstale_us = 2000'))
    report, events = capture(cli, profile, output / "faults", "--simulate-faults", seconds=1.8)
    inputs = [event["data"] for event in events if event["data"]["kind"] == "input"]
    assert any(e["validity"] == "invalid" and e["values"] is None for e in inputs)
    assert any(e["stale"] is True and e["values"] is None for e in inputs)
    assert any(e["classification"] == "duplicate" for e in inputs)
    assert any(e["data"]["kind"] == "disconnected" for e in events)
    assert max(e["generation"] for e in events) >= 2
    assert any(a["raw_hex"] == b["raw_hex"] and a["source_counter"] != b["source_counter"] for a, b in zip(inputs, inputs[1:]))
    small_report, _ = capture(cli, profile, output / "bounded", history=8, seconds=1.1, exercise=False)
    assert small_report["local_api"]["history_evicted"] > 0
    changed_profile = output / "on-change.toml"
    changed_profile.write_bytes(profile.read_bytes().replace(b'mode = "every_update"', b'mode = "on_change"'))
    changed_report, changed_events = capture(cli, changed_profile, output / "on-change", seconds=1.1, exercise=False)
    assert changed_report["counts"]["unchanged"] > 0
    assert any(e["data"]["kind"] == "input" and e["data"]["classification"] == "forward" and e["data"]["csv_sample_index"] is None for e in changed_events)
    pressure, pressure_events = capture(cli, profile, output / "backpressure", "--queue-capacity", 2, "--simulate-writer-delay-ms", 20, seconds=1.1, exercise=False)
    assert pressure["counts"]["logger_overruns"] > 0
    assert any(e["data"].get("csv_enqueued") is False for e in pressure_events)
    # Verification rejects altered saved API accounting without changing raw evidence.
    saved_report = output / "backpressure/capture.toml"
    saved_bytes = saved_report.read_bytes()
    marker = f'queue_dropped = {pressure["local_api"]["queue_dropped"]}'.encode()
    assert saved_bytes.count(marker) == 1
    try:
        saved_report.write_bytes(saved_bytes.replace(marker, b'queue_dropped = 18446744073709551615'))
        rejected_summary = subprocess.run([str(cli), "verify-capture", str(saved_report.parent)], capture_output=True, timeout=10)
        assert rejected_summary.returncode == 1
    finally:
        saved_report.write_bytes(saved_bytes)
    snapshots = output / "snapshots.toml"
    snapshots.write_bytes(profile.read_bytes().replace(b'port = 1', b'port = 1\nsimulate_snapshots = true'))
    snapshot_report, snapshot_events = capture(cli, snapshots, output / "snapshot-capture", seconds=1.1, exercise=False)
    assert snapshot_report["row_kind"] == "host_poll" and snapshot_report["source_update_loss"] == "unknown"
    for event in snapshot_events:
        if event["data"]["kind"] == "input":
            data = event["data"]
            assert data["row_kind"] == "host_poll" and data["source_update_loss"] == "unknown"
            assert data["source_counter"] is data["source_time_us"] is data["source_age_us"] is data["source_gaps_observed"] is None
    # Invalid API configuration and bind failures leave the output destination absent.
    rejected = output / "must-not-exist"
    for options, expected in [(["--api-listen", "0.0.0.0:0"], 1), (["--api-listen", "127.0.0.1:0", "--api-queue-capacity", "0"], 1), (["--api-listen", "127.0.0.1:0", "--api-history-capacity", "16385"], 1), (["--api-queue-capacity", "4"], 2)]:
        result = subprocess.run([str(cli), "capture", str(profile), str(rejected), *options], capture_output=True, timeout=10)
        assert result.returncode == expected and not rejected.exists(), result.stderr
    with socket.socket() as occupied:
        occupied.bind(("127.0.0.1", 0))
        occupied.listen()
        result = subprocess.run([str(cli), "capture", str(profile), str(rejected), "--api-listen", f"127.0.0.1:{occupied.getsockname()[1]}"], capture_output=True, timeout=10)
        assert result.returncode == 1 and not rejected.exists()
    baseline_dir = output / "api-disabled"
    baseline_result = subprocess.run([str(cli), "capture", str(profile), str(baseline_dir), "--seconds", "1.1"], capture_output=True, text=True, encoding="utf-8", timeout=10, check=True)
    baseline = tomllib.loads(baseline_result.stdout)
    assert "local_api" not in baseline
    assert baseline == tomllib.loads((baseline_dir / "capture.toml").read_text(encoding="utf-8"))
    subprocess.run([str(cli), "verify-capture", str(baseline_dir)], capture_output=True, timeout=10, check=True)
    receipt = {"scope": "synthetic gateway and loopback API only; no physical/desktop interaction", "version": subprocess.check_output([str(cli), "--version"], text=True).strip(), "fault_capture_rows": report["csv_rows_flushed"], "observed_api_events": len(events), "source_reconnects": report["counts"]["reconnects"], "bounded_history_evictions": small_report["local_api"]["history_evicted"], "independent_logger_overruns": pressure["counts"]["logger_overruns"], "snapshots_source_loss": snapshot_report["source_update_loss"]}
    keys = ("host_poll_start_spacing", "host_receive_spacing", "host_new_update_spacing", "gateway_update_spacing", "logger_metrics")
    receipt["timing_observations"] = {name: {key: run.get(key) for key in keys} for name, run in [("api_disabled", baseline), ("api_enabled", small_report), ("api_with_csv_backpressure", pressure)]}
    (output / "receipt.json").write_bytes((json.dumps(receipt, indent=2) + "\n").encode("utf-8"))
    print(json.dumps({key: value for key, value in receipt.items() if key != "timing_observations"}, indent=2))
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=lambda value: Path(value).resolve())
    parser.add_argument("--samples", type=Path, default=ROOT / "samples")
    args = parser.parse_args()
    destination = ROOT / ".local" / ("api-verify-" + uuid.uuid4().hex[:8])
    verify(args.cli, args.samples.resolve(), destination)
    print(f"Evidence directory: {destination}")
