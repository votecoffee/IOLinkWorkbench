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
                {"index": 19, "subindex": 0}, {"index": 36, "subindex": 0}, {"index": 37, "subindex": 0}, {"index": 64, "subindex": 0}, {"index": 65, "subindex": 0},
                *({"index": 66, "subindex": subindex} for subindex in range(5)),
                *({"index": 67, "subindex": subindex} for subindex in range(8))):
            type(self).isdu_reads += 1
            index = request["data"]["index"]
            if index == self.block_index and self.block_parameter:
                self.parameter_entered.set()
                if not self.parameter_release.wait(timeout=3):
                    response["code"] = 500
            response["data"] = {"value": {
                (19, 0): "53494D2D53454E534F52", (64, 0): "01F4", (65, 0): "666978747572652073656E736F72",
                (36, 0): "02", (37, 0): "E48DFE000000",
                (66, 0): "FFEC010A54455354", (66, 1): "FFEC", (66, 2): "01", (66, 3): "0A", (66, 4): "54455354",
                (67, 0): "02C495F0", (67, 1): "02", (67, 2): "FC", (67, 3): "04", (67, 4): "F9", (67, 5): "05", (67, 6): "FF", (67, 7): "00",
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
    for key in ("host_poll_start_spacing", "host_receive_spacing", "host_new_update_spacing", "gateway_update_spacing"):
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
        reading = subprocess.Popen([cli, "read-diagnostics", fixture] if diagnostics else [cli, "read-parameter", fixture, "V_SIM_Threshold"],
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

    print("PASS: ZIP and variant selection, offline decoding, equal updates, on-change, raw-only logging and ordered field overrides, rotation, "
          "stale and invalid data, reconnect, bounded-queue loss, saved-capture verification, bounded signal review, revised mapping exports, local/source bundle verification, hash receipts, CSV joining, "
          "named scalar/record/array reads, binary/hex/plain-decimal parameter views and saved display preferences, array and localized CSV fields, supplied units, offline ISDU/event decoding, explicit/partial diagnostics, local XSD receipts and blocked external resolution, subindex restrictions, parallel polling, and read-only AL1350 loopback")
    print(f"Evidence directory: {scratch}")


if __name__ == "__main__":
    main()
