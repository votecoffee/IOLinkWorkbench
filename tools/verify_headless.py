"""Windows CLI acceptance using synthetic data and loopback-only ifm fixtures."""

import argparse
import csv
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import math
import statistics
import shutil
import os
import re
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
    detailed_payload = "E48DFE000000"
    change_serial_after_status = False
    change_master_after_status = False
    payload = "03C9"
    threshold_payload = "01F4"
    record_payload = "FFEC010A54455354"
    array_payload = "02C495F0"
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
                *({"index": index, "subindex": 0} for index in (13, 14, 15)),
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
                (13, 0): "40008000FFFF", (14, 0): "010100030F01", (15, 0): "010200020302",
                (19, 0): "53494D2D53454E534F52", (48, 0): "2A", (64, 0): self.threshold_payload, (65, 0): "666978747572652073656E736F72",
                (36, 0): "02", (37, 0): self.detailed_payload,
                (66, 0): self.record_payload, (66, 1): "FFEC", (66, 2): "01", (66, 3): "0A", (66, 4): "54455354",
                (67, 0): self.array_payload, (67, 1): "02", (67, 2): "FC", (67, 3): "04", (67, 4): "F9", (67, 5): "05", (67, 6): "FF", (67, 7): "00",
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
            if index == 36 and self.change_master_after_status:
                self.data["/deviceinfo/serialnumber"] = "CHANGED-MASTER"
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
    nonblank = [cell for cell in cells if cell]
    if field["kind"] == "octet_string":
        assert signal["octet_values"] == len(nonblank)
        numbers = []
    elif field["kind"] in {"utf8_string", "ascii_string"}:
        valid = [cell for cell in nonblank if cell.startswith('"')]
        assert all(isinstance(json.loads(cell), str) for cell in valid)
        assert all(cell.startswith("Invalid StringT (") for cell in nonblank if not cell.startswith('"'))
        assert signal["strings"] == {"valid_values": len(valid), "malformed_values": len(nonblank) - len(valid)}
        numbers = []
    elif field["kind"] in {"time", "time_span"}:
        assert signal["temporal_values"] == len(nonblank)
        assert all(str(int(cell)) == cell for cell in nonblank)
        numbers = []
    else:
        assert "octet_values" not in signal
        assert "strings" not in signal
        assert "temporal_values" not in signal
        numbers = [float({"false": "0", "true": "1"}.get(cell, cell)) for cell in nonblank]
    finite = [value for value in numbers if math.isfinite(value)]
    numeric = signal["numeric"]
    assert numeric["finite_values"] == len(finite)
    assert numeric["blank_values"] == cells.count("")
    assert numeric["non_finite_values"] == len(numbers) - len(finite)
    assert numeric.get("first_csv_value") == (nonblank[0] if nonblank else None)
    assert numeric.get("last_csv_value") == (nonblank[-1] if nonblank else None)
    if finite:
        assert numeric["minimum"] == min(finite) and numeric["maximum"] == max(finite)
        assert math.isclose(numeric["mean"], statistics.fmean(finite), rel_tol=1e-12, abs_tol=1e-12)
    assert len(signal["buckets"]) == signal["buckets_requested"] == 7
    assert sum(bucket["rows"] for bucket in signal["buckets"]) == len(rows)
    assert sum(bucket["finite_values"] for bucket in signal["buckets"]) == len(finite)
    assert signal["recorded_logger_overruns"] == report["counts"]["logger_overruns"]
    assert review["capture"]["recorded_source_update_loss"] == report["source_update_loss"]
    for key in ("host_poll_start_spacing", "host_receive_spacing", "host_new_update_spacing", "gateway_update_spacing", "time_since_last_receive_us", "logger_metrics"):
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


