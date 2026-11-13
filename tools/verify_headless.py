"""Windows CLI acceptance using synthetic data and a loopback-only AL1350 fixture."""

import argparse
import csv
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
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
    violations = []
    device_id = 1
    inject_faults = True
    block_parameter = False
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
                {"index": 19, "subindex": 0}, {"index": 64, "subindex": 0}, {"index": 65, "subindex": 0},
                *({"index": 66, "subindex": subindex} for subindex in range(5)),
                *({"index": 67, "subindex": subindex} for subindex in range(8))):
            type(self).isdu_reads += 1
            index = request["data"]["index"]
            if index == 64 and self.block_parameter:
                self.parameter_entered.set()
                if not self.parameter_release.wait(timeout=3):
                    response["code"] = 500
            response["data"] = {"value": {
                (19, 0): "53494D2D53454E534F52", (64, 0): "01F4", (65, 0): "666978747572652073656E736F72",
                (66, 0): "FFEC010A54455354", (66, 1): "FFEC", (66, 2): "01", (66, 3): "0A", (66, 4): "54455354",
                (67, 0): "02C495F0", (67, 1): "02", (67, 2): "FC", (67, 3): "04", (67, 4): "F9", (67, 5): "05", (67, 6): "FF", (67, 7): "00",
            }[(index, request["data"]["subindex"])]}
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
    return report, rows


def parallel_parameter_read(cli, fixture, output):
    Gateway.device_id = 1
    Gateway.polls = 0
    Gateway.inject_faults = False
    Gateway.block_parameter = True
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
        reading = subprocess.Popen([cli, "read-parameter", fixture, "V_SIM_Threshold"],
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
        for process in processes:
            if process.poll() is None:
                process.kill()
            process.communicate(timeout=5)


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
    resolved = scratch / "resolved.toml"
    definitions = ROOT / "samples" / "synthetic-standard-definitions.xml"
    run(cli, "import", scratch / "iodd.zip", resolved, "--variant", "SIM-SENSOR",
        "--standard-definitions", definitions)
    parameters = tomllib.loads(run(cli, "parameters", resolved))
    assert len(parameters["parameters"]) == 4 and not parameters["standard_parameter_refs"]
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

    clean, rows = capture(cli, source, scratch / "sim-every", "--seconds", 1)
    assert clean["last_status"]["route"] == "synthetic simulator"
    assert clean["row_kind"] == "source_update"
    assert any(a["raw_hex"] == b["raw_hex"] for a, b in zip(rows, rows[1:]))
    assert len({row["source_counter"] for row in rows}) == len(rows)
    assert clean["gateway_update_spacing"]["p50_us"] == 1000

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
            parallel_parameter_read(cli, parameter_fixture, scratch / "ifm-parallel-parameter")
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

    print("PASS: ZIP and variant selection, offline decoding, equal updates, on-change, rotation, "
          "stale and invalid data, reconnect, bounded-queue loss, saved-capture verification, CSV joining, "
          "named scalar/record/array reads, array and localized CSV fields, supplied units, offline ISDU decoding, local XSD receipts and blocked external resolution, subindex restrictions, parallel polling, and read-only AL1350 loopback")
    print(f"Evidence directory: {scratch}")


if __name__ == "__main__":
    main()
