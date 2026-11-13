"""Windows CLI acceptance using synthetic data and a loopback-only AL1350 fixture."""

import argparse
import csv
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import math
import statistics
import shutil
import os
from pathlib import Path
import subprocess
import threading
import time
import tomllib
import uuid
import zipfile
import zlib


ROOT = Path(__file__).resolve().parents[1]


class Gateway(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    polls = 0
    isdu_reads = 0
    requests = 0
    violations = []
    device_id = 1
    inject_faults = True
    block_parameter = False
    block_index = 64
    fail_detailed = False
    change_serial_after_status = False
    payload = "03C9"
    threshold_payload = "01F4"
    octet_payload = "0080FF2C"
    text_payload = "C2B56D0000000000"
    wide_payload = "FFFFFFFFFFFFFFFF"
    parameter_entered = threading.Event()
    parameter_release = threading.Event()
    port = "/iolinkmaster/port[1]"
    device = port + "/iolinkdevice"
    data = {
        "/deviceinfo/productcode": "AL1350",
        "/deviceinfo/swrevision": "3.1.42",
        "/deviceinfo/serialnumber": "FIXTURE-MASTER",
        port + "/mode": 3,
        port + "/mastercycletime_actual": 1200,
        port + "/portevent": 0,
        device + "/status": 0,
        device + "/iolinkevent": 0,
        device + "/vendorid": "65535",
        device + "/productname": "Fixture level sensor",
        device + "/serial": "FIXTURE-SENSOR",
    }

    def log_message(self, _format, *args):
        pass

    def do_POST(self):
        type(self).requests += 1
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        response = {"code": 200, "cid": request["cid"]}
        address = request["adr"]
        if self.path != "/" or request["code"] != "request" or "auth" in request:
            self.violations.append(request)
            response["code"] = 400
        elif address == "/getdatamulti":
            response["data"] = {}
            for path in request["data"]["datatosend"]:
                if path == self.device + "/pdin":
                    type(self).polls += 1
                    code = {4: 530, 7: 503}.get(self.polls, 200) if self.inject_faults else 200
                    result = {"code": code}
                    if code == 200:
                        result["data"] = self.payload
                elif path == self.device + "/deviceid":
                    result = {"code": 200, "data": self.device_id}
                elif path in self.data:
                    result = {"code": 200, "data": self.data[path]}
                else:
                    self.violations.append(path)
                    result = {"code": 400}
                # Documented examples use both styles of path key.
                response["data"][path if self.polls % 2 else path.lstrip("/")] = result
        elif address == self.device + "/iolreadacyclic" and request["data"] in (
                {"index": 19, "subindex": 0}, {"index": 36, "subindex": 0}, {"index": 37, "subindex": 0}, {"index": 48, "subindex": 0}, {"index": 64, "subindex": 0}, {"index": 65, "subindex": 0},
                *({"index": 66, "subindex": subindex} for subindex in range(5)),
                *({"index": 67, "subindex": subindex} for subindex in range(8)),
                {"index": 71, "subindex": 0}, {"index": 72, "subindex": 0},
                *({"index": index, "subindex": subindex} for index in (73, 74, 75) for subindex in range(3)),
                {"index": 76, "subindex": 0}, {"index": 77, "subindex": 0},
                *({"index": 78, "subindex": subindex} for subindex in range(4)),
                *({"index": index, "subindex": subindex} for index in (79, 80) for subindex in range(3)),
                *({"index": index, "subindex": 0} for index in (81, 82, 83)),
                *({"index": index, "subindex": subindex} for index in (84, 85) for subindex in range(4))):
            type(self).isdu_reads += 1
            index = request["data"]["index"]
            if index == self.block_index and self.block_parameter:
                self.parameter_entered.set()
                if not self.parameter_release.wait(timeout=3):
                    response["code"] = 500
            response["data"] = {"value": {
                (19, 0): "53494D2D53454E534F52", (48, 0): "2A", (64, 0): self.threshold_payload, (65, 0): "666978747572652073656E736F72",
                (36, 0): "02", (37, 0): "E48DFE000000",
                (66, 0): "FFEC010A54455354", (66, 1): "FFEC", (66, 2): "01", (66, 3): "0A", (66, 4): "54455354",
                (67, 0): "02C495F0", (67, 1): "02", (67, 2): "FC", (67, 3): "04", (67, 4): "F9", (67, 5): "05", (67, 6): "FF", (67, 7): "00",
                (71, 0): "9DFF440080000000", (72, 0): "FFFFFFFD40000000",
                (73, 0): "9DFF440080000000FFFFFFFD40000000", (73, 1): "9DFF440080000000", (73, 2): "FFFFFFFD40000000",
                (74, 0): "9DFF4400800000000000000000000000", (74, 1): "9DFF440080000000", (74, 2): "0000000000000000",
                (75, 0): "00000001800000008000000000000000", (75, 1): "0000000180000000", (75, 2): "8000000000000000",
                (76, 0): self.octet_payload, (77, 0): self.text_payload,
                (78, 0): "0080FF2CC2B56D00000000004F4B0000", (78, 1): "0080FF2C", (78, 2): "C2B56D0000000000", (78, 3): "4F4B0000",
                (79, 0): "0080FF2C10200000", (79, 1): "0080FF2C", (79, 2): "10200000",
                (80, 0): "C3A900004F4B0000", (80, 1): "C3A9", (80, 2): "4F4B0000",
                (81, 0): "01", (82, 0): self.wide_payload, (83, 0): "3FA00000",
                (84, 0): "01FFFB03", (84, 1): "01", (84, 2): "FFFB", (84, 3): "03",
                (85, 0): "0F23", (85, 1): "FF", (85, 2): "02", (85, 3): "03",
            }[(index, request["data"]["subindex"])]}
            if index == 37 and self.fail_detailed:
                response["code"] = 500
            if index == 36 and self.change_serial_after_status:
                self.data[self.device + "/serial"] = "CHANGED-SENSOR"
        else:
            self.violations.append(request)
            response["code"] = 400
        body = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run(cli, *args, expected=0):
    result = subprocess.run(
        [str(cli), *map(str, args)], cwd=ROOT, capture_output=True, text=True,
        encoding="utf-8", timeout=30, check=False,
        env={k: v for k, v in os.environ.items() if k.upper() != "IOLINK_IFM_PASSWORD"},
    )
    assert result.returncode == expected, (args, result.returncode, result.stdout, result.stderr)
    return result.stdout


def review_capture(cli, path, rows, report):
    before = {file: hashlib.sha256(file.read_bytes()).hexdigest() for file in path.iterdir() if file.is_file()}
    polls = Gateway.polls
    review = tomllib.loads(run(cli, "review-capture", path, "--buckets", 7))
    assert Gateway.polls == polls, "offline review contacted the gateway fixture"
    assert review["capture"]["rows_checked"] == len(rows)
    signal = review["signal"]
    field = signal["field"]
    header = "value." + field["name"] + (" [" + field["unit"] + "]" if field["unit"] else "")
    cells = [row[header] for row in rows]
    numbers = [float({"false": "0", "true": "1"}.get(cell, cell)) for cell in cells if cell]
    finite = [value for value in numbers if math.isfinite(value)]
    numeric = signal["numeric"]
    assert numeric["finite_values"] == len(finite)
    assert numeric["blank_values"] == cells.count("")
    assert numeric["non_finite_values"] == len(numbers) - len(finite)
    if finite:
        assert numeric["minimum"] == min(finite) and numeric["maximum"] == max(finite)
        assert math.isclose(numeric["mean"], statistics.fmean(finite), rel_tol=1e-12, abs_tol=1e-12)
    assert len(signal["buckets"]) == signal["buckets_requested"] == 7
    assert sum(bucket["rows"] for bucket in signal["buckets"]) == len(rows)
    assert sum(bucket["finite_values"] for bucket in signal["buckets"]) == len(finite)
    assert signal["recorded_logger_overruns"] == report["counts"]["logger_overruns"]
    assert review["capture"]["recorded_source_update_loss"] == report["source_update_loss"]
    for key in ("host_poll_start_spacing", "host_receive_spacing", "host_new_update_spacing", "gateway_update_spacing", "logger_metrics"):
        assert signal["recorded_timing"].get(key) == report.get(key)
    explicit = tomllib.loads(run(cli, "review-capture", path, "--field", field["name"], "--buckets", 1))
    assert explicit["signal"]["numeric"] == numeric
    assert not run(cli, "review-capture", path, "--field", "no such saved field", expected=1)
    assert not run(cli, "review-capture", path, "--buckets", 0, expected=2)
    assert {file: hashlib.sha256(file.read_bytes()).hexdigest() for file in before} == before


def redecode_capture(cli, path, rows, report):
    before = {file: hashlib.sha256(file.read_bytes()).hexdigest() for file in path.iterdir() if file.is_file()}
    width = tomllib.loads((path / "profile.toml").read_text(encoding="utf-8"))["input_bytes"]
    mapping = path.with_name(path.name + "-mapping.toml")
    mapping_text = f'''name = "Revised offline mapping"
input_bytes = {width}
[connection]
gateway = "ifm_al1350"
endpoint = "http://127.0.0.1:1"
ifm_mode = 3
ifm_firmware = "offline-only"
[expected_identity]
vendor_id = 1234
[logging]
fields = ["Flag", 'Revised, "µm"']
[[fields]]
name = 'Revised, "µm"'
bits = 16
kind = "signed"
byte_order = "little_endian"
scale = 0.25
offset = 1.0
decimals = 3
unit = "µm"
[[fields]]
name = "Flag"
bits = 1
kind = "bool"
byte_order = "little_endian"
'''
    mapping.write_text(mapping_text, encoding="utf-8", newline="\n")
    output = path.with_name(path.name + "-redecoded")
    polls = Gateway.polls
    receipt = tomllib.loads(run(cli, "redecode-capture", path, mapping, output))
    assert Gateway.polls == polls
    assert receipt == tomllib.loads((output / "receipt.toml").read_text(encoding="utf-8"))
    assert receipt["kind"] == "offline_redecode" and "not performed" in receipt["identity_verification"]
    local_check = tomllib.loads(run(cli, "verify-export", output))
    assert local_check["rows_checked"] == len(rows) and local_check["local_file_hashes_checked"] == 4
    assert not local_check["original_source_reverified"] and local_check["original_csv_hashes_checked"] == 0
    assert "original_source_directory" not in local_check
    full_check = tomllib.loads(run(cli, "verify-export", output, "--source", path))
    assert full_check["original_source_reverified"] and full_check["original_csv_hashes_checked"] == report["csv_parts"]
    assert full_check["source_capture"] == local_check["source_capture"]
    assert not run(cli, "verify-export", output, "--source", path.with_name(path.name + "-missing-source"), expected=1)
    assert Gateway.polls == polls
    assert receipt["source_capture"]["recorded_source_update_loss"] == report["source_update_loss"]
    assert receipt["source_capture"]["missing_sample_indexes"] == report["counts"]["logger_overruns"]
    with (output / "data.csv").open(encoding="utf-8", newline="") as file:
        reader = csv.DictReader(file)
        headers = reader.fieldnames
        derived = list(reader)
    assert len(derived) == receipt["rows_written"] == len(rows)
    assert headers[12:] == ["value.Flag", 'value.Revised, "µm" [µm]']
    eligible = 0
    for original, exported in zip(rows, derived):
        assert all(original[key] == exported[key] for key in headers[:12])
        if original["validity"] == "valid" and original["stale"] != "true":
            eligible += 1
            raw = bytes.fromhex(original["raw_hex"])
            assert exported["value.Flag"] == str(bool(raw[0] & 1)).lower()
            assert exported['value.Revised, "µm" [µm]'] == f"{int.from_bytes(raw[:2], 'little', signed=True) * 0.25 + 1:.3f}"
        else:
            assert not exported["value.Flag"] and not exported['value.Revised, "µm" [µm]']
    assert receipt["typed_rows"] == eligible and receipt["blank_rows"] == len(rows) - eligible
    for item in receipt["source_files"]:
        data = (path / item["name"]).read_bytes()
        assert item["bytes"] == len(data) and item["sha256"] == hashlib.sha256(data).hexdigest()
    for item in receipt["exported_files"]:
        data = (output / item["name"]).read_bytes()
        assert item["bytes"] == len(data) and item["sha256"] == hashlib.sha256(data).hexdigest()
    assert (output / "source-profile.toml").read_bytes() == (path / "profile.toml").read_bytes()
    assert (output / "source-capture.toml").read_bytes() == (path / "capture.toml").read_bytes()
    assert not (output / "capture.toml").exists()
    assert not run(cli, "redecode-capture", path, mapping, output, expected=1)
    wrong = mapping.with_name(mapping.stem + "-wrong.toml")
    wrong.write_text(mapping_text.replace(f"input_bytes = {width}", f"input_bytes = {width + 1}"), encoding="utf-8")
    wrong_output = output.with_name(output.name + "-wrong")
    assert not run(cli, "redecode-capture", path, wrong, wrong_output, expected=1)
    assert not wrong_output.exists()
    assert {file: hashlib.sha256(file.read_bytes()).hexdigest() for file in before} == before


def capture(cli, profile, path, *args):
    text = run(cli, "capture", profile, path, *args)
    report = tomllib.loads((path / "capture.toml").read_text(encoding="utf-8"))
    assert tomllib.loads(text) == report
    rows = []
    parts = sorted(path.glob("capture-*.csv"))
    assert len(parts) == report["csv_parts"]
    for part in parts:
        with part.open(encoding="utf-8", newline="") as csv_file:
            rows.extend(csv.DictReader(csv_file))
    assert len(rows) == report["csv_rows_flushed"]
    assert report["csv_rows_unconfirmed"] == 0
    assert report["counts"]["selected"] == len(rows) + report["counts"]["logger_overruns"]
    metrics = report["logger_metrics"]
    saved_profile = tomllib.loads((path / "profile.toml").read_text(encoding="utf-8"))
    assert metrics["queue_capacity"] == saved_profile["logging"]["queue_capacity"]
    assert 1 <= metrics["queue_high_water_observed"] <= metrics["queue_capacity"]
    assert metrics["rows_dequeued"] == len(rows)
    assert metrics["queue_wait"]["count"] == metrics["row_service"]["count"] == len(rows)
    assert metrics["flush"]["count"] >= metrics["sync"]["count"] == report["csv_parts"]
    for key in ("queue_wait", "row_service", "flush", "sync"):
        latency = metrics[key]
        assert 0 <= latency["p50_us"] <= latency["p95_us"] <= latency["p99_us"] <= latency["longest_us"]
        assert math.isfinite(latency["mean_us"]) and 0 <= latency["mean_us"] <= latency["longest_us"]
    if report["counts"]["logger_overruns"]:
        assert metrics["queue_high_water_observed"] == metrics["queue_capacity"]
    if report["simulated_writer_delay_ms"]:
        assert metrics["row_service"]["p50_us"] >= report["simulated_writer_delay_ms"] * 1000
    checked = tomllib.loads(run(cli, "verify-capture", path))
    assert checked["rows_checked"] == len(rows)
    assert checked["missing_sample_indexes"] == report["counts"]["logger_overruns"]
    assert checked.get("recorded_source_gaps") == report.get("source_gaps_observed")
    assert checked["recorded_source_update_loss"] == report["source_update_loss"]
    assert len({row["capture_utc_start"] for row in rows}) == 1
    for earlier, later in zip(rows, rows[1:]):
        assert int(earlier["sample_index"]) < int(later["sample_index"])
        assert int(earlier["host_elapsed_us"]) <= int(later["host_elapsed_us"])
    if any(key.startswith("value.") for key in rows[0]):
        review_capture(cli, path, rows, report)
    else:
        assert len(rows[0]) == 12
        assert not run(cli, "review-capture", path, expected=1)
    redecode_capture(cli, path, rows, report)
    return report, rows


def parallel_parameter_read(cli, fixture, output, diagnostics=False):
    Gateway.device_id = 1
    Gateway.polls = 0
    Gateway.inject_faults = False
    Gateway.block_parameter = True
    Gateway.block_index = 37 if diagnostics else 64
    Gateway.parameter_entered.clear()
    Gateway.parameter_release.clear()
    processes = []
    environment = {key: value for key, value in os.environ.items() if key.upper() != "IOLINK_IFM_PASSWORD"}
    try:
        sampling = subprocess.Popen([cli, "capture", fixture, output, "--seconds", "3"],
                                    cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8", env=environment)
        processes.append(sampling)
        deadline = time.monotonic() + 2
        while Gateway.polls < 5 and time.monotonic() < deadline:
            assert sampling.poll() is None, "capture exited before the parallel read"
            time.sleep(0.01)
        assert Gateway.polls >= 5
        snapshot_path = output.with_suffix(".json")
        reading = subprocess.Popen([cli, "read-diagnostics", fixture] if diagnostics else [cli, "read-parameter", fixture, "V_SIM_Threshold", "--snapshot", snapshot_path],
                                   cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8", env=environment)
        processes.append(reading)
        assert Gateway.parameter_entered.wait(timeout=1)
        before = Gateway.polls
        deadline = time.monotonic() + 1
        while Gateway.polls < before + 5 and time.monotonic() < deadline:
            time.sleep(0.01)
        assert Gateway.polls >= before + 5, "cyclic polling stopped during the held ISDU read"
        assert reading.poll() is None, "ISDU read completed before fixture release"
        Gateway.parameter_release.set()
        stdout, stderr = reading.communicate(timeout=5)
        assert reading.returncode == 0, (stdout, stderr)
        if diagnostics:
            assert tomllib.loads(stdout)["detailed_status"]["events"][0]["code"] == 36350
        else:
            assert tomllib.loads(stdout)["raw_value"] == "500"
            assert json.loads(snapshot_path.read_text(encoding="utf-8"))["raw_hex"] == "01F4"
        stdout, stderr = sampling.communicate(timeout=5)
        assert sampling.returncode == 0, (stdout, stderr)
        report = tomllib.loads(stdout)
        checked = tomllib.loads(run(cli, "verify-capture", output))
        assert checked["rows_checked"] == report["csv_rows_flushed"] > 10
        assert report["counts"]["logger_overruns"] == 0
    finally:
        Gateway.parameter_release.set()
        Gateway.block_parameter = False
        Gateway.block_index = 64
        for process in processes:
            if process.poll() is None:
                process.kill()
            process.communicate(timeout=5)


def verify_display_preferences(cli, source, output):
    output.mkdir()
    original = source.read_bytes()
    preferred = output / "preferred.toml"
    before_reads = Gateway.isdu_reads
    before_requests = Gateway.requests
    run(cli, "select-display", source, "V_SIM_Threshold", preferred, "--display", "M_SIM_Threshold/2")
    run(cli, "select-display", preferred, "V_SIM_Record", preferred, "--subindex", 1, "--display", "M_SIM_Record/2")
    run(cli, "select-display", preferred, "V_SIM_Record", preferred, "--subindex", 2, "--display", "M_SIM_Record/4")
    run(cli, "select-display", preferred, "V_SIM_History", preferred, "--subindex", 2, "--display", "M_SIM_History/2")
    assert Gateway.isdu_reads == before_reads and Gateway.requests == before_requests and source.read_bytes() == original
    for variable, payload, subindex, display_id, value, raw in [
            ("V_SIM_Threshold", "01F4", 0, "M_SIM_Threshold/2", "01F4h", "500"),
            ("V_SIM_Record", "FFEC", 1, "M_SIM_Record/2", "FFECh", "-20"),
            ("V_SIM_Record", "01", 2, "M_SIM_Record/4", "1", "true"),
            ("V_SIM_History", "FC", 2, "M_SIM_History/2", "1111 1100b", "-4")]:
        live = tomllib.loads(run(cli, "read-parameter", preferred, variable, "--subindex", subindex))
        before_requests = Gateway.requests
        offline = tomllib.loads(run(cli, "decode-parameter", preferred, variable, payload, "--subindex", subindex))
        assert Gateway.requests == before_requests
        assert live["preferred_display"] == offline["preferred_display"] == {"id": display_id, "value": value, "unit": ""}
        assert live["raw_value"] == offline["raw_value"] == raw
        assert live["displays"] == offline["displays"]
    record = tomllib.loads(run(cli, "read-parameter", preferred, "V_SIM_Record"))
    assert record["record_items"][0]["value"]["preferred_display"]["value"] == "FFECh"
    array = tomllib.loads(run(cli, "read-parameter", preferred, "V_SIM_History"))
    assert all(item["value"]["preferred_display"]["id"] == "M_SIM_History/2" for item in array["array_items"])
    assert all(len(item["value"]["displays"]) == 2 for item in array["array_items"])
    bad = tomllib.loads(run(cli, "decode-parameter", preferred, "V_SIM_Threshold", "FF", expected=1))
    assert bad["raw_hex"] == "FF" and bad["decode_error"] and "preferred_display" not in bad
    before_reads = Gateway.isdu_reads
    before_requests = Gateway.requests
    saved = preferred.read_bytes()
    invalid = output / "invalid.toml"
    run(cli, "select-display", preferred, "V_SIM_Threshold", invalid, "--display", "M_SIM_Record/2", expected=1)
    run(cli, "select-display", preferred, "V_SIM_Threshold", preferred, "--display", "missing", expected=1)
    assert preferred.read_bytes() == saved
    run(cli, "select-display", preferred, "V_SIM_Record", invalid, "--subindex", 8, "--clear", expected=1)
    run(cli, "select-display", preferred, "V_SIM_Threshold", invalid, expected=2)
    run(cli, "select-display", preferred, "V_SIM_Threshold", invalid, "--clear", "--display", "M_SIM_Threshold/1", expected=2)
    assert not invalid.exists() and Gateway.isdu_reads == before_reads and Gateway.requests == before_requests
    stale = output / "stale-id.toml"
    stale.write_text(preferred.read_text(encoding="utf-8").replace('preferred_display = "M_SIM_Threshold/2"', 'preferred_display = "missing"'), encoding="utf-8")
    run(cli, "read-parameter", stale, "V_SIM_Threshold", expected=1)
    assert Gateway.isdu_reads == before_reads and Gateway.requests == before_requests
    cleared = output / "cleared.toml"
    run(cli, "select-display", preferred, "V_SIM_Threshold", cleared, "--clear")
    decoded = tomllib.loads(run(cli, "decode-parameter", cleared, "V_SIM_Threshold", "01F4"))
    assert "preferred_display" not in decoded and len(decoded["displays"]) == 3
    assert source.read_bytes() == original
    return preferred


def verify_parameter_snapshots(cli, profile, output, loopback=False):
    output.mkdir()
    original = profile.read_bytes()
    preferred = output / "preferred.toml"
    run(cli, "select-display", profile, "V_SIM_Threshold", preferred, "--display", "M_SIM_Threshold/2")
    for variable, subindex in [("V_SIM_Threshold", 0), ("V_SIM_Record", 0), ("V_SIM_Record", 1), ("V_SIM_History", 0), ("V_SIM_History", 2)]:
        path = output / f"{variable}-{subindex}.json"
        before_reads = Gateway.isdu_reads
        reading = tomllib.loads(run(cli, "read-parameter", preferred, variable, "--subindex", subindex, "--snapshot", path))
        assert not loopback or Gateway.isdu_reads == before_reads + 1
        saved = json.loads(path.read_text(encoding="utf-8"))
        assert saved["raw_hex"] == reading["raw_hex"] and saved["subindex"] == subindex
        assert saved["host_read_utc"] == reading["host_read_utc"] and saved["read_duration_us"] == reading["read_duration_us"]
        assert saved["observation"]["route"] == ("AL1350 loopback fixture" if loopback else "synthetic simulator")
        assert saved["observation"]["identity"]["device_id"] == 1
        pinned = path.read_bytes()
        before_requests = Gateway.requests
        review = tomllib.loads(run(cli, "review-parameter", path))
        assert Gateway.requests == before_requests and path.read_bytes() == pinned
        for key in ("raw_hex", "raw_value", "displays", "preferred_display", "record_items", "array_items"):
            assert review["decoded"].get(key) == reading.get(key)
        run(cli, "read-parameter", preferred, variable, "--subindex", subindex, "--snapshot", path, expected=1)
        assert Gateway.requests == before_requests and path.read_bytes() == pinned
    # Reopening never resolves saved paths or endpoints. A decoder error is distinct from corrupt context.
    document = json.loads((output / "V_SIM_Threshold-0.json").read_text(encoding="utf-8"))
    document["profile"]["connection"]["endpoint"] = "http://127.0.0.1:1"
    document["profile"]["iodd"]["source"] = "Z:/missing/vendor.zip"
    document["raw_hex"] = "FF"
    short = output / "short.json"
    short.write_text(json.dumps(document), encoding="utf-8")
    before_requests = Gateway.requests
    review = tomllib.loads(run(cli, "review-parameter", short, expected=1))
    assert review["decoded"]["raw_hex"] == "FF" and review["decoded"]["decode_error"]
    assert Gateway.requests == before_requests
    document["index"] = 19
    bad = output / "wrong-query.json"
    bad.write_text(json.dumps(document), encoding="utf-8")
    run(cli, "review-parameter", bad, expected=1)
    assert Gateway.requests == before_requests
    denied = output / "write-only.toml"
    denied.write_text(preferred.read_text(encoding="utf-8").replace('access = "rw"', 'access = "wo"', 1), encoding="utf-8")
    destination = output / "denied.json"
    run(cli, "read-parameter", denied, "V_SIM_Threshold", "--snapshot", destination, expected=1)
    assert Gateway.requests == before_requests and not destination.exists()
    if loopback:
        Gateway.threshold_payload = "FF"
        try:
            response = tomllib.loads(run(cli, "read-parameter", preferred, "V_SIM_Threshold", "--snapshot", output / "malformed-response.json"))
            assert response["raw_hex"] == "FF" and response["decode_error"]
            run(cli, "review-parameter", output / "malformed-response.json", expected=1)
        finally:
            Gateway.threshold_payload = "01F4"
        Gateway.device_id = 99
        before_reads = Gateway.isdu_reads
        try:
            rejected = output / "identity-rejected.json"
            run(cli, "read-parameter", preferred, "V_SIM_Threshold", "--snapshot", rejected, expected=1)
            assert not rejected.exists() and Gateway.isdu_reads == before_reads
        finally:
            Gateway.device_id = 1
    assert profile.read_bytes() == original


def verify_standard_languages(cli, source, output):
    output.mkdir()
    definitions = ROOT / "samples/synthetic-standard-definitions.xml"
    profile = output / "german.toml"
    options = ["--variant", "SIM-SENSOR", "--language", "de", "--standard-definitions", definitions]
    run(cli, "import", source, profile, *options)
    metadata = tomllib.loads(profile.read_text(encoding="utf-8"))["iodd"]["localization"]
    standard = metadata["standard_definitions"]
    assert standard["selected_language"] == standard["requested_language"] == "de"
    assert standard["language_source"] == "synthetic-standard-definitions-de.xml" and "offset" in standard["fallback_text_ids"]
    reading = tomllib.loads(run(cli, "read-parameter", profile, "V_ProductID", "--snapshot", output / "product.json"))
    assert reading["name"] == "Synthetische Produktkennung" and reading["raw_value"] == "SIM-SENSOR"
    assert tomllib.loads(run(cli, "review-parameter", output / "product.json"))["name"] == reading["name"]
    assert any(event["name"] == "Synthetischer Standardalarm" for event in tomllib.loads(run(cli, "events", profile))["events"])
    explicit = output / "custom.xml"
    explicit.write_bytes((ROOT / "samples/synthetic-standard-definitions-de.xml").read_bytes())
    override = output / "override.toml"
    run(cli, "import", source, override, *options, "--standard-language-file", explicit)
    assert tomllib.loads(override.read_text(encoding="utf-8"))["iodd"]["localization"]["standard_definitions"]["language_source"] == "custom.xml"
    before = profile.read_bytes()
    explicit.write_text(explicit.read_text(encoding="utf-8").replace('id="product"', 'id="missing"'), encoding="utf-8")
    run(cli, "import", source, profile, *options, "--standard-language-file", explicit, expected=1)
    assert profile.read_bytes() == before
    run(cli, "import", source, profile, "--standard-language-file", explicit, expected=2)
    assert profile.read_bytes() == before
    missing = output / "without-translation.xml"
    missing.write_bytes(definitions.read_bytes())
    fallback = output / "fallback.toml"
    run(cli, "import", source, fallback, "--variant", "SIM-SENSOR", "--language", "de", "--standard-definitions", missing)
    locale = tomllib.loads(fallback.read_text(encoding="utf-8"))["iodd"]["localization"]
    assert locale["selected_language"] == "de" and locale["standard_definitions"]["selected_language"] == "en"
    assert len(locale["standard_definitions"]["fallback_text_ids"]) == 8
    assert any("Standard definitions:" in note and "unavailable" in note for note in locale["review"])
    capture(cli, profile, output / "capture", "--seconds", 0.25)
    return profile


def verify_time_parameters(cli, profile, output):
    output.mkdir()
    for variable, subindex, payload, value in [
        ("V_SIM_Time", 0, "9DFF440080000000", "1984-01-01 00:00:00.500 UTC"),
        ("V_SIM_TimeSpan", 0, "FFFFFFFD40000000", "-00:00:02.750"),
        ("V_SIM_TimeRecord", 1, "9DFF440080000000", "1984-01-01 00:00:00.500 UTC"),
        ("V_SIM_TimeRecord", 2, "FFFFFFFD40000000", "-00:00:02.750"),
        ("V_SIM_Times", 2, "0000000000000000", "2036-02-07 06:28:16.000 UTC"),
        ("V_SIM_TimeSpans", 2, "8000000000000000", "-24855 03:14:08.000")]:
        path = output / f"{variable}-{subindex}.json"
        reading = tomllib.loads(run(cli, "read-parameter", profile, variable, "--subindex", subindex, "--snapshot", path))
        before = Gateway.requests
        offline = tomllib.loads(run(cli, "decode-parameter", profile, variable, payload, "--subindex", subindex))
        restored = tomllib.loads(run(cli, "review-parameter", path))
        assert Gateway.requests == before
        assert reading["raw_hex"] == offline["raw_hex"] == restored["decoded"]["raw_hex"] == payload
        assert reading["time_value"] == offline["time_value"] == restored["decoded"]["time_value"]
        assert reading["time_value"]["value"] == value
        assert reading["raw_value"] == reading["time_value"]["raw_ticks"]
        assert reading["time_value"]["bits"] == 64 and reading["time_value"]["byte_order"] == "big_endian"
        assert restored["decoder_version"] == run(cli, "--version").strip().split()[-1]
        default = tomllib.loads(run(cli, "defaults", profile, variable, "--subindex", subindex))["entries"][0]["value"]
        assert restored["declared_default"] == default and default["status"] == "decoded"
        if variable == "V_SIM_TimeRecord":
            expected_default = "1984-01-01 00:00:00.001 UTC" if subindex == 1 else "-00:00:00.001"
            assert default["decoded"]["time_value"]["value"] == expected_default
            assert not default["temporal_conversion"]["exact"]
    for variable, key in [("V_SIM_TimeRecord", "record_items"), ("V_SIM_Times", "array_items"), ("V_SIM_TimeSpans", "array_items")]:
        whole = tomllib.loads(run(cli, "read-parameter", profile, variable))
        offline = tomllib.loads(run(cli, "decode-parameter", profile, variable, whole["raw_hex"]))
        assert len(whole[key]) == 2 and whole[key] == offline[key]
        assert all("time_value" in item["value"] for item in whole[key])
    short = tomllib.loads(run(cli, "decode-parameter", profile, "V_SIM_Time", "FF" * 7, expected=1))
    assert short["raw_hex"] == "FF" * 7 and short["decode_error"] and "time_value" not in short
    negative_tick = tomllib.loads(run(cli, "decode-parameter", profile, "V_SIM_TimeSpan", "FFFFFFFFFFFFFFFF"))
    assert negative_tick["time_value"]["sub_millisecond_truncated"] and negative_tick["raw_value"] == "-1"
    maximum = tomllib.loads(run(cli, "decode-parameter", profile, "V_SIM_Time", "FFFFFFFFFFFFFFFF"))
    assert maximum["raw_value"] == "18446744073709551615" and maximum["time_value"]["sub_millisecond_truncated"]


def verify_string_parameters(cli, profile, output, loopback=False):
    output.mkdir()
    for variable, subindex, payload, value in [
        ("V_SIM_Octets", 0, "0080FF2C", "0x00,0x80,0xFF,0x2C"),
        ("V_SIM_Text", 0, "C2B56D0000000000", "µm"),
        ("V_SIM_BlobRecord", 1, "0080FF2C", "0x00,0x80,0xFF,0x2C"),
        ("V_SIM_BlobRecord", 2, "C2B56D0000000000", "µm"),
        ("V_SIM_BlobRecord", 3, "4F4B0000", "OK"),
        ("V_SIM_BlobArray", 2, "10200000", "0x10,0x20,0x00,0x00"),
        ("V_SIM_TextArray", 1, "C3A9", "é")]:
        snapshot = output / f"{variable}-{subindex}.json"
        reading = tomllib.loads(run(cli, "read-parameter", profile, variable, "--subindex", subindex, "--snapshot", snapshot))
        before = Gateway.requests
        offline = tomllib.loads(run(cli, "decode-parameter", profile, variable, payload, "--subindex", subindex))
        restored = tomllib.loads(run(cli, "review-parameter", snapshot))
        assert Gateway.requests == before
        assert reading["raw_value"] == offline["raw_value"] == restored["decoded"]["raw_value"] == value
        assert reading["raw_hex"] == offline["raw_hex"] == payload
        assert "decode_error" not in reading
    for variable, key in [("V_SIM_BlobRecord", "record_items"), ("V_SIM_BlobArray", "array_items"), ("V_SIM_TextArray", "array_items")]:
        whole = tomllib.loads(run(cli, "read-parameter", profile, variable))
        offline = tomllib.loads(run(cli, "decode-parameter", profile, variable, whole["raw_hex"]))
        assert whole[key] == offline[key] and all("raw_value" in item["value"] for item in whole[key])
    for variable, payload in [("V_SIM_Text", "410042"), ("V_SIM_Text", "C2"), ("V_SIM_Octets", "0080FF")]:
        bad = tomllib.loads(run(cli, "decode-parameter", profile, variable, payload, expected=1))
        assert bad["raw_hex"] == payload and bad["decode_error"] and "raw_value" not in bad
    bad_member = tomllib.loads(run(cli, "decode-parameter", profile, "V_SIM_BlobRecord", "0080FF2C41004200000000004F4B0000", expected=1))
    assert bad_member["record_items"][0]["value"]["raw_value"] == "0x00,0x80,0xFF,0x2C"
    assert "padding" in bad_member["record_items"][1]["value"]["decode_error"]
    assert bad_member["record_items"][2]["value"]["raw_value"] == "OK"
    if loopback:
        Gateway.octet_payload = "0080FF"
        Gateway.text_payload = "4100420000000000"
        try:
            for variable in ("V_SIM_Octets", "V_SIM_Text"):
                path = output / f"malformed-{variable}.json"
                reading = tomllib.loads(run(cli, "read-parameter", profile, variable, "--snapshot", path))
                assert reading["decode_error"] and "raw_value" not in reading
                before = Gateway.requests
                restored = tomllib.loads(run(cli, "review-parameter", path, expected=1))
                assert restored["decoded"]["raw_hex"] == reading["raw_hex"] and Gateway.requests == before
        finally:
            Gateway.octet_payload = "0080FF2C"
            Gateway.text_payload = "C2B56D0000000000"


def verify_value_definitions(cli, profile, output, loopback=False):
    output.mkdir()
    for variable, subindex, raw, value, choice in [
        ("V_SIM_Enabled", 0, "01", "true", "Enabled"),
        ("V_SIM_Wide", 0, "FFFFFFFFFFFFFFFF", "18446744073709551615", "Maximum unsigned value"),
        ("V_SIM_Float", 0, "3FA00000", "1.25", "Nominal float"),
        ("V_SIM_ValueRecord", 1, "01", "true", "Enabled"),
        ("V_SIM_ValueRecord", 2, "FFFB", "-5", "Negative offset"),
        ("V_SIM_ValueRecord", 3, "03", "3", None),
        ("V_SIM_ValueArray", 1, "FF", "-1", "Minus one"),
        ("V_SIM_ValueArray", 2, "02", "2", None)]:
        path = output / f"{variable}-{subindex}.json"
        reading = tomllib.loads(run(cli, "read-parameter", profile, variable, "--subindex", subindex, "--snapshot", path))
        before = Gateway.requests
        restored = tomllib.loads(run(cli, "review-parameter", path))["decoded"]
        offline = tomllib.loads(run(cli, "decode-parameter", profile, variable, raw, "--subindex", subindex))
        assert Gateway.requests == before
        for result in (reading, restored, offline):
            assert result["raw_value"] == value and result["raw_hex"] == raw
            assert result.get("choice_name") == choice
            assert result["value_assessment"]["status"] == "allowed" and not result["value_assessment"]["notes"]
        assert reading["value_assessment"] == restored["value_assessment"] == offline["value_assessment"]
    for variable, key in [("V_SIM_ValueRecord", "record_items"), ("V_SIM_ValueArray", "array_items")]:
        whole = tomllib.loads(run(cli, "read-parameter", profile, variable))
        offline = tomllib.loads(run(cli, "decode-parameter", profile, variable, whole["raw_hex"]))
        assert whole[key] == offline[key]
        assert all(item["value"]["value_assessment"]["status"] == "allowed" for item in whole[key])
    for variable, raw, status in [
        ("V_SIM_Wide", "0020000000000001", "allowed"),
        ("V_SIM_Wide", "0020000000000002", "outside_definition"),
        ("V_SIM_Float", "3FC00000", "outside_definition"),
        ("V_SIM_Float", "7FC00000", "not_comparable"),
        ("V_SIM_Float", "7F800000", "allowed")]:
        result = tomllib.loads(run(cli, "decode-parameter", profile, variable, raw))
        assert result["raw_hex"] == raw and result["value_assessment"]["status"] == status
        assert "decode_error" not in result
    if loopback:
        Gateway.wide_payload = "0020000000000002"
        try:
            path = output / "outside-definition.json"
            result = tomllib.loads(run(cli, "read-parameter", profile, "V_SIM_Wide", "--snapshot", path))
            before = Gateway.requests
            restored = tomllib.loads(run(cli, "review-parameter", path))["decoded"]
            assert Gateway.requests == before
            assert result["value_assessment"] == restored["value_assessment"]
            assert restored["value_assessment"]["status"] == "outside_definition"
        finally:
            Gateway.wide_payload = "FFFFFFFFFFFFFFFF"


def verify_defaults(cli, output):
    output.mkdir()
    profile = output / "defaults.toml"
    run(cli, "import", ROOT / "samples/sensor-defaults.xml", profile)
    saved = profile.read_bytes()
    requests = Gateway.requests
    report = tomllib.loads(run(cli, "defaults", profile, expected=1))
    assert (report["decoded"], report["not_declared"], report["needs_review"], report["unsupported"]) == (12, 2, 2, 1)
    entries = {(entry["parameter_id"], entry["subindex"]): entry for entry in report["entries"]}
    for variable, raw, value in [
        ("V_DEFAULT_Bool", "01", "true"), ("V_DEFAULT_Wide", "FFFFFFFFFFFFFFFF", "18446744073709551615"),
        ("V_DEFAULT_Signed", "FFEF", "-17"), ("V_DEFAULT_Float", "3FA00000", "1.25"),
        ("V_DEFAULT_Text", "20C2B56D20000000", " µm "), ("V_DEFAULT_Octets", "0080FF2C", "0x00,0x80,0xFF,0x2C")]:
        parsed = entries[(variable, 0)]["value"]
        assert parsed["status"] == "decoded" and parsed["representation_hex"] == raw and parsed["decoded"]["raw_value"] == value
        selected = tomllib.loads(run(cli, "defaults", profile, variable))["entries"][0]["value"]
        assert selected == parsed
    assert entries[("V_DEFAULT_Float", 0)]["value"]["decoded"]["displays"][0]["value"] == "12.5"
    array = entries[("V_DEFAULT_Array", 0)]["value"]
    assert array["array_elements"] == 3 and array["representation_hex"] == "FF"
    member = tomllib.loads(run(cli, "defaults", profile, "V_DEFAULT_Array", "--subindex", 2))["entries"][0]
    assert member["bit_offset"] == 4 and "array_elements" not in member["value"]
    record = tomllib.loads(run(cli, "defaults", profile, "V_DEFAULT_Record"))
    assert len(record["entries"]) == 4 and record["decoded"] == 3 and record["not_declared"] == 1
    assert entries[("V_DEFAULT_Record", 2)]["value"]["representation_hex"] == "002A"
    assert entries[("V_DEFAULT_Record", 3)]["value"]["representation_hex"] == "4F4B0000"
    invalid = entries[("V_DEFAULT_Invalid", 0)]["value"]
    assert invalid["declared"] == "256" and "representation_hex" not in invalid
    outside = entries[("V_DEFAULT_Outside", 0)]["value"]
    assert outside["decoded"]["value_assessment"]["status"] == "outside_definition"
    assert entries[("V_DEFAULT_Time", 0)]["value"]["representation_hex"] == "FFFFFFFE00000000"
    assert entries[("V_DEFAULT_NaN", 0)]["value"]["status"] == "unsupported"
    command = tomllib.loads(run(cli, "defaults", profile, "V_DEFAULT_Command"))
    assert command["entries"][0]["access"] == "wo" and command["entries"][0]["value"]["representation_hex"] == "80"
    run(cli, "read-parameter", profile, "V_DEFAULT_Command", expected=1)
    run(cli, "defaults", profile, "V_DEFAULT_Array", "--subindex", 4, expected=1)
    run(cli, "defaults", profile, "missing", expected=1)
    run(cli, "defaults", profile, "--subindex", 1, expected=2)
    offline = output / "unreachable.toml"
    offline.write_bytes(saved.replace(b'gateway = "simulator"', b'gateway = "ifm_al1350"').replace(b'endpoint = ""', b'endpoint = "http://127.0.0.1:9"'))
    assert tomllib.loads(run(cli, "defaults", offline, expected=1)) == report
    assert profile.read_bytes() == saved and Gateway.requests == requests
    # Default issues do not gate the independently configured cyclic pipeline.
    capture(cli, profile, output / "capture", "--seconds", .25)
    snapshot_profile = output / "snapshot-profile.toml"
    run(cli, "import", ROOT / "samples/sensor-standard-values.xml", snapshot_profile,
        "--standard-definitions", ROOT / "samples/synthetic-value-standard-definitions.xml")
    snapshot = output / "parameter.json"
    run(cli, "read-parameter", snapshot_profile, "V_OffsetTime", "--snapshot", snapshot)
    original = snapshot.read_bytes()
    expected = tomllib.loads(run(cli, "defaults", snapshot_profile, "V_OffsetTime"))["entries"][0]["value"]
    requests = Gateway.requests
    restored = tomllib.loads(run(cli, "review-parameter", snapshot))
    assert restored["declared_default"] == expected and restored["decoded"]["raw_value"] == "42"
    assert snapshot.read_bytes() == original and Gateway.requests == requests
    distinct = output / "different-default.toml"
    saved = snapshot_profile.read_bytes()
    assert saved.count(b'default_value = "42"') == 1
    distinct.write_bytes(saved.replace(b'default_value = "42"', b'default_value = "41"'))
    distinct_snapshot = output / "different-default.json"
    run(cli, "read-parameter", distinct, "V_OffsetTime", "--snapshot", distinct_snapshot)
    different = tomllib.loads(run(cli, "review-parameter", distinct_snapshot))
    assert different["decoded"]["raw_value"] == "42"
    assert different["declared_default"]["decoded"]["raw_value"] == "41"


def verify_connection_settings(cli, output):
    output.mkdir()
    imported = output / "imported.toml"
    source = output / "source.toml"
    run(cli, "import", ROOT / "samples/sensor-formats.xml", imported)
    run(cli, "select-display", imported, "V_SIM_Threshold", source, "--display", "M_SIM_Threshold/2")
    source.write_bytes(b'# Original profile comment remains untouched\n' + source.read_bytes())
    original = source.read_bytes()
    before = tomllib.loads(original.decode("utf-8"))
    requests = Gateway.requests
    edited = output / "edited.toml"
    run(cli, "set-connection", source, edited, "--gateway", "ifm-al1350", "--endpoint", " http://127.0.0.1:9 ",
        "--port", 4, "--firmware", " 3.1.42 ", "--expected-mode", 3, "--cycle-unit", "milliseconds",
        "--poll-us", 20000, "--timeout-ms", 2000, "--stale-us", 150000, "--reconnect-ms", 500, "--simulator-snapshots", "true")
    result = tomllib.loads(edited.read_text(encoding="utf-8"))
    expected_connection = dict(before["connection"], gateway="ifm_al1350", endpoint="http://127.0.0.1:9", port=4,
        ifm_firmware="3.1.42", ifm_mode=3, ifm_cycle_unit="milliseconds", poll_us=20000, timeout_ms=2000,
        stale_us=150000, reconnect_ms=500, simulate_snapshots=True)
    assert result == dict(before, connection=expected_connection)
    assert Gateway.requests == requests and source.read_bytes() == original
    saved = edited.read_bytes()
    run(cli, "set-connection", source, edited, "--poll-us", 40000, expected=1)
    assert edited.read_bytes() == saved
    run(cli, "set-connection", source, source, "--poll-us", 40000, expected=1)
    assert source.read_bytes() == original
    invalid = output / "must-not-exist.toml"
    for args in [
        ["--gateway", "ifm-al1350", "--endpoint", "http://localhost/path"],
        ["--gateway", "ifm-al1350", "--endpoint", "https://user:secret123@localhost"],
        ["--port", 0], ["--port", 5], ["--poll-us", 99], ["--poll-us", 60000001],
        ["--timeout-ms", 0], ["--timeout-ms", 30001], ["--reconnect-ms", 0], ["--reconnect-ms", 60001]]:
        run(cli, "set-connection", source, invalid, *args, expected=1)
        assert not invalid.exists() and source.read_bytes() == original
    for args in [[], ["--expected-mode", 3, "--clear-expected-mode"], ["--gateway", "unknown"], ["--cycle-unit", "guess"], ["--port", 256]]:
        run(cli, "set-connection", source, invalid, *args, expected=2)
        assert not invalid.exists()
    unfinished = output / "unfinished.toml"
    run(cli, "set-connection", ROOT / "samples/ifm-al1350.toml", unfinished, "--poll-us", 1000)
    assert tomllib.loads(unfinished.read_text(encoding="utf-8"))["connection"]["endpoint"] == ""
    restored = output / "simulator.toml"
    run(cli, "set-connection", edited, restored, "--gateway", "simulator", "--port", 1, "--poll-us", 1000,
        "--firmware", "", "--clear-expected-mode", "--cycle-unit", "unknown", "--simulator-snapshots", "false")
    sim = tomllib.loads(restored.read_text(encoding="utf-8"))
    assert sim["connection"]["endpoint"] == "http://127.0.0.1:9"
    assert "ifm_mode" not in sim["connection"] and "ifm_cycle_unit" not in sim["connection"] and sim["connection"]["ifm_firmware"] == ""
    assert dict(sim, connection=before["connection"]) == before
    assert run(cli, "decode", restored, "FFF9") == run(cli, "decode", source, "FFF9")
    snapshot = output / "parameter.json"
    reading = tomllib.loads(run(cli, "read-parameter", restored, "V_SIM_Threshold", "--snapshot", snapshot))
    review = tomllib.loads(run(cli, "review-parameter", snapshot))
    assert reading["raw_value"] == review["decoded"]["raw_value"] == "500"
    assert reading["preferred_display"] == review["decoded"]["preferred_display"]
    assert Gateway.requests == requests and edited.read_bytes() == saved and source.read_bytes() == original
    capture(cli, restored, output / "capture", "--seconds", .25)


def verify_identity_settings(cli, output):
    output.mkdir()
    imported = output / "imported.toml"
    source = output / "source.toml"
    run(cli, "import", ROOT / "samples/sensor-formats.xml", imported)
    run(cli, "select-display", imported, "V_SIM_Threshold", source, "--display", "M_SIM_Threshold/2")
    source.write_bytes(b'# Keep original identity profile bytes\n' + source.read_bytes())
    original = source.read_bytes()
    before = tomllib.loads(original.decode("utf-8"))
    requests = Gateway.requests
    pinned = output / "pinned.toml"
    run(cli, "set-identity", source, pinned, "--vendor-id", "0xFFFF", "--device-id", "0X000001",
        "--product-id", "SIM-SENSOR", "--product-name", "Simulated level sensor", "--serial", "SIM-0001")
    after = tomllib.loads(pinned.read_text(encoding="utf-8"))
    expected = dict(vendor_id=65535, device_id=1, product_id="SIM-SENSOR", product_name="Simulated level sensor", serial="SIM-0001")
    assert after == dict(before, expected_identity=expected)
    assert source.read_bytes() == original and Gateway.requests == requests
    saved = pinned.read_bytes()
    run(cli, "set-identity", source, pinned, "--serial", "other", expected=1)
    assert pinned.read_bytes() == saved
    run(cli, "set-identity", source, source, "--serial", "other", expected=1)
    assert source.read_bytes() == original
    invalid = output / "must-not-exist.toml"
    for args in [["--vendor-id", "65536"], ["--device-id", "0x1000000"], ["--vendor-id", ""],
                 ["--device-id=-1"], ["--device-id", "0xGG"], ["--vendor-id", "18446744073709551616"]]:
        run(cli, "set-identity", source, invalid, *args, expected=1)
        assert not invalid.exists()
    for args in [[], ["--vendor-id", 1, "--clear-vendor-id"], ["--serial", "x", "--clear-serial"]]:
        run(cli, "set-identity", source, invalid, *args, expected=2)
        assert not invalid.exists()
    subset = output / "subset.toml"
    run(cli, "set-identity", pinned, subset, "--clear-all", "--vendor-id", "0xFFFF")
    assert tomllib.loads(subset.read_text(encoding="utf-8")) == dict(after, expected_identity=dict(vendor_id=65535))
    literals = output / "literals.toml"
    run(cli, "set-identity", pinned, literals, "--product-name", "  µm sensor  ", "--serial", "")
    literal_identity = tomllib.loads(literals.read_text(encoding="utf-8"))["expected_identity"]
    assert literal_identity["product_name"] == "  µm sensor  " and literal_identity["serial"] == ""
    cleared = output / "cleared.toml"
    run(cli, "set-identity", literals, cleared, "--clear-vendor-id", "--clear-device-id", "--clear-product-id", "--clear-product-name", "--clear-serial")
    assert tomllib.loads(cleared.read_text(encoding="utf-8")) == dict(after, expected_identity={})
    conflict = output / "variant-conflict.toml"
    text = run(cli, "set-identity", pinned, conflict, "--product-id", "OTHER")
    assert "Review:" in text and tomllib.loads(conflict.read_text(encoding="utf-8"))["iodd"] == after["iodd"]
    route = output / "unreachable-route.toml"
    run(cli, "set-connection", source, route, "--gateway", "ifm-al1350", "--endpoint", "http://127.0.0.1:9")
    offline = output / "unreachable-pinned.toml"
    run(cli, "set-identity", route, offline, "--serial", "offline-only")
    assert tomllib.loads(offline.read_text(encoding="utf-8"))["connection"] == tomllib.loads(route.read_text(encoding="utf-8"))["connection"]
    assert Gateway.requests == requests and source.read_bytes() == original and pinned.read_bytes() == saved
    snapshot = output / "parameter.json"
    reading = tomllib.loads(run(cli, "read-parameter", pinned, "V_SIM_Threshold", "--snapshot", snapshot))
    review = tomllib.loads(run(cli, "review-parameter", snapshot))
    assert reading["raw_value"] == review["decoded"]["raw_value"] == "500"
    assert reading["preferred_display"] == review["decoded"]["preferred_display"]
    assert review["identity_check"] == "matched all configured identity fields"
    unpinned_snapshot = output / "unpinned.json"
    run(cli, "read-parameter", cleared, "V_SIM_Threshold", "--snapshot", unpinned_snapshot)
    assert tomllib.loads(run(cli, "review-parameter", unpinned_snapshot))["identity_check"] == "no identity pin configured"
    wrong = output / "wrong-serial.toml"
    run(cli, "set-identity", pinned, wrong, "--serial", "SIM-0002")
    rejected_snapshot = output / "must-not-save.json"
    run(cli, "read-parameter", wrong, "V_SIM_Threshold", "--snapshot", rejected_snapshot, expected=1)
    assert not rejected_snapshot.exists()
    assert Gateway.requests == requests and source.read_bytes() == original and pinned.read_bytes() == saved
    capture(cli, pinned, output / "capture", "--seconds", .25)


def verify_temporal_defaults(cli, output):
    output.mkdir()
    profile = output / "time-defaults.toml"
    run(cli, "import", ROOT / "samples/sensor-time.xml", profile)
    saved = profile.read_bytes()
    requests = Gateway.requests
    report = tomllib.loads(run(cli, "defaults", profile))
    assert (report["decoded"], report["not_declared"], report["needs_review"], report["unsupported"]) == (6, 0, 0, 0)
    entries = {(entry["parameter_id"], entry["subindex"]): entry["value"] for entry in report["entries"]}
    assert entries[("V_SIM_Time", 0)]["representation_hex"] == "9DFF440080000000"
    assert entries[("V_SIM_TimeSpan", 0)]["representation_hex"] == "FFFFFFFD40000000"
    assert entries[("V_SIM_TimeRecord", 1)]["representation_hex"] == "9DFF440000418938"
    assert entries[("V_SIM_TimeRecord", 2)]["representation_hex"] == "FFFFFFFFFFBE76C8"
    assert not entries[("V_SIM_TimeRecord", 1)]["temporal_conversion"]["exact"]
    assert entries[("V_SIM_Times", 0)]["array_elements"] == 2
    assert entries[("V_SIM_Times", 0)]["representation_hex"] == "0000000000000000"
    assert entries[("V_SIM_TimeSpans", 0)]["array_elements"] == 2
    for variable, old, cases in [
        ("V_SIM_Time", b'1984-01-01T00:00:00.5', ["1900-01-01", "2120-02-07T06:28:16", "2000-02-30", "2000-01-01T00:00:00Z"]),
        ("V_SIM_TimeSpan", b'-PT2.75S', ["P1D", "PT1H", "PT0.0001S", "-PT2147483648.001S"])]:
        assert saved.count(old) >= 1
        for i, invalid in enumerate(cases):
            bad = output / f"bad-{variable}-{i}.toml"
            bad.write_bytes(saved.replace(b'default_value = "' + old + b'"', b'default_value = "' + invalid.encode('ascii') + b'"'))
            value = tomllib.loads(run(cli, "defaults", bad, variable, expected=1))["entries"][0]["value"]
            assert value["status"] == "needs_review" and value["declared"] == invalid and "representation_hex" not in value
    unreachable = output / "unreachable.toml"
    unreachable.write_bytes(saved.replace(b'gateway = "simulator"', b'gateway = "ifm_al1350"').replace(b'endpoint = ""', b'endpoint = "http://127.0.0.1:9"'))
    assert tomllib.loads(run(cli, "defaults", unreachable)) == report
    assert Gateway.requests == requests and profile.read_bytes() == saved
    verify_time_parameters(cli, profile, output / "observations")
    capture(cli, profile, output / "capture", "--seconds", .25)


def verify_range_reads(cli, profile, output, band="Operating band", measurement="Measurement range", commands="Vendor commands (offline metadata only)"):
    output.mkdir()
    snapshot = output / "offset.json"
    result = tomllib.loads(run(cli, "read-parameter", profile, "V_OffsetTime", "--snapshot", snapshot))
    assert result["raw_hex"] == "2A" and result["raw_value"] == "42" and result["displays"][0]["value"] == "21"
    assert result["value_assessment"]["matched_range_names"] == [band]
    before = Gateway.requests
    restored = tomllib.loads(run(cli, "review-parameter", snapshot))["decoded"]
    offline = tomllib.loads(run(cli, "decode-parameter", profile, "V_OffsetTime", "2A"))
    assert restored["value_assessment"] == offline["value_assessment"] == result["value_assessment"]
    outside = tomllib.loads(run(cli, "decode-parameter", profile, "V_OffsetTime", "13"))
    assert outside["raw_value"] == "19" and outside["value_assessment"]["status"] == "outside_definition"
    assert "matched_range_names" not in outside["value_assessment"]
    inline = tomllib.loads(run(cli, "decode-parameter", profile, "V_SIM_Threshold", "01F4"))
    assert inline["value_assessment"]["matched_range_names"] == [measurement]
    for variable, subindex in [("V_SystemCommand", 0), ("V_DirectParameters_1", 16)]:
        decoded = tomllib.loads(run(cli, "decode-parameter", profile, variable, "CC", "--subindex", subindex))
        assert decoded["value_assessment"]["matched_range_names"] == [commands]
        run(cli, "read-parameter", profile, variable, "--subindex", subindex, expected=1)
    assert Gateway.requests == before


def verify_standard_ranges(cli, output):
    output.mkdir()
    xml = ROOT / "samples/sensor-standard-values.xml"
    definitions = ROOT / "samples/synthetic-value-standard-definitions.xml"
    profile = output / "standard-values.toml"
    run(cli, "import", xml, profile, "--standard-definitions", definitions)
    verify_range_reads(cli, profile, output / "english")
    german = output / "german.toml"
    run(cli, "import", xml, german, "--standard-definitions", definitions, "--language", "de")
    verify_range_reads(cli, german, output / "german", "Arbeitsbereich", "Messbereich", "Herstellerbefehle (nur Offline-Metadaten)")
    text = xml.read_text(encoding="utf-8")
    before = Gateway.requests
    saved = profile.read_bytes()
    variants = [
        text.replace('lowerValue="200" upperValue="210"', 'lowerValue="127" upperValue="129"'),
        text.replace('<StdSingleValueRef value="128"/>', '').replace('lowerValue="200" upperValue="210"', 'lowerValue="120" upperValue="128"'),
        text.replace('lowerValue="+020"', 'lowerValue="0"'),
        text.replace('<StdSingleValueRef value="+00"/>', '<StdSingleValueRef value="+00"/><StdSingleValueRef value="0"/>'),
        text.replace('upperValue="+050"', 'upperValue="256"'),
        text.replace('upperValue="+050"', 'upperValue="20"'),
        text.replace('<StdSingleValueRef value="+00"/>', '<StdValueRangeRef lowerValue="1" upperValue="255"/>'),
    ]
    for number, contents in enumerate(variants):
        path = output / f"rejected-{number}.xml"
        path.write_bytes(contents.encode("utf-8"))
        run(cli, "import", path, profile, "--standard-definitions", definitions, expected=1)
        assert profile.read_bytes() == saved and Gateway.requests == before
    return profile


def verify_cycle_capture(cli, profile, output, expected_port, snapshots=False):
    report, rows = capture(cli, profile, output, "--seconds", .25)
    assert report["iodd_communication"]["minimum_cycle_us"] == 2300
    assert report["last_status"]["port_cycle_us"] == expected_port
    review = tomllib.loads(run(cli, "review-capture", output, "--buckets", 3))
    context = review["signal"]["cycle_context"]
    assert context["iodd_minimum_cycle_us"] == 2300 and context["reported_port_cycle_us"] == expected_port
    assert context["target_vs_iodd_minimum"] == context["reported_port_vs_iodd_minimum"] == "shorter"
    if snapshots:
        assert context["freshness"] == "snapshots" and report["source_update_loss"] == "unknown"
        assert any("source-update loss remains unknown" in note for note in context["notes"])
    else:
        assert context["configured_gateway"] == "simulator" and context["freshness"] == "counter32"
    review_capture(cli, output, rows, report)
    return report, rows


def verify_communication(cli, output):
    output.mkdir()
    xml = ROOT / "samples/sensor-communication.xml"
    profile = output / "communication.toml"
    run(cli, "import", xml, profile)
    saved = tomllib.loads(profile.read_text(encoding="utf-8"))
    declaration = saved["iodd"]["communication"]
    assert declaration == {"iolink_revision": "V1.1", "compatible_with": "V1.0", "bitrate": "COM2", "minimum_cycle_us": 2300, "sio_supported": True, "m_sequence_capability": 27}
    assert "minimum cycle 2300" in run(cli, "validate", profile)
    before = Gateway.requests
    context = tomllib.loads(run(cli, "timing", profile))
    assert Gateway.requests == before
    assert context["target_us"] == context["requested_host_poll_us"] == 1000
    assert context["target_vs_iodd_minimum"] == context["host_poll_vs_iodd_minimum"] == "shorter"
    assert context["target_vs_reported_port"] == "unknown" and "reported_port_cycle_us" not in context
    assert tomllib.loads(run(cli, "timing", profile, "--target-us", 5000))["target_vs_iodd_minimum"] == "longer"
    run(cli, "timing", profile, "--target-us", 0, expected=2)
    unreachable = output / "offline-only.toml"
    unreachable.write_bytes(profile.read_bytes().replace(b'gateway = "simulator"', b'gateway = "ifm_al1350"').replace(b'endpoint = ""', b'endpoint = "http://127.0.0.1:9"'))
    assert tomllib.loads(run(cli, "timing", unreachable))["target_vs_reported_port"] == "unknown"
    assert Gateway.requests == before
    for minimum, relation in [(0, "unknown"), (1000, "equal"), (6400, "shorter"), (132800, "shorter")]:
        changed = output / f"minimum-{minimum}.xml"
        changed.write_bytes(xml.read_bytes().replace(b'minCycleTime="2300"', f'minCycleTime="{minimum}"'.encode()))
        mapped = output / f"minimum-{minimum}.toml"
        run(cli, "import", changed, mapped)
        assert tomllib.loads(run(cli, "timing", mapped))["target_vs_iodd_minimum"] == relation
    bad = output / "bad-step.xml"
    bad.write_bytes(xml.read_bytes().replace(b'minCycleTime="2300"', b'minCycleTime="6500"'))
    original = profile.read_bytes()
    run(cli, "import", bad, profile, expected=1)
    assert profile.read_bytes() == original
    snapshot = output / "parameter.json"
    run(cli, "read-parameter", profile, "V_SIM_Threshold", "--snapshot", snapshot)
    assert json.loads(snapshot.read_bytes())["profile"]["iodd"]["communication"] == declaration
    before = Gateway.requests
    assert tomllib.loads(run(cli, "review-parameter", snapshot))["decoded"]["raw_value"] == "500"
    assert Gateway.requests == before
    verify_cycle_capture(cli, profile, output / "sim-cycle", 1000)
    corrupt = output / "corrupt-cycle"
    shutil.copytree(output / "sim-cycle", corrupt)
    path = corrupt / "capture.toml"
    original = path.read_bytes()
    path.write_bytes(original.replace(b'minimum_cycle_us = 2300', b'minimum_cycle_us = 2400'))
    run(cli, "verify-capture", corrupt, expected=1)
    path.write_bytes(original.replace(b'requested_poll_us = 1000', b'requested_poll_us = 2000'))
    run(cli, "verify-capture", corrupt, expected=1)
    return profile


def verify_stamps(cli, output):
    output.mkdir()
    main = ROOT / "samples/sensor-checksum.xml"
    language = ROOT / "samples/sensor-checksum-de.xml"
    before = {path: path.read_bytes() for path in (main, language)}
    report = tomllib.loads(run(cli, "check-stamp", main, "--language", "de"))
    assert report["main"]["status"] == report["language"]["status"] == "matched"
    assert report["main"]["declared_crc"] == report["main"]["calculated_crc"] == 2556892730
    assert report["language"]["main_crc"] == 2556892730
    for path, key in [(main, "main"), (language, "language")]:
        assert report[key]["sha256"] == hashlib.sha256(before[path]).hexdigest()
        assert report[key]["bytes"] == len(before[path])
    profile = output / "stamped.toml"
    run(cli, "import", main, profile, "--language", "de", "--require-stamp")
    saved = tomllib.loads(profile.read_text(encoding="utf-8"))
    assert saved["iodd"]["stamp_validation"] == report
    assert saved["iodd"]["parameters"][0]["name"] == "Schwellwert"
    assert "IODD main checksum matched" in run(cli, "validate", profile)
    snapshot = output / "parameter.json"
    run(cli, "read-parameter", profile, "V_SIM_Threshold", "--snapshot", snapshot)
    original_snapshot = snapshot.read_bytes()
    saved_stamp = json.loads(original_snapshot)["profile"]["iodd"]["stamp_validation"]
    assert saved_stamp["main"]["sha256"] == report["main"]["sha256"] and saved_stamp["language"]["main_crc"] == 2556892730
    requests = Gateway.requests
    restored = tomllib.loads(run(cli, "review-parameter", snapshot))
    assert restored["decoded"]["raw_value"] == "500" and restored["iodd_stamp"] == report
    assert Gateway.requests == requests and snapshot.read_bytes() == original_snapshot

    missing = tomllib.loads(run(cli, "check-stamp", ROOT / "samples/sensor.xml", expected=1))
    assert missing["main"]["status"] == "missing"
    destination = output / "preserved.toml"
    destination.write_bytes(profile.read_bytes())
    original = destination.read_bytes()
    run(cli, "import", ROOT / "samples/sensor.xml", destination, "--require-stamp", "--variant", "SIM-SENSOR", expected=1)
    assert destination.read_bytes() == original

    # Actual bytes, including a BOM, CRLF, UTF-16 and non-BMP text, affect the CRC.
    xml = main.read_text(encoding="utf-8").replace('crc="2556892730"', 'crc=""')
    encodings = {
        "utf8-bom": (b"\xef\xbb\xbf", "utf-8", "utf8"),
        "utf16-le": (b"\xff\xfe", "utf-16le", "utf16_le"),
        "utf16-be": (b"\xfe\xff", "utf-16be", "utf16_be"),
        "crlf": (b"", "utf-8", "utf8"),
    }
    for name, (bom, codec, expected_encoding) in encodings.items():
        text = xml.replace('encoding="UTF-8"', 'encoding="UTF-16"') if codec.startswith("utf-16") else xml
        if name == "crlf":
            text = text.replace("\n", "\r\n")
        empty = bom + text.encode(codec)
        crc = zlib.crc32(empty)
        binary = bom + text.replace('crc=""', f'crc="{crc}"').encode(codec)
        path = output / f"{name}.xml"
        path.write_bytes(binary)
        actual = tomllib.loads(run(cli, "check-stamp", path))["main"]
        assert actual["encoding"] == expected_encoding and actual["calculated_crc"] == crc
        assert actual["sha256"] == hashlib.sha256(binary).hexdigest() and path.read_bytes() == binary
        run(cli, "import", path, output / f"{name}.toml", "--require-stamp")
        language_text = language.read_text(encoding="utf-8").replace('crc="3967000593"', 'crc=""')
        if codec.startswith("utf-16"):
            language_text = language_text.replace('encoding="UTF-8"', 'encoding="UTF-16"')
        if name == "crlf":
            language_text = language_text.replace("\n", "\r\n")
        language_crc = zlib.crc32(bom + language_text.encode(codec) + b"2556892730")
        language_bytes = bom + language_text.replace('crc=""', f'crc="{language_crc}"').encode(codec)
        language_path = output / f"{name}-de.xml"
        language_path.write_bytes(language_bytes)
        actual = tomllib.loads(run(cli, "check-stamp", main, "--language", "de", "--language-file", language_path))["language"]
        assert actual["calculated_crc"] == language_crc and actual["encoding"] == expected_encoding
        assert actual["main_crc"] == 2556892730 and language_path.read_bytes() == language_bytes

    bad = output / "changed.xml"
    bad.write_bytes(before[main].replace(b"Threshold", b"Changed threshold"))
    bad_report = tomllib.loads(run(cli, "check-stamp", bad, expected=1))
    assert bad_report["main"]["status"] == "mismatch"
    run(cli, "import", bad, destination, expected=1)
    assert destination.read_bytes() == original
    malformed = output / "malformed.xml"
    malformed.write_bytes(before[main].replace(b'crc="2556892730"', b'crc="4294967296"'))
    assert tomllib.loads(run(cli, "check-stamp", malformed, expected=1))["main"]["status"] == "malformed"
    run(cli, "import", malformed, destination, expected=1)
    assert destination.read_bytes() == original

    # A correct changed main CRC cannot validate the old external language binding.
    changed = xml.replace("Threshold", "New threshold")
    crc = zlib.crc32(changed.encode("utf-8"))
    bound = output / "bound.xml"
    bound.write_bytes(changed.replace('crc=""', f'crc="{crc}"').encode("utf-8"))
    binding = tomllib.loads(run(cli, "check-stamp", bound, "--language", "de", "--language-file", language, expected=1))
    assert binding["main"]["status"] == "matched" and binding["language"]["status"] == "mismatch"
    run(cli, "import", bound, destination, "--language", "de", "--language-file", language, expected=1)
    assert destination.read_bytes() == original
    unbound = tomllib.loads(run(cli, "check-stamp", ROOT / "samples/sensor.xml", "--language", "de", "--language-file", language, expected=1))
    assert unbound["language"]["status"] == "unbound"

    bundle = output / "iodd.zip"
    entry = "fixtures/Workbench-Synthetic-20250101-IODD1.1.xml"
    with zipfile.ZipFile(bundle, "x") as archive:
        archive.writestr(entry, before[main])
        archive.writestr(entry[:-4] + "-de.xml", before[language])
    zipped = tomllib.loads(run(cli, "check-stamp", bundle, "--language", "de"))
    assert zipped["main"]["sha256"] == report["main"]["sha256"] and zipped["language"]["sha256"] == report["language"]["sha256"]
    run(cli, "import", bundle, output / "zip.toml", "--language", "de", "--require-stamp")
    assert {path: path.read_bytes() for path in before} == before


def verify_xsd(cli, scratch):
    folder = scratch / "schema '$ local"
    folder.mkdir()
    namespace = "http://www.io-link.com/IODD/2010/10"
    root = folder / "fixture.xsd"
    types = folder / "types.xsd"
    root_text = (f'<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:i="{namespace}" targetNamespace="{namespace}" elementFormDefault="qualified">'
                 '<xs:include schemaLocation="types.xsd"/><xs:element name="IODevice" type="i:DeviceT"/></xs:schema>')
    types_text = (f'<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="{namespace}" elementFormDefault="qualified">'
                  '<xs:complexType name="DeviceT"><xs:sequence><xs:any minOccurs="0" maxOccurs="unbounded" processContents="skip"/></xs:sequence>'
                  '<xs:attribute name="validation_marker" type="xs:integer" use="required"/><xs:anyAttribute processContents="skip"/></xs:complexType></xs:schema>')
    root.write_text(root_text, encoding="utf-8")
    types.write_text(types_text, encoding="utf-8")
    xml = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8").replace("<IODevice ", '<IODevice validation_marker="7" ', 1)
    source = folder / "sensor.xml"
    source.write_text(xml, encoding="utf-8")
    archive = folder / "checked.zip"
    entry = "bundle/Checked-IODD1.1.xml"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
        bundle.write(source, entry)
    report = tomllib.loads(run(cli, "check-iodd", archive, "--entry", entry, "--xsd", root))
    assert report["passed"] and report["xml_sha256"] == hashlib.sha256(source.read_bytes()).hexdigest()
    assert {item["name"]: item["sha256"] for item in report["schemas"]} == {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in (root, types)}
    profile = folder / "checked.toml"
    run(cli, "import", archive, profile, "--variant", "SIM-SENSOR", "--xsd", root)
    restored = tomllib.loads(profile.read_text(encoding="utf-8"))
    assert restored["iodd"]["schema_validation"]["xml_sha256"] == report["xml_sha256"]
    assert "Main XML XSD passed" in run(cli, "validate", profile)
    before = profile.read_bytes()
    source.write_text(xml.replace('validation_marker="7"', 'validation_marker="bad"'), encoding="utf-8")
    failure = tomllib.loads(run(cli, "check-iodd", source, "--xsd", root, expected=1))
    assert not failure["passed"] and failure["diagnostics"][0]["line"] > 0
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR", "--xsd", root, expected=1)
    assert profile.read_bytes() == before
    source.write_text(xml, encoding="utf-8")
    requests = []

    class ExternalSchema(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_GET(self):
            requests.append(self.path)
            self.send_response(404)
            self.end_headers()

    with ThreadingHTTPServer(("127.0.0.1", 0), ExternalSchema) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            url = f"http://127.0.0.1:{server.server_port}/never.xsd"
            hinted = xml.replace('validation_marker="7"', f'validation_marker="7" xsi:schemaLocation="{namespace} {url}"')
            source.write_text(hinted, encoding="utf-8")
            assert tomllib.loads(run(cli, "check-iodd", source, "--xsd", root))["passed"]
            root.write_text(root_text.replace("types.xsd", url), encoding="utf-8")
            assert not tomllib.loads(run(cli, "check-iodd", source, "--xsd", root, expected=1))["passed"]
            assert not requests
        finally:
            server.shutdown()
            thread.join()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("cli", type=lambda p: Path(p).resolve())
    cli = parser.parse_args().cli
    scratch = ROOT / ".local" / ("verify-" + uuid.uuid4().hex[:8])
    scratch.mkdir(parents=True)
    verify_stamps(cli, scratch / "stamps")
    communication_profile = verify_communication(cli, scratch / "communication")
    standard_range_profile = verify_standard_ranges(cli, scratch / "standard-ranges")
    verify_defaults(cli, scratch / "defaults")
    verify_temporal_defaults(cli, scratch / "temporal-defaults")
    verify_connection_settings(cli, scratch / "connection-settings")
    verify_identity_settings(cli, scratch / "identity-settings")
    verify_xsd(cli, scratch)
    source = ROOT / "samples" / "simulator.toml"
    profile = source.read_text(encoding="utf-8")

    with zipfile.ZipFile(scratch / "iodd.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        archive.write(ROOT / "samples" / "sensor.xml", "bundle/Synthetic-IODD1.1.xml")
        archive.write(ROOT / "samples" / "sensor-de.xml", "bundle/Synthetic-IODD1.1-de.xml")
    catalog = tomllib.loads(run(cli, "iodd", scratch / "iodd.zip"))
    assert len(catalog["variants"]) == 2
    assert "select the connected product ID" in subprocess.run(
        [str(cli), "import", str(scratch / "iodd.zip"), str(scratch / "ambiguous.toml")],
        capture_output=True, text=True, encoding="utf-8", timeout=10, check=False,
    ).stderr
    imported = scratch / "imported.toml"
    run(cli, "import", scratch / "iodd.zip", imported, "--variant", "SIM-SENSOR")
    assert "level [1] [IODD]" in run(cli, "decode", imported, "FFF9")
    run(cli, "validate", imported)
    localized = scratch / "localized.toml"
    run(cli, "import", scratch / "iodd.zip", localized, "--variant", "SIM-SENSOR", "--language", "de")
    locale = tomllib.loads(localized.read_text(encoding="utf-8"))["iodd"]["localization"]
    assert locale["selected_language"] == "de" and locale["fallback_text_ids"]
    assert "Füllstand [1]" in run(cli, "decode", localized, "FFF9")
    reading = tomllib.loads(run(cli, "read-parameter", localized, "V_SIM_Threshold"))
    assert reading["name"] == "Grenzwert" and reading["choice_name"] == "Nennwert" and reading["raw_value"] == "500"
    _, localized_rows = capture(cli, localized, scratch / "sim-localized", "--seconds", 0.25)
    assert any("Füllstand [1]" in name for name in localized_rows[0])
    _, selected_locale = capture(cli, localized, scratch / "sim-selected-language", "--seconds", 0.25,
                                 "--field", "Füllstand [1]")
    assert list(selected_locale[0])[12:] == ["value.Füllstand [1]"]
    before = localized.read_bytes()
    run(cli, "import", scratch / "iodd.zip", localized, "--variant", "SIM-SENSOR", "--language", "fr", expected=1)
    assert localized.read_bytes() == before
    custom_units = scratch / "custom-units.xml"
    custom_units.write_text((ROOT / "samples/sensor.xml").read_text(encoding="utf-8").replace('unitCode="1013"', 'unitCode="1015"'), encoding="utf-8")
    unit_profile = scratch / "unit-profile.toml"
    run(cli, "import", custom_units, unit_profile, "--variant", "SIM-SENSOR", "--language", "de",
        "--language-file", ROOT / "samples/sensor-de.xml", "--unit-definitions", ROOT / "samples/synthetic-unit-definitions.xml")
    unit_reading = tomllib.loads(run(cli, "read-parameter", unit_profile, "V_SIM_Threshold"))
    assert unit_reading["displays"][0]["unit"] == "nm" and unit_reading["displays"][0]["value"] == "52.0"
    parameters = tomllib.loads(run(cli, "parameters", imported))
    assert len(parameters["parameters"]) == 3
    reading = tomllib.loads(run(cli, "read-parameter", imported, "V_SIM_Threshold"))
    assert reading["raw_value"] == "500" and reading["choice_name"] == "Nominal setting"
    assert reading["displays"] == [{"id": "M_SIM_Settings/1", "value": "52.0", "unit": "mm"}]
    run(cli, "read-parameter", imported, "V_SIM_Command", expected=1)
    events = tomllib.loads(run(cli, "events", imported))
    assert len(events["events"]) == 1 and events["standard_event_refs"] == [16912]
    diagnostic = tomllib.loads(run(cli, "decode-diagnostics", imported, "E48DFE000000", "--device-status", "02"))
    assert diagnostic["device_status"]["meaning"] == "Out of specification"
    assert diagnostic["detailed_status"]["events"][0]["definition"]["name"] == "Synthetic range warning"
    assert diagnostic["detailed_status"]["empty_slots"] == 1
    assert "definition" not in tomllib.loads(run(cli, "decode-diagnostics", imported, "EC8DFE"))["detailed_status"]["events"][0]
    malformed = tomllib.loads(run(cli, "decode-diagnostics", imported, "E48DFE00", expected=1))
    assert malformed["detailed_status"]["raw_hex"] == "E48DFE00" and malformed["detailed_status"]["decode_error"]
    explicit = tomllib.loads(run(cli, "read-diagnostics", imported))
    assert explicit["observed_identities_equal"] and all(read["status"]["route"] == "synthetic simulator" for read in explicit["reads"])
    resolved = scratch / "resolved.toml"
    definitions = ROOT / "samples" / "synthetic-standard-definitions.xml"
    run(cli, "import", scratch / "iodd.zip", resolved, "--variant", "SIM-SENSOR",
        "--standard-definitions", definitions)
    parameters = tomllib.loads(run(cli, "parameters", resolved))
    assert len(parameters["parameters"]) == 4 and not parameters["standard_parameter_refs"]
    assert not tomllib.loads(run(cli, "events", resolved))["standard_event_refs"]
    product = tomllib.loads(run(cli, "read-parameter", resolved, "V_ProductID"))
    assert product["raw_value"] == "SIM-SENSOR" and product["index"] == 19
    standard_language_profile = verify_standard_languages(cli, scratch / "iodd.zip", scratch / "standard-languages")
    run(cli, "iodd", ROOT / "samples" / "sensor.xml", "--standard-definitions",
        ROOT / "samples" / "sensor.xml", expected=1)
    record_profile = scratch / "record.toml"
    run(cli, "import", ROOT / "samples" / "sensor-record.xml", record_profile)
    decoded = tomllib.loads(run(cli, "decode-parameter", record_profile, "V_SIM_Record", "FFEC010A54455354"))
    assert [item["value"]["raw_value"] for item in decoded["record_items"]] == ["-20", "true", "10", "TEST"]
    assert decoded["record_items"][0]["value"]["displays"][0]["value"] == "0.0"
    short = tomllib.loads(run(cli, "decode-parameter", record_profile, "V_SIM_Record", "FFEC", expected=1))
    assert short["raw_hex"] == "FFEC" and "width differs" in short["decode_error"]
    run(cli, "decode-parameter", record_profile, "V_SIM_Record", "00" * 233, expected=1)
    for subindex in range(1, 5):
        member = tomllib.loads(run(cli, "read-parameter", record_profile, "V_SIM_Record", "--subindex", subindex))
        assert member["raw_value"] == decoded["record_items"][subindex - 1]["value"]["raw_value"]
    # Offline parameter decoding must not attempt to open a configured endpoint.
    offline_only = scratch / "record-offline.toml"
    offline_only.write_text(record_profile.read_text(encoding="utf-8").replace('gateway = "simulator"',
        'gateway = "ifm_al1350"').replace('endpoint = ""', 'endpoint = "unreachable offline endpoint"'), encoding="utf-8")
    run(cli, "decode-parameter", offline_only, "V_SIM_Record", "FFEC", "--subindex", 1)
    array_profile = scratch / "array.toml"
    run(cli, "import", ROOT / "samples" / "sensor-array.xml", array_profile)
    array = tomllib.loads(run(cli, "decode-parameter", array_profile, "V_SIM_History", "02C495F0"))
    expected = ["2", "-4", "4", "-7", "5", "-1", "0"]
    assert [item["value"]["raw_value"] for item in array["array_items"]] == expected
    assert array["array_items"][1]["value"]["displays"][0]["value"] == "-1.0"
    for subindex, value in enumerate(expected, 1):
        assert tomllib.loads(run(cli, "read-parameter", array_profile, "V_SIM_History", "--subindex", subindex))["raw_value"] == value
    capture(cli, array_profile, scratch / "sim-array", "--seconds", 0.25)

    formats_profile = scratch / "formats.toml"
    run(cli, "import", ROOT / "samples/sensor-formats.xml", formats_profile)
    default_xml = scratch / "default-formats.xml"
    default_xml.write_text((ROOT / "samples/sensor-formats.xml").read_text(encoding="utf-8").replace('displayFormat="Dec"', 'displayFormat="default"'), encoding="utf-8")
    default_profile = scratch / "default-formats.toml"
    run(cli, "import", default_xml, default_profile)
    assert tomllib.loads(run(cli, "read-parameter", default_profile, "V_SIM_Threshold"))["displays"][0]["value"] == "500"
    assert tomllib.loads(run(cli, "read-parameter", default_profile, "V_SIM_Record", "--subindex", 2))["displays"][0]["value"] == "true"
    views = tomllib.loads(run(cli, "read-parameter", formats_profile, "V_SIM_Threshold"))
    assert views["raw_value"] == "500"
    assert [display["value"] for display in views["displays"]] == ["500", "01F4h", "0000 0001 1111 0100b"]
    offline_views = tomllib.loads(run(cli, "decode-parameter", formats_profile, "V_SIM_Threshold", "01F4"))
    assert offline_views["displays"] == views["displays"]
    format_record = tomllib.loads(run(cli, "read-parameter", formats_profile, "V_SIM_Record"))
    assert format_record["record_items"][0]["value"]["displays"][1]["value"] == "FFECh"
    assert format_record["record_items"][1]["value"]["raw_value"] == "true"
    assert format_record["record_items"][1]["value"]["displays"][0]["value"] == "1"
    format_array = tomllib.loads(run(cli, "read-parameter", formats_profile, "V_SIM_History"))
    assert format_array["array_items"][1]["value"]["displays"][0]["value"] == "FCh"
    assert format_array["array_items"][1]["value"]["displays"][1]["value"] == "1111 1100b"
    run(cli, "decode-parameter", formats_profile, "V_SIM_Threshold", "FF", expected=1)
    verify_display_preferences(cli, formats_profile, scratch / "sim-display-preferences")
    verify_parameter_snapshots(cli, formats_profile, scratch / "sim-parameter-snapshots")
    time_profile = scratch / "time.toml"
    run(cli, "import", ROOT / "samples/sensor-time.xml", time_profile)
    verify_time_parameters(cli, time_profile, scratch / "sim-time")
    strings_profile = scratch / "strings.toml"
    run(cli, "import", ROOT / "samples/sensor-strings.xml", strings_profile)
    verify_string_parameters(cli, strings_profile, scratch / "sim-strings")
    values_profile = scratch / "values.toml"
    run(cli, "import", ROOT / "samples/sensor-values.xml", values_profile)
    verify_value_definitions(cli, values_profile, scratch / "sim-values")
    # The older fixture intentionally overlaps its nominal choice and range.
    overlapping = tomllib.loads(run(cli, "read-parameter", imported, "V_SIM_Threshold"))
    assert overlapping["choice_name"] == "Nominal setting" and overlapping["value_assessment"]["status"] == "definition_needs_review"

    clean, rows = capture(cli, source, scratch / "sim-every", "--seconds", 1)
    assert clean["last_status"]["route"] == "synthetic simulator"
    assert clean["row_kind"] == "source_update"
    assert any(a["raw_hex"] == b["raw_hex"] for a, b in zip(rows, rows[1:]))
    assert len({row["source_counter"] for row in rows}) == len(rows)
    assert clean["gateway_update_spacing"]["p50_us"] == 1000

    before_selection = source.read_bytes()
    raw_report, raw_rows = capture(cli, source, scratch / "sim-raw-only", "--seconds", 0.4, "--raw-only", "--simulate-faults")
    raw_profile = scratch / "sim-raw-only/profile.toml"
    saved_raw = tomllib.loads(raw_profile.read_text(encoding="utf-8"))
    assert saved_raw["logging"]["raw_only"] and not saved_raw["logging"]["fields"]
    assert len(saved_raw["fields"]) == 2 and len(raw_rows[0]) == 12
    assert "level [manual]" in run(cli, "decode", raw_profile, "FFF9")
    raw_profile_before = raw_profile.read_bytes()
    rotating_raw = scratch / "raw-rotating.toml"
    rotating_raw.write_text(raw_profile.read_text(encoding="utf-8").replace('mode = "every_update"', 'mode = "on_change"').replace('rotate_rows = 0', 'rotate_rows = 11'), encoding="utf-8")
    rotated_raw, rotated_rows = capture(cli, rotating_raw, scratch / "sim-raw-change-rotation", "--seconds", 0.35)
    assert rotated_raw["csv_parts"] > 1 and rotated_raw["counts"]["unchanged"] > 0
    assert len(rotated_rows[0]) == 12
    joined_raw = scratch / "raw-joined.csv"
    run(cli, "verify-capture", scratch / "sim-raw-change-rotation", "--joined-csv", joined_raw)
    with joined_raw.open(encoding="utf-8", newline="") as file:
        assert list(csv.DictReader(file)) == rotated_rows
    _, ordered_rows = capture(cli, raw_profile, scratch / "sim-selected-order", "--seconds", 0.25,
                              "--field", "switch", "--field", "level")
    assert list(ordered_rows[0])[12:] == ["value.switch", "value.level [mm]"]
    ordered_profile = tomllib.loads((scratch / "sim-selected-order/profile.toml").read_text(encoding="utf-8"))
    assert not ordered_profile["logging"].get("raw_only", False)
    assert ordered_profile["logging"]["fields"] == ["switch", "level"]
    _, all_rows = capture(cli, raw_profile, scratch / "sim-all-values", "--seconds", 0.25, "--all-fields")
    assert list(all_rows[0])[12:] == ["value.level [mm]", "value.switch"]
    assert raw_profile.read_bytes() == raw_profile_before and source.read_bytes() == before_selection
    for index, (arguments, selection_exit_code) in enumerate([
            (["--field", "missing"], 1), (["--field", "level", "--field", "level"], 1),
            (["--raw-only", "--field", "level"], 2), (["--all-fields", "--raw-only"], 2)]):
        failed = scratch / f"bad-selection-{index}"
        assert not run(cli, "capture", source, failed, "--seconds", 0.1, *arguments, expected=selection_exit_code)
        assert not failed.exists()

    punctuation = scratch / "punctuation.toml"
    punctuation.write_text('''name = "Punctuation selection"
input_bytes = 2
[[fields]]
name = 'Level, "quoted"'
bits = 16
[[fields]]
name = "Unused"
bits = 1
kind = "bool"
''', encoding="utf-8")
    _, punctuation_rows = capture(cli, punctuation, scratch / "sim-selected-punctuation", "--seconds", 0.25,
                                  "--field", 'Level, "quoted"')
    assert list(punctuation_rows[0])[12:] == ['value.Level, "quoted"']

    changed = scratch / "changed.toml"
    changed.write_text(profile.replace('"every_update"', '"on_change"').replace(
        '[logging]', '[logging]\nrotate_rows = 11'), encoding="utf-8")
    change, rows = capture(cli, changed, scratch / "sim-change", "--seconds", 1)
    assert change["counts"]["unchanged"] > 0 and change["csv_parts"] > 1
    assert all(a["raw_hex"] != b["raw_hex"] for a, b in zip(rows, rows[1:]))
    joined = scratch / "changed-joined.csv"
    checked = tomllib.loads(run(cli, "verify-capture", scratch / "sim-change", "--joined-csv", joined))
    with joined.open(encoding="utf-8", newline="") as csv_file:
        assert list(csv.DictReader(csv_file)) == rows
    assert checked["csv_parts"] == change["csv_parts"]

    copied_bundle = scratch / "copied-redecode-bundle"
    shutil.copytree(scratch / "sim-change-redecoded", copied_bundle)
    receipt_path = copied_bundle / "receipt.toml"
    text = receipt_path.read_text(encoding="utf-8")
    text = "\n".join(line if not line.startswith(("source_directory = ", "output_directory = "))
                      else line.split(" = ")[0] + " = " + json.dumps(str(scratch / "unavailable-recorded-path"))
                      for line in text.splitlines()) + "\n"
    receipt_path.write_text(text, encoding="utf-8", newline="\n")
    assert not tomllib.loads(run(cli, "verify-export", copied_bundle))["original_source_reverified"]
    relocated_source = scratch / "relocated-source"
    shutil.copytree(scratch / "sim-change", relocated_source)
    assert tomllib.loads(run(cli, "verify-export", copied_bundle, "--source", relocated_source))["original_source_reverified"]
    with (copied_bundle / "data.csv").open("a", encoding="utf-8", newline="") as file:
        file.write("\n")
    assert not run(cli, "verify-export", copied_bundle, expected=1)

    corrupt = scratch / "corrupt-for-redecode"
    shutil.copytree(scratch / "sim-change", corrupt)
    part = corrupt / "capture-0001.csv"
    with part.open(encoding="utf-8", newline="") as file:
        csv_rows = list(csv.reader(file))
    csv_rows[1][12] = "corrupted original value"
    with part.open("w", encoding="utf-8", newline="") as file:
        csv.writer(file, lineterminator="\n").writerows(csv_rows)
    assert not run(cli, "redecode-capture", corrupt, scratch / "sim-change-mapping.toml", scratch / "corrupt-export", expected=1)
    assert not (scratch / "corrupt-export").exists()

    faults, rows = capture(cli, source, scratch / "sim-faults", "--seconds", 2, "--simulate-faults")
    counts = faults["counts"]
    assert counts["duplicate"] > 0 and counts["stale"] > 0 and counts["invalid"] > 0
    assert counts["disconnects"] == counts["reconnects"] == 1
    assert faults["source_gaps_observed"] > 0
    assert {row["generation"] for row in rows} == {"1", "2"}
    assert any(row["validity"] == "invalid" and not row["value.level [mm]"] for row in rows)

    slow, rows = capture(cli, source, scratch / "sim-backpressure", "--seconds", 0.6,
                         "--queue-capacity", 3, "--simulate-writer-delay-ms", 9)
    assert slow["counts"]["logger_overruns"] > 0
    assert any(int(b["sample_index"]) - int(a["sample_index"]) > 1 for a, b in zip(rows, rows[1:]))

    with ThreadingHTTPServer(("127.0.0.1", 0), Gateway) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            fixture = scratch / "fixture.toml"
            fixture.write_text(profile.replace('gateway = "simulator"', (
                'gateway = "ifm_al1350"\n'
                f'endpoint = "http://127.0.0.1:{server.server_port}"\n'
                'ifm_mode = 3\nifm_cycle_unit = "microseconds"\n'
                'ifm_firmware = "3.1.42"'
            )).replace("poll_us = 1000", "poll_us = 15000"), encoding="utf-8")
            identity = tomllib.loads(run(cli, "status", fixture))
            assert identity["firmware"] == "3.1.42" and identity["port_cycle_us"] == 1200
            assert identity["identity_check"] == "partial identity match"
            assert "SIM-SENSOR" in run(cli, "isdu", fixture, 19, 0)
            report, rows = capture(cli, fixture, scratch / "ifm-loopback", "--seconds", 2)
            assert report["last_status"]["route"] == "AL1350 loopback fixture"
            assert report["source_update_loss"] == "unknown"
            assert "source_gaps_observed" not in report and "gateway_update_spacing" not in report
            assert report["counts"]["reconnects"] == 1
            assert report["counts"]["invalid"] == 1
            assert report["counts"]["received"] == report["counts"]["freshness_unknown"]
            assert all(row["row_kind"] == "host_poll" and row["source_counter"] == "" for row in rows)
            assert all(row["source_age_us"] == "" and row["stale"] == "" for row in rows)
            assert sum(row["raw_hex"] == "03C9" for row in rows) > 1
            assert Gateway.isdu_reads == 1 and not Gateway.violations
            raw_snapshot, raw_snapshot_rows = capture(cli, fixture, scratch / "ifm-raw-only", "--seconds", 0.4, "--raw-only")
            assert raw_snapshot["source_update_loss"] == "unknown"
            assert len(raw_snapshot_rows[0]) == 12 and all(row["row_kind"] == "host_poll" for row in raw_snapshot_rows)
            assert Gateway.isdu_reads == 1 and not Gateway.violations
            matched = tomllib.loads(run(cli, "status", fixture, "--verify-product-id"))
            assert matched["identity_check"] == "matched all configured identity fields"
            assert matched["identity"]["product_id"] == "SIM-SENSOR"
            Gateway.device_id = 99
            rejected = scratch / "identity-rejected"
            run(cli, "capture", fixture, rejected, "--seconds", 0.5, expected=1)
            denied = tomllib.loads((rejected / "capture.toml").read_text(encoding="utf-8"))
            assert "identity mismatch" in denied["terminal_error"] and denied["counts"]["poll_attempts"] == 0
            checked = tomllib.loads(run(cli, "verify-capture", rejected))
            assert checked["capture_had_error"] and checked["rows_checked"] == 0
            parameter_fixture = scratch / "parameter-fixture.toml"
            text = imported.read_text(encoding="utf-8").replace('gateway = "simulator"', 'gateway = "ifm_al1350"')
            text = text.replace('endpoint = ""', f'endpoint = "http://127.0.0.1:{server.server_port}"')
            text = text.replace('ifm_firmware = ""', 'ifm_firmware = "3.1.42"')
            text = text.replace('port = 1', 'port = 1\nifm_mode = 3\nifm_cycle_unit = "microseconds"')
            parameter_fixture.write_text(text.replace('poll_us = 1000', 'poll_us = 10000'), encoding="utf-8")
            Gateway.device_id = 1
            cycle_fixture = scratch / "cycle-fixture.toml"
            cycle_text = communication_profile.read_text(encoding="utf-8").replace('gateway = "simulator"', 'gateway = "ifm_al1350"')
            cycle_text = cycle_text.replace('endpoint = ""', f'endpoint = "http://127.0.0.1:{server.server_port}"')
            cycle_text = cycle_text.replace('ifm_firmware = ""', 'ifm_firmware = "3.1.42"').replace('port = 1', 'port = 1\nifm_mode = 3\nifm_cycle_unit = "microseconds"').replace('poll_us = 1000', 'poll_us = 10000')
            cycle_fixture.write_bytes(cycle_text.encode("utf-8"))
            verify_cycle_capture(cli, cycle_fixture, scratch / "ifm-cycle", 1200, snapshots=True)
            range_fixture = scratch / "range-fixture.toml"
            range_text = standard_range_profile.read_text(encoding="utf-8").replace('gateway = "simulator"', 'gateway = "ifm_al1350"')
            range_text = range_text.replace('endpoint = ""', f'endpoint = "http://127.0.0.1:{server.server_port}"')
            range_text = range_text.replace('ifm_firmware = ""', 'ifm_firmware = "3.1.42"').replace('port = 1', 'port = 1\nifm_mode = 3')
            range_fixture.write_bytes(range_text.encode("utf-8"))
            verify_range_reads(cli, range_fixture, scratch / "ifm-ranges")
            strings_fixture = scratch / "strings-fixture.toml"
            strings_text = strings_profile.read_text(encoding="utf-8").replace('gateway = "simulator"', 'gateway = "ifm_al1350"')
            strings_text = strings_text.replace('endpoint = ""', f'endpoint = "http://127.0.0.1:{server.server_port}"')
            strings_text = strings_text.replace('ifm_firmware = ""', 'ifm_firmware = "3.1.42"').replace('port = 1', 'port = 1\nifm_mode = 3')
            strings_fixture.write_text(strings_text, encoding="utf-8")
            verify_string_parameters(cli, strings_fixture, scratch / "ifm-strings", loopback=True)
            values_fixture = scratch / "values-fixture.toml"
            values_text = values_profile.read_text(encoding="utf-8").replace('gateway = "simulator"', 'gateway = "ifm_al1350"')
            values_text = values_text.replace('endpoint = ""', f'endpoint = "http://127.0.0.1:{server.server_port}"')
            values_text = values_text.replace('ifm_firmware = ""', 'ifm_firmware = "3.1.42"').replace('port = 1', 'port = 1\nifm_mode = 3')
            values_fixture.write_text(values_text, encoding="utf-8")
            verify_value_definitions(cli, values_fixture, scratch / "ifm-values", loopback=True)
            time_fixture = scratch / "time-fixture.toml"
            time_text = time_profile.read_text(encoding="utf-8").replace('gateway = "simulator"', 'gateway = "ifm_al1350"')
            time_text = time_text.replace('endpoint = ""', f'endpoint = "http://127.0.0.1:{server.server_port}"')
            time_text = time_text.replace('ifm_firmware = ""', 'ifm_firmware = "3.1.42"').replace('port = 1', 'port = 1\nifm_mode = 3')
            time_fixture.write_text(time_text, encoding="utf-8")
            verify_time_parameters(cli, time_fixture, scratch / "ifm-time")
            language_fixture = scratch / "standard-language-fixture.toml"
            language_text = standard_language_profile.read_text(encoding="utf-8").replace('gateway = "simulator"', 'gateway = "ifm_al1350"')
            language_text = language_text.replace('endpoint = ""', f'endpoint = "http://127.0.0.1:{server.server_port}"')
            language_text = language_text.replace('ifm_firmware = ""', 'ifm_firmware = "3.1.42"').replace('port = 1', 'port = 1\nifm_mode = 3')
            language_fixture.write_text(language_text, encoding="utf-8")
            translated_read = tomllib.loads(run(cli, "read-parameter", language_fixture, "V_ProductID"))
            assert translated_read["name"] == "Synthetische Produktkennung" and translated_read["raw_value"] == "SIM-SENSOR"
            format_fixture = scratch / "format-fixture.toml"
            format_text = formats_profile.read_text(encoding="utf-8").replace('gateway = "simulator"', 'gateway = "ifm_al1350"')
            format_text = format_text.replace('endpoint = ""', f'endpoint = "http://127.0.0.1:{server.server_port}"')
            format_text = format_text.replace('ifm_firmware = ""', 'ifm_firmware = "3.1.42"')
            format_text = format_text.replace('port = 1', 'port = 1\nifm_mode = 3\nifm_cycle_unit = "microseconds"')
            format_fixture.write_text(format_text, encoding="utf-8")
            format_read = tomllib.loads(run(cli, "read-parameter", format_fixture, "V_SIM_Threshold"))
            assert format_read["displays"][1]["value"] == "01F4h"
            bool_read = tomllib.loads(run(cli, "read-parameter", format_fixture, "V_SIM_Record", "--subindex", 2))
            assert bool_read["raw_value"] == "true" and bool_read["displays"][0]["value"] == "1"
            packed_read = tomllib.loads(run(cli, "read-parameter", format_fixture, "V_SIM_History"))
            assert packed_read["array_items"][1]["value"]["displays"][0]["value"] == "FCh"
            verify_display_preferences(cli, format_fixture, scratch / "ifm-display-preferences")
            verify_parameter_snapshots(cli, format_fixture, scratch / "ifm-parameter-snapshots", loopback=True)
            parallel_parameter_read(cli, parameter_fixture, scratch / "ifm-parallel-parameter")
            parallel_parameter_read(cli, parameter_fixture, scratch / "ifm-parallel-diagnostics", diagnostics=True)
            Gateway.fail_detailed = True
            partial = tomllib.loads(run(cli, "read-diagnostics", parameter_fixture, expected=1))
            assert partial["device_status"]["value"] == 2 and "detailed_status" not in partial
            assert partial["reads"][1]["read_error"]
            Gateway.fail_detailed = False
            Gateway.change_serial_after_status = True
            changed_identity = tomllib.loads(run(cli, "read-diagnostics", parameter_fixture, expected=1))
            assert not changed_identity["observed_identities_equal"]
            assert changed_identity["reads"][0]["status"]["identity"]["serial"] == "FIXTURE-SENSOR"
            assert changed_identity["reads"][1]["status"]["identity"]["serial"] == "CHANGED-SENSOR"
            Gateway.change_serial_after_status = False
            Gateway.data[Gateway.device + "/serial"] = "FIXTURE-SENSOR"
            before = Gateway.isdu_reads
            run(cli, "read-parameter", parameter_fixture, "V_SIM_Command", expected=1)
            assert Gateway.isdu_reads == before and not Gateway.violations
            array_fixture = scratch / "array-fixture.toml"
            text = array_profile.read_text(encoding="utf-8").replace('gateway = "simulator"', 'gateway = "ifm_al1350"')
            text = text.replace('endpoint = ""', f'endpoint = "http://127.0.0.1:{server.server_port}"')
            text = text.replace('port = 1', 'port = 1\nifm_mode = 3').replace('poll_us = 1000', 'poll_us = 10000')
            array_fixture.write_text(text, encoding="utf-8")
            Gateway.payload = "02C495F0"
            array_report, array_rows = capture(cli, array_fixture, scratch / "ifm-array", "--seconds", 0.25)
            assert array_report["source_update_loss"] == "unknown" and array_rows
            assert all(row["value.Temperature [2] [°C]"] == "-1.0" for row in array_rows)
            for subindex in range(1, 8):
                member = tomllib.loads(run(cli, "read-parameter", array_fixture, "V_SIM_History", "--subindex", subindex))
                assert member["raw_value"] == expected[subindex - 1]
            before = Gateway.isdu_reads
            disabled_array = scratch / "array-disabled.toml"
            disabled_array.write_text(text.replace('subindex_access = true', 'subindex_access = false'), encoding="utf-8")
            run(cli, "read-parameter", disabled_array, "V_SIM_History", "--subindex", 1, expected=1)
            assert Gateway.isdu_reads == before and not Gateway.violations
            Gateway.payload = "03C9"
            record_fixture = scratch / "record-fixture.toml"
            text = record_profile.read_text(encoding="utf-8").replace('gateway = "simulator"', 'gateway = "ifm_al1350"')
            text = text.replace('endpoint = ""', f'endpoint = "http://127.0.0.1:{server.server_port}"')
            text = text.replace('port = 1', 'port = 1\nifm_mode = 3')
            record_fixture.write_text(text, encoding="utf-8")
            whole = tomllib.loads(run(cli, "read-parameter", record_fixture, "V_SIM_Record"))
            for subindex in range(1, 5):
                member = tomllib.loads(run(cli, "read-parameter", record_fixture, "V_SIM_Record", "--subindex", subindex))
                assert member["raw_hex"] == whole["record_items"][subindex - 1]["value"]["raw_hex"]
            before = Gateway.isdu_reads
            disabled = scratch / "record-disabled.toml"
            disabled.write_text(text.replace('subindex_access = true', 'subindex_access = false'), encoding="utf-8")
            run(cli, "read-parameter", disabled, "V_SIM_Record", "--subindex", 1, expected=1)
            assert Gateway.isdu_reads == before and not Gateway.violations
        finally:
            server.shutdown()
            thread.join()

    print("PASS: offline identity pins with explicit clears and separate observed matching, offline connection edits with source/destination preservation, temporal declared defaults with explicit precision and era limits, offline declared defaults with separate snapshot observations, standard allowed-value validation and localized range names, IODD declared cycles and separate timing comparisons, binary IODD main/language checksum receipts, ZIP and variant selection, offline decoding, equal updates, on-change, raw-only logging and ordered field overrides, rotation, logger queue/duration telemetry, "
          "stale and invalid data, reconnect, bounded-queue loss, saved-capture verification, bounded signal review, revised mapping exports, local/source bundle verification, hash receipts, CSV joining, "
          "named scalar/record/array reads, typed choice labels and declared-value assessments, text/octet default views and strict width/padding checks, TimeT/TimeSpanT exact ticks and default views, binary/hex/plain-decimal parameter views and saved display preferences, saved parameter snapshots and offline reopening, array and localized CSV fields, translated standard parameters/events and fallbacks, supplied units, offline ISDU/event decoding, explicit/partial diagnostics, local XSD receipts and blocked external resolution, subindex restrictions, parallel polling during snapshot reads, and read-only AL1350 loopback")
    print(f"Evidence directory: {scratch}")


if __name__ == "__main__":
    main()