def parallel_parameter_read(cli, fixture, output, diagnostics=False, layout_fixture=None):
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
        if layout_fixture is not None:
            command = [cli, "check-layout", layout_fixture, "--read"]
        elif diagnostics:
            command = [cli, "read-diagnostics", fixture]
        else:
            command = [cli, "read-parameter", fixture, "V_SIM_Threshold", "--snapshot", snapshot_path]
        reading = subprocess.Popen(command,
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
        if layout_fixture is not None:
            observed = tomllib.loads(stdout)
            assert observed["outcome"] == "matched" and observed["reading"]["raw_value"] == "1"
        elif diagnostics:
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


def verify_layout_condition_plan(cli, output):
    output.mkdir()
    original = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    marker = '<ProcessData id="PD">'
    assert original.count(marker) == 1
    source = output / "conditional.xml"
    source.write_text(original.replace(marker, marker + '<Condition variableId="V_SIM_Threshold" value="1"/>'),
                      encoding="utf-8")
    profile = output / "conditional.toml"
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR", "--layout", "IN")
    before = profile.read_bytes()
    plan = tomllib.loads(run(cli, "check-layout", profile))
    assert plan["layout"] == "IN" and plan["parameter_id"] == "V_SIM_Threshold"
    assert (plan["index"], plan["subindex"], plan["read_subindex"], plan["expected"]) == (64, 0, 0, 1)
    observation = tomllib.loads(run(cli, "check-layout", profile, "--read", expected=1))
    assert observation["plan"] == plan and observation["outcome"] == "mismatch"
    assert observation["reading"]["raw_value"] == "500"
    assert observation["observed"]["port"] == 1 and profile.read_bytes() == before
    return profile


def verify_layout_condition_reads(cli, source, fixture, endpoint, output):
    output.mkdir()
    live = output / "conditional.toml"
    run(cli, "set-connection", source, live, "--gateway", "ifm-al1350", "--endpoint", endpoint,
        "--expected-mode", "3", "--firmware", "3.1.42", "--poll-us", "20000")
    original = live.read_bytes()
    profile = original.decode("utf-8")
    assert tomllib.loads(profile)["confirm_iodd_condition"] is False
    before = Gateway.requests
    plan = tomllib.loads(run(cli, "check-layout", live))
    assert plan["declaration"] == "V_SIM_Threshold = 1"
    assert Gateway.requests == before

    invalid = output / "unresolved.toml"
    bad = profile.replace('condition = "V_SIM_Threshold = 1"', 'condition = "V_Missing = 1"')
    assert bad != profile
    invalid.write_text(bad, encoding="utf-8")
    run(cli, "check-layout", invalid, "--read", expected=1)
    write_only = output / "write-only.xml"
    text = source.with_name("conditional.xml").read_text(encoding="utf-8")
    text = text.replace('index="64" accessRights="rw"', 'index="64" accessRights="wo"')
    write_only.write_text(text, encoding="utf-8")
    blocked = output / "write-only.toml"
    run(cli, "import", write_only, blocked, "--variant", "SIM-SENSOR", "--layout", "IN")
    blocked_live = output / "write-only-live.toml"
    run(cli, "set-connection", blocked, blocked_live, "--gateway", "ifm-al1350", "--endpoint", endpoint,
        "--expected-mode", "3", "--firmware", "3.1.42")
    run(cli, "check-layout", blocked_live, "--read", expected=1)
    assert Gateway.requests == before

    old_threshold, old_id, old_faults = Gateway.threshold_payload, Gateway.device_id, Gateway.inject_faults
    try:
        for payload, status, raw, code in [
            ("0001", "matched", "1", 0), ("0002", "mismatch", "2", 1), ("FF", "unavailable", None, 1),
        ]:
            Gateway.threshold_payload = payload
            reads = Gateway.isdu_reads
            observed = tomllib.loads(run(cli, "check-layout", live, "--read", expected=code))
            assert observed["plan"] == plan and observed["outcome"] == status
            assert observed["reading"]["raw_hex"] == payload and observed["reading"].get("raw_value") == raw
            assert observed["observed"]["model"] == "AL1350" and observed["observed"]["port"] == 1
            assert Gateway.isdu_reads == reads + 1
            if status == "matched":
                assert any(display["value"] == "2.1" for display in observed["reading"]["displays"])
            if status == "unavailable":
                assert observed["reading"]["decode_error"]

        before = Gateway.requests
        run(cli, "capture", live, output / "unconfirmed", "--seconds", ".25", expected=1)
        assert Gateway.requests == before
        reads = Gateway.isdu_reads
        Gateway.device_id = 99
        run(cli, "check-layout", live, "--read", expected=1)
        assert Gateway.isdu_reads == reads
        Gateway.device_id = old_id
        Gateway.threshold_payload = "0001"
        parallel_parameter_read(cli, fixture, output / "parallel", layout_fixture=live)
        assert not Gateway.violations and live.read_bytes() == original
    finally:
        Gateway.threshold_payload, Gateway.device_id, Gateway.inject_faults = old_threshold, old_id, old_faults


def verify_whole_layout_conditions(cli, endpoint, output):
    output.mkdir()
    for kind, selector, index, subindex, condition, expected in [
        ("record", "V_SIM_Record", 66, 2, 1, "true"),
        ("array", "V_SIM_History", 67, 1, 2, "2"),
    ]:
        source = output / f"{kind}.xml"
        xml = (ROOT / "samples" / f"sensor-{kind}.xml").read_text(encoding="utf-8")
        marker = '<ProcessData id="PD">'
        assert xml.count(marker) == 1 and 'subindexAccessSupported="true"' in xml
        xml = xml.replace('subindexAccessSupported="true"', 'subindexAccessSupported="false"')
        xml = xml.replace(marker, marker + f'<Condition variableId="{selector}" subindex="{subindex}" value="{condition}"/>')
        source.write_text(xml, encoding="utf-8")
        profile = output / f"{kind}.toml"
        run(cli, "import", source, profile, "--variant", "SIM-SENSOR", "--layout", "IN")
        live = output / f"{kind}-live.toml"
        run(cli, "set-connection", profile, live, "--gateway", "ifm-al1350", "--endpoint", endpoint,
            "--expected-mode", "3", "--firmware", "3.1.42")
        saved = live.read_bytes()
        requests = Gateway.requests
        plan = tomllib.loads(run(cli, "check-layout", live))
        assert (plan["index"], plan["subindex"], plan["read_subindex"]) == (index, subindex, 0)
        run(cli, "read-parameter", live, selector, "--subindex", subindex, expected=1)
        assert Gateway.requests == requests
        raw = getattr(Gateway, f"{kind}_payload")
        reads = Gateway.isdu_reads
        result = tomllib.loads(run(cli, "check-layout", live, "--read"))
        assert result["outcome"] == "matched" and result["selector_raw_value"] == expected
        assert result["reading"]["subindex"] == 0 and result["reading"]["raw_hex"] == raw
        assert result["plan"] == plan and Gateway.isdu_reads == reads + 1
        if kind == "record":
            try:
                Gateway.record_payload = "FFEC000A54455354"
                mismatch = tomllib.loads(run(cli, "check-layout", live, "--read", expected=1))
                assert mismatch["outcome"] == "mismatch" and mismatch["selector_raw_value"] == "false"
                assert mismatch["reading"]["subindex"] == 0
                Gateway.record_payload = "FFEC01"
                unavailable = tomllib.loads(run(cli, "check-layout", live, "--read", expected=1))
                assert unavailable["outcome"] == "unavailable" and "selector_raw_value" not in unavailable
                assert unavailable["reading"]["raw_hex"] == "FFEC01" and unavailable["reading"]["decode_error"]
                assert Gateway.isdu_reads == reads + 3
            finally:
                Gateway.record_payload = raw
        assert live.read_bytes() == saved and not Gateway.violations


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


def verify_padded_integer_input(cli, output):
    output.mkdir()
    source = output / "uint17.xml"
    original = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    old_input = '<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>'
    new_input = ('<ProcessDataIn id="IN" bitLength="17"><Datatype xsi:type="UIntegerT" bitLength="17"/>'
                 '<Name textId="integer"/></ProcessDataIn>')
    assert original.count(old_input) == 1
    source.write_text(original.replace(old_input, new_input), encoding="utf-8")
    layout = tomllib.loads(run(cli, "iodd", source))["layouts"][0]
    assert layout["padded_input_bit_length"] == 17 and layout["input_bytes"] == 4
    assert layout["fields"][0]["bits"] == 17
    profile = output / "uint17.toml"
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR")
    saved = tomllib.loads(profile.read_text(encoding="utf-8"))
    assert saved["input_bytes"] == saved["iodd"]["input_bytes"] == 4
    assert saved["iodd"]["padded_input_bit_length"] == 17
    assert "17 value bits in 4 wire bytes" in run(cli, "validate", profile)
    for payload, warning in (("00016E5A", False), ("80016E5A", True)):
        result = subprocess.run([str(cli), "decode", str(profile), payload], cwd=ROOT,
                                capture_output=True, encoding="utf-8", timeout=15)
        assert result.returncode == 0 and "raw 93786" in result.stdout
        assert ("noncanonical padding" in result.stderr) == warning
    rejected = subprocess.run([str(cli), "decode", str(profile), "016E5A"], cwd=ROOT,
                              capture_output=True, encoding="utf-8", timeout=15)
    assert rejected.returncode == 1 and "expected 4 bytes" in rejected.stderr and not rejected.stdout
    return profile


def verify_standalone_boolean_input(cli, output):
    output.mkdir()
    source = output / "boolean.xml"
    original = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    old_input = '<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>'
    datatype = ('<Datatype xsi:type="BooleanT">'
                '<SingleValue value="1"><Name textId="nominal"/></SingleValue></Datatype>')
    new_input = (f'<ProcessDataIn id="IN" bitLength="1">{datatype}'
                 '<Name textId="switch"/></ProcessDataIn>'
                 f'<ProcessDataOut id="OUT" bitLength="1">{datatype}</ProcessDataOut>')
    assert original.count(old_input) == 1
    source.write_text(original.replace(old_input, new_input), encoding="utf-8")
    profile = output / "boolean.toml"
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR")
    saved = tomllib.loads(profile.read_text(encoding="utf-8"))
    assert saved["input_bytes"] == saved["iodd"]["input_bytes"] == 1
    assert [(field["kind"], field["bits"]) for field in saved["iodd"]["fields"]] == [("bool", 8)]
    for payload, value, warning in (("00", "false", False), ("FF", "true", False), ("02", "true", True)):
        result = subprocess.run([str(cli), "decode", str(profile), payload], cwd=ROOT,
                                capture_output=True, encoding="utf-8", timeout=15)
        assert result.returncode == 0 and f"raw {value}" in result.stdout
        assert ("noncanonical byte" in result.stderr) == warning
        assert ("Nominal setting (true)" in result.stdout) == (value == "true")
        decoded = json.loads(run(cli, "decode-output", profile, payload))["iodd_output"]
        observed = decoded["fields"][0]["value"]
        assert observed["value"] == value and observed["raw"] == value
        assert ("value_name" in observed) == (value == "true")
        if value == "true":
            assert observed["value_name"] == "Nominal setting"
        assert ("encoding_warning" in decoded) == warning
    unit_only = output / "unit-override.toml"
    run(cli, "set-field", profile, unit_only, "switch", "--unit", "state")
    assert "Nominal setting (true)" in run(cli, "decode", unit_only, "FF")
    kind_override = output / "wire-override.toml"
    run(cli, "set-field", unit_only, kind_override, "switch", "--bits", 1)
    assert "Nominal setting" not in run(cli, "decode", kind_override, "01")
    restored = output / "restored.toml"
    run(cli, "remove-field", kind_override, restored, "switch")
    assert "Nominal setting (true)" in run(cli, "decode", restored, "FF")
    return profile


def verify_process_float_names(cli, output):
    output.mkdir()
    original = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    old_input = '<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>'
    choice = '<SingleValue value="1.250e+0"><Name textId="nominal"/></SingleValue>'
    cyclic = (
        f'<ProcessDataIn id="IN" bitLength="32" gradient="2" displayFormat="Dec.2">'
        f'<Datatype xsi:type="Float32T">{choice}</Datatype><Name textId="level"/></ProcessDataIn>'
        f'<ProcessDataOut id="OUT" bitLength="32"><Datatype xsi:type="RecordT" bitLength="32">'
        f'<RecordItem subindex="1" bitOffset="0"><SimpleDatatype xsi:type="Float32T">{choice}</SimpleDatatype>'
        '<Name textId="switch"/></RecordItem></Datatype></ProcessDataOut>'
    )
    assert original.count(old_input) == 1
    source = output / "float.xml"
    source.write_text(original.replace(old_input, cyclic), encoding="utf-8")
    profile = output / "float.toml"
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR")
    pinned = profile.read_bytes()
    lang = output / "german.toml"
    run(cli, "import", source, lang, "--variant", "SIM-SENSOR", "--language", "de",
        "--language-file", ROOT / "samples/sensor-de.xml")
    assert "Nominal setting (2.50)" in run(cli, "decode", profile, "3FA00000")
    assert "Nennwert (2.50)" in run(cli, "decode", lang, "3FA00000")
    for payload in ("3FA00001", "3F9FFFFF", "7F800000", "7FC00001"):
        assert "Nominal setting" not in run(cli, "decode", profile, payload)
    fields = json.loads(run(cli, "decode-output", lang, "3FA00000"))["iodd_output"]["fields"]
    assert fields[0]["value"] == {
        "value": "1.25", "raw": "1.25", "bits_hex": "3FA00000", "value_name": "Nennwert",
    }
    override = output / "scaled.toml"
    run(cli, "set-field", profile, override, "level", "--scale", "4")
    assert "Nominal setting (5.00)" in run(cli, "decode", override, "3FA00000")
    swapped = output / "swapped.toml"
    run(cli, "set-field", profile, swapped, "level", "--byte-order", "little-endian")
    assert "Nominal setting" not in run(cli, "decode", swapped, "0000A03F")
    restored = output / "restored.toml"
    run(cli, "remove-field", swapped, restored, "level")
    assert "Nominal setting (2.50)" in run(cli, "decode", restored, "3FA00000")
    assert profile.read_bytes() == pinned
    return profile


def verify_process_range_names(cli, output):
    output.mkdir()
    original = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    old_input = '<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>'
    cyclic = (
        '<ProcessDataIn id="IN" bitLength="32" gradient="2" displayFormat="Dec.2">'
        '<Datatype xsi:type="Float32T"><ValueRange lowerValue="1.25e0" upperValue="2.5">'
        '<Name textId="nominal"/></ValueRange><ValueRange lowerValue="-1" upperValue="-0">'
        '<Name textId="switch"/></ValueRange></Datatype><Name textId="level"/></ProcessDataIn>'
        '<ProcessDataOut id="OUT" bitLength="16"><Datatype xsi:type="IntegerT" bitLength="16">'
        '<ValueRange lowerValue="-04" upperValue="-02"><Name textId="nominal"/></ValueRange>'
        '</Datatype></ProcessDataOut>'
    )
    assert original.count(old_input) == 1
    source = output / "ranges.xml"
    source.write_text(original.replace(old_input, cyclic), encoding="utf-8")
    profile = output / "ranges.toml"
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR")
    pinned = profile.read_bytes()
    lang = output / "german.toml"
    run(cli, "import", source, lang, "--variant", "SIM-SENSOR", "--language", "de",
        "--language-file", ROOT / "samples/sensor-de.xml")
    assert "Nominal setting (2.50)" in run(cli, "decode", profile, "3FA00000")
    assert "Nennwert (5.00)" in run(cli, "decode", lang, "40200000")
    assert "Nominal setting" not in run(cli, "decode", profile, "3F9FFFFF")
    assert "Nominal setting" not in run(cli, "decode", profile, "40200001")
    assert "switch (0.00)" in run(cli, "decode", profile, "00000000")
    negative_zero = run(cli, "decode", profile, "80000000")
    assert "switch (0.00)" in negative_zero and "raw -0" in negative_zero
    for payload, name in (("FFFC", "Nennwert"), ("FFFE", "Nennwert"), ("FFFD", "Nennwert"), ("FFFF", None)):
        observed = json.loads(run(cli, "decode-output", lang, payload))["iodd_output"]["fields"][0]["value"]
        assert observed.get("value_name") == name
    scaled = output / "scaled.toml"
    run(cli, "set-field", profile, scaled, "level", "--scale", "4")
    assert "Nominal setting (5.00)" in run(cli, "decode", scaled, "3FA00000")
    swapped = output / "swapped.toml"
    run(cli, "set-field", profile, swapped, "level", "--byte-order", "little-endian")
    assert "Nominal setting" not in run(cli, "decode", swapped, "0000A03F")
    restored = output / "restored.toml"
    run(cli, "remove-field", swapped, restored, "level")
    assert "Nominal setting (2.50)" in run(cli, "decode", restored, "3FA00000")
    assert profile.read_bytes() == pinned
    return profile


def verify_cyclic_display_formats(cli, output):
    output.mkdir()
    original = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    old_input = '<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>'
    new_process = (
        '<ProcessDataIn id="IN" bitLength="16"><Datatype xsi:type="IntegerT" bitLength="16">'
        '<SingleValue value="-4"><Name textId="nominal"/></SingleValue></Datatype>'
        '<Name textId="level"/></ProcessDataIn>'
        '<ProcessDataOut id="OUT" bitLength="16"><Datatype xsi:type="RecordT" bitLength="16">'
        '<RecordItem subindex="1" bitOffset="4"><SimpleDatatype xsi:type="IntegerT" bitLength="5"/></RecordItem>'
        '<RecordItem subindex="2" bitOffset="0"><SimpleDatatype xsi:type="BooleanT"/></RecordItem>'
        '</Datatype></ProcessDataOut>'
    )
    displays = (
        '<ProcessDataRefCollection>'
        '<ProcessDataRef processDataId="IN"><ProcessDataInfo displayFormat="Hex"/></ProcessDataRef>'
        '<ProcessDataRef processDataId="OUT"><ProcessDataRecordItemInfo subindex="1" displayFormat="Bin"/>'
        '<ProcessDataRecordItemInfo subindex="2" displayFormat="Dec"/></ProcessDataRef>'
        '</ProcessDataRefCollection>'
    )
    assert original.count(old_input) == 1 and original.count("<UserInterface>") == 1
    xml = original.replace(old_input, new_process).replace("<UserInterface>", "<UserInterface>" + displays)
    source = output / "process-formats.xml"
    source.write_text(xml, encoding="utf-8")
    profile = output / "formats.toml"
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR")
    pinned = profile.read_bytes()
    saved = tomllib.loads(pinned.decode("utf-8"))
    assert saved["iodd"]["fields"][0]["display_format"] == "hexadecimal"
    assert not saved["iodd"].get("needs_input")
    assert "Nominal setting (FFFCh)" in run(cli, "decode", profile, "FFFC")
    observed = json.loads(run(cli, "decode-output", profile, "01C1"))["iodd_output"]["fields"]
    assert [entry["field"]["display_format"] for entry in observed] == ["binary", "boolean_decimal"]
    integer, boolean = [entry["value"] for entry in observed]
    assert integer == {"bits_hex": "1C", "raw": "-4", "value": "-4", "formatted_value": "1111 1100b"}
    assert boolean == {"bits_hex": "1", "raw": "true", "value": "true", "formatted_value": "1"}
    scaled = output / "scaled.toml"
    run(cli, "set-field", profile, scaled, "level", "--scale", "2")
    assert "Nominal setting (-8)" in run(cli, "decode", scaled, "FFFC")
    restored = output / "restored.toml"
    run(cli, "remove-field", scaled, restored, "level")
    assert "Nominal setting (FFFCh)" in run(cli, "decode", restored, "FFFC")
    assert profile.read_bytes() == pinned

    conflict = output / "conflict.xml"
    conflict.write_text(xml.replace('displayFormat="Hex"', 'displayFormat="Hex" gradient="1"'), encoding="utf-8")
    fallback = output / "fallback.toml"
    run(cli, "import", conflict, fallback, "--variant", "SIM-SENSOR")
    rejected = tomllib.loads(fallback.read_text(encoding="utf-8"))
    assert any("ignored Hex display" in note for note in rejected["iodd"]["needs_input"])
    assert "display_format" not in rejected["iodd"]["fields"][0]
    assert "Nominal setting (-4)" in run(cli, "decode", fallback, "FFFC")
    return profile


def verify_numeric_process_capture(cli, profile, endpoint, output, payload, value):
    output.mkdir()
    live = output / "loopback.toml"
    run(cli, "set-connection", profile, live, "--gateway", "ifm-al1350", "--endpoint", endpoint,
        "--expected-mode", "3", "--firmware", "3.1.42", "--poll-us", "20000")
    old_payload, old_faults = Gateway.payload, Gateway.inject_faults
    try:
        Gateway.payload, Gateway.inject_faults = payload, False
        captured = output / "typed"
        _, rows = capture(cli, live, captured, "--seconds", ".35")
        assert rows and all(row["raw_hex"] == payload and row["value.level"] == value for row in rows)
        verified = tomllib.loads(run(cli, "verify-capture", captured))
        assert verified["rows_checked"] == len(rows)
    finally:
        Gateway.payload, Gateway.inject_faults = old_payload, old_faults


def verify_process_octets(cli, output):
    output.mkdir()
    original = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    old_input = '<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>'
    new_input = (
        '<ProcessDataIn id="IN" bitLength="256"><Datatype xsi:type="OctetStringT" fixedLength="32"/>'
        '<Name textId="octets"/></ProcessDataIn>'
        '<ProcessDataOut id="OUT" bitLength="32"><Datatype xsi:type="OctetStringT" fixedLength="4"/>'
        '<Name textId="output"/></ProcessDataOut>'
    )
    assert original.count(old_input) == 1
    source = output / "process-octets.xml"
    source.write_text(original.replace(old_input, new_input), encoding="utf-8")
    profile = output / "octets.toml"
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR")
    saved = tomllib.loads(profile.read_text(encoding="utf-8"))
    assert saved["input_bytes"] == 32
    assert [(field["kind"], field["bits"]) for field in saved["iodd"]["fields"]] == [("octet_string", 256)]
    payload = bytes([0, 128, 255, *range(28), 0])
    rendered = ",".join(f"0x{byte:02X}" for byte in payload)
    assert rendered in run(cli, "decode", profile, payload.hex())
    run(cli, "decode", profile, payload[:-1].hex(), expected=1)
    observed = json.loads(run(cli, "decode-output", profile, "0080FF00"))
    assert observed["iodd_output"]["fields"][0]["value"]["value"] == "0x00,0x80,0xFF,0x00"
    manual = output / "manual.toml"
    run(cli, "set-field", profile, manual, "Manual bytes", "--kind", "octet-string",
        "--bits", 256, "--bit-offset", 0)
    assert rendered in run(cli, "decode", manual, payload.hex())
    rejected = output / "misaligned.toml"
    run(cli, "set-field", manual, rejected, "Manual bytes", "--bits", 248, "--bit-offset", 1, expected=1)
    assert not rejected.exists()
    return profile, payload.hex().upper(), rendered


def verify_process_octet_capture(cli, fixture, endpoint, output):
    profile, payload, rendered = fixture
    output.mkdir()
    live = output / "loopback.toml"
    run(cli, "set-connection", profile, live, "--gateway", "ifm-al1350", "--endpoint", endpoint,
        "--expected-mode", "3", "--firmware", "3.1.42", "--poll-us", "20000")
    old_payload, old_faults = Gateway.payload, Gateway.inject_faults
    Gateway.inject_faults = False
    try:
        Gateway.payload = payload
        typed = output / "typed"
        report, rows = capture(cli, live, typed, "--seconds", ".3")
        assert report["row_kind"] == "host_poll" and report["source_update_loss"] == "unknown"
        assert rows and all(row["raw_hex"] == payload and row["value.octets"] == rendered for row in rows)
        raw = output / "raw"
        _, raw_rows = capture(cli, live, raw, "--seconds", ".3", "--raw-only")
        assert raw_rows and all(row["raw_hex"] == payload and "value.octets" not in row for row in raw_rows)
        before = Gateway.requests
        stored = tomllib.loads(run(cli, "verify-capture", typed))
        assert stored["valid_rows"] == len(rows)
        export = output / "export"
        receipt = tomllib.loads(run(cli, "redecode-capture", raw, profile, export))
        checked = tomllib.loads(run(cli, "verify-export", export, "--source", raw))
        assert checked["original_source_reverified"] and checked["source_capture"] == receipt["source_capture"]
        with (export / "data.csv").open(encoding="utf-8", newline="") as stream:
            assert all(row["raw_hex"] == payload and row["value.octets"] == rendered
                       for row in csv.DictReader(stream))
        assert Gateway.requests == before
        Gateway.payload = payload[:-2]
        _, short = capture(cli, live, output / "short", "--seconds", ".3")
        assert short and all(row["raw_hex"] == Gateway.payload and row["validity"] == "invalid"
                             and not row["value.octets"] for row in short)
    finally:
        Gateway.payload, Gateway.inject_faults = old_payload, old_faults


def verify_process_text(cli, output):
    output.mkdir()
    original = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    old = '<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>'
    layout = (
        '<ProcessDataIn id="IN" bitLength="56"><Datatype xsi:type="RecordT" bitLength="56">'
        '<RecordItem subindex="1" bitOffset="8"><SimpleDatatype xsi:type="StringT" fixedLength="6" encoding="UTF-8"/>'
        '<Name textId="label"/></RecordItem><RecordItem subindex="2" bitOffset="0"><SimpleDatatype xsi:type="BooleanT"/>'
        '<Name textId="flag"/></RecordItem></Datatype><Name textId="record"/></ProcessDataIn>'
        '<ProcessDataOut id="OUT" bitLength="32"><Datatype xsi:type="StringT" fixedLength="4" encoding="US-ASCII"/>'
        '<Name textId="output"/></ProcessDataOut>'
    )
    assert original.count(old) == 1
    source = output / "process-text.xml"
    source.write_text(original.replace(old, layout), encoding="utf-8")
    profile = output / "text.toml"
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR")
    assert '"Café"' in run(cli, "decode", profile, "436166C3A90001")
    decoded = run(cli, "decode", profile, "41004200000001")
    assert "Invalid StringT (nonzero string padding follows the first NUL)" in decoded
    assert "flag [2]" in decoded and "raw true" in decoded
    def output_value(payload):
        inspection = json.loads(run(cli, "decode-output", profile, payload))
        return inspection["iodd_output"]["fields"][0]["value"]

    assert output_value("41424300")["value"] == '"ABC"'
    assert output_value("C3A90000")["encoding_error"] == "text is not US-ASCII"
    override = output / "ascii.toml"
    run(cli, "set-field", profile, override, "label [1]", "--kind", "ascii-string")
    assert "text is not US-ASCII" in run(cli, "decode", override, "436166C3A90001")
    return profile


def verify_process_text_capture(cli, profile, endpoint, output):
    output.mkdir()
    fixture = output / "loopback.toml"
    run(cli, "set-connection", profile, fixture, "--gateway", "ifm-al1350", "--endpoint", endpoint,
        "--expected-mode", "3", "--firmware", "3.1.42", "--poll-us", "20000")
    old_payload, old_faults = Gateway.payload, Gateway.inject_faults
    Gateway.inject_faults = False
    invalid = "Invalid StringT (nonzero string padding follows the first NUL)"
    try:
        for name, payload, validity, text, flag in [
            ("utf8", "436166C3A90001", "valid", '"Café"', "true"),
            ("empty", "00000000000001", "valid", '""', "true"),
            ("malformed", "41004200000001", "valid", invalid, "true"),
            ("bad-utf8", "C3000000000001", "valid", "Invalid StringT (invalid UTF-8 at byte 0)", "true"),
            ("short", "436166C3A900", "invalid", "", ""),
        ]:
            Gateway.payload = payload
            report, rows = capture(cli, fixture, output / name, "--seconds", ".3")
            assert report["row_kind"] == "host_poll" and report["source_update_loss"] == "unknown"
            assert rows and all(row["raw_hex"] == payload and row["validity"] == validity
                                and row["value.label [1]"] == text and row["value.flag [2]"] == flag for row in rows)

        Gateway.payload = "41004200000001"
        raw = output / "raw"
        _, rows = capture(cli, fixture, raw, "--seconds", ".3", "--raw-only")
        before = Gateway.requests
        export = output / "export"
        receipt = tomllib.loads(run(cli, "redecode-capture", raw, profile, export))
        verification = tomllib.loads(run(cli, "verify-export", export, "--source", raw))
        assert verification["original_source_reverified"] and receipt["typed_rows"] == len(rows)
        with (export / "data.csv").open(encoding="utf-8", newline="") as stream:
            assert all(row["raw_hex"] == Gateway.payload and row["value.label [1]"] == invalid
                       and row["value.flag [2]"] == "true" for row in csv.DictReader(stream))
        assert Gateway.requests == before
    finally:
        Gateway.payload, Gateway.inject_faults = old_payload, old_faults


def verify_process_temporal(cli, output):
    output.mkdir()
    original = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    old = '<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>'
    layout = (
        '<ProcessDataIn id="IN" bitLength="128"><Datatype xsi:type="RecordT" bitLength="128">'
        '<RecordItem subindex="1" bitOffset="64"><SimpleDatatype xsi:type="TimeT"/>'
        '<Name textId="clock"/></RecordItem><RecordItem subindex="2" bitOffset="0"><SimpleDatatype xsi:type="TimeSpanT"/>'
        '<Name textId="duration"/></RecordItem></Datatype><Name textId="record"/></ProcessDataIn>'
        '<ProcessDataOut id="OUT" bitLength="64"><Datatype xsi:type="TimeSpanT"/><Name textId="output"/></ProcessDataOut>'
    )
    assert original.count(old) == 1
    source = output / "process-temporal.xml"
    source.write_text(original.replace(old, layout), encoding="utf-8")
    profile = output / "temporal.toml"
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR")
    first = run(cli, "decode", profile, "9DFF440080000000FFFFFFFFFFFFFFFF")
    later = run(cli, "decode", profile, "9DFF4400800000018000000000000000")
    assert "1984-01-01 00:00:00.500 UTC" in first and "1984-01-01 00:00:00.500 UTC" in later
    assert f"raw {int('9DFF440080000000', 16)}" in first and "raw -1" in first
    assert f"raw {int('9DFF440080000001', 16)}" in later and "raw -9223372036854775808" in later
    observed = json.loads(run(cli, "decode-output", profile, "FFFFFFFFFFFFFFFF"))
    assert observed["iodd_output"]["fields"][0]["value"]["raw"] == "-1"
    assert observed["iodd_output"]["fields"][0]["value"]["value"] == "-00:00:00.000"
    manual = output / "manual.toml"
    run(cli, "set-field", profile, manual, "Manual duration", "--kind", "time-span",
        "--bits", 64, "--bit-offset", 0)
    assert "Manual duration" in run(cli, "decode", manual, "9DFF440080000000FFFFFFFFFFFFFFFF")
    rejected = output / "invalid.toml"
    run(cli, "set-field", manual, rejected, "Manual duration", "--byte-order", "little-endian", expected=1)
    assert not rejected.exists()
    return profile


def verify_process_temporal_capture(cli, profile, endpoint, output):
    output.mkdir()
    fixture = output / "loopback.toml"
    run(cli, "set-connection", profile, fixture, "--gateway", "ifm-al1350", "--endpoint", endpoint,
        "--expected-mode", "3", "--firmware", "3.1.42", "--poll-us", "20000")
    old_payload, old_faults = Gateway.payload, Gateway.inject_faults
    Gateway.inject_faults = False
    clock, duration = "value.clock [1]", "value.duration [2]"
    try:
        for name, payload, valid in [
            ("first", "9DFF440080000000FFFFFFFFFFFFFFFF", True),
            ("next-tick", "9DFF4400800000018000000000000000", True),
            ("extreme", "FFFFFFFFFFFFFFFF7FFFFFFFFFFFFFFF", True),
            ("short", "9DFF440080000000FFFFFFFFFFFFFF", False),
        ]:
            Gateway.payload = payload
            folder = output / name
            report, rows = capture(cli, fixture, folder, "--seconds", ".3")
            assert report["row_kind"] == "host_poll" and report["source_update_loss"] == "unknown"
            expected_clock = str(int(payload[:16], 16)) if valid else ""
            expected_duration = str(int.from_bytes(bytes.fromhex(payload[16:]), "big", signed=True)) if valid else ""
            assert rows and all(row["raw_hex"] == payload and row["validity"] == ("valid" if valid else "invalid")
                                and row[clock] == expected_clock and row[duration] == expected_duration
                                and not row["source_time_us"] for row in rows)
            review = tomllib.loads(run(cli, "review-capture", folder, "--field", "duration [2]"))["signal"]
            assert review["temporal_values"] == (len(rows) if valid else 0)
            assert review["numeric"].get("last_csv_value") == (expected_duration if valid else None)

        Gateway.payload = "9DFF4400800000018000000000000000"
        raw = output / "raw"
        _, rows = capture(cli, fixture, raw, "--seconds", ".3", "--raw-only")
        before = Gateway.requests
        export = output / "export"
        receipt = tomllib.loads(run(cli, "redecode-capture", raw, profile, export))
        verified = tomllib.loads(run(cli, "verify-export", export, "--source", raw))
        assert verified["original_source_reverified"] and receipt["typed_rows"] == len(rows)
        with (export / "data.csv").open(encoding="utf-8", newline="") as stream:
            assert all(row["raw_hex"] == Gateway.payload and row[clock] == str(int(Gateway.payload[:16], 16))
                       and row[duration] == "-9223372036854775808" for row in csv.DictReader(stream))
        assert Gateway.requests == before
    finally:
        Gateway.payload, Gateway.inject_faults = old_payload, old_faults


def verify_standalone_input_capture(cli, profile, endpoint, output, field, metric, cases, manual_bits):
    output.mkdir()
    input_profile = profile.read_bytes()
    fixture = output / "loopback.toml"
    run(cli, "set-connection", profile, fixture, "--gateway", "ifm-al1350", "--endpoint", endpoint,
        "--expected-mode", "3", "--firmware", "3.1.42", "--poll-us", "20000")
    old_payload, old_faults = Gateway.payload, Gateway.inject_faults
    Gateway.inject_faults = False
    try:
        for name, payload, validity, value in cases:
            Gateway.payload = payload
            path = output / name
            report = tomllib.loads(run(cli, "capture", fixture, path, "--seconds", ".3"))
            assert report["row_kind"] == "host_poll" and report["source_update_loss"] == "unknown"
            with (path / "capture-0001.csv").open(encoding="utf-8", newline="") as stream:
                rows = list(csv.DictReader(stream))
            assert rows and all(row["raw_hex"] == payload and row["validity"] == validity
                                and row[f"value.{field}"] == value for row in rows)
            assert report["csv_rows_flushed"] == len(rows)
            expect = len(rows) if name == "noncanonical" else 0
            before = Gateway.requests
            verified = tomllib.loads(run(cli, "verify-capture", path))
            assert verified["rows_checked"] == len(rows)
            assert verified[metric] == expect
            interface = tomllib.loads(run(cli, "review-interfaces", path))
            assert interface["capture"][metric] == expect
            if name == "noncanonical":
                export = output / "redecoded"
                mapping = output / "manual-mapping.toml"
                mapping.write_text(
                    f'version = 1\nname = "Manual mapping"\ninput_bytes = {len(payload) // 2}\n'
                    f'[[fields]]\nname = "Manual"\nkind = "unsigned"\nbits = {manual_bits}\n',
                    encoding="utf-8",
                )
                receipt = tomllib.loads(run(cli, "redecode-capture", path, mapping, export))
                assert receipt["source_capture"][metric] == expect
                checked = tomllib.loads(run(cli, "verify-export", export, "--source", path))
                assert checked["original_source_reverified"] and checked["source_capture"] == receipt["source_capture"]
                with (export / "data.csv").open(encoding="utf-8", newline="") as stream:
                    manual_value = str(int(payload, 16) & ((1 << manual_bits) - 1))
                    assert all(row["raw_hex"] == payload and row["value.Manual"] == manual_value
                               for row in csv.DictReader(stream))
            assert Gateway.requests == before
    finally:
        Gateway.payload, Gateway.inject_faults = old_payload, old_faults
    assert profile.read_bytes() == input_profile


IFM_PORT_COUNTS = {"AL1350": 4, "AL1351": 4, "AL1352": 8, "AL1353": 8}


def verify_port_statuses_and_captures(cli, output):
    output.mkdir()
    source = (ROOT / "samples/simulator.toml").read_text(encoding="utf-8")

    def profile(name, port, origin=None, firmware=None, gateway="ifm_al1350"):
        unit = "microseconds" if gateway == "ifm_al1350" else "milliseconds"
        endpoint = "" if origin is None else (
            f'endpoint = "{origin}"\nifm_mode = 3\nifm_cycle_unit = "{unit}"\n'
        )
        if firmware is not None:
            endpoint += f'ifm_firmware = "{firmware}"\n'
        contents = source.replace("port = 1\n", f"port = {port}\n{endpoint}timeout_ms = 4500\n")
        if origin is not None:
            contents = contents.replace('gateway = "simulator"', f'gateway = "{gateway}"')
        if origin is not None and gateway != "ifm_al1350":
            contents = contents.replace("poll_us = 1000", "poll_us = 20000")
        path = output / f"{name}.toml"
        path.write_text(contents, encoding="utf-8")
        return path

    simulations = [profile("sim-2", 2), profile("sim-1", 1)]
    simulated = tomllib.loads(run(cli, "status-ports", *simulations))
    assert simulated["gateway"] == "simulator"
    assert [entry["port"] for entry in simulated["ports"]] == [2, 1]
    assert all(entry["status"]["route"] == "synthetic simulator" for entry in simulated["ports"])

    def capture_set(destination, profiles, expected=0, *options):
        pinned = {path: path.read_bytes() for path in profiles}
        batch = tomllib.loads(run(cli, "capture-ports", destination, *profiles, *options, expected=expected))
        assert batch == tomllib.loads((destination / "capture-set.toml").read_text(encoding="utf-8"))
        assert batch["format_version"] == 1 and not batch["stopped_by_request"]
        assert {path: path.read_bytes() for path in profiles} == pinned
        assert [entry["port"] for entry in batch["ports"]] == [
            tomllib.loads(content.decode("utf-8"))["connection"]["port"] for content in pinned.values()
        ]
        return batch

    def port_capture(destination, entry):
        path = destination / entry["directory"]
        assert entry["directory"] == f"port-{entry['port']}" and entry["report_saved"]
        report = tomllib.loads((path / "capture.toml").read_text(encoding="utf-8"))
        assert entry["received"] == report["counts"]["received"] and entry["csv_rows_flushed"] == report["csv_rows_flushed"]
        rows = []
        for part in sorted(path.glob("capture-*.csv")):
            with part.open(encoding="utf-8", newline="") as stream:
                rows.extend(csv.DictReader(stream))
        assert len(rows) == report["csv_rows_flushed"]
        if entry["success"]:
            verified = tomllib.loads(run(cli, "verify-capture", path))
            assert verified["rows_checked"] == len(rows) and report["csv_rows_unconfirmed"] == 0
            assert report["counts"]["logger_overruns"] == verified["missing_sample_indexes"]
        return report, rows

    slow, fast = simulations
    slow.write_text(slow.read_text(encoding="utf-8")
                    .replace("poll_us = 1000", "poll_us = 30000\nsimulate_snapshots = true")
                    .replace('[logging]\n', '[logging]\nqueue_capacity = 128\n'), encoding="utf-8")
    fast.write_text(fast.read_text(encoding="utf-8")
                    .replace('[logging]\n', '[logging]\nqueue_capacity = 4\nrotate_rows = 7\n'), encoding="utf-8")
    destination = output / "simulator-set"
    batch = capture_set(destination, simulations, 0, "--seconds", ".62", "--simulate-faults",
                        "--simulate-writer-delay-ms", "15")
    assert batch["gateway"] == "simulator" and not batch["observed_master_conflict"]
    delayed, delayed_rows = port_capture(destination, batch["ports"][0])
    pressured, pressured_rows = port_capture(destination, batch["ports"][1])
    assert delayed_rows and all(row["row_kind"] == "host_poll" for row in delayed_rows)
    assert delayed["source_update_loss"] == "unknown" and "source_gaps_observed" not in delayed
    assert delayed["counts"]["logger_overruns"] == 0 and delayed["requested_poll_us"] == 30000
    assert pressured["counts"]["logger_overruns"] > 0 and pressured["counts"]["stale"] > 0
    assert pressured["counts"]["reconnects"] > 0 and pressured["csv_parts"] > 1
    assert pressured["logger_metrics"]["queue_high_water_observed"] == 4
    assert "source_gaps_observed" in pressured and all(row["row_kind"] == "source_update" for row in pressured_rows)
    assert delayed["last_status"]["route"] == pressured["last_status"]["route"] == "synthetic simulator"

    class PortGateway(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            self.server.requests.append(request)
            assert self.path == "/"
            read_only = self.server.model != "AL1350"
            max_port = IFM_PORT_COUNTS[self.server.model]
            if read_only and request["adr"] == f"/iolinkmaster/port[{max_port}]/iolinkdevice/iolreadacyclic":
                index = request["data"]["index"]
                assert request["data"]["subindex"] == 0 and index in (36, 37, 64)
                self.respond(request, {"value": {36: "02", 37: "E48DFE000000", 64: "01F4"}[index]})
                return
            assert request["adr"] == "/getdatamulti"
            paths = request["data"]["datatosend"]
            port = next(n for n in range(1, max_port + 1) if f"/iolinkmaster/port[{n}]/mode" in paths)
            prefix = f"/iolinkmaster/port[{port}]"
            pdin = prefix + "/iolinkdevice/pdin"
            polling = pdin in paths
            if polling and port in (1, 2) and self.server.poll_barrier is not None and port not in self.server.entered_polls:
                self.server.entered_polls.add(port)
                self.server.poll_barrier.wait(timeout=3)
            if not polling and port in (1, 2) and self.server.barrier is not None:
                self.server.barrier.wait(timeout=3)
            data = dict(Gateway.data)
            data["/deviceinfo/productcode"] = self.server.model
            data[Gateway.port + "/iolinkdevice/deviceid"] = 2 if port == 3 else 1
            data[Gateway.port + "/mastercycletime_actual"] = 1200 * port
            data[Gateway.port + "/mode"] = 0 if port == 4 else 3
            if read_only:
                data["/deviceinfo/revision"] = data.pop("/deviceinfo/swrevision")
                data[Gateway.port + "/iolinkdevice/deviceid"] = 1
                data[Gateway.port + "/mastercycletime_actual"] = port
                data[Gateway.port + "/mode"] = 3
            if port == self.server.different_master_port:
                data["/deviceinfo/serialnumber"] = "OTHER-MASTER"
            allowed = {key.replace(Gateway.port, prefix): value for key, value in data.items()}
            allowed[pdin] = f"{port:04X}" if read_only else {1: "0002", 2: "0005", 3: "0006", 4: "0009"}[port]
            values = {}
            for key in paths:
                if read_only and key == "/deviceinfo/swrevision":
                    values[key] = {"code": 503}
                elif read_only and port == max_port and key == pdin and not self.server.invalid_seen:
                    values[key] = {"code": 530}
                    self.server.invalid_seen = True
                else:
                    values[key] = {"code": 200, "data": allowed[key]}
            self.respond(request, values)

        def respond(self, request, data):
            body = json.dumps({"code": 200, "cid": request["cid"], "data": data}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    class PortServer(ThreadingHTTPServer):
        request_queue_size = 16

    with PortServer(("127.0.0.1", 0), PortGateway) as server:
        server.requests = []
        server.model = "AL1350"
        server.invalid_seen = False
        server.barrier = threading.Barrier(2)
        server.poll_barrier = None
        server.entered_polls = set()
        server.different_master_port = None
        host = f"http://127.0.0.1:{server.server_port}"
        ports = [profile(f"port-{port}", port, host + ("/" if port == 2 else "")) for port in range(1, 5)]
        other_origin = profile("other-origin", 2, "http://127.0.0.1:1")
        conflicting = [profile(f"firmware-{port}", port, host, f"3.1.{port}") for port in (1, 2)]
        worker = threading.Thread(target=server.serve_forever)
        worker.start()
        try:
            for selection in [(ports[0], ports[0]), (ports[0], simulations[0]),
                              (ports[0], other_origin), tuple(conflicting)]:
                run(cli, "status-ports", *selection, expected=1)
                assert not server.requests
            report = tomllib.loads(run(cli, "status-ports", ports[1], ports[0]))
            assert report["gateway"] == "ifm_al1350" and not report["observed_master_conflict"]
            assert len(server.requests) == 2
            assert [entry["port"] for entry in report["ports"]] == [2, 1]
            for entry in report["ports"]:
                assert entry["read_duration_us"] > 0 and entry["host_finished_utc"].endswith("Z")
                assert entry["status"]["route"] == "AL1350 loopback fixture"
                assert entry["status"]["port_cycle_us"] == 1200 * entry["port"]
                assert entry["status"]["freshness"] == "snapshots"
            server.barrier = None
            partial = tomllib.loads(run(cli, "status-ports", ports[2], ports[0], ports[3], expected=1))["ports"]
            assert [entry["port"] for entry in partial] == [3, 1, 4]
            assert "identity mismatch" in partial[0]["error"] and partial[0]["status"]["identity"]["device_id"] == 2
            assert "error" not in partial[1] and "identity_check" in partial[1]
            assert "mode" in partial[2]["error"] and "status" not in partial[2]
            server.different_master_port = 2
            report = tomllib.loads(run(cli, "status-ports", *ports[:2], expected=1))
            assert report["observed_master_conflict"]
            assert len(server.requests) == 7

            before_capture = len(server.requests)
            invalid = output / "must-not-create"
            for selection in [(ports[0], ports[0]), (ports[0], slow), (ports[0], other_origin), tuple(conflicting)]:
                run(cli, "capture-ports", invalid, *selection, "--seconds", ".1", expected=1)
                assert not invalid.exists()
            run(cli, "capture-ports", invalid, *ports[:2], "--seconds", ".1", "--simulate-faults", expected=1)
            assert not invalid.exists()
            existing = output / "already-there"
            existing.mkdir()
            sentinel = existing / "keep.txt"
            sentinel.write_bytes(b"existing user bytes")
            run(cli, "capture-ports", existing, *ports[:2], "--seconds", ".1", expected=1)
            assert list(existing.iterdir()) == [sentinel] and sentinel.read_bytes() == b"existing user bytes"
            assert len(server.requests) == before_capture

            server.different_master_port = None
            server.poll_barrier = threading.Barrier(2)
            destination = output / "gateway-set"
            batch = capture_set(destination, [ports[1], ports[2], ports[0]], 1, "--seconds", ".45")
            assert batch["gateway"] == "ifm_al1350" and not batch["observed_master_conflict"]
            assert [entry["success"] for entry in batch["ports"]] == [True, False, True]
            assert server.entered_polls == {1, 2}
            for entry in batch["ports"]:
                report, rows = port_capture(destination, entry)
                if entry["port"] == 3:
                    assert not rows and "identity mismatch" in report["terminal_error"]
                    assert entry["error"] == report["terminal_error"]
                    assert report["rejected_status"]["identity"]["device_id"] == 2
                else:
                    assert report["last_status"]["route"] == "AL1350 loopback fixture"
                    assert report["last_status"]["port_cycle_us"] == 1200 * entry["port"]
                    assert report["source_update_loss"] == "unknown" and "source_gaps_observed" not in report
                    assert rows and {row["raw_hex"] for row in rows} == {"0002" if entry["port"] == 1 else "0005"}
                    assert all(row["row_kind"] == "host_poll" and not row["source_counter"] for row in rows)

            server.different_master_port = 2
            server.poll_barrier = None
            destination = output / "gateway-conflict"
            batch = capture_set(destination, ports[:2], 1, "--seconds", ".22")
            assert batch["observed_master_conflict"] and all(entry["success"] for entry in batch["ports"])
            for entry in batch["ports"]:
                port_capture(destination, entry)

            server.different_master_port = None
            for model, max_port in IFM_PORT_COUNTS.items():
                if model == "AL1350":
                    continue
                name = model.lower()
                gateway = f"ifm_{name}"
                server.model = model
                server.invalid_seen = False
                new_ports = [profile(f"{name}-{port}", port, host, "3.1.42", gateway)
                             for port in range(1, max_port + 1)]
                out_of_range = profile(f"{name}-out-of-range", max_port + 1, host, "3.1.42", gateway)
                before = len(server.requests)
                run(cli, "status-ports", out_of_range, expected=1)
                run(cli, "status-ports", new_ports[0], ports[1], expected=1)
                assert len(server.requests) == before
                other_model = "ifm_al1351" if gateway == "ifm_al1353" else "ifm_al1353"
                wrong_model = profile(f"{name}-wrong-model", 1, host, "3.1.42", other_model)
                run(cli, "status", wrong_model, expected=1)
                assert len(server.requests) == before + 1
                report = tomllib.loads(run(cli, "status-ports", *reversed(new_ports)))
                assert report["gateway"] == gateway and not report["observed_master_conflict"]
                assert [entry["port"] for entry in report["ports"]] == list(range(max_port, 0, -1))
                for entry in report["ports"]:
                    status = entry["status"]
                    assert status["route"] == f"{model} loopback fixture" and status["firmware"] == "3.1.42"
                    assert status["port_cycle_us"] == 1000 * entry["port"] and status["freshness"] == "snapshots"

                before = len(server.requests)
                run(cli, "write", new_ports[-1], "--master-serial", "FIXTURE-MASTER",
                    "isdu", "--index", 64, "--expected", "01F4", "--desired", "01F5", expected=1)
                run(cli, "read-current", new_ports[-1], "--master-serial", "FIXTURE-MASTER", "port-mode", expected=1)
                assert len(server.requests) == before
                assert "01F4" in run(cli, "isdu", new_ports[-1], 64)
                snapshot = output / f"{name}-port{max_port}-diagnostics.json"
                result = tomllib.loads(run(cli, "read-diagnostics", new_ports[-1], "--snapshot", snapshot))
                assert result["device_status"]["value"] == 2 and result["detailed_status"]["raw_hex"] == "E48DFE000000"
                before = len(server.requests)
                saved = tomllib.loads(run(cli, "review-diagnostics", snapshot))
                assert saved["reads"][1]["observation"]["port"] == max_port and len(server.requests) == before

                destination = output / f"{name}-set"
                batch = capture_set(destination, new_ports, 0, "--seconds", ".9")
                assert batch["gateway"] == gateway and len(batch["ports"]) == max_port
                for entry in batch["ports"]:
                    report, rows = port_capture(destination, entry)
                    assert rows and report["source_update_loss"] == "unknown"
                    assert report["last_status"]["port_cycle_us"] == 1000 * entry["port"]
                    assert all(row["row_kind"] == "host_poll" and not row["source_counter"] for row in rows)
                    valid = {row["raw_hex"] for row in rows if row["validity"] == "valid"}
                    assert valid == {f"{entry['port']:04X}"}
                    if entry["port"] == max_port:
                        assert report["counts"]["invalid"] == 1
        finally:
            server.shutdown()
            worker.join()


def verify_master_inspection(cli, output):
    output.mkdir()
    master_points = {
        "/deviceinfo/productcode": "AL1350",
        "/deviceinfo/swrevision": "3.1.42",
        "/deviceinfo/serialnumber": "INVENTORY-MASTER",
        "/deviceinfo/hwrevision": "AC",
    }
    diagnostics = {
        "/processdatamaster/temperature": -6.5,
        "/processdatamaster/voltage": "24050",
        "/processdatamaster/current": 0,
        "/processdatamaster/supervisionstatus": "0x0004",
    }
    port_points = {
        "mode": "IO-Link", "comspeed": "COM3", "mastercycletime_actual": 1200,
        "portevent": 0, "iolinkdevice/vendorid": 65535, "iolinkdevice/deviceid": 1,
        "iolinkdevice/productname": "Synthetic sensor", "iolinkdevice/serial": "SYNTHETIC",
        "iolinkdevice/status": 0, "iolinkdevice/iolinkevent": 0,
    }

    config_points = {
        "mastercycletime_preset": 0, "validation_datastorage_mode": "0x007F",
        "validation_vendorid": 310, "validation_deviceid": "0x010203",
    }

    class InspectionGateway(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            assert self.path == "/" and request["adr"] == "/getdatamulti"
            paths = request["data"]["datatosend"]
            checking_ports = any(path.startswith("/iolinkmaster/") for path in paths)
            self.server.requests.append(checking_ports)
            assert set(paths) & diagnostics.keys() == (diagnostics.keys() if checking_ports else set())
            revision_alias = self.server.model != "AL1350"
            max_port = IFM_PORT_COUNTS[self.server.model]
            if checking_ports:
                assert {path for path in paths if path.rsplit("/", 1)[-1] in config_points} == {
                    f"/iolinkmaster/port[{port}]/{point}"
                    for port in range(1, max_port + 1) for point in config_points
                }
            data = dict(master_points)
            data["/deviceinfo/productcode"] = self.server.model
            if revision_alias:
                data["/deviceinfo/revision"] = data.pop("/deviceinfo/swrevision")
            values = {}
            for path in paths:
                if path in data:
                    value = data[path]
                    if path == "/deviceinfo/serialnumber" and checking_ports and self.server.changed_serial:
                        value = "CHANGED-MASTER"
                    values[path] = {"code": 200, "data": value}
                elif path in diagnostics:
                    if not self.server.partial_diagnostics:
                        values[path.lstrip("/")] = {"code": 200, "data": diagnostics[path]}
                    elif path.endswith("/temperature"):
                        values[path] = {"code": 200, "data": "NaN"}
                    elif path.endswith("/voltage"):
                        values[path] = {"code": 503, "data": 99999}
                    elif path.endswith("/supervisionstatus"):
                        values[path] = {"code": 200, "data": 0}
                elif revision_alias and path == "/deviceinfo/swrevision":
                    values[path] = {"code": 503}
                else:
                    prefix, point = path.split("]/", 1)
                    assert prefix.startswith("/iolinkmaster/port[")
                    port = int(prefix.rsplit("[", 1)[1])
                    assert 1 <= port <= max_port and point in (port_points.keys() | config_points.keys())
                    if point in config_points:
                        if port == 2:
                            if point == "validation_datastorage_mode":
                                continue
                            unavailable = {
                                "mastercycletime_preset": {"code": 200, "data": "unusable"},
                                "validation_vendorid": {"code": 200, "data": 65536},
                                "validation_deviceid": {"code": 503, "data": 22},
                            }
                            values[path] = unavailable[point]
                            continue
                        value = config_points[point]
                        if point == "mastercycletime_preset" and port == max_port:
                            value = 3 if revision_alias else 3500
                        values[path.lstrip("/")] = {"code": 200, "data": value}
                        continue
                    if port == 2 and point == "comspeed":
                        continue
                    if port == 2 and point.startswith("iolinkdevice/"):
                        values[path.lstrip("/")] = {"code": 503}
                    else:
                        value = port_points[point]
                        if point == "mastercycletime_actual" and revision_alias:
                            value = 2
                        if port == 2 and point == "mode":
                            value = "Disabled"
                        values[path.lstrip("/")] = {"code": 200, "data": value}
            bad_batch = self.server.fail_ports if checking_ports else self.server.fail_master
            status = 503 if bad_batch else 200
            body = json.dumps({"cid": request["cid"], "code": status, "data": values}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    with ThreadingHTTPServer(("127.0.0.1", 0), InspectionGateway) as server:
        server.requests = []
        server.changed_serial = False
        server.fail_ports = False
        server.fail_master = False
        server.partial_diagnostics = False
        endpoint = f"http://127.0.0.1:{server.server_port}"
        worker = threading.Thread(target=server.serve_forever)
        worker.start()
        try:
            for model, length in IFM_PORT_COUNTS.items():
                server.model = model
                expected_cycle = 1200 if model == "AL1350" else 2000
                name = f"ifm-{model.lower()}"
                route = output / f"{name}.toml"
                template = ROOT / "samples" / f"{name}.toml"
                if model in ("AL1351", "AL1353"):
                    unconverted = output / f"{name}-raw-cycle.toml"
                    run(cli, "set-connection", template, unconverted, "--endpoint", endpoint)
                    raw = tomllib.loads(run(cli, "inspect-master", unconverted))
                    assert raw["ports"][-1]["port_cycle_raw"] == "2"
                    assert "port_cycle_us" not in raw["ports"][-1]
                    preset = raw["ports"][-1]["configuration"]
                    assert preset["preset_cycle_raw"] == "3" and not preset["preset_cycle_automatic"]
                    assert "preset_cycle_us" not in preset
                unit = "microseconds" if model == "AL1350" else "milliseconds"
                run(cli, "set-connection", template, route, "--gateway", name, "--endpoint", endpoint,
                    "--port", length, "--cycle-unit", unit, "--firmware", "3.1.42")
                pinned = route.read_bytes()
                before = len(server.requests)
                snapshot = output / f"{name}.json"
                report = tomllib.loads(run(cli, "inspect-master", route, "--snapshot", snapshot))
                assert server.requests[before:] == [False, True]
                original_snapshot = snapshot.read_bytes()
                review = tomllib.loads(run(cli, "review-master", snapshot))
                assert review["format_version"] == 1 and review["workbench_version"]
                assert review["endpoint"] == endpoint and review["cycle_unit"] == unit
                assert review["firmware_pin"] == "3.1.42" and review["inspection"] == report
                run(cli, "inspect-master", route, "--snapshot", snapshot, expected=1)
                assert server.requests[before:] == [False, True]
                assert snapshot.read_bytes() == original_snapshot
                assert report["master"] == {
                    "model": model, "firmware": "3.1.42", "serial": "INVENTORY-MASTER", "hardware_revision": "AC",
                }
                assert report["master_diagnostics"] == {
                    "temperature_c": -6.5, "supply_voltage_mv": 24050, "sensor_current_ma": 0,
                    "supply_status_raw": "0x0004", "unavailable": [],
                }
                assert report["host_finished_utc"].endswith("Z") and report["read_duration_us"] > 0
                assert [port["port"] for port in report["ports"]] == list(range(1, length + 1))
                assert report["ports"][-1]["identity"]["device_id"] == 1
                assert report["ports"][-1]["port_cycle_us"] == expected_cycle
                assert report["ports"][-1]["comspeed_raw"] == "COM3"
                automatic = report["ports"][0]["configuration"]
                assert automatic == {
                    "preset_cycle_raw": "0", "preset_cycle_automatic": True,
                    "validation_mode_raw": "0x007F", "validation_vendor_id": 310,
                    "validation_device_id": 0x010203, "unavailable": [],
                }
                preset = report["ports"][-1]["configuration"]
                assert preset["preset_cycle_us"] == (3500 if model == "AL1350" else 3000)
                assert not preset["preset_cycle_automatic"] and preset["unavailable"] == []
                disabled = report["ports"][1]
                assert disabled["mode"] == "Disabled" and disabled["identity"] == {}
                assert {"path": "/iolinkmaster/port[2]/iolinkdevice/vendorid", "service_code": 503} in disabled["unavailable"]
                assert {"path": "/iolinkmaster/port[2]/comspeed"} in disabled["unavailable"]
                prefix = "/iolinkmaster/port[2]"
                assert disabled["configuration"] == {
                    "preset_cycle_raw": "unusable",
                    "unavailable": [
                        {"path": f"{prefix}/mastercycletime_preset", "service_code": 200},
                        {"path": f"{prefix}/validation_datastorage_mode"},
                        {"path": f"{prefix}/validation_vendorid", "service_code": 200},
                        {"path": f"{prefix}/validation_deviceid", "service_code": 503},
                    ],
                }
                assert route.read_bytes() == pinned

            before = len(server.requests)
            run(cli, "inspect-master", route, "--snapshot", output / "missing-parent" / "master.json", expected=1)
            assert len(server.requests) == before
            server.fail_master = True
            absent = output / "first-batch-failed.json"
            run(cli, "inspect-master", route, "--snapshot", absent, expected=1)
            server.fail_master = False
            assert not absent.exists() and server.requests[before:] == [False]
            before = len(server.requests)

            original_snapshot = snapshot.read_bytes()
            corrupted = output / "invalid-snapshot.json"
            for name in ("version", "pin", "time", "ports", "cycles", "unexpected"):
                wrong = json.loads(original_snapshot)
                if name == "version":
                    wrong["format_version"] = 2
                elif name == "pin":
                    wrong["firmware_pin"] = "3.1.99"
                elif name == "time":
                    wrong["inspection"]["host_finished_utc"] = "2026-01-01T00:00:00-05:00"
                elif name == "ports":
                    wrong["inspection"]["ports"][1]["port"] = 1
                elif name == "cycles":
                    wrong["cycle_unit"] = None
                else:
                    wrong["inspection"]["ports"][0]["unexpected"] = True
                corrupted.write_text(json.dumps(wrong), encoding="utf-8")
                run(cli, "review-master", corrupted, expected=1)
            assert len(server.requests) == before and snapshot.read_bytes() == original_snapshot

            server.partial_diagnostics = True
            partial = tomllib.loads(run(cli, "inspect-master", route))
            server.partial_diagnostics = False
            assert server.requests[before:] == [False, True]
            assert len(partial["ports"]) == 8 and "error" not in partial
            assert partial["master_diagnostics"] == {
                "supply_status_raw": "0",
                "unavailable": [
                    {"path": "/processdatamaster/temperature", "service_code": 200},
                    {"path": "/processdatamaster/voltage", "service_code": 503},
                    {"path": "/processdatamaster/current"},
                ],
            }

            before = len(server.requests)
            mismatch = tomllib.loads(run(cli, "inspect-master", output / "ifm-al1350.toml", expected=1))
            assert mismatch["master"]["model"] == "AL1353" and mismatch["ports"] == []
            assert "expected master AL1350" in mismatch["error"] and server.requests[before:] == [False]
            assert "master_diagnostics" not in mismatch
            before = len(server.requests)
            wrong_firmware = output / "wrong-firmware.toml"
            run(cli, "set-connection", route, wrong_firmware, "--firmware", "3.1.99")
            mismatch = tomllib.loads(run(cli, "inspect-master", wrong_firmware, expected=1))
            assert "expected firmware" in mismatch["error"] and server.requests[before:] == [False]
            assert "master_diagnostics" not in mismatch
            for fault in ("changed_serial", "fail_ports"):
                before = len(server.requests)
                setattr(server, fault, True)
                partial_path = output / f"{fault}.json"
                failure = tomllib.loads(run(cli, "inspect-master", route, "--snapshot", partial_path, expected=1))
                setattr(server, fault, False)
                assert tomllib.loads(run(cli, "review-master", partial_path, expected=1))["inspection"] == failure
                assert server.requests[before:] == [False, True]
                assert failure["master"]["serial"] == "INVENTORY-MASTER" and failure["ports"] == []
                assert "master_diagnostics" not in failure
                assert ("identity changed" if fault == "changed_serial" else "service code 503") in failure["error"]

            before = len(server.requests)
            verify_master_comparison(cli, output, snapshot)
            assert len(server.requests) == before
        finally:
            server.shutdown()
            worker.join()


def verify_master_comparison(cli, output, source):
    original = source.read_bytes()
    same = tomllib.loads(run(cli, "compare-master", source, source))
    assert same["left"] == same["right"]
    assert same["observations"] == {"cycles_comparable": True, "supply_changes": [], "changed_ports": []}

    def compared(name, edit, expected=0):
        changed = json.loads(original)
        edit(changed)
        path = output / f"compare-{name}.json"
        path.write_text(json.dumps(changed), encoding="utf-8")
        pinned = path.read_bytes()
        result = tomllib.loads(run(cli, "compare-master", source, path, expected=expected))
        assert result["left"] == same["left"] and path.read_bytes() == pinned
        return result

    def observations(changed):
        inspection = changed["inspection"]
        inspection["master_diagnostics"]["supply_voltage_mv"] = 24120
        port = inspection["ports"][0]
        port["identity"]["device_id"] = 2
        port["configuration"]["validation_vendor_id"] = 311
        port["port_event"] = None
        port["unavailable"].append({"path": "/iolinkmaster/port[1]/portevent", "service_code": 503})

    changes = compared("observations", observations)["observations"]
    assert changes["supply_changes"] == [{"field": "Supply voltage (mV)", "left": "24050", "right": "24120"}]
    port, = changes["changed_ports"]
    assert port["port"] == 1
    assert port["connected_device"] == [{"field": "Connected Device ID", "left": "1", "right": "2"}]
    assert port["configuration"] == [{"field": "Configured Vendor ID", "left": "310", "right": "311"}]
    assert port["link"] == [
        {"field": "Port event", "left": "0"},
        {"field": "Unavailable data points", "left": "none reported", "right": "/iolinkmaster/port[1]/portevent (IoT Core 503)"},
    ]

    def different_cycle_unit(changed):
        changed["cycle_unit"] = "microseconds"
        for port in changed["inspection"]["ports"]:
            port["port_cycle_us"] = int(port["port_cycle_raw"])
            settings = port["configuration"]
            if settings["preset_cycle_us"] is not None:
                settings["preset_cycle_us"] = int(settings["preset_cycle_raw"])
        changed["inspection"]["ports"][0]["port_cycle_raw"] = "7"
        changed["inspection"]["ports"][0]["port_cycle_us"] = 7

    units = compared("cycle-unit", different_cycle_unit)
    assert units["context_changes"] == [{"field": "Cycle unit", "left": "milliseconds", "right": "microseconds"}]
    assert not units["observations"]["cycles_comparable"]
    port, = units["observations"]["changed_ports"]
    assert port["link"] == [{"field": "Reported cycle (raw)", "left": "2", "right": "7"}]

    def different_firmware(changed):
        changed["firmware_pin"] = ""
        changed["inspection"]["master"]["firmware"] = "3.1.77"

    for name, edit, issue in (
        ("route", lambda s: s.update(endpoint="http://127.0.0.1:1"), "routes differ"),
        ("serial", lambda s: s["inspection"]["master"].update(serial="OTHER"), "serial numbers differ"),
        ("serial-missing", lambda s: s["inspection"]["master"].update(serial=" "), "serial numbers are unavailable"),
        ("firmware", different_firmware, "firmware revisions differ"),
    ):
        blocked = compared(name, edit, expected=1)
        assert "observations" not in blocked and issue in blocked["comparison_issue"]

    partial = tomllib.loads(run(cli, "compare-master", source, output / "fail_ports.json", expected=1))
    assert "incomplete" in partial["comparison_issue"] and "observations" not in partial
    assert partial["right"]["inspection"]["error"]
    run(cli, "compare-master", source, output / "invalid-snapshot.json", expected=1)
    assert source.read_bytes() == original


def verify_error_types(cli, output):
    output.mkdir()
    requests = Gateway.requests
    standard_code = tomllib.loads(run(cli, "decode-error-type", "80 11"))
    assert (standard_code["raw_hex"], standard_code["code"]) == ("8011", 0x8011)
    assert standard_code["standard_meaning"] == "Index is unavailable"
    assert "iodd_definition" not in standard_code and not standard_code["unresolved_standard_reference"]
    assert "standard_meaning" not in tomllib.loads(run(cli, "decode-error-type", "FF11"))
    for malformed in ("80", "801102", "80Q1"):
        assert not run(cli, "decode-error-type", malformed, expected=1)

    original = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    entries = ('<ErrorTypeCollection><StdErrorTypeRef additionalCode="17"/>'
               '<StdErrorTypeRef code="128" additionalCode="254"/>'
               '<ErrorType additionalCode="17"><Name textId="vendorError"/>'
               '<Description textId="eventdetail"/></ErrorType></ErrorTypeCollection>')
    source = output / "device.xml"
    document = original.replace("<EventCollection>", entries + "<EventCollection>")
    name = '<Text id="vendorError" value="Synthetic vendor failure"/>'
    source.write_text(document.replace("</PrimaryLanguage>", name + "</PrimaryLanguage>"), encoding="utf-8")
    translation = output / "device-de.xml"
    translated = (ROOT / "samples/sensor-de.xml").read_text(encoding="utf-8")
    name = '<Text id="vendorError" value="Synthetischer Gerätefehler"/>'
    translation.write_text(translated.replace("</Language>", name + "</Language>"), encoding="utf-8")
    definitions = output / "definitions.xml"
    definitions.write_text((ROOT / "samples/synthetic-standard-definitions.xml").read_text(encoding="utf-8")
                           .replace("<EventCollection>", '<ErrorTypeCollection><ErrorType additionalCode="17">'
                                    '<Name textId="standardevent"/></ErrorType></ErrorTypeCollection><EventCollection>'),
                           encoding="utf-8")

    catalog = tomllib.loads(run(cli, "iodd", source))
    assert [entry["code"] for entry in catalog["error_types"]] == [0x8111]
    assert catalog["standard_error_type_refs"] == [0x8011, 0x80FE]
    profile = output / "device.toml"
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR", "--language", "de",
        "--language-file", translation)
    unresolved = tomllib.loads(run(cli, "decode-error-type", "8011", "--profile", profile))
    assert unresolved["unresolved_standard_reference"]
    assert unresolved["standard_meaning"] == standard_code["standard_meaning"]
    resolved_profile = output / "resolved.toml"
    run(cli, "import", source, resolved_profile, "--variant", "SIM-SENSOR", "--language", "de",
        "--language-file", translation, "--standard-definitions", definitions,
        "--standard-language-file", ROOT / "samples/synthetic-standard-definitions-de.xml")
    offline_profile = output / "offline.toml"
    run(cli, "set-connection", resolved_profile, offline_profile,
        "--gateway", "ifm-al1350", "--endpoint", "http://127.0.0.1:1")
    before = offline_profile.read_bytes()
    for resource in (source, translation, definitions):
        resource.rename(resource.with_suffix(".used.xml"))

    vendor = tomllib.loads(run(cli, "decode-error-type", "81 11", "--profile", offline_profile))
    assert vendor["error_code"] == 0x81 and vendor["additional_code"] == 17
    assert vendor["iodd_definition"]["name"] == "Synthetischer Gerätefehler"
    assert vendor["iodd_definition"]["description"].startswith("Simulatordiagnose.")
    resolved = tomllib.loads(run(cli, "decode-error-type", "8011", "--profile", offline_profile))
    assert not resolved["unresolved_standard_reference"]
    assert resolved["standard_meaning"] == standard_code["standard_meaning"]
    assert resolved["iodd_definition"]["origin"].startswith("definitions.xml")
    assert resolved["iodd_definition"]["name"] == "Synthetischer Standardalarm"
    unknown = tomllib.loads(run(cli, "decode-error-type", "80FE", "--profile", offline_profile))
    assert unknown["unresolved_standard_reference"] and "standard_meaning" not in unknown
    assert offline_profile.read_bytes() == before and Gateway.requests == requests

    bad_source = output / "invalid.xml"
    for invalid in (
        entries.replace("</ErrorTypeCollection>", '<StdErrorTypeRef additionalCode="17"/></ErrorTypeCollection>'),
        entries.replace('<ErrorType additionalCode="17">', '<ErrorType code="128" additionalCode="17">'),
        entries.replace('additionalCode="254"', 'additionalCode="256"'),
    ):
        bad_source.write_text(original.replace("<EventCollection>", invalid + "<EventCollection>"), encoding="utf-8")
        assert not run(cli, "iodd", bad_source, expected=1)


def verify_diagnostic_snapshots(cli, profile, output, loopback=False):
    output.mkdir()
    source = profile.read_bytes()
    path = output / "diagnostics.json"
    before_reads = Gateway.isdu_reads
    report = tomllib.loads(run(cli, "read-diagnostics", profile, "--snapshot", path))
    assert Gateway.isdu_reads == before_reads + (2 if loopback else 0)
    pinned = path.read_bytes()
    document = json.loads(pinned)
    assert document["format_version"] == 1
    assert [(read["index"], read["subindex"]) for read in document["reads"]] == [(36, 0), (37, 0)]
    assert [read["raw_hex"] for read in document["reads"]] == ["02", "E48DFE000000"]
    for original, read in zip(report["reads"], document["reads"]):
        for key in ("host_read_utc", "read_duration_us", "raw_hex"):
            assert original[key] == read[key]
        assert read["observation"]["route"] == ("AL1350 loopback fixture" if loopback else "synthetic simulator")
        assert read["observation"]["identity"]["device_id"] == 1

    before_requests = Gateway.requests
    summary = tomllib.loads(run(cli, "review-diagnostics", path))
    assert summary["observed_identities_equal"] and not summary["observed_master_conflict"]
    assert summary["device_status"] == report["device_status"] and summary["detailed_status"] == report["detailed_status"]
    run(cli, "read-diagnostics", profile, "--snapshot", path, expected=1)
    assert Gateway.requests == before_requests and path.read_bytes() == pinned and profile.read_bytes() == source
    run(cli, "review-parameter", path, expected=1)

    offline = output / "offline.json"
    document["profile"]["connection"]["endpoint"] = "http://127.0.0.1:1"
    document["profile"]["iodd"]["source"] = "Z:/missing/device.zip"
    offline.write_text(json.dumps(document), encoding="utf-8")
    assert tomllib.loads(run(cli, "review-diagnostics", offline))["iodd_source"] == "Z:/missing/device.zip"
    malformed = output / "malformed.json"
    document["reads"][1]["raw_hex"] = "FF"
    malformed.write_text(json.dumps(document), encoding="utf-8")
    review = tomllib.loads(run(cli, "review-diagnostics", malformed, expected=1))
    assert review["detailed_status"]["raw_hex"] == "FF" and review["detailed_status"]["decode_error"]
    wrong = output / "wrong-port.json"
    document["reads"][0]["observation"]["port"] = 2
    wrong.write_text(json.dumps(document), encoding="utf-8")
    assert not run(cli, "review-diagnostics", wrong, expected=1)
    assert Gateway.requests == before_requests

    right = output / "slot-movement.json"
    other = json.loads(pinned)
    other["reads"][0]["raw_hex"] = "04"
    other["reads"][1]["raw_hex"] = "000000E48DFE000000"
    right.write_text(json.dumps(other), encoding="utf-8")
    saved_right = right.read_bytes()
    difference = tomllib.loads(run(cli, "compare-diagnostics", path, right))
    assert difference["device_status_values"] == {"left": 2, "right": 4}
    assert not difference["device_status_reply"]["same_wire_bytes"]
    assert not difference["detailed_status_reply"]["same_wire_bytes"]
    assert difference["changed_slots"] == [
        {"slot": 1, "left_hex": "E48DFE", "right_hex": "000000"},
        {"slot": 2, "left_hex": "000000", "right_hex": "E48DFE"},
        {"slot": 3, "right_hex": "000000"},
    ]
    assert difference["left"]["detailed_status"]["raw_hex"] == "E48DFE000000"
    assert difference["right"]["detailed_status"]["raw_hex"] == "000000E48DFE000000"
    assert difference["pending_events"] == {
        "left_other_nonempty_entries": 0, "right_other_nonempty_entries": 0, "changed_counts": [],
    }
    assert tomllib.loads(run(cli, "compare-diagnostics", path, path))["changed_slots"] == []
    assert path.read_bytes() == pinned and right.read_bytes() == saved_right

    changed_counts = output / "pending-event-counts.json"
    counts_fixture = json.loads(pinned)
    counts_fixture["reads"][1]["raw_hex"] = "E48DFEEC8DFEF48DFEE48DFE"
    changed_counts.write_text(json.dumps(counts_fixture), encoding="utf-8")
    saved_counts = changed_counts.read_bytes()
    counts = tomllib.loads(run(cli, "compare-diagnostics", path, changed_counts))["pending_events"]
    assert counts["left_other_nonempty_entries"] == 0 and counts["right_other_nonempty_entries"] == 1
    assert [(row["raw_hex"], row["left_count"], row["right_count"]) for row in counts["changed_counts"]] == [
        ("E48DFE", 1, 2), ("F48DFE", 0, 1),
    ]
    assert [row["type_name"] for row in counts["changed_counts"]] == ["Warning", "Error"]
    assert counts["changed_counts"][0]["right_name"] == "Synthetic range warning"
    assert path.read_bytes() == pinned and changed_counts.read_bytes() == saved_counts

    another_port = output / "another-port.json"
    other["profile"]["connection"]["port"] = 4
    for read in other["reads"]:
        read["observation"]["port"] = 4
    another_port.write_text(json.dumps(other), encoding="utf-8")
    diff_port = tomllib.loads(run(cli, "compare-diagnostics", path, another_port))
    assert "port" in diff_port["detailed_status_reply"]["interpretation_issue"]
    assert "changed_slots" not in diff_port and "pending_events" not in diff_port
    assert not diff_port["detailed_status_reply"]["same_wire_bytes"]
    assert not run(cli, "compare-diagnostics", path, wrong, expected=1)
    assert Gateway.requests == before_requests

    if loopback:
        unavailable_master = output / "unknown-master.json"
        other = json.loads(pinned)
        for read in other["reads"]:
            read["observation"]["master_serial"] = None
        unavailable_master.write_text(json.dumps(other), encoding="utf-8")
        no_serial = tomllib.loads(run(cli, "compare-diagnostics", path, unavailable_master))
        assert no_serial["detailed_status_reply"]["same_wire_bytes"]
        assert "master serial" in no_serial["detailed_status_reply"]["interpretation_issue"]
        assert "changed_slots" not in no_serial and "pending_events" not in no_serial
        assert Gateway.requests == before_requests

        partial_path = output / "partial.json"
        Gateway.fail_detailed = True
        try:
            partial = tomllib.loads(run(cli, "read-diagnostics", profile, "--snapshot", partial_path, expected=1))
        finally:
            Gateway.fail_detailed = False
        assert partial["device_status"]["value"] == 2 and "detailed_status" not in partial
        assert partial["reads"][1]["read_error"] and partial["reads"][1]["status"]["port"] == 1
        before_requests = Gateway.requests
        restored = tomllib.loads(run(cli, "review-diagnostics", partial_path, expected=1))
        assert restored["reads"][1]["read_error"] == partial["reads"][1]["read_error"]
        assert restored["reads"][1]["observation"]["master_serial"] == "FIXTURE-MASTER"
        assert "detailed_status" not in restored and Gateway.requests == before_requests
        partial_comparison = tomllib.loads(run(cli, "compare-diagnostics", path, partial_path, expected=1))
        assert partial_comparison["device_status_reply"]["same_wire_bytes"]
        assert partial_comparison["device_status_values"] == {"left": 2, "right": 2}
        assert "same_wire_bytes" not in partial_comparison["detailed_status_reply"]
        assert "changed_slots" not in partial_comparison and "pending_events" not in partial_comparison
        assert Gateway.requests == before_requests

        empty_path = output / "empty-reply.json"
        Gateway.detailed_payload = ""
        try:
            empty = tomllib.loads(run(cli, "read-diagnostics", profile, "--snapshot", empty_path, expected=1))
        finally:
            Gateway.detailed_payload = "E48DFE000000"
        assert empty["detailed_status"]["raw_hex"] == "" and empty["detailed_status"]["decode_error"]
        before_requests = Gateway.requests
        restored = tomllib.loads(run(cli, "review-diagnostics", empty_path, expected=1))
        assert restored["detailed_status"] == empty["detailed_status"] and Gateway.requests == before_requests

        changed_path = output / "changed-master.json"
        Gateway.change_master_after_status = True
        try:
            changed = tomllib.loads(run(cli, "read-diagnostics", profile, "--snapshot", changed_path, expected=1))
        finally:
            Gateway.change_master_after_status = False
            Gateway.data["/deviceinfo/serialnumber"] = "FIXTURE-MASTER"
        assert changed["observed_identities_equal"] and changed["observed_master_conflict"]
        before_requests = Gateway.requests
        changed = tomllib.loads(run(cli, "review-diagnostics", changed_path, expected=1))
        assert changed["observed_master_conflict"] and Gateway.requests == before_requests
        assert [read["observation"]["master_serial"] for read in changed["reads"]] == ["FIXTURE-MASTER", "CHANGED-MASTER"]
        conflict = tomllib.loads(run(cli, "compare-diagnostics", path, changed_path, expected=1))
        assert "within a snapshot" in conflict["device_status_reply"]["interpretation_issue"]
        assert conflict["detailed_status_reply"]["same_wire_bytes"]
        assert "changed_slots" not in conflict and "pending_events" not in conflict
        assert Gateway.requests == before_requests


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
    record = output / "V_SIM_Record-0.json"
    record_bytes = record.read_bytes()
    changed_record = json.loads(record_bytes)
    changed_record["raw_hex"] = "FFED010A54455354"
    changed_record["observation"]["device_event"] = "36350"
    second = output / "changed-record.json"
    second.write_text(json.dumps(changed_record), encoding="utf-8")
    second_bytes = second.read_bytes()
    before_requests = Gateway.requests
    comparison = tomllib.loads(run(cli, "compare-parameters", record, record))
    assert comparison["same_wire_bytes"] and comparison["changed_members"] == []
    comparison = tomllib.loads(run(cli, "compare-parameters", record, second))
    assert not comparison["same_wire_bytes"] and comparison["same_saved_parameter_definition"]
    assert comparison["observed_device_ids_match"]
    assert comparison["right"]["observation"]["device_event"] == "36350"
    assert [(item["subindex"], item["left_raw_hex"], item["right_raw_hex"])
            for item in comparison["changed_members"]] == [(1, "FFEC", "FFED")]
    run(cli, "compare-parameters", record, output / "V_SIM_Record-1.json", expected=1)
    assert Gateway.requests == before_requests
    assert record.read_bytes() == record_bytes and second.read_bytes() == second_bytes

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
    comparison = tomllib.loads(run(cli, "compare-parameters", output / "V_SIM_Threshold-0.json", short, expected=1))
    assert comparison["right"]["decoded"]["raw_hex"] == "FF" and "changed_members" not in comparison
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


def verify_mapping_settings(cli, output):
    output.mkdir()
    imported = output / "imported.toml"
    source = output / "source.toml"
    run(cli, "import", ROOT / "samples/sensor-formats.xml", imported)
    run(cli, "select-display", imported, "V_SIM_Threshold", source, "--display", "M_SIM_Threshold/2")
    source.write_bytes(b'# Original mapping profile bytes remain unchanged\n' + source.read_bytes())
    original = source.read_bytes()
    before = tomllib.loads(original.decode("utf-8"))
    base = before["iodd"]["fields"][0]
    name = base["name"]
    assert not before.get("fields")
    requests = Gateway.requests
    edited = output / "override.toml"
    run(cli, "set-field", source, edited, name, "--bit-offset", 0, "--bits", 12, "--kind", "signed",
        "--byte-order", "big-endian", "--scale", .5, "--offset", -1, "--unit", "µm, scale", "--decimals", 3)
    changed = tomllib.loads(edited.read_text(encoding="utf-8"))
    expected = dict(base, bit_offset=0, bits=12, kind="signed", byte_order="big_endian", scale=.5, offset=-1., unit="µm, scale", decimals=3)
    assert changed == dict(before, fields=[expected])
    assert "-4.500" in run(cli, "decode", edited, "FFF9") and "override" in run(cli, "decode", edited, "FFF9")
    saved = edited.read_bytes()
    run(cli, "set-field", source, edited, name, "--unit", "must not replace", expected=1)
    run(cli, "set-field", source, source, name, "--unit", "must not replace", expected=1)
    assert source.read_bytes() == original and edited.read_bytes() == saved
    little = output / "little-endian.toml"
    run(cli, "set-field", edited, little, name, "--byte-order", "little-endian", "--scale", 1, "--offset", 0, "--clear-decimals")
    assert "-1537" in run(cli, "decode", little, "FFF9")
    auxiliary = "aux, µm"
    added = output / "manual.toml"
    run(cli, "set-field", edited, added, auxiliary, "--bit-offset", 0, "--bits", 1, "--kind", "bool")
    assert "true" in run(cli, "decode", added, "FFF9")
    updated = output / "updated.toml"
    run(cli, "set-field", added, updated, auxiliary, "--unit", "changed")
    assert [field["name"] for field in tomllib.loads(updated.read_text(encoding="utf-8"))["fields"]] == [name, auxiliary]
    invalid = output / "must-not-exist.toml"
    for args in [["--bits", 0], ["--bits", 65], ["--bit-offset", 16], ["--kind", "bool", "--bits", 4], ["--kind", "float32", "--bits", 16],
                 ["--scale", "NaN"], ["--offset", "inf"], ["--decimals", 13]]:
        run(cli, "set-field", edited, invalid, name, *args, expected=1)
        assert not invalid.exists()
    run(cli, "set-field", source, invalid, "new", "--unit", "incomplete", expected=1)
    for args in [[], ["--decimals", 2, "--clear-decimals"], ["--kind", "unknown"]]:
        run(cli, "set-field", edited, invalid, name, *args, expected=2)
        assert not invalid.exists()
    run(cli, "remove-field", source, invalid, name, expected=1)
    run(cli, "remove-field", source, invalid, "missing", expected=1)
    restored_aux = output / "removed-aux.toml"
    run(cli, "remove-field", added, restored_aux, auxiliary)
    restored = output / "restored.toml"
    run(cli, "remove-field", restored_aux, restored, name)
    assert tomllib.loads(restored.read_text(encoding="utf-8")) == before
    route = output / "unreachable-route.toml"
    run(cli, "set-connection", added, route, "--gateway", "ifm-al1350", "--endpoint", "http://127.0.0.1:9")
    offline = output / "unreachable-edited.toml"
    run(cli, "set-field", route, offline, auxiliary, "--unit", "offline-only")
    assert tomllib.loads(offline.read_text(encoding="utf-8"))["connection"] == tomllib.loads(route.read_text(encoding="utf-8"))["connection"]
    assert Gateway.requests == requests and source.read_bytes() == original and edited.read_bytes() == saved
    wide_source = output / "wide-source.toml"
    wide_source.write_bytes((ROOT / "samples/simulator.toml").read_bytes().replace(b'input_bytes = 2', b'input_bytes = 8'))
    wide = output / "wide.toml"
    run(cli, "set-field", wide_source, wide, "wide", "--bit-offset", 0, "--bits", 64, "--kind", "unsigned", "--decimals", 2)
    assert "18446744073709551615.00" in run(cli, "decode", wide, "FFFFFFFFFFFFFFFF")
    snapshot = output / "parameter.json"
    reading = tomllib.loads(run(cli, "read-parameter", added, "V_SIM_Threshold", "--snapshot", snapshot))
    assert reading["raw_value"] == "500" and reading["preferred_display"] == tomllib.loads(run(cli, "review-parameter", snapshot))["decoded"]["preferred_display"]
    capture(cli, added, output / "capture", "--seconds", .25, "--field", auxiliary)
    selected = output / "capture/profile.toml"
    selected_bytes = selected.read_bytes()
    run(cli, "remove-field", selected, invalid, auxiliary, expected=1)
    assert not invalid.exists() and selected.read_bytes() == selected_bytes
    assert Gateway.requests == requests and source.read_bytes() == original and edited.read_bytes() == saved


def verify_logging_settings(cli, output):
    output.mkdir()
    imported = output / "imported.toml"
    preferred = output / "preferred.toml"
    source = output / "source.toml"
    run(cli, "import", ROOT / "samples/sensor-formats.xml", imported)
    run(cli, "select-display", imported, "V_SIM_Threshold", preferred, "--display", "M_SIM_Threshold/2")
    auxiliary = "aux, µm"
    run(cli, "set-field", preferred, source, auxiliary, "--bit-offset", 0, "--bits", 1, "--kind", "bool")
    source.write_bytes(b'# Original logging source stays unchanged\n' + source.read_bytes())
    original = source.read_bytes()
    before = tomllib.loads(original.decode("utf-8"))
    field = before["iodd"]["fields"][0]["name"]
    requests = Gateway.requests
    ordered = output / "ordered.toml"
    run(cli, "set-logging", source, ordered, "--mode", "on-change", "--queue-capacity", 64,
        "--rotate-rows", 7, "--rotate-seconds", 0, "--field", auxiliary, "--field", field)
    changed = tomllib.loads(ordered.read_text(encoding="utf-8"))
    assert {k: v for k, v in changed.items() if k != "logging"} == {k: v for k, v in before.items() if k != "logging"}
    assert changed["logging"] == dict(before["logging"], mode="on_change", queue_capacity=64,
                                      rotate_rows=7, rotate_seconds=0, fields=[auxiliary, field])
    plan = tomllib.loads(run(cli, "logging", ordered))
    assert plan["settings"] == changed["logging"] and plan["selection"] == "explicit_order"
    assert plan["configured_row_kind"] == "source_update" and "no gateway accessed" in plan["basis"]
    assert [f["mapping"]["name"] for f in plan["selected_fields"]] == [auxiliary, field]
    assert [f["origin"] for f in plan["selected_fields"]] == ["manual", "IODD"]

    def check_headers(directory, columns):
        for part in directory.glob("capture-*.csv"):
            with part.open(encoding="utf-8", newline="") as stream:
                assert next(csv.reader(stream)) == columns

    report, _ = capture(cli, ordered, output / "capture", "--seconds", .25)
    assert report["csv_parts"] > 1
    check_headers(output / "capture", plan["csv_columns"])
    raw = output / "raw.toml"
    run(cli, "set-logging", ordered, raw, "--mode", "every-update", "--queue-capacity", 128,
        "--raw-only", "--rotate-rows", 0, "--rotate-seconds", 1)
    raw_plan = tomllib.loads(run(cli, "logging", raw))
    assert raw_plan["selection"] == "raw_only" and not raw_plan["selected_fields"]
    assert len(raw_plan["csv_columns"]) == 12
    report, _ = capture(cli, raw, output / "time-rotation", "--seconds", 1.2)
    assert report["csv_parts"] >= 2
    check_headers(output / "time-rotation", raw_plan["csv_columns"])
    all_fields = output / "all.toml"
    run(cli, "set-logging", raw, all_fields, "--all-fields", "--rotate-seconds", 0)
    assert tomllib.loads(run(cli, "logging", all_fields))["selection"] == "all_mapped"
    run(cli, "remove-field", all_fields, output / "removed.toml", auxiliary)
    retained = output / "retained.toml"
    run(cli, "set-logging", ordered, retained, "--queue-capacity", 100)
    assert tomllib.loads(retained.read_text(encoding="utf-8"))["logging"]["fields"] == [auxiliary, field]
    invalid = output / "must-not-exist.toml"
    for options in [["--queue-capacity", 0], ["--queue-capacity", 1000001], ["--field", "missing"],
                    ["--field", field, "--field", field], ["--rotate-rows", 18446744073709551615],
                    ["--rotate-seconds", 18446744073709551615]]:
        run(cli, "set-logging", source, invalid, *options, expected=1)
        assert not invalid.exists()
    for options in [[], ["--mode", "bad"], ["--raw-only", "--all-fields"], ["--raw-only", "--field", field]]:
        run(cli, "set-logging", source, invalid, *options, expected=2)
        assert not invalid.exists()
    saved = ordered.read_bytes()
    run(cli, "set-logging", source, ordered, "--mode", "on-change", expected=1)
    run(cli, "set-logging", source, source, "--mode", "on-change", expected=1)
    assert ordered.read_bytes() == saved
    route = output / "unreachable.toml"
    run(cli, "set-connection", source, route, "--gateway", "ifm-al1350", "--endpoint", "http://127.0.0.1:9")
    offline = output / "offline.toml"
    run(cli, "set-logging", route, offline, "--mode", "on-change")
    route_plan = tomllib.loads(run(cli, "logging", offline))
    assert route_plan["configured_row_kind"] == "host_poll" and route_plan["source_update_loss"].startswith("unknown")
    snapshots = output / "snapshots.toml"
    snapshots.write_bytes(source.read_bytes().replace(b'simulate_snapshots = false', b'simulate_snapshots = true'))
    assert tomllib.loads(run(cli, "logging", snapshots))["configured_row_kind"] == "host_poll"
    snapshot = output / "parameter.json"
    reading = tomllib.loads(run(cli, "read-parameter", ordered, "V_SIM_Threshold", "--snapshot", snapshot))
    assert reading["raw_value"] == "500" and reading["preferred_display"] == tomllib.loads(run(cli, "review-parameter", snapshot))["decoded"]["preferred_display"]
    assert source.read_bytes() == original and Gateway.requests == requests


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


def verify_iodd_features(cli, output):
    output.mkdir()
    original = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    marker = "<DeviceFunction>"
    assert original.count(marker) == 1
    features = (
        '<Features blockParameter="1" dataStorage="false" profileCharacteristic="  16384   32768  65535 ">'
        '<SupportedAccessLocks parameter="0" dataStorage="false" localParameterization="true" localUserInterface="1"/>'
        '</Features>'
    )
    expected = {
        "block_parameter": True,
        "data_storage": False,
        "profile_characteristic": [16384, 32768, 65535],
        "access_locks": {
            "parameter": False, "data_storage": False,
            "local_parameterization": True, "local_user_interface": True,
        },
    }

    def source_with(declaration):
        return original.replace(marker, marker + declaration)

    source = output / "features.xml"
    source.write_text(source_with(features), encoding="utf-8")
    assert tomllib.loads(run(cli, "iodd", source))["features"] == expected
    archive = output / "features.zip"
    with zipfile.ZipFile(archive, "x", zipfile.ZIP_DEFLATED) as bundle:
        bundle.write(source, "Synthetic-Features-20250101-IODD1.1.xml")
    profile = output / "features.toml"
    run(cli, "import", archive, profile, "--variant", "SIM-SENSOR")
    pinned = profile.read_bytes()
    saved = tomllib.loads(pinned.decode("utf-8"))
    assert saved["iodd"]["features"] == expected
    summary = run(cli, "validate", profile)
    assert "IODD declared capabilities" in summary and "Device Data Storage · No" in summary
    assert "0x4000 (16384), 0x8000 (32768), 0xFFFF (65535)" in summary
    assert "Local parameterization Yes · Local user interface Yes" in summary
    omitted = output / "no-optional.xml"
    omitted.write_text(source_with('<Features blockParameter="0" dataStorage="true"/>'), encoding="utf-8")
    assert tomllib.loads(run(cli, "iodd", omitted))["features"] == {
        "block_parameter": False, "data_storage": True,
    }
    assert "features" not in tomllib.loads(run(cli, "iodd", ROOT / "samples/sensor.xml"))

    for name, invalid in (
        ("empty-pids", features.replace("  16384   32768  65535 ", " ")),
        ("pid-overflow", features.replace("65535", "65536")),
        ("pid-limit", features.replace("65535", " ".join(["1"] * 31))),
        ("bad-flag", features.replace('blockParameter="1"', 'blockParameter="yes"')),
        ("missing-lock", features.replace(' localUserInterface="1"', "")),
        ("duplicate-features", features + features),
    ):
        bad = output / f"{name}.xml"
        bad.write_text(source_with(invalid), encoding="utf-8")
        run(cli, "import", bad, profile, "--variant", "SIM-SENSOR", expected=1)
        assert profile.read_bytes() == pinned
    edited = output / "broken-profile.toml"
    damaged, replaced = re.subn(
        r"profile_characteristic = \[([^]]*)\]", "profile_characteristic = []", pinned.decode("utf-8")
    )
    assert replaced == 1
    edited.write_text(damaged, encoding="utf-8")
    run(cli, "validate", edited, expected=1)

    bundle_path = output / "capture"
    _, rows = capture(cli, profile, bundle_path, "--seconds", ".16")
    assert rows and tomllib.loads(run(cli, "verify-capture", bundle_path))["rows_checked"] == len(rows)
    assert tomllib.loads((bundle_path / "profile.toml").read_text(encoding="utf-8"))["iodd"]["features"] == expected
    return profile


def verify_profile_characteristic(cli, profile, output):
    output.mkdir()
    before = Gateway.requests
    original = profile.read_bytes()
    command = "decode-profile-characteristic"
    matching = tomllib.loads(run(cli, command, "0x4000 8000_FFFF", "--profile", profile))
    assert (matching["index"], matching["subindex"], matching["raw_hex"]) == (13, 0, "40008000FFFF")
    assert matching["status"] == "match"
    assert matching["reply_ids"] == matching["declared_ids"] == [0x4000, 0x8000, 0xFFFF]
    assert matching["comparison"] == {"missing_ids": [], "unexpected_ids": []}

    reordered = output / "reordered.toml"
    text = original.decode("utf-8")
    changed, replaced = re.subn(
        r"profile_characteristic = \[([^]]*)\]",
        "profile_characteristic = [65535, 32768, 16384]", text
    )
    assert replaced == 1
    reordered.write_text(changed, encoding="utf-8")
    ordered = tomllib.loads(run(cli, command, "40008000FFFF", "--profile", reordered))
    assert ordered["status"] == "match" and ordered["declared_ids"] == [0xFFFF, 0x8000, 0x4000]

    different = tomllib.loads(run(cli, command, "40008101FFFF", "--profile", profile, expected=1))
    assert different["status"] == "different"
    assert different["comparison"] == {"missing_ids": [0x8000], "unexpected_ids": [0x8101]}
    undecided = tomllib.loads(run(cli, command, "40008000", "--profile", ROOT / "samples/simulator.toml"))
    assert undecided["status"] == "undeclared" and undecided["reply_ids"] == [0x4000, 0x8000]
    assert "declared_ids" not in undecided and "comparison" not in undecided
    assert tomllib.loads(run(cli, command, "40008000")) == undecided

    for payload, decoded in [("400080", []), ("80004000", [0x8000, 0x4000]), ("40004000", [0x4000, 0x4000])]:
        invalid = tomllib.loads(run(cli, command, payload, "--profile", profile, expected=1))
        assert invalid["status"] == "invalid_reply" and invalid["raw_hex"] == payload
        assert invalid["reply_ids"] == decoded and invalid["decode_error"] and "comparison" not in invalid
    run(cli, command, "00" * 233, expected=1)
    run(cli, command, "GG", expected=1)
    assert profile.read_bytes() == original and Gateway.requests == before


def verify_profile_characteristic_loopback(cli, profile, endpoint, output):
    output.mkdir()
    loopback = output / "loopback.toml"
    run(cli, "set-connection", profile, loopback, "--gateway", "ifm-al1350", "--endpoint", endpoint,
        "--expected-mode", "3", "--firmware", "3.1.42")
    reads = Gateway.isdu_reads
    reply = tomllib.loads(run(cli, "isdu", loopback, 13, 0))["hex"]
    assert reply == "40008000FFFF" and Gateway.isdu_reads == reads + 1
    before = Gateway.requests
    compared = tomllib.loads(run(cli, "decode-profile-characteristic", reply, "--profile", loopback))
    assert compared["status"] == "match" and Gateway.requests == before


def verify_process_descriptors(cli, output):
    output.mkdir()
    saved = ROOT / "samples/sensor.xml"
    xml = saved.read_text(encoding="utf-8")
    input_decl = '<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>'
    output_decl = (
        '<ProcessDataOut id="OUT" bitLength="8"><Datatype xsi:type="RecordT" bitLength="8">'
        '<RecordItem subindex="1" bitOffset="0"><SimpleDatatype xsi:type="BooleanT"/></RecordItem>'
        '<RecordItem subindex="2" bitOffset="1"><SimpleDatatype xsi:type="BooleanT"/></RecordItem>'
        '<RecordItem subindex="3" bitOffset="2"><SimpleDatatype xsi:type="UIntegerT" bitLength="3"/></RecordItem>'
        '</Datatype></ProcessDataOut>'
    )
    assert xml.count(input_decl) == 1
    source = output / "descriptors.xml"
    source.write_text(xml.replace(input_decl, input_decl + output_decl), encoding="utf-8")
    profile = output / "descriptors.toml"
    run(cli, "import", source, profile, "--variant", "SIM-SENSOR")
    pinned = profile.read_bytes()
    before = Gateway.requests

    def check(direction, payload, status, selected=profile):
        args = ["decode-process-descriptor", direction, payload]
        if selected is not None:
            args.extend(["--profile", selected])
        report = tomllib.loads(run(cli, *args, expected=0 if status in {"match", "no_iodd"} else 1))
        assert report["status"] == status and report["raw_hex"] == payload.replace(" ", "")
        assert report["index"] == (14 if direction == "input" else 15) and report["subindex"] == 0
        return report

    matched = check("input", "010100 030F01", "match")
    assert matched["comparison"]["layout_id"] == "IN" and matched["comparison"]["payload_bytes"] == 2
    assert [(entry["datatype_code"], entry["bits"], entry["bit_offset"]) for entry in matched["entries"]] == [
        (1, 1, 0), (3, 15, 1),
    ]
    grouped = check("output", "010200 020302", "match")
    assert grouped["comparison"]["layout_id"] == "OUT"
    assert [field["status"] for field in grouped["comparison"]["fields"]] == ["Match"] * 3
    check("output", "010100 010101 020302", "match")
    check("input", "010100 020F01", "different")
    check("input", "010100", "different")
    outside = check("input", "010100 030F01 020110", "different")
    assert outside["comparison"]["outside_payload_entries"] == [3]
    unknown = check("input", "010100 800F01", "partial")
    assert unknown["entries"][1]["datatype"] == "Profile-specific datatype"
    gap = check("output", "010200 020302 020305", "partial")
    assert gap["comparison"]["partly_or_wholly_unmapped_entries"] == [3]
    assert check("input", "000000", "no_iodd", selected=None)["entries"][0]["bits"] == 256
    assert check("output", "", "no_iodd", selected=None)["entries"] == []
    for payload, review in (
        ("01010003", "complete three-byte"),
        ("030F01010100", "ascending offset order"),
        ("010200030F01", "overlaps"),
        ("0202FF", "past bit 255"),
    ):
        invalid = check("input", payload, "invalid_reply")
        assert any(review in issue for issue in invalid["review"]) and "comparison" not in invalid
    check("output", "", "different")

    manual = output / "host-override.toml"
    run(cli, "set-field", profile, manual, "level [1]", "--kind", "unsigned")
    assert check("input", "010100030F01", "match", manual)["comparison"] == matched["comparison"]
    no_output = output / "no-output.toml"
    run(cli, "import", saved, no_output, "--variant", "SIM-SENSOR")
    assert check("output", "", "match", no_output)["comparison"]["payload_bytes"] == 0

    boolean_xml = output / "boolean.xml"
    boolean_xml.write_text(xml.replace(
        input_decl, '<ProcessDataIn id="IN" bitLength="1"><Datatype xsi:type="BooleanT"/></ProcessDataIn>',
    ), encoding="utf-8")
    boolean = output / "boolean.toml"
    run(cli, "import", boolean_xml, boolean, "--variant", "SIM-SENSOR")
    assert check("input", "010100", "partial", boolean)["comparison"]["fields"][0]["status"] == (
        "Standalone Boolean width needs review"
    )
    assert profile.read_bytes() == pinned and Gateway.requests == before
    return profile


def verify_process_descriptor_loopback(cli, profile, endpoint, output):
    output.mkdir()
    loopback = output / "loopback.toml"
    run(cli, "set-connection", profile, loopback, "--gateway", "ifm-al1350", "--endpoint", endpoint,
        "--expected-mode", "3", "--firmware", "3.1.42")
    reads = Gateway.isdu_reads
    for index, direction, expected in [(14, "input", "010100030F01"), (15, "output", "010200020302")]:
        raw = tomllib.loads(run(cli, "isdu", loopback, index, 0))
        assert raw == {"hex": expected, "utf8": bytes.fromhex(expected).decode()}
        before = Gateway.requests
        comparison = tomllib.loads(run(cli, "decode-process-descriptor", direction, raw["hex"], "--profile", loopback))
        assert comparison["status"] == "match" and Gateway.requests == before
    assert Gateway.isdu_reads == reads + 2


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
    features_profile = verify_iodd_features(cli, scratch / "iodd-features")
    verify_profile_characteristic(cli, features_profile, scratch / "profile-characteristic")
    descriptor_profile = verify_process_descriptors(cli, scratch / "process-descriptors")
    standard_range_profile = verify_standard_ranges(cli, scratch / "standard-ranges")
    verify_error_types(cli, scratch / "error-types")
    verify_defaults(cli, scratch / "defaults")
    verify_temporal_defaults(cli, scratch / "temporal-defaults")
    verify_connection_settings(cli, scratch / "connection-settings")
    verify_port_statuses_and_captures(cli, scratch / "multi-port")
    verify_master_inspection(cli, scratch / "master-inspection")
    padded_input_profile = verify_padded_integer_input(cli, scratch / "padded-integer")
    boolean_input_profile = verify_standalone_boolean_input(cli, scratch / "standalone-boolean")
    float_names_profile = verify_process_float_names(cli, scratch / "process-float-names")
    range_names_profile = verify_process_range_names(cli, scratch / "process-range-names")
    cyclic_formats_profile = verify_cyclic_display_formats(cli, scratch / "process-display-formats")
    octet_fixture = verify_process_octets(cli, scratch / "process-octets")
    text_profile = verify_process_text(cli, scratch / "process-text")
    temporal_profile = verify_process_temporal(cli, scratch / "process-temporal")
    layout_profile = verify_layout_condition_plan(cli, scratch / "layout-condition")
    verify_identity_settings(cli, scratch / "identity-settings")
    verify_mapping_settings(cli, scratch / "mapping-settings")
    verify_logging_settings(cli, scratch / "logging-settings")
    verify_xsd(cli, scratch)
    source = ROOT / "samples" / "simulator.toml"
    profile = source.read_text(encoding="utf-8")

    with zipfile.ZipFile(scratch / "iodd.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        main_xml = (ROOT / "samples" / "sensor.xml").read_text(encoding="utf-8")
        primary_state = '<SingleValue value="true"><Name textId="ready"/></SingleValue>'
        assert main_xml.count('<SimpleDatatype xsi:type="BooleanT"/>') == 1
        main_xml = main_xml.replace('<SimpleDatatype xsi:type="BooleanT"/>',
                                    f'<SimpleDatatype xsi:type="BooleanT">{primary_state}</SimpleDatatype>')
        main_xml = main_xml.replace('</PrimaryLanguage>', '<Text id="ready" value="Ready"/></PrimaryLanguage>')
        language_xml = (ROOT / "samples" / "sensor-de.xml").read_text(encoding="utf-8")
        language_xml = language_xml.replace('</Language>', '<Text id="ready" value="Bereit"/></Language>')
        archive.writestr("bundle/Synthetic-IODD1.1.xml", main_xml)
        archive.writestr("bundle/Synthetic-IODD1.1-de.xml", language_xml)
    catalog = tomllib.loads(run(cli, "iodd", scratch / "iodd.zip"))
    assert len(catalog["variants"]) == 2
    assert "select the connected product ID" in subprocess.run(
        [str(cli), "import", str(scratch / "iodd.zip"), str(scratch / "ambiguous.toml")],
        capture_output=True, text=True, encoding="utf-8", timeout=10, check=False,
    ).stderr
    imported = scratch / "imported.toml"
    run(cli, "import", scratch / "iodd.zip", imported, "--variant", "SIM-SENSOR")
    decoded = run(cli, "decode", imported, "FFF9")
    assert "level [1] [IODD]" in decoded and "Ready (true)" in decoded
    run(cli, "validate", imported)
    localized = scratch / "localized.toml"
    run(cli, "import", scratch / "iodd.zip", localized, "--variant", "SIM-SENSOR", "--language", "de")
    locale = tomllib.loads(localized.read_text(encoding="utf-8"))["iodd"]["localization"]
    assert locale["selected_language"] == "de" and locale["fallback_text_ids"]
    decoded = run(cli, "decode", localized, "FFF9")
    assert "Füllstand [1]" in decoded and "Bereit (true)" in decoded
    reading = tomllib.loads(run(cli, "read-parameter", localized, "V_SIM_Threshold"))
    assert reading["name"] == "Grenzwert" and reading["choice_name"] == "Nennwert" and reading["raw_value"] == "500"
    _, localized_rows = capture(cli, localized, scratch / "sim-localized", "--seconds", 0.25)
    assert any("Füllstand [1]" in name for name in localized_rows[0])
    logged_states = {row["value.Schalter [2]"] for row in localized_rows}
    assert logged_states <= {"", "true", "false"} and logged_states & {"true", "false"}
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
    verify_diagnostic_snapshots(cli, imported, scratch / "sim-diagnostic-snapshots")
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
            endpoint = f"http://127.0.0.1:{server.server_port}"
            verify_profile_characteristic_loopback(cli, features_profile, endpoint, scratch / "ifm-profile-characteristic")
            verify_process_descriptor_loopback(cli, descriptor_profile, endpoint, scratch / "ifm-process-descriptors")
            verify_standalone_input_capture(
                cli, padded_input_profile, endpoint, scratch / "ifm-padded-input", "integer",
                "noncanonical_input_padding_rows",
                [("canonical", "00016E5A", "valid", "93786"),
                 ("noncanonical", "80016E5A", "valid", "93786"),
                 ("short", "016E5A", "invalid", "")],
                manual_bits=17,
            )
            verify_standalone_input_capture(
                cli, boolean_input_profile, endpoint, scratch / "ifm-boolean-input", "switch",
                "noncanonical_input_boolean_rows",
                [("false", "00", "valid", "false"), ("true", "FF", "valid", "true"),
                 ("noncanonical", "02", "valid", "true"), ("long", "0002", "invalid", "")],
                manual_bits=1,
            )
            verify_numeric_process_capture(cli, float_names_profile, endpoint, scratch / "ifm-process-float-names", "3FA00000", "2.50")
            verify_numeric_process_capture(cli, range_names_profile, endpoint, scratch / "ifm-process-range-names", "3FA00000", "2.50")
            verify_numeric_process_capture(cli, cyclic_formats_profile, endpoint, scratch / "ifm-process-display-formats", "FFFC", "-4")
            verify_process_octet_capture(cli, octet_fixture, endpoint, scratch / "ifm-process-octets")
            verify_process_text_capture(cli, text_profile, endpoint, scratch / "ifm-process-text")
            verify_process_temporal_capture(cli, temporal_profile, endpoint, scratch / "ifm-process-temporal")
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
            verify_layout_condition_reads(cli, layout_profile, parameter_fixture, endpoint,
                                          scratch / "ifm-layout-condition")
            verify_whole_layout_conditions(cli, endpoint, scratch / "ifm-layout-condition-whole")
            verify_diagnostic_snapshots(cli, parameter_fixture, scratch / "ifm-diagnostic-snapshots", loopback=True)
            changed_identity_path = scratch / "changed-device.json"
            Gateway.change_serial_after_status = True
            try:
                changed_identity = tomllib.loads(run(cli, "read-diagnostics", parameter_fixture, "--snapshot", changed_identity_path, expected=1))
            finally:
                Gateway.change_serial_after_status = False
                Gateway.data[Gateway.device + "/serial"] = "FIXTURE-SENSOR"
            assert not changed_identity["observed_identities_equal"]
            assert changed_identity["reads"][0]["status"]["identity"]["serial"] == "FIXTURE-SENSOR"
            assert changed_identity["reads"][1]["status"]["identity"]["serial"] == "CHANGED-SENSOR"
            before_requests = Gateway.requests
            changed = tomllib.loads(run(cli, "review-diagnostics", changed_identity_path, expected=1))
            assert not changed["observed_identities_equal"] and Gateway.requests == before_requests
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

    print("PASS: saved logging candidates and offline plans with exact CSV headers, row/time rotation and selection resets, cyclic manual/override field edits with restoration and CSV selection guards, offline identity pins with explicit clears and separate observed matching, offline connection edits with source/destination preservation, temporal declared defaults with explicit precision and era limits, offline declared defaults with separate snapshot observations, standard allowed-value validation and localized range names, IODD declared cycles and separate timing comparisons, saved IODD device capabilities and offline ProfileCharacteristic reply comparison, process descriptor geometry review for input and output, binary IODD main/language checksum receipts, ZIP and variant selection, standalone Boolean bytes and integer padding review, localized cyclic state and range names, cyclic binary/hex/Boolean decimal views with underlying CSV values, offline decoding, equal updates, on-change, raw-only logging and ordered field overrides, rotation, logger queue/duration telemetry, "
          "stale and invalid data, reconnect, bounded-queue loss, saved-capture verification, bounded signal review, revised mapping exports, local/source bundle verification, hash receipts, CSV joining, "
          "exact cyclic TimeT/TimeSpanT ticks and independent acquisition timing, offline layout selector plans, whole-value layout selector reads and explicit condition reads during cyclic sampling, cyclic text with field-local encoding errors and CSV review, cyclic octet strings with exact byte order and CSV verification, named scalar/record/array reads, typed choice labels and declared-value assessments, text/octet default views and strict width/padding checks, TimeT/TimeSpanT exact ticks and default views, binary/hex/plain-decimal parameter views and saved display preferences, saved parameter snapshots and offline comparison, concurrent explicit port status and cyclic captures with independent reports, read-only AL1351, AL1352 and AL1353 status, ISDU and capture across all ports, one-shot ifm master inspection with supply diagnostics, read-only port configuration and offline snapshot review and comparison, array and localized CSV fields, translated standard parameters/events and fallbacks, supplied units, offline ISDU/event decoding, explicit diagnostics with offline review and comparison of complete, partial and changed-target snapshots, pending-event counts independent of diagnostic slot positions, offline ISDU ErrorType lookup with localized definitions, local XSD receipts and blocked external resolution, subindex restrictions, parallel polling during snapshot reads, and read-only AL1350 loopback")
    print(f"Evidence directory: {scratch}")


if __name__ == "__main__":
    main()
