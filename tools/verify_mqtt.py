"""Qualify MQTT with an original bounded loopback peer and synthetic IO-Link input."""
import argparse
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
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]


def read_exact(stream, count):
    result = bytearray()
    while len(result) < count:
        block = stream.recv(count - len(result))
        if not block:
            raise EOFError()
        result.extend(block)
    return bytes(result)


def packet(stream):
    header = read_exact(stream, 1)[0]
    length = 0
    multiplier = 1
    for _ in range(4):
        byte = read_exact(stream, 1)[0]
        length += (byte & 127) * multiplier
        if byte < 128:
            assert length <= 1_049_090, length
            return header, read_exact(stream, length)
        multiplier *= 128
    raise AssertionError("oversized MQTT length encoding")


class Broker:
    def __init__(self, mode="normal"):
        self.mode = mode
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(8)
        self.listener.settimeout(.1)
        self.address = f"127.0.0.1:{self.listener.getsockname()[1]}"
        self.stop = threading.Event()
        self.messages = []
        self.clients = []
        self.threads = []
        self.identities = []
        self.errors = []
        self.disconnects = 0
        self.thread = threading.Thread(target=self.accept, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def accept(self):
        while not self.stop.is_set():
            try:
                stream, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            assert len(self.clients) < 16
            self.clients.append(stream)
            worker = threading.Thread(target=self.handle, args=(stream, len(self.clients)), daemon=True)
            self.threads.append(worker)
            worker.start()

    def handle(self, stream, number):
        try:
            stream.settimeout(2)
            kind, body = packet(stream)
            assert kind == 0x10 and body[:10] == b"\x00\x04MQTT\x04\x02\x00\x0a"
            length = int.from_bytes(body[10:12], "big")
            client = body[12:].decode("ascii")
            assert length == len(client) and 1 <= length <= 23 and client.isalnum()
            self.identities.append(client)
            if self.mode == "stall" and number == 1:
                self.stop.wait(.75)
                return
            if self.mode == "malformed" and number == 1:
                stream.sendall(b"\x20\x02\x01\x00")
                return
            if self.mode == "late-ack":
                self.stop.wait(.15)
            stream.sendall(b"\x20\x02")
            time.sleep(.005)
            stream.sendall(b"\x00\x00")
            if self.mode == "no-read":
                self.stop.wait(4)
                return
            while not self.stop.is_set():
                kind, body = packet(stream)
                if kind == 0xE0:
                    assert body == b""
                    self.disconnects += 1
                    return
                if kind == 0xC0:
                    assert body == b""
                    stream.sendall(b"\xd0\x00")
                    continue
                assert kind == 0x30, "QoS/DUP/RETAIN flags must all be zero"
                size = int.from_bytes(body[:2], "big")
                topic = body[2:2 + size].decode("utf-8")
                assert 1 <= size <= 512 and "+" not in topic and "#" not in topic
                payload = json.loads(body[2 + size:])
                assert len(self.messages) < 1000
                self.messages.append((topic, payload))
                if self.mode == "reset" and number == 1:
                    return
        except (EOFError, OSError):
            pass
        except Exception as error:
            self.errors.append(repr(error))
        finally:
            stream.close()

    def __exit__(self, *_):
        self.stop.set()
        self.listener.close()
        for stream in self.clients:
            stream.close()
        self.thread.join(timeout=2)
        for worker in self.threads:
            worker.join(timeout=2)
        assert not self.errors, self.errors


def capture(cli, profile, output, broker, *extra, seconds=1.8, api=False, interval=10):
    original = profile.read_bytes()
    command = [str(cli), "capture", str(profile), str(output), "--seconds", str(seconds), "--mqtt-broker", broker.address,
               "--mqtt-topic", "test/IO-Link/µm", "--mqtt-interval-ms", str(interval), *map(str, extra)]
    if api:
        command.extend(["--api-listen", "127.0.0.1:0"])
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8",
                               env={k: v for k, v in os.environ.items() if k.upper() != "IOLINK_IFM_PASSWORD"},
                               creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    stderr = []
    stdout = []
    ready = queue.Queue()

    def errors():
        for line in process.stderr:
            stderr.append(line)
            ready.put(line)

    threads = [threading.Thread(target=errors, daemon=True), threading.Thread(target=lambda: stdout.append(process.stdout.read(1_048_577)), daemon=True)]
    for thread in threads:
        thread.start()
    api_received = 0
    try:
        if api:
            deadline = time.monotonic() + 5
            url = None
            while time.monotonic() < deadline and url is None:
                try:
                    line = ready.get(timeout=.1)
                except queue.Empty:
                    continue
                match = re.search(r"Local API listening at (http://127\.0\.0\.1:\d+)", line)
                if match:
                    url = match[1]
            assert url, stderr
            client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            until = time.monotonic() + seconds * .5
            while time.monotonic() < until:
                with client.open(url + "/v1/status", timeout=3) as response:
                    status = json.load(response)
                latest = status["body"]["latest_event"]
                if latest and latest["data"]["kind"] == "input":
                    api_received = max(api_received, latest["data"]["receive_index"])
                time.sleep(.02)
            assert api_received > 10
        assert process.wait(timeout=20) == 0, stderr
        for thread in threads:
            thread.join(timeout=2)
        assert len(stdout) == 1 and len(stdout[0]) <= 1_048_576
        report = tomllib.loads(stdout[0])
        assert report == tomllib.loads((output / "capture.toml").read_text(encoding="utf-8"))
        mqtt = report["mqtt"]
        assert not mqtt["worker_failed"] and mqtt["encoding_failures"] == 0
        assert mqtt["events_offered"] == mqtt["events_dequeued"] + mqtt["queue_dropped"]
        assert mqtt["events_dequeued"] == mqtt["coalesced"] + mqtt["publish_attempts"] + mqtt["shutdown_discarded"]
        assert mqtt["publish_attempts"] == mqtt["socket_writes_completed"] + mqtt["publish_write_failures"] + mqtt["encoding_failures"]
        assert mqtt["broker_connect_attempts"] == mqtt["broker_connections"] + mqtt["connect_failures"]
        assert mqtt["events_offered"] == report["counts"]["received"] + report["counts"]["reconnects"] + report["counts"]["disconnects"] + 2
        assert mqtt["coalesced"] > 0
        subprocess.run([str(cli), "verify-capture", str(output)], capture_output=True, timeout=10, check=True)
        assert profile.read_bytes() == original
        time.sleep(.03)
        for topic, frame in broker.messages:
            assert topic == mqtt["topic"] and frame["format_version"] == 1
            assert frame["session_id"] == mqtt["session_id"] and frame["capture_utc_start"] == report["utc_start"]
            assert frame["host_elapsed_us"] >= frame["event_elapsed_us"]
            if frame["input"]:
                data = frame["input"]
                assert data["capture_counts"]["received"] <= report["counts"]["received"]
                if data["validity"] != "valid" or data["stale"] is True:
                    assert data["values"] is None
        return report, [frame for _, frame in broker.messages], api_received
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        for thread in threads:
            thread.join(timeout=2)
        process.stdout.close()
        process.stderr.close()


def verify(cli, samples, output):
    output.mkdir()
    profile = output / "simulator.toml"
    source = (samples / "simulator.toml").read_bytes()
    profile.write_bytes(source.replace(b"poll_us = 1000", b"poll_us = 4000\nstale_us = 2000"))
    with Broker() as broker:
        report, frames, _ = capture(cli, profile, output / "faults", broker, "--simulate-faults")
        assert broker.disconnects == 1 and report["mqtt"]["socket_writes_completed"] > 1
        inputs = [frame["input"] for frame in frames if frame["input"]]
        assert any(data["validity"] == "invalid" for data in inputs)
        assert any(data["stale"] is True for data in inputs)
        assert max(frame["generation"] for frame in frames) >= 2
        assert frames[-1]["sampling_active"] is False
    with Broker("reset") as broker:
        reconnect, frames, _ = capture(cli, profile, output / "broker-reconnect", broker)
        assert reconnect["mqtt"]["broker_connections"] >= 2 and reconnect["mqtt"]["connection_losses"] > 0
        assert reconnect["counts"]["reconnects"] == 0 and max(frame["publisher_connection"] for frame in frames) >= 2
        assert len(set(broker.identities)) == 1
    with Broker("stall") as broker:
        delayed, _, api_received = capture(cli, profile, output / "delayed-handshake", broker, "--mqtt-queue-capacity", 2, api=True)
        assert delayed["mqtt"]["connect_failures"] > 0 and delayed["mqtt"]["queue_dropped"] > 0
    with Broker("malformed") as broker:
        malformed, _, _ = capture(cli, profile, output / "malformed-handshake", broker)
        assert malformed["mqtt"]["connect_failures"] > 0 and malformed["mqtt"]["broker_connections"] > 0
    with Broker("late-ack") as broker:
        final_drop, frames, _ = capture(cli, profile, output / "final-event-drop", broker, "--mqtt-queue-capacity", 1, seconds=.1)
        assert final_drop["mqtt"]["queue_dropped"] > 0 and frames[-1]["sampling_active"] is False
    snapshots = output / "snapshots.toml"
    snapshots.write_bytes(profile.read_bytes().replace(b"port = 1", b"port = 1\nsimulate_snapshots = true"))
    with Broker() as broker:
        snapshot_report, frames, _ = capture(cli, snapshots, output / "snapshots", broker, "--queue-capacity", 2, "--simulate-writer-delay-ms", 20)
        assert snapshot_report["counts"]["logger_overruns"] > 0
        for frame in frames:
            if frame["input"]:
                assert frame["input"]["row_kind"] == "host_poll" and frame["input"]["source_update_loss"] == "unknown"
                assert frame["input"]["source_counter"] is frame["input"]["source_time_us"] is frame["input"]["source_gaps_observed"] is None
    wide = output / "wide.toml"
    extra = "".join(f'\n[[fields]]\nname = "f{i:03}{"x" * 246}"\nbit_offset = 0\nbits = 16\nkind = "unsigned"\nunit = "{"u" * 256}"\n' for i in range(126))
    wide.write_bytes(source + extra.encode("utf-8"))
    with Broker("no-read") as broker:
        stalled, _, _ = capture(cli, wide, output / "slow-reader", broker, "--mqtt-queue-capacity", 2, seconds=2.2, api=True)
        assert stalled["mqtt"]["queue_dropped"] > 0 and stalled["mqtt"]["publish_write_failures"] > 0
        assert stalled["csv_rows_flushed"] > 100
    rejected = output / "must-not-exist"
    for args, expected in [(["--mqtt-topic", "x"], 2), (["--mqtt-broker", "0.0.0.0:1883"], 1), (["--mqtt-broker", "127.0.0.1:0"], 1), (["--mqtt-broker", "127.0.0.1:1883", "--mqtt-topic", "bad/#"], 1), (["--mqtt-broker", "127.0.0.1:1883", "--mqtt-interval-ms", "0"], 1), (["--mqtt-broker", "127.0.0.1:1883", "--mqtt-queue-capacity", "16385"], 1)]:
        result = subprocess.run([str(cli), "capture", str(profile), str(rejected), *args], capture_output=True, timeout=10)
        assert result.returncode == expected and not rejected.exists(), result.stderr
    saved = output / "faults/capture.toml"
    original = saved.read_bytes()
    target = f'coalesced = {report["mqtt"]["coalesced"]}'.encode()
    assert original.count(target) == 1
    try:
        saved.write_bytes(original.replace(target, b"coalesced = 18446744073709551615"))
        assert subprocess.run([str(cli), "verify-capture", str(saved.parent)], capture_output=True, timeout=10).returncode == 1
    finally:
        saved.write_bytes(original)
    baseline_dir = output / "mqtt-disabled"
    baseline_result = subprocess.run([str(cli), "capture", str(profile), str(baseline_dir), "--seconds", "1.8", "--simulate-faults"], capture_output=True, text=True, encoding="utf-8", timeout=10, check=True)
    baseline = tomllib.loads(baseline_result.stdout)
    assert "mqtt" not in baseline and "local_api" not in baseline
    subprocess.run([str(cli), "verify-capture", str(baseline_dir)], capture_output=True, timeout=10, check=True)
    receipt = {"scope": "original local MQTT peer and synthetic IO-Link only; not a production broker or physical qualification", "version": subprocess.check_output([str(cli), "--version"], text=True).strip(), "fault_capture_rows": report["csv_rows_flushed"], "socket_writes_completed": report["mqtt"]["socket_writes_completed"], "coalesced": report["mqtt"]["coalesced"], "broker_connections_after_reset": reconnect["mqtt"]["broker_connections"], "handshake_queue_drops": delayed["mqtt"]["queue_dropped"], "api_received_during_handshake_stall": api_received, "slow_reader_write_failures": stalled["mqtt"]["publish_write_failures"], "slow_reader_queue_drops": stalled["mqtt"]["queue_dropped"], "independent_csv_overruns": snapshot_report["counts"]["logger_overruns"], "snapshot_source_loss": snapshot_report["source_update_loss"]}
    keys = ("host_poll_start_spacing", "host_receive_spacing", "host_new_update_spacing", "gateway_update_spacing", "logger_metrics")
    receipt["timing_observations"] = {name: {key: run.get(key) for key in keys} for name, run in [("mqtt_disabled_faults", baseline), ("mqtt_enabled_faults", report), ("mqtt_and_api_during_stalled_handshake", delayed), ("mqtt_slow_reader_with_api", stalled)]}
    (output / "receipt.json").write_bytes((json.dumps(receipt, indent=2) + "\n").encode("utf-8"))
    print(json.dumps({key: value for key, value in receipt.items() if key != "timing_observations"}, indent=2))
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=lambda value: Path(value).resolve())
    parser.add_argument("--samples", type=Path, default=ROOT / "samples")
    args = parser.parse_args()
    destination = ROOT / ".local" / ("mqtt-verify-" + uuid.uuid4().hex[:8])
    verify(args.cli, args.samples.resolve(), destination)
    print(f"Evidence directory: {destination}")
