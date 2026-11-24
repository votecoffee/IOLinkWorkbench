"""Test the packaged write CLI against an isolated AL1350 loopback fixture."""

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
from threading import Thread


ROOT = Path(__file__).resolve().parents[1]
PORT = "/iolinkmaster/port[2]"
DEVICE = f"{PORT}/iolinkdevice"
SERIAL = "SYNTHETIC-MASTER"


class Gateway(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        address = request["adr"]
        state = self.server
        state.requests.append(address)
        data = request["data"]
        response = {"cid": request["cid"], "code": 200}
        if address == "/getdatamulti":
            points = {
                "/deviceinfo/productcode": "AL1350", "/deviceinfo/swrevision": "3.1.42",
                "/deviceinfo/serialnumber": SERIAL, f"{PORT}/mode": state.mode,
                f"{DEVICE}/vendorid": state.vendor, f"{DEVICE}/deviceid": 1,
                f"{DEVICE}/productname": "Synthetic sensor",
            }
            response["data"] = {
                point: {"code": 200, "data": points[point]} if point in points else {"code": 404}
                for point in data["datatosend"]
            }
        elif address == f"{DEVICE}/iolreadacyclic":
            response["data"] = {"value": state.parameters[data["index"], data["subindex"]]}
        elif address == f"{DEVICE}/pdout/getdata":
            response["data"] = {"value": state.output}
        elif address in (f"{PORT}/mode/setdata", f"{DEVICE}/pdout/setdata", f"{DEVICE}/iolwriteacyclic"):
            state.writes.append((address, data))
            action, state.next_write = state.next_write, "ack"
            if action == "reject":
                response["code"] = 403
            elif action != "ignore":
                if address == f"{PORT}/mode/setdata":
                    state.mode = data["newvalue"]
                elif address == f"{DEVICE}/pdout/setdata":
                    state.output = data["newvalue"]
                else:
                    value = data["value"] if state.next_isdu_readback is None else state.next_isdu_readback
                    state.next_isdu_readback = None
                    state.parameters[data["index"], data["subindex"]] = value
            if action == "drop_response":
                return
        else:
            response["code"] = 404

        body = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def verify(cli, profile, server):
    env = os.environ.copy()
    env.pop("IOLINK_IFM_PASSWORD", None)

    def run(operation, *, serial=SERIAL, confirm=None, status=0, target=profile, command="write"):
        args = [cli, command, target, "--master-serial", serial]
        if confirm is not None:
            args.extend(["--confirm", confirm])
        process = subprocess.run([*args, *operation], env=env, capture_output=True,
                                 text=True, encoding="utf-8", timeout=15)
        assert process.returncode == status, (args, process.stdout, process.stderr)
        return json.loads(process.stdout) if process.stdout else process.stderr

    def preview(operation, *, serial=SERIAL, target=profile):
        before = len(server.requests)
        planned = run(operation, serial=serial, target=target)
        assert len(server.requests) == before, "preview accessed the network"
        assert len(planned["confirmation"]) == 64
        return planned

    def apply(operation, outcome, *, status=0, target=profile):
        planned = preview(operation, target=target)
        before = len(server.writes)
        receipt = run(operation, confirm=planned["confirmation"], status=status, target=target)
        assert receipt["outcome"] == outcome, receipt
        assert receipt["preview"] == planned
        request = planned["target_and_request"]
        assert server.writes[before:] == [(request["address"], request["request_data"])]
        return request, receipt

    mode = ["port-mode", "--expected", "3", "--desired", "0"]
    plan = preview(mode)
    assert "isdu_comparison" not in plan
    assert "confirmation mismatch" in run([*mode[:-1], "1"], confirm=plan["confirmation"], status=1)
    assert not server.requests
    for subindex in ("0", "1", "2", "5"):
        reserved = ["isdu", "--index", "3", "--subindex", subindex,
                    "--expected", "00", "--desired", "01"]
        assert "DataStorageIndex 3 is reserved" in run(reserved, status=1)
        assert "DataStorageIndex 3 is reserved" in run(reserved[:5], status=1, command="read-current")
        assert "DataStorageIndex 3 is reserved" in run(reserved, confirm=plan["confirmation"], status=1)
    count = ["isdu", "--index", "32", "--subindex", "0", "--expected", "0000", "--desired", "0001"]
    assert "ErrorCount index 32 is read-only" in run(count, status=1)
    assert "ErrorCount index 32 is read-only" in run(count, confirm=plan["confirmation"], status=1)
    assert not server.requests and not server.writes
    observed = run(["port-mode"], command="read-current")
    assert observed["point"] == {"kind": "port_mode"} and observed["value"] == 3
    assert "iodd_value" not in observed
    assert observed["master_serial"] == SERIAL and observed["port"] == 2
    assert observed["observed_utc"].endswith("Z")
    assert "serial pin mismatch" in run(["port-mode"], serial="OTHER-MASTER", status=1,
                                        command="read-current")
    wrong_master = preview(mode, serial="OTHER-MASTER")
    assert "serial pin mismatch" in run(mode, serial="OTHER-MASTER", confirm=wrong_master["confirmation"], status=1)
    assert not server.writes
    request, receipt = apply(mode, "readback_matched")
    assert request["request_data"] == {"newvalue": 0}
    assert receipt["readback"] == server.mode == 0
    assert "expected current" in run(mode, confirm=plan["confirmation"], status=1)
    assert len(server.writes) == 1
    apply(["port-mode", "--expected", "0", "--desired", "3"], "readback_matched")

    parameter = ["isdu", "--index", "65", "--subindex", "1", "--expected", "0001", "--desired", "0002"]
    raw_plan = preview(parameter)
    assert "isdu_comparison" not in raw_plan
    pin = raw_plan["confirmation"]
    server.vendor = 1234
    assert "identity mismatch" in run(parameter[:5], status=1, command="read-current")
    assert "identity mismatch" in run(parameter, confirm=pin, status=1)
    assert len(server.writes) == 2
    server.vendor = 65535
    observed = run(parameter[:5], command="read-current")
    assert observed["point"] == {"kind": "isdu", "index": 65, "subindex": 1}
    assert observed["value"] == "0001"
    assert "iodd_value" not in observed
    server.parameters[65, 1] = "0003"
    assert "expected current" in run(parameter, confirm=pin, status=1)
    assert len(server.writes) == 2
    server.parameters[65, 1] = "0001"
    request, receipt = apply(parameter, "readback_matched")
    assert request["request_data"] == {"index": 65, "subindex": 1, "value": "0002"}
    assert request["isdu_value_review"] == {"basis": "no_iodd_parameter"}
    assert receipt["readback"] == "0002"
    assert "SystemCommand" in run([*parameter[:2], "2", *parameter[3:]], status=1)

    command = ["device-command", "--code", "160", "--meaning", "Synthetic one-shot command"]
    request, receipt = apply(command, "gateway_acknowledged")
    assert request["request_data"] == {"index": 2, "subindex": 0, "value": "A0"}
    assert receipt["readback"] is None
    server.next_write = "drop_response"
    _, receipt = apply(command, "unconfirmed", status=1)
    assert receipt["gateway_code"] is None

    output = ["process-output", "--expected", "01 02", "--desired", "03 04"]
    observed = run(["process-output"], command="read-current")
    assert observed["point"] == {"kind": "process_output"} and observed["value"] == "0102"
    assert "iodd_value" not in observed and "iodd_output" not in observed
    assert len(server.writes) == 5
    server.next_write = "ignore"
    _, receipt = apply(output, "unconfirmed", status=1)
    assert receipt["gateway_code"] == 200 and receipt["readback"] == "0102"
    server.next_write = "reject"
    _, receipt = apply(output, "unconfirmed", status=1)
    assert receipt["gateway_code"] == 403 and receipt["readback"] is None
    request, receipt = apply(output, "readback_matched")
    assert request["request_data"] == {"newvalue": "0304"}
    assert receipt["readback"] == server.output == "0304"
    assert "same full byte width" in run([*output[:-1], "03"], status=1)

    def import_for_write(source):
        imported = profile.parent / f"{source.stem}-imported.toml"
        configured = profile.parent / f"{source.stem}-configured.toml"
        subprocess.run([cli, "import", source, imported, "--variant", "SIM-SENSOR"],
                       env=env, check=True, capture_output=True, timeout=15)
        subprocess.run([cli, "set-connection", imported, configured, "--gateway", "ifm-al1350",
                        "--endpoint", f"http://127.0.0.1:{server.server_port}", "--port", "2",
                        "--firmware", "3.1.42", "--expected-mode", "3"],
                       env=env, check=True, capture_output=True, timeout=15)
        return configured

    output_source = profile.parent / "paired-output.xml"
    input_xml = (ROOT / "samples/sensor.xml").read_text(encoding="utf-8")
    condition = '<Condition variableId="V_SIM_Threshold" value="1"/>'
    output_xml = input_xml.replace(
        '<ProcessData id="PD">', f'<ProcessData id="PD">{condition}', 1
    ).replace('</ProcessData>', """
          <ProcessDataOut id="OUT" bitLength="13">
            <Datatype xsi:type="RecordT" bitLength="13">
              <RecordItem subindex="1" bitOffset="3">
                <SimpleDatatype xsi:type="IntegerT" bitLength="8"/><Name textId="level"/>
              </RecordItem>
              <RecordItem subindex="2" bitOffset="0">
                <SimpleDatatype xsi:type="BooleanT"/><Name textId="switch"/>
              </RecordItem>
            </Datatype><Name textId="sensor"/>
          </ProcessDataOut>
        </ProcessData>
        <ProcessData id="PD_STANDALONE">
          <Condition variableId="V_SIM_Threshold" value="2"/>
          <ProcessDataOut id="OUT_SINGLE" bitLength="8">
            <Datatype xsi:type="UIntegerT" bitLength="8"/><Name textId="level"/>
          </ProcessDataOut>
        </ProcessData>""", 1).replace("<UserInterface>", """
        <UserInterface><ProcessDataRefCollection><ProcessDataRef processDataId="OUT">
          <ProcessDataRecordItemInfo subindex="1" gradient="0.5" offset="1" unitCode="1013"/>
        </ProcessDataRef></ProcessDataRefCollection>""", 1)
    output_source.write_text(output_xml, encoding="utf-8", newline="\n")
    catalog = tomllib.loads(subprocess.check_output([cli, "iodd", output_source], text=True, timeout=15))
    assert catalog["layouts"][0]["output"]["id"] == "OUT"
    assert catalog["layouts"][0]["output"]["packed"] is True
    assert "packed" not in catalog["output_only_layouts"][0]
    assert catalog["output_only_layouts"][0]["id"] == "OUT_SINGLE"
    assert catalog["output_only_layouts"][0]["condition"] == "V_SIM_Threshold = 2"
    output_imported = profile.parent / "output-imported.toml"
    subprocess.run([cli, "import", output_source, output_imported, "--variant", "SIM-SENSOR",
                    "--layout", "IN"], env=env, check=True, capture_output=True, timeout=15)
    assert tomllib.loads(output_imported.read_text(encoding="utf-8"))["iodd"]["output"]["id"] == "OUT"
    paired = profile.parent / "output-configured.toml"
    subprocess.run([cli, "set-connection", output_imported, paired, "--gateway", "ifm-al1350",
                    "--endpoint", f"http://127.0.0.1:{server.server_port}", "--port", "2",
                    "--firmware", "3.1.42", "--expected-mode", "3"], env=env, check=True,
                   capture_output=True, timeout=15)
    request_count = len(server.requests)
    paired_output = ["process-output", "--expected", "A7F1", "--desired", "A012"]
    assert "confirm the active IODD layout condition" in run(paired_output, target=paired, status=1)
    checked = profile.parent / "output-checked.toml"
    checked_text = paired.read_text(encoding="utf-8").replace(
        "confirm_iodd_condition = false", "confirm_iodd_condition = true", 1)
    checked.write_text(checked_text, encoding="utf-8", newline="\n")

    before_writes = len(server.writes)
    server.output = "A7F1"
    live = run(["process-output"], command="read-current", target=checked)
    assert live["value"] == "A7F1" and "iodd_value" not in live
    typed_fields = live["iodd_output"]
    assert typed_fields["layout_id"] == "OUT" and typed_fields["bit_length"] == 13
    assert typed_fields["recorded_condition_confirmation"] is True
    assert typed_fields["fields"][0]["field"]["name"] == "level [1]"
    assert typed_fields["fields"][0]["value"] == {"bits_hex": "FE", "raw": "-2", "value": "0"}
    assert typed_fields["fields"][1]["value"]["raw"] == "true"
    assert "decode_error" not in typed_fields
    unconfirmed = run(["process-output"], command="read-current", target=paired)
    assert unconfirmed["value"] == "A7F1" and unconfirmed["iodd_output"]["fields"] == []
    assert unconfirmed["iodd_output"]["recorded_condition_confirmation"] is False
    assert "active IODD layout condition" in unconfirmed["iodd_output"]["decode_error"]
    server.output = "7F"
    short = run(["process-output"], command="read-current", target=checked)
    assert short["value"] == "7F" and short["iodd_output"]["fields"] == []
    assert "received 1" in short["iodd_output"]["decode_error"]
    server.output = "0304"
    assert len(server.writes) == before_writes

    def decode_output(target, payload):
        return json.loads(subprocess.check_output(
            [cli, "decode-output", target, payload], env=env, timeout=15))

    request_count = len(server.requests)
    recalled = decode_output(paired, "a7 f1")
    assert recalled["raw_hex"] == "A7F1"
    expected = {key: value for key, value in typed_fields.items() if key != "recorded_condition_confirmation"}
    assert recalled["iodd_output"] == expected
    assert decode_output(profile, "01 02") == {"raw_hex": "0102"}
    longer = decode_output(checked, "000001")
    assert longer["raw_hex"] == "000001" and longer["iodd_output"]["fields"] == []
    assert "received 3" in longer["iodd_output"]["decode_error"]

    assert "requires 2 bytes" in run(["process-output", "--expected", "01", "--desired", "02"],
                                     target=checked, status=1)
    assert "changes padding" in run([*paired_output[:-1], "8012"], target=checked, status=1)
    dry = preview(paired_output, target=checked)
    review = dry["target_and_request"]["process_output_review"]
    assert review["layout_id"] == "OUT" and review["bit_length"] == 13
    assert review["condition"] == "V_SIM_Threshold = 1"
    assert review["changed_unmapped_bits"] == [1]
    assert review["fields"][0]["field"]["kind"] == "signed"
    assert review["fields"][0]["field"]["bit_offset"] == 3
    assert review["fields"][0]["field"]["scale"] == 0.5
    assert review["fields"][0]["expected"] == {"bits_hex": "FE", "raw": "-2", "value": "0"}
    assert review["fields"][0]["proposed"] == {"bits_hex": "2", "raw": "2", "value": "2"}
    assert review["fields"][1]["expected"]["raw"] == "true"
    assert review["fields"][1]["proposed"]["raw"] == "false"
    changed_layout = profile.parent / "output-remapped.toml"
    changed_layout.write_text(checked_text.replace("scale = 0.5", "scale = 2.0"),
                              encoding="utf-8", newline="\n")
    assert "confirmation mismatch" in run(paired_output, confirm=dry["confirmation"],
                                          target=changed_layout, status=1)
    malformed = profile.parent / "output-invalid.toml"
    malformed.write_text(checked_text.replace("bit_offset = 3", "bit_offset = 0"),
                         encoding="utf-8", newline="\n")
    assert "overlaps another field" in run(paired_output, target=malformed, status=1)
    assert len(server.requests) == request_count
    writes = len(server.writes)
    server.output = "A7F3"
    assert "expected current" in run(paired_output, confirm=dry["confirmation"], target=checked, status=1)
    assert len(server.writes) == writes
    server.output = "A7F1"
    _, receipt = apply(paired_output, "readback_matched", target=checked)
    assert receipt["preview"]["target_and_request"]["process_output_review"] == review
    assert len(server.writes) == writes + 1 and receipt["readback"] == server.output == "A012"

    field_draft = ["process-output", "--expected", "A7F1", "--field", "level [1]", "--desired-value"]
    typed_output = [*field_draft, "2"]
    typed_preview = preview(typed_output, target=checked)
    assert typed_preview == preview(
        ["process-output", "--expected", "A7F1", "--desired", "A011"], target=checked)
    assert typed_preview["target_and_request"]["process_output_review"]["changed_unmapped_bits"] == []
    typed_boolean = ["process-output", "--expected", "A7F0", "--field", "switch [2]",
                     "--desired-value", "true"]
    assert preview(typed_boolean, target=checked)["target_and_request"]["requested_value"] == "A7F1"
    assert preview([*field_draft, "-1"], target=checked)["target_and_request"]["requested_value"] == "A7F9"
    for value in ("-129", "128"):
        assert "exceeds the declared signed width" in run([*field_draft, value], target=checked, status=1)
    assert "requires 2 bytes" in run([*typed_output[:2], "01", *typed_output[3:]], target=checked, status=1)
    assert "no editable IODD output field" in run(
        [*typed_output[:4], "missing", *typed_output[5:]], target=checked, status=1)
    assert "cannot be used with" in run([*typed_output, "--desired", "A011"], target=checked, status=2)
    assert "--desired-value" in run(typed_output[:-2], target=checked, status=2)
    assert "--field" in run([*typed_output[:3], *typed_output[5:]], target=checked, status=2)
    assert "requires an imported IODD output layout" in run(typed_output, status=1)
    assert "confirm the active IODD layout condition" in run(typed_output, target=paired, status=1)
    legacy = profile.parent / "output-without-packing.toml"
    legacy.write_text(checked_text.replace("packed = true\n", "", 1), encoding="utf-8", newline="\n")
    assert preview(paired_output, target=legacy) == dry
    assert "no editable IODD output field" in run(typed_output, target=legacy, status=1)
    before = len(server.writes)
    assert "expected current" in run(
        typed_output, confirm=typed_preview["confirmation"], target=checked, status=1)
    assert len(server.writes) == before
    server.output = "A7F1"
    request, receipt = apply(typed_output, "readback_matched", target=checked)
    assert request["request_data"] == {"newvalue": "A011"}
    assert len(server.writes) == before + 1 and receipt["readback"] == server.output == "A011"

    # The alternate conditional item carries outputs but declares no input image.
    output_only_profile = profile.parent / "output-only.toml"
    output_only_route = profile.parent / "output-only-route.toml"
    subprocess.run([cli, "import", output_source, output_only_profile, "--variant", "SIM-SENSOR",
                    "--layout", "OUT_SINGLE"], env=env, check=True, capture_output=True, timeout=15)
    standalone = tomllib.loads(output_only_profile.read_text(encoding="utf-8"))
    assert standalone["input_bytes"] == standalone["iodd"]["input_bytes"] == 0
    assert standalone["iodd"]["output"]["id"] == "OUT_SINGLE"
    assert standalone["iodd"]["fields"] == []
    subprocess.run([cli, "set-connection", output_only_profile, output_only_route, "--gateway", "ifm-al1350",
                    "--endpoint", f"http://127.0.0.1:{server.server_port}", "--port", "2",
                    "--firmware", "3.1.42", "--expected-mode", "3"], env=env, check=True,
                   capture_output=True, timeout=15)
    read_requests = len(server.requests)
    recording = profile.parent / "output-only-capture"
    for argv in ([cli, "capture", output_only_route, recording, "--seconds", "0.01"],
                 [cli, "logging", output_only_route], [cli, "decode", output_only_route, "01"]):
        denied = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=15)
        assert denied.returncode == 1 and "output-only layout" in denied.stderr, (argv, denied.returncode, denied.stderr)
    assert not recording.exists() and len(server.requests) == read_requests
    status = tomllib.loads(subprocess.check_output(
        [cli, "status", output_only_route, "--verify-product-id"], env=env, timeout=15).decode("utf-8"))
    assert status["identity"]["product_id"] == "SIM-SENSOR"
    assert server.requests[read_requests:] == ["/getdatamulti", "/getdatamulti", f"{DEVICE}/iolreadacyclic"]
    server.parameters[64, 0] = "000A"
    parameter = tomllib.loads(subprocess.check_output(
        [cli, "read-parameter", output_only_route, "V_SIM_Threshold"], env=env, timeout=15).decode("utf-8"))
    assert parameter["raw_hex"] == "000A"
    recorded = decode_output(output_only_route, "01")
    assert recorded["iodd_output"]["fields"][0]["value"]["raw"] == "1"
    confirm_layout = profile.parent / "output-only-confirmed.toml"
    confirm_layout.write_text(output_only_route.read_text(encoding="utf-8").replace(
        "confirm_iodd_condition = false", "confirm_iodd_condition = true", 1),
        encoding="utf-8", newline="\n")
    server.output = "01"
    current = run(["process-output"], command="read-current", target=confirm_layout)
    assert current["iodd_output"]["fields"][0]["value"]["raw"] == "1"
    _, receipt = apply(["process-output", "--expected", "01", "--field",
                        "level", "--desired-value", "2"], "readback_matched", target=confirm_layout)
    assert receipt["readback"] == server.output == "02"

    acyclic_source = profile.parent / "acyclic.xml"
    acyclic_source.write_text(input_xml.replace(
        '<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>', "", 1),
        encoding="utf-8", newline="\n")
    acyclic = import_for_write(acyclic_source)
    saved = acyclic.read_bytes()
    config = tomllib.loads(saved.decode("utf-8"))
    assert config["input_bytes"] == config["iodd"]["input_bytes"] == 0
    assert config["iodd"]["layout"] == "PD" and config["iodd"]["acyclic_only"]
    assert "output" not in config["iodd"]
    before_requests = len(server.requests)
    denied_capture = profile.parent / "acyclic-capture"
    denied_operations = [
        ["capture", acyclic, denied_capture, "--seconds", "0.01"],
        ["read-current", acyclic, "--master-serial", SERIAL, "process-output"],
        ["write", acyclic, "--master-serial", SERIAL, "process-output", "--expected", "01", "--desired", "02"],
        ["decode-output", acyclic, "01"],
    ]
    for operation in denied_operations:
        denied = subprocess.run([cli, *operation], env=env, capture_output=True, text=True, timeout=15)
        assert denied.returncode == 1 and "acyclic-only layout" in denied.stderr, (operation, denied.stderr)
    assert len(server.requests) == before_requests and not denied_capture.exists()

    server.parameters.update({(36, 0): "02", (37, 0): "E48DFE000000", (64, 0): "000A"})
    snapshot = profile.parent / "acyclic-parameter.json"
    reading = subprocess.run([cli, "read-parameter", acyclic, "V_SIM_Threshold", "--snapshot", snapshot],
                             capture_output=True, env=env, check=True, timeout=15)
    assert tomllib.loads(reading.stdout.decode("utf-8"))["raw_hex"] == "000A"
    snapshot_contents = snapshot.read_bytes()
    before_review = len(server.requests)
    review = subprocess.check_output([cli, "review-parameter", snapshot], env=env, timeout=15)
    assert tomllib.loads(review.decode("utf-8"))["decoded"]["raw_hex"] == "000A"
    assert len(server.requests) == before_review and snapshot.read_bytes() == snapshot_contents
    status = subprocess.check_output([cli, "status", acyclic, "--verify-product-id"], env=env, timeout=15)
    assert tomllib.loads(status.decode("utf-8"))["identity"]["product_id"] == "SIM-SENSOR"
    events = subprocess.check_output([cli, "read-diagnostics", acyclic], env=env, timeout=15)
    diagnostics = tomllib.loads(events.decode("utf-8"))
    assert diagnostics["device_status"]["value"] == 2
    assert diagnostics["detailed_status"]["events"][0]["code"] == 36350
    assert diagnostics["detailed_status"]["events"][0]["definition"]["name"] == "Synthetic range warning"
    _, receipt = apply(["isdu", "--parameter-id", "V_SIM_Threshold", "--expected",
                        "000A", "--desired-value", "11"], "readback_matched", target=acyclic)
    assert receipt["readback"] == "000B" and acyclic.read_bytes() == saved
    assert all("/pdin" not in address and "/pdout" not in address for address in server.requests[before_requests:])

    bool_source = profile.parent / "standalone-boolean.xml"
    bool_source.write_text(input_xml.replace("</ProcessData>", """
          <ProcessDataOut id="OUT_BOOL" bitLength="1">
            <Datatype xsi:type="BooleanT"/><Name textId="switch"/>
          </ProcessDataOut>
        </ProcessData>""", 1), encoding="utf-8", newline="\n")
    bool_profile = import_for_write(bool_source)
    bool_layout = tomllib.loads(bool_profile.read_text(encoding="utf-8"))["iodd"]["output"]
    assert bool_layout["standalone_boolean"] and "packed" not in bool_layout
    typed_on = ["process-output", "--expected", "00", "--field", "switch", "--desired-value", "true"]
    raw_on = ["process-output", "--expected", "00", "--desired", "FF"]
    typed_plan = preview(typed_on, target=bool_profile)
    assert typed_plan == preview(raw_on, target=bool_profile)
    bool_review = typed_plan["target_and_request"]["process_output_review"]
    assert bool_review["standalone_boolean"] and bool_review["changed_unmapped_bits"] == []
    assert bool_review["fields"][0]["expected"] == {"bits_hex": "00", "raw": "false", "value": "false"}
    assert bool_review["fields"][0]["proposed"] == {"bits_hex": "FF", "raw": "true", "value": "true"}
    before_requests = len(server.requests)
    assert "requires 00 (false) or FF (true)" in run([*raw_on[:-1], "02"], target=bool_profile, status=1)
    assert len(server.requests) == before_requests

    server.output = "00"
    _, receipt = apply(typed_on, "readback_matched", target=bool_profile)
    assert receipt["readback"] == server.output == "FF"
    typed_off = ["process-output", "--expected", "FF", "--field", "switch", "--desired-value", "false"]
    _, receipt = apply(typed_off, "readback_matched", target=bool_profile)
    assert receipt["readback"] == server.output == "00"
    server.output = "02"
    live = run(["process-output"], command="read-current", target=bool_profile)
    assert live["value"] == "02" and live["iodd_output"]["fields"][0]["value"]["raw"] == "true"
    assert "noncanonical" in live["iodd_output"]["encoding_warning"]
    assert decode_output(bool_profile, "02")["iodd_output"] == live["iodd_output"]
    assert "encoding_warning" not in decode_output(bool_profile, "FF")["iodd_output"]
    normalize = [*typed_on[:2], "02", *typed_on[3:]]
    normalized_plan = preview(normalize, target=bool_profile)
    assert "noncanonical" in normalized_plan["target_and_request"]["process_output_review"]["expected_encoding_warning"]
    server.output = "01"
    before_writes = len(server.writes)
    assert "expected current" in run(
        normalize, target=bool_profile, confirm=normalized_plan["confirmation"], status=1)
    assert len(server.writes) == before_writes
    server.output = "02"
    server.next_write = "ignore"
    _, receipt = apply(normalize, "unconfirmed", status=1, target=bool_profile)
    assert receipt["gateway_code"] == 200 and receipt["readback"] == "02"

    integer_source = profile.parent / "standalone-integer.xml"
    integer_source.write_text(input_xml.replace("</ProcessData>", """
          <ProcessDataOut id="OUT_INT" bitLength="16">
            <Datatype xsi:type="IntegerT" bitLength="12"/><Name textId="setpoint"/>
          </ProcessDataOut>
        </ProcessData>""", 1), encoding="utf-8", newline="\n")
    integer_profile = import_for_write(integer_source)
    integer_layout = tomllib.loads(integer_profile.read_text(encoding="utf-8"))["iodd"]["output"]
    assert integer_layout["padded_integer"] and "packed" not in integer_layout
    typed_negative = ["process-output", "--expected", "0FFF", "--field", "setpoint", "--desired-value=-2048"]
    raw_negative = ["process-output", "--expected", "0FFF", "--desired", "F800"]
    server.output = "0FFF"
    live = run(["process-output"], command="read-current", target=integer_profile)
    assert live["value"] == "0FFF"
    assert live["iodd_output"]["fields"][0]["value"]["raw"] == "-1"
    assert "noncanonical" in live["iodd_output"]["encoding_warning"]
    assert decode_output(integer_profile, "0FFF")["iodd_output"] == live["iodd_output"]
    before_requests = len(server.requests)
    plan = preview(typed_negative, target=integer_profile)
    assert plan == preview(raw_negative, target=integer_profile)
    integer_review = plan["target_and_request"]["process_output_review"]
    assert integer_review["padded_integer"] and integer_review["changed_unmapped_bits"] == []
    assert "noncanonical" in integer_review["expected_encoding_warning"]
    assert integer_review["fields"][0]["proposed"]["raw"] == "-2048"
    assert "requires canonical sign or zero padding" in run(
        [*raw_negative[:-1], "0800"], target=integer_profile, status=1)
    assert len(server.requests) == before_requests
    _, receipt = apply(typed_negative, "readback_matched", target=integer_profile)
    assert receipt["readback"] == server.output == "F800"
    assert "encoding_warning" not in decode_output(integer_profile, "F800")["iodd_output"]

    configured = import_for_write(ROOT / "samples/sensor.xml")
    formats = import_for_write(ROOT / "samples/sensor-formats.xml")
    relocated_source = profile.parent / "relocated-index.xml"
    formats_xml = (ROOT / "samples/sensor-formats.xml").read_text(encoding="utf-8")
    relocated_source.write_text(formats_xml.replace('index="64"', 'index="69"', 1),
                                encoding="utf-8", newline="\n")
    relocated = import_for_write(relocated_source)
    array_source = profile.parent / "writable-array.xml"
    array_text = (ROOT / "samples/sensor-array.xml").read_text(encoding="utf-8")
    array_source.write_text(array_text.replace('accessRights="ro"', 'accessRights="rw"')
                            .replace('upperValue="7"', 'upperValue="5"')
                            .replace('subindexAccessSupported="true"', 'subindexAccessSupported="false"'),
                            encoding="utf-8", newline="\n")
    arrays = import_for_write(array_source)
    write_only_source = profile.parent / "record-with-write-only.xml"
    record_text = (ROOT / "samples/sensor-formats.xml").read_text(encoding="utf-8")
    whole_record_source = profile.parent / "whole-record-only.xml"
    whole_record_source.write_text(record_text.replace(
        'RecordT" bitLength="64" subindexAccessSupported="true"',
        'RecordT" bitLength="64" subindexAccessSupported="false"'), encoding="utf-8", newline="\n")
    whole_record = import_for_write(whole_record_source)
    write_only_source.write_text(record_text.replace('subindex="2" bitOffset="40"',
                                 'subindex="2" bitOffset="40" accessRightRestriction="wo"'),
                                 encoding="utf-8", newline="\n")
    write_only_record = import_for_write(write_only_source)
    unknown_width_source = profile.parent / "record-with-unknown-width.xml"
    unknown_width_source.write_text(record_text.replace('fixedLength="4" encoding="US-ASCII"',
                                                       'encoding="US-ASCII"'),
                                    encoding="utf-8", newline="\n")
    unknown_width_record = import_for_write(unknown_width_source)
    def import_writable_sample(name):
        source = profile.parent / f"{name}-writable.xml"
        text = (ROOT / f"samples/{name}.xml").read_text(encoding="utf-8")
        source.write_text(text.replace('accessRights="ro"', 'accessRights="rw"'),
                          encoding="utf-8", newline="\n")
        return import_for_write(source)

    values = import_writable_sample("sensor-values")
    strings = import_writable_sample("sensor-strings")
    temporal = import_writable_sample("sensor-time")
    integer_source = profile.parent / "integer-containers.xml"
    integer_xml = (ROOT / "samples/sensor-formats.xml").read_text(encoding="utf-8")
    integer_source.write_text(
        integer_xml.replace('xsi:type="IntegerT" bitLength="16"', 'xsi:type="IntegerT" bitLength="20"', 1)
        .replace('</VariableCollection>', '''
        <Variable id="V_SIM_U40" index="68" accessRights="rw">
          <Datatype xsi:type="UIntegerT" bitLength="40"/><Name textId="threshold"/>
        </Variable></VariableCollection>'''), encoding="utf-8", newline="\n")
    integers = import_for_write(integer_source)
    member = ["isdu", "--index", "66", "--subindex", "3", "--expected", "FF", "--desired", "0A"]
    named_member = ["isdu", "--parameter-id", "V_SIM_Record", *member[3:]]
    signed = ["isdu", "--index", "64", "--expected", "0001", "--desired", "FFFE"]
    typed_signed = [*signed[:-2], "--desired-value", "-2"]
    named_signed = ["isdu", "--parameter-id", "V_SIM_Threshold", *typed_signed[3:]]
    typed_bool = ["isdu", "--index", "66", "--subindex", "2", "--expected", "01", "--desired-value", "false"]
    typed_true = [*typed_bool[:6], "00", "--desired-value", "true"]
    typed_short_member = ["isdu", "--index", "85", "--subindex", "1", "--expected", "00", "--desired-value", "-1"]
    typed_20 = [*signed[:4], "00000001", "--desired-value", "-2"]
    typed_40 = ["isdu", "--index", "68", "--expected", "0000000000000000", "--desired-value", "1099511627775"]
    typed_wide = ["isdu", "--index", "82", "--expected", "0000000000000000", "--desired-value", "18446744073709551615"]
    typed_float = ["isdu", "--index", "83", "--expected", "3FA00000", "--desired-value", "-2.5"]
    typed_text = ["isdu", "--index", "77", "--expected", "58", "--desired-value", " µm "]
    typed_octets = ["isdu", "--index", "79", "--subindex", "2", "--expected", "00000000",
                    "--desired-value", "0x00,0x80,0xFF,0x2C"]
    typed_time = ["isdu", "--index", "71", "--expected", "9DFF440000000000",
                  "--desired-value", "1984-01-01T00:00:00.001Z"]
    span_address = ["isdu", "--index", "73", "--subindex", "2", "--expected", "0000000000000000"]
    typed_span = [*span_address, "--desired-value=-PT0.001S"]
    compound = ["isdu", "--index", "66", "--expected", "FFEC010A41000000", "--desired", "FFEC000A42000000"]
    array = ["isdu", "--index", "67", "--expected", "A2C49570", "--desired", "A2C495F0"]
    packed_record = ["isdu", "--parameter-id", "V_SIM_Record", "--expected", "FFEC800A41000000",
                     "--member-subindex", "2", "--desired-value", "true"]
    packed_array = [*array[:-2], "--member-subindex", "6", "--desired-value", "-1"]
    before = len(server.requests)
    for command, options in [("write", signed[3:]), ("read-current", [])]:
        assert "--index" in run(["isdu", *options], status=2, target=formats, command=command)
        assert "cannot be used with" in run(
            ["isdu", "--index", "64", "--parameter-id", "V_SIM_Threshold", *options],
            status=2, target=formats, command=command)
        assert "requires an imported IODD" in run(
            ["isdu", "--parameter-id", "V_SIM_Threshold", *options], status=1, command=command)
        assert "not in the imported IODD catalog" in run(
            ["isdu", "--parameter-id", "v_sim_threshold", *options], status=1, target=formats, command=command)
        assert "does not declare this parameter read/write" in run(
            ["isdu", "--parameter-id", "V_SIM_Label", *options], status=1, target=configured, command=command)
        assert "Use device-command" in run(
            ["isdu", "--parameter-id", "V_SIM_Command", *options], status=1, target=configured, command=command)
        assert "no declared subindex" in run(
            ["isdu", "--parameter-id", "V_SIM_Record", "--subindex", "99", *options],
            status=1, target=formats, command=command)
    signed_by_id = preview(named_signed, target=formats)
    assert signed_by_id == preview(typed_signed, target=formats)
    moved = preview(named_signed, target=relocated)
    assert moved["target_and_request"]["request_data"]["index"] == 69
    assert "confirmation mismatch" in run(
        named_signed, confirm=signed_by_id["confirmation"], status=1, target=relocated)
    assert preview(named_member, target=formats) == preview(member, target=formats)
    assert "does not permit access to this subindex" in run(member, status=1, target=whole_record)
    record_edit = preview(packed_record, target=whole_record)
    assert record_edit == preview([*packed_record[:5], "--desired", "FFEC810A41000000"], target=whole_record)
    assert [element["subindex"] for element in record_edit["isdu_comparison"]["changed_members"]] == [2]
    assert preview(packed_array, target=arrays) == preview(array, target=arrays)
    assert "does not declare this member read/write" in run(
        [*packed_record[:6], "1", *packed_record[7:]], status=1, target=whole_record)
    assert "greater than zero" in run([*packed_array[:6], "0", *packed_array[7:]], status=1, target=arrays)
    assert "no declared subindex" in run([*packed_array[:6], "8", *packed_array[7:]], status=1, target=arrays)
    assert "whole-parameter write" in run([*packed_array, "--subindex", "2"], status=1, target=arrays)
    assert "cannot be used with" in run([*array, "--member-subindex", "6"], status=2, target=arrays)
    assert "expected compound payload width" in run(
        [*packed_record[:4], "01", *packed_record[5:]], status=1, target=whole_record)
    assert "write-only and cannot be verified by readback" in run(packed_record, status=1, target=write_only_record)
    assert "requires an IODD declaration" in run(packed_array, status=1)
    assert "whole IODD record or array" in run(
        [*typed_signed, "--member-subindex", "1"], status=1, target=formats)
    assert "does not declare this parameter read/write" in run(
        ["isdu", "--index", "65", "--expected", "01", "--desired", "02"], status=1, target=configured)
    assert "does not declare this parameter read/write" in run(
        [*member[:4], "1", *member[5:]], status=1, target=formats)
    assert "--desired" in run(signed[:-2], status=2, target=formats)
    assert "cannot be used with" in run([*signed, "--desired-value", "-2"], status=2, target=formats)
    assert "requires an IODD declaration" in run(typed_signed, status=1)
    assert "does not declare this parameter read/write" in run(
        [*typed_signed[:2], "65", *typed_signed[3:]], status=1, target=configured)
    assert "unavailable for the declared datatype or width" in run(
        [*compound[:-2], "--desired-value", "10"], status=1, target=formats)
    assert "expected true, false, 1 or 0" in run([*typed_bool[:-1], "2"], status=1, target=formats)
    assert "exceeds the declared signed width" in run([*typed_signed[:-1], "-32769"], status=1, target=formats)
    assert "outside the IODD choices or ranges" in run([*member[:-2], "--desired-value", "11"], status=1, target=formats)
    assert "outside the IODD choices or ranges" in run([*member[:-1], "0B"], status=1, target=formats)
    assert "width differs from the IODD" in run([*signed[:-1], "FE"], status=1, target=formats)
    assert "width differs from the IODD" in run([*compound[:-1], "00"], status=1, target=formats)
    assert "expected compound payload width differs" in run(
        [*compound[:4], "00", *compound[5:]], status=1, target=formats)
    assert "changes read-only subindex 1" in run([*compound[:-1], "FFED000A42000000"], status=1, target=formats)
    assert "outside declared writable members" in run([*compound[:-1], "FFEC020A42000000"], status=1, target=formats)
    assert "outside the IODD choices or ranges" in run([*compound[:-1], "FFEC000B42000000"], status=1, target=formats)
    assert "is not US-ASCII" in run([*compound[:-1], "FFEC000AFF000000"], status=1, target=formats)
    assert "subindex 2 is write-only" in run(compound, status=1, target=write_only_record)
    assert "IODD width is unavailable for subindex 4" in run(compound, status=1, target=unknown_width_record)
    compound_plan = preview(compound, target=formats)
    compound_review = compound_plan["target_and_request"]["isdu_value_review"]
    assert compound_review["datatype"] == "RecordT" and "manual_review" not in compound_review
    assert compound_review["preserved_read_only"] == [1]
    fields = compound_review["interpretation"]["record_items"]
    assert [(field["subindex"], field["value"]["raw_value"]) for field in fields] == [
        (1, "-20"), (2, "false"), (3, "10"), (4, "B")]
    record_comparison = compound_plan["isdu_comparison"]
    assert record_comparison["expected"]["raw_hex"] == compound[4]
    assert record_comparison["changed_members"] == [
        {"subindex": 2, "name": "Switch", "bit_offset": 40, "expected_hex": "01", "proposed_hex": "00"},
        {"subindex": 4, "name": "Label", "bit_offset": 0, "expected_hex": "41000000", "proposed_hex": "42000000"},
    ]
    assert "does not permit access to this subindex" in run(
        [*array[:3], "--subindex", "6", "--expected", "07", "--desired", "0F"], status=1, target=arrays)
    assert "outside declared writable members" in run([*array[:-1], "02C495F0"], status=1, target=arrays)
    assert "outside the IODD choices or ranges" in run([*array[:-1], "A2C49560"], status=1, target=arrays)
    array_plan = preview(array, target=arrays)
    array_review = array_plan["target_and_request"]["isdu_value_review"]
    assert array_review["datatype"] == "ArrayT" and "manual_review" not in array_review
    assert [(field["bit_offset"], field["value"]["raw_value"])
            for field in array_review["interpretation"]["array_items"]] == [
                (24, "2"), (20, "-4"), (16, "4"), (12, "-7"), (8, "5"), (4, "-1"), (0, "0")]
    array_comparison = array_plan["isdu_comparison"]
    assert array_comparison["expected"]["raw_hex"] == array[4]
    assert [(member["subindex"], member["bit_offset"], member["expected_hex"], member["proposed_hex"])
            for member in array_comparison["changed_members"]] == [(6, 4, "07", "0F")]
    signed_review = preview(signed, target=formats)["target_and_request"]["isdu_value_review"]
    assert signed_review["datatype"] == "IntegerT" and signed_review["interpretation"]["raw_value"] == "-2"
    assert "manual_review" not in signed_review
    prior = preview([*signed[:4], "01", *signed[5:]], target=formats)["isdu_comparison"]
    assert prior["expected"]["raw_hex"] == "01" and "width differs" in prior["expected"]["decode_error"]
    assert "changed_members" not in prior
    prior = preview(member, target=formats)["isdu_comparison"]["expected"]
    assert prior["raw_value"] == "255" and prior["value_assessment"]["status"] == "outside_definition"
    assert preview(typed_signed, target=formats) == preview(signed, target=formats)
    assert preview(typed_bool, target=formats) == preview([*typed_bool[:-2], "--desired", "00"], target=formats)
    assert preview(typed_true, target=formats) == preview([*typed_true[:-2], "--desired", "FF"], target=formats)
    assert preview(typed_short_member, target=values) == preview(
        [*typed_short_member[:-2], "--desired", "FF"], target=values)
    signed_wire = preview(typed_20, target=integers)["target_and_request"]
    assert signed_wire["request_data"]["value"] == "FFFFFFFE"
    assert signed_wire["isdu_value_review"]["interpretation"]["raw_value"] == "-2"
    compact = preview([*typed_20[:-2], "--desired", "FFFFFE"], target=integers)["target_and_request"]
    assert compact["isdu_value_review"]["interpretation"]["raw_value"] == "-2"
    unsigned_wire = preview(typed_40, target=integers)["target_and_request"]
    assert unsigned_wire["request_data"]["value"] == "000000FFFFFFFFFF"
    assert unsigned_wire["isdu_value_review"]["interpretation"]["raw_value"] == typed_40[-1]
    for operation, invalid in [(typed_20, "000FFFFE"), (typed_40, "010000FFFFFFFFFF")]:
        assert "integer container padding differs" in run([*operation[:-2], "--desired", invalid], status=1, target=integers)
    assert "exceeds the declared unsigned width" in run([*typed_40[:-1], "1099511627776"], status=1, target=integers)
    wide_plan = preview(typed_wide, target=values)["target_and_request"]
    assert wide_plan["request_data"]["value"] == "FFFFFFFFFFFFFFFF"
    assert wide_plan["isdu_value_review"]["interpretation"]["raw_value"] == typed_wide[-1]
    assert preview(typed_float, target=values) == preview([*typed_float[:-2], "--desired", "C0200000"], target=values)
    infinity = preview([*typed_float[:-2], "--desired-value=-INF"], target=values)["target_and_request"]
    assert infinity["request_data"]["value"] == "FF800000"
    assert "outside the IODD choices or ranges" in run([*typed_float[:-1], "0"], status=1, target=values)
    assert "NaN" in run([*typed_float[:-1], "NaN"], status=1, target=values)
    assert "non-finite numeric literal" in run([*typed_float[:-1], "1e40"], status=1, target=values)
    text_plan = preview(typed_text, target=strings)["target_and_request"]
    assert text_plan["request_data"]["value"] == "20C2B56D20000000"
    assert text_plan["isdu_value_review"]["interpretation"]["raw_value"] == " µm "
    assert text_plan["text_readback"] == {"fixed_length": 8, "encoding": "UTF-8"}
    assert "exceeds the declared octet length" in run([*typed_text[:-1], "µ" * 5], status=1, target=strings)
    assert "text is not US-ASCII" in run(
        ["isdu", "--index", "78", "--subindex", "3", "--expected", "4F4B", "--desired-value", "µm"],
        status=1, target=strings)
    empty = preview([*typed_text[:-2], "--desired-value="], target=strings)["target_and_request"]
    assert empty["request_data"]["value"] == "0000000000000000"
    assert empty["isdu_value_review"]["interpretation"]["raw_value"] == ""
    assert preview(typed_octets, target=strings) == preview([*typed_octets[:-2], "--desired", "0080FF2C"], target=strings)
    assert "octet count differs from the declared length" in run(
        [*typed_octets[:-1], "0x00,0x80,0xFF"], status=1, target=strings)
    assert "invalid octet" in run([*typed_octets[:-1], "0x00,0xXX,0xFF,0x2C"], status=1, target=strings)
    assert preview(typed_time, target=temporal) == preview(
        [*typed_time[:-2], "--desired", "9DFF440000418938"], target=temporal)
    assert preview(typed_span, target=temporal) == preview(
        [*span_address, "--desired", "FFFFFFFFFFBE76C8"], target=temporal)
    time_detail = preview(typed_time, target=temporal)["target_and_request"]["isdu_value_review"]["interpretation"]["time_value"]
    span_detail = preview(typed_span, target=temporal)["target_and_request"]["isdu_value_review"]["interpretation"]["time_value"]
    assert time_detail["value"] == "1984-01-01 00:00:00.001 UTC" and time_detail["sub_millisecond_truncated"]
    assert span_detail["value"] == "-00:00:00.001" and span_detail["raw_ticks"] == "-4294968"
    rollover = preview([*typed_time[:-1], "2036-02-07T06:28:16Z"], target=temporal)["target_and_request"]
    assert rollover["request_data"]["value"] == "0000000000000000"
    minimum = preview([*span_address, "--desired-value=-PT2147483648S"], target=temporal)["target_and_request"]
    assert minimum["request_data"]["value"] == "8000000000000000"
    for invalid in ("1984-01-01T00:00:00", "1984-01-01T00:00:00+01:00", "1984-01-01Z"):
        assert "requires yyyy-mm-ddThh:mm:ss[.fff]Z" in run([*typed_time[:-1], invalid], status=1, target=temporal)
    for invalid in ("1983-12-31T23:59:59.999Z", "2120-02-07T06:28:16Z"):
        assert "outside the unambiguous default era window" in run([*typed_time[:-1], invalid], status=1, target=temporal)
    assert "one to three fractional digits" in run(
        [*typed_time[:-1], "1984-01-01T00:00:00.0001Z"], status=1, target=temporal)
    assert "signed 64-bit tick range" in run([*span_address, "--desired-value=-PT2147483648.001S"], status=1, target=temporal)
    overlap = preview([*signed[:-1], "01F4"], target=configured)["target_and_request"]["isdu_value_review"]
    assert overlap["interpretation"]["value_assessment"]["status"] == "definition_needs_review"
    assert "manual_review" in overlap
    assert len(server.requests) == before

    writes = len(server.writes)
    before = len(server.requests)
    server.parameters[64, 0] = "01F4"
    observed = run(["isdu", "--index", "64"], target=configured, command="read-current")
    value = observed["iodd_value"]
    assert value["parameter_id"] == "V_SIM_Threshold" and value["datatype"] == "IntegerT"
    assert observed["value"] == value["interpretation"]["raw_hex"] == "01F4"
    assert value["interpretation"]["raw_value"] == "500"
    assert any(display["value"] == "52.0" for display in value["interpretation"]["displays"])
    assert server.requests[before:] == ["/getdatamulti", f"{DEVICE}/iolreadacyclic", f"{DEVICE}/iolreadacyclic"]
    by_id = run(named_signed[:3], target=configured, command="read-current")
    assert by_id.pop("observed_utc").endswith("Z")
    observed.pop("observed_utc")
    assert by_id == observed

    server.parameters[66, 0] = "0102"
    observed = run(["isdu", "--index", "66"], target=formats, command="read-current")
    bad = observed["iodd_value"]["interpretation"]
    assert observed["value"] == bad["raw_hex"] == "0102"
    assert bad["raw_value"] is None and "width differs" in bad["decode_error"]
    assert len(server.writes) == writes

    server.parameters[66, 3] = "FF"
    observed = run(named_member[:5], target=formats, command="read-current")
    assert observed["value"] == "FF" and observed["iodd_value"]["interpretation"]["raw_value"] == "255"
    assert observed["iodd_value"]["interpretation"]["value_assessment"]["status"] == "outside_definition"
    request, receipt = apply(named_member, "readback_matched", target=formats)
    value_review = request["isdu_value_review"]
    assert value_review["basis"] == "iodd" and value_review["datatype"] == "UIntegerT"
    assert value_review["interpretation"]["raw_value"] == "10"
    assert value_review["interpretation"]["value_assessment"]["status"] == "allowed"
    assert receipt["readback"] == server.parameters[66, 3] == "0A"

    server.parameters[66, 2] = "01"
    _, receipt = apply(typed_bool, "readback_matched", target=formats)
    assert receipt["readback"] == server.parameters[66, 2] == "00"
    _, receipt = apply(typed_true, "readback_matched", target=formats)
    assert receipt["readback"] == server.parameters[66, 2] == "FF"
    server.parameters[66, 2] = "00"
    server.next_isdu_readback = "01"
    _, receipt = apply(typed_true, "unconfirmed", status=1, target=formats)
    assert receipt["readback"] == "01"

    server.parameters[64, 0] = "00000001"
    _, receipt = apply(typed_20, "readback_matched", target=integers)
    assert receipt["readback"] == "FFFFFFFE"
    server.parameters[68, 0] = "0000000000000000"
    _, receipt = apply(typed_40, "readback_matched", target=integers)
    assert receipt["readback"] == "000000FFFFFFFFFF"

    server.parameters[83, 0] = "3FA00000"
    _, receipt = apply(typed_float, "readback_matched", target=values)
    assert receipt["readback"] == "C0200000"
    text_plan = preview(typed_text, target=strings)
    server.parameters[77, 0] = "5800"
    writes = len(server.writes)
    assert "write blocked before dispatch" in run(
        typed_text, confirm=text_plan["confirmation"], status=1, target=strings)
    assert len(server.writes) == writes

    server.parameters[77, 0] = "58"
    _, receipt = apply(typed_text, "readback_matched", target=strings)
    assert receipt["readback"] == "20C2B56D20000000"

    text_update = [*typed_text[:-1], "µm"]
    for readback, outcome, status in [
        ("C2B56D", "text_readback_matched", 0),
        ("C2B56D20", "unconfirmed", 1),
        ("C2B56D0041", "unconfirmed", 1),
        ("C2B56D" + "00" * 6, "unconfirmed", 1),
        ("C2", "unconfirmed", 1),
    ]:
        server.parameters[77, 0] = "58"
        server.next_isdu_readback = readback
        _, receipt = apply(text_update, outcome, status=status, target=strings)
        assert receipt["gateway_code"] == 200 and receipt["readback"] == readback

    server.parameters[77, 0] = "58"
    server.next_isdu_readback = "C2B56D"
    server.next_write = "drop_response"
    _, receipt = apply(text_update, "unconfirmed", status=1, target=strings)
    assert receipt["gateway_code"] is None and receipt["readback"] is None
    assert server.parameters[77, 0] == "C2B56D"

    server.parameters[77, 0] = "58"
    server.next_isdu_readback = "00"
    _, receipt = apply([*typed_text[:-2], "--desired-value="], "text_readback_matched", target=strings)
    assert receipt["readback"] == "00"

    server.parameters[78, 3] = "5A"
    server.next_isdu_readback = "4F4B0000"
    ascii_member = ["isdu", "--index", "78", "--subindex", "3", "--expected", "5A", "--desired", "4F4B"]
    member_request, receipt = apply(ascii_member, "text_readback_matched", target=strings)
    assert member_request["text_readback"] == {"fixed_length": 4, "encoding": "US-ASCII"}
    assert receipt["readback"] == "4F4B0000"

    server.parameters[77, 0] = "58"
    server.next_isdu_readback = "C2B56D"
    unknown_request, receipt = apply([*typed_text[:-2], "--desired", "C2B56D0000000000"],
                                     "unconfirmed", status=1)
    assert "text_readback" not in unknown_request and receipt["readback"] == "C2B56D"

    server.parameters[79, 2] = "00000000"
    _, receipt = apply(typed_octets, "readback_matched", target=strings)
    assert receipt["readback"] == "0080FF2C"

    server.parameters[71, 0] = "9DFF440000000000"
    _, receipt = apply(typed_time, "readback_matched", target=temporal)
    assert receipt["readback"] == "9DFF440000418938"
    server.parameters[73, 2] = "0000000000000000"
    _, receipt = apply(typed_span, "readback_matched", target=temporal)
    assert receipt["readback"] == "FFFFFFFFFFBE76C8"
    server.parameters[73, 2] = "0000000000000000"
    server.next_isdu_readback = "FFFFFFFFFFBE76C7"
    _, receipt = apply(typed_span, "unconfirmed", status=1, target=temporal)
    assert receipt["readback"] == "FFFFFFFFFFBE76C7"

    compound_plan = preview(compound, target=formats)
    server.parameters[66, 0] = "FFED010A41000000"
    writes = len(server.writes)
    assert "write blocked before dispatch" in run(
        compound, confirm=compound_plan["confirmation"], status=1, target=formats)
    assert len(server.writes) == writes
    server.parameters[66, 0] = compound[4]
    _, receipt = apply(compound, "readback_matched", target=formats)
    assert receipt["readback"] == server.parameters[66, 0] == compound[-1]
    server.parameters[66, 0] = compound[4]
    server.next_isdu_readback = compound[-1][:-2]
    compound_request, receipt = apply(compound, "unconfirmed", status=1, target=formats)
    assert "text_readback" not in compound_request and receipt["readback"] == compound[-1][:-2]
    server.parameters[67, 0] = array[4]
    _, receipt = apply(packed_array, "readback_matched", target=arrays)
    assert receipt["readback"] == server.parameters[67, 0] == array[-1]
    server.parameters[66, 0] = "FFED800A41000000"
    writes = len(server.writes)
    assert "write blocked before dispatch" in run(
        packed_record, confirm=record_edit["confirmation"], status=1, target=whole_record)
    assert len(server.writes) == writes
    server.parameters[66, 0] = packed_record[4]
    request, receipt = apply(packed_record, "readback_matched", target=whole_record)
    assert request["request_data"]["subindex"] == 0
    assert receipt["readback"] == "FFEC810A41000000"
    print("PASS: checked reads and ISDU writes by index or IODD ID, typed compound member edits, observed, offline, and staged process outputs including standalone Boolean and padded integer outputs, output-only profiles, acyclic-only profiles and diagnostic reads, exact target checks, exact and StringT readback, and single dispatch on uncertain outcomes")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=lambda path: Path(path).resolve())
    args = parser.parse_args()
    local = ROOT / ".local"
    local.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="write-fixture-", dir=local) as directory, \
            ThreadingHTTPServer(("127.0.0.1", 0), Gateway) as server:
        server.mode = 3
        server.vendor = 65535
        server.output = "0102"
        server.parameters = {(65, 1): "0001", (19, 0): "SIM-SENSOR".encode().hex(),
                             (21, 0): "SIM-0001".encode().hex()}
        server.requests = []
        server.writes = []
        server.next_write = "ack"
        server.next_isdu_readback = None
        text = (ROOT / "samples/simulator.toml").read_text(encoding="utf-8")
        text = text.replace('gateway = "simulator"', 'gateway = "ifm_al1350"')
        text = text.replace("port = 1", f'port = 2\nendpoint = "http://127.0.0.1:{server.server_port}"\n'
                                      'ifm_mode = 3\nifm_firmware = "3.1.42"\n')
        text = text.replace('product_id = "SIM-SENSOR"', 'product_id = "SIM-SENSOR"\nserial = "SIM-0001"')
        profile = Path(directory) / "fixture.toml"
        profile.write_text(text, encoding="utf-8")
        worker = Thread(target=server.serve_forever)
        worker.start()
        try:
            verify(args.cli, profile, server)
            assert profile.read_text(encoding="utf-8") == text
        finally:
            server.shutdown()
            worker.join()


if __name__ == "__main__":
    main()
