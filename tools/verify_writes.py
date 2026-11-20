"""Test the packaged write CLI against an isolated AL1350 loopback fixture."""

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import tempfile
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
                    state.parameters[data["index"], data["subindex"]] = data["value"]
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
    assert "confirmation mismatch" in run([*mode[:-1], "1"], confirm=plan["confirmation"], status=1)
    assert not server.requests
    observed = run(["port-mode"], command="read-current")
    assert observed["point"] == {"kind": "port_mode"} and observed["value"] == 3
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
    pin = preview(parameter)["confirmation"]
    server.vendor = 1234
    assert "identity mismatch" in run(parameter[:5], status=1, command="read-current")
    assert "identity mismatch" in run(parameter, confirm=pin, status=1)
    assert len(server.writes) == 2
    server.vendor = 65535
    observed = run(parameter[:5], command="read-current")
    assert observed["point"] == {"kind": "isdu", "index": 65, "subindex": 1}
    assert observed["value"] == "0001"
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

    def import_for_write(name):
        imported = profile.parent / f"{name}-imported.toml"
        configured = profile.parent / f"{name}-configured.toml"
        subprocess.run([cli, "import", ROOT / f"samples/{name}.xml", imported, "--variant", "SIM-SENSOR"],
                       env=env, check=True, capture_output=True, timeout=15)
        subprocess.run([cli, "set-connection", imported, configured, "--gateway", "ifm-al1350",
                        "--endpoint", f"http://127.0.0.1:{server.server_port}", "--port", "2",
                        "--firmware", "3.1.42", "--expected-mode", "3"],
                       env=env, check=True, capture_output=True, timeout=15)
        return configured

    configured = import_for_write("sensor")
    formats = import_for_write("sensor-formats")
    member = ["isdu", "--index", "66", "--subindex", "3", "--expected", "FF", "--desired", "0A"]
    signed = ["isdu", "--index", "64", "--expected", "0001", "--desired", "FFFE"]
    compound = ["isdu", "--index", "66", "--expected", "0000000000000000", "--desired", "0000000A41000000"]
    before = len(server.requests)
    assert "does not declare this parameter read/write" in run(
        ["isdu", "--index", "65", "--expected", "01", "--desired", "02"], status=1, target=configured)
    assert "does not declare this parameter read/write" in run(
        [*member[:4], "1", *member[5:]], status=1, target=formats)
    assert "outside the IODD choices or ranges" in run([*member[:-1], "0B"], status=1, target=formats)
    assert "width differs from the IODD" in run([*signed[:-1], "FE"], status=1, target=formats)
    assert "width differs from the IODD" in run([*compound[:-1], "00"], status=1, target=formats)
    compound_review = preview(compound, target=formats)["target_and_request"]["isdu_value_review"]
    assert compound_review["datatype"] == "RecordT" and "manual_review" in compound_review
    assert "interpretation" not in compound_review
    signed_review = preview(signed, target=formats)["target_and_request"]["isdu_value_review"]
    assert signed_review["datatype"] == "IntegerT" and signed_review["interpretation"]["raw_value"] == "-2"
    assert "manual_review" not in signed_review
    overlap = preview([*signed[:-1], "01F4"], target=configured)["target_and_request"]["isdu_value_review"]
    assert overlap["interpretation"]["value_assessment"]["status"] == "definition_needs_review"
    assert "manual_review" in overlap
    assert len(server.requests) == before

    server.parameters[66, 3] = "FF"
    request, receipt = apply(member, "readback_matched", target=formats)
    value_review = request["isdu_value_review"]
    assert value_review["basis"] == "iodd" and value_review["datatype"] == "UIntegerT"
    assert value_review["interpretation"]["raw_value"] == "10"
    assert value_review["interpretation"]["value_assessment"]["status"] == "allowed"
    assert receipt["readback"] == server.parameters[66, 3] == "0A"
    print("PASS: checked reads, offline plans and IODD value review, exact target checks, four write paths, readback, and single dispatch on uncertain outcomes")


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
