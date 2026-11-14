"""Windows CLI interruption/flush checks using an isolated hidden console and synthetic input."""

import argparse
import csv
import ctypes
from ctypes import wintypes
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time
import tomllib
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]
LIMIT = 1_048_576
TOKEN_ENV = "IOLINK_CONSOLE_TEST_TOKEN"


def require(condition, message):
    # Keep safety and acceptance checks enabled even under python -O.
    if not condition:
        raise AssertionError(message)


def environment():
    return {k: v for k, v in os.environ.items()
            if k.upper() not in ("IOLINK_IFM_PASSWORD", TOKEN_ENV)}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_text(path):
    data = path.read_bytes()
    require(len(data) <= LIMIT, f"Oversized test output: {path}")
    return data.decode("utf-8")


class Windows:
    def __init__(self):
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.user = ctypes.WinDLL("user32", use_last_error=True)
        signatures = {
            "GetConsoleProcessList": ([ctypes.POINTER(wintypes.DWORD), wintypes.DWORD], wintypes.DWORD),
            "GetConsoleWindow": ([], wintypes.HWND),
            "GenerateConsoleCtrlEvent": ([wintypes.DWORD, wintypes.DWORD], wintypes.BOOL),
            "SetConsoleCtrlHandler": ([ctypes.c_void_p, wintypes.BOOL], wintypes.BOOL),
            "CreateJobObjectW": ([ctypes.c_void_p, wintypes.LPCWSTR], wintypes.HANDLE),
            "SetInformationJobObject": ([wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD], wintypes.BOOL),
            "AssignProcessToJobObject": ([wintypes.HANDLE, wintypes.HANDLE], wintypes.BOOL),
            "CloseHandle": ([wintypes.HANDLE], wintypes.BOOL),
        }
        for name, (args, result) in signatures.items():
            function = getattr(self.kernel, name)
            function.argtypes, function.restype = args, result
        self.user.IsWindowVisible.argtypes = [wintypes.HWND]
        self.user.IsWindowVisible.restype = wintypes.BOOL

    @staticmethod
    def checked(result):
        if not result:
            raise ctypes.WinError(ctypes.get_last_error())
        return result

    def console(self, expected):
        ids = (wintypes.DWORD * 64)()
        count = self.checked(self.kernel.GetConsoleProcessList(ids, len(ids)))
        require(count <= len(ids) and set(ids[:count]) == expected,
                "Refusing console event: console must contain only this worker and its test CLI")
        window = self.kernel.GetConsoleWindow()
        require(not window or not self.user.IsWindowVisible(window), "Test console must stay hidden")

    def signal(self, run, event):
        require(run.process.poll() is None, "Test CLI exited before interruption")
        self.console({os.getpid(), run.process.pid})
        # CTRL+C cannot target a group: broadcast only in this private console.
        # CTRL+BREAK targets the newly created CLI process group.
        self.checked(self.kernel.GenerateConsoleCtrlEvent(event, run.process.pid if event else 0))

    def job(self):
        class Basic(ctypes.Structure):
            _fields_ = [("process_time", ctypes.c_longlong), ("job_time", ctypes.c_longlong),
                        ("flags", wintypes.DWORD), ("minimum", ctypes.c_size_t),
                        ("maximum", ctypes.c_size_t), ("active", wintypes.DWORD),
                        ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                        ("scheduling", wintypes.DWORD)]

        class Extended(ctypes.Structure):
            _fields_ = [("basic", Basic), ("io", ctypes.c_ulonglong * 6),
                        ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                        ("peak_process", ctypes.c_size_t), ("peak_job", ctypes.c_size_t)]

        handle = self.checked(self.kernel.CreateJobObjectW(None, None))
        try:
            limits = Extended()
            limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            self.checked(self.kernel.SetInformationJobObject(handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)))
            return handle
        except BaseException:
            self.kernel.CloseHandle(handle)
            raise


class Run:
    def __init__(self, cli, profile, output, event, extra=()):
        self.cli, self.profile, self.output = cli, profile, output
        self.original = profile.read_bytes()
        self.outpath, self.errpath = output.with_suffix(".stdout.toml"), output.with_suffix(".stderr.log")
        self.stdout, self.stderr = self.outpath.open("xb"), self.errpath.open("xb")
        self.process = None
        try:
            self.process = subprocess.Popen(
                [str(cli), "capture", str(profile), str(output), "--seconds", "60",
                 "--api-listen", "127.0.0.1:0", *map(str, extra)],
                stdin=subprocess.DEVNULL, stdout=self.stdout, stderr=self.stderr, env=environment(),
                creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if event else 0)
            deadline = time.monotonic() + 8
            self.url = None
            while time.monotonic() < deadline:
                require(self.process.poll() is None, read_text(self.errpath))
                for line in read_text(self.errpath).splitlines():
                    if "Local API listening at " in line:
                        self.url = line.split("Local API listening at ", 1)[1].split()[0]
                if self.url and output.joinpath("profile.toml").exists():
                    break
                time.sleep(.005)
            require(self.url and output.joinpath("profile.toml").exists(), "Capture did not initialize")
        except BaseException:
            self.close()
            raise

    def ready(self, predicate):
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            require(self.process.poll() is None, read_text(self.errpath))
            with opener.open(self.url + "/v1/status", timeout=2) as response:
                data = response.read(LIMIT + 1)
                require(len(data) <= LIMIT, "Oversized API test response")
                latest = json.loads(data)["body"]["latest_event"]
            if latest and latest["data"]["kind"] == "input" and predicate(latest["data"]["capture_counts"]):
                return
            time.sleep(.005)
        raise AssertionError("Capture did not reach the required input population")

    def finish(self, expected=130):
        code = self.process.wait(timeout=12)
        require(code == expected, f"Exit {code}, expected {expected}: {read_text(self.errpath)}")
        report = tomllib.loads(read_text(self.outpath))
        require(report == tomllib.loads(read_text(self.output / "capture.toml")), "stdout/saved reports differ")
        require(report["stopped_by_request"], "Interruption was not recorded")
        require(report["elapsed_us"] < 60_000_000, "Capture ended by its duration instead of interruption")
        require(self.profile.read_bytes() == self.original, "Source profile changed")
        require(not report.get("terminal_error"), "Unexpected terminal acquisition error")
        for interface in ("local_api", "mqtt", "shared_memory"):
            require(not report.get(interface, {}).get("worker_failed"), "Integration worker failed")
        counts = report["counts"]
        require(counts["selected"] == report["csv_rows_flushed"] + report["csv_rows_unconfirmed"] + counts["logger_overruns"],
                "CSV accounting does not balance")
        rows = []
        for index in range(1, report["csv_parts"] + 1):
            with (self.output / f"capture-{index:04}.csv").open(encoding="utf-8", newline="") as stream:
                rows.extend(csv.DictReader(stream))
        require(len(rows) == report["csv_rows_flushed"], "Flushed CSV count differs")
        if expected == 130:
            require(not report.get("writer_error") and report["csv_rows_unconfirmed"] == 0, "Clean interruption lost rows")
            checked = subprocess.run([str(self.cli), "verify-capture", str(self.output)], capture_output=True,
                                     timeout=12, env=environment(), creationflags=subprocess.CREATE_NO_WINDOW)
            require(checked.returncode == 0, checked.stderr.decode("utf-8", errors="replace"))
            require(tomllib.loads(checked.stdout.decode("utf-8"))["rows_checked"] == len(rows), "Saved verification differs")
        require(all(int(a["sample_index"]) < int(b["sample_index"])
                    and int(a["host_elapsed_us"]) <= int(b["host_elapsed_us"]) for a, b in zip(rows, rows[1:])),
                "CSV sample/receive order differs")
        return {"exit_code": code, "rows_flushed": len(rows), "parts": report["csv_parts"],
                "logger_overruns": counts["logger_overruns"], "unconfirmed_rows": report["csv_rows_unconfirmed"],
                "report_sha256": digest(self.output / "capture.toml"), "source_profile_sha256": digest(self.profile),
                "capture_verified": expected == 130}, report

    def close(self):
        if self.process is not None and self.process.poll() is None:
            self.process.kill()  # Failure cleanup only, never a successful stop.
            self.process.wait(timeout=5)
        self.stdout.close()
        self.stderr.close()

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.close()


class HeldGateway(BaseHTTPRequestHandler):
    """Original synthetic, allowlisted read-only replies; first poll held through timeout."""
    entered = threading.Event()
    release = threading.Event()
    requests = []
    violations = []

    def log_message(self, _format, *_args):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        port = "/iolinkmaster/port[1]"
        device = port + "/iolinkdevice"
        values = {"/deviceinfo/productcode": "AL1350", "/deviceinfo/swrevision": "3.1.42",
                  "/deviceinfo/serialnumber": "SYNTHETIC-MASTER", port + "/mode": 3,
                  port + "/mastercycletime_actual": 2000, port + "/portevent": 0,
                  device + "/vendorid": 65535, device + "/deviceid": 1,
                  device + "/productname": "Synthetic sensor", device + "/serial": "SYNTHETIC-SENSOR",
                  device + "/status": 0, device + "/iolinkevent": 0, device + "/pdin": "01F4"}
        paths = request.get("data", {}).get("datatosend", [])
        if (self.path != "/" or request.get("code") != "request" or request.get("adr") != "/getdatamulti"
                or "auth" in request or not paths or any(path not in values for path in paths)):
            self.violations.append(request)
            self.send_error(400)
            return
        self.requests.append(paths)
        if device + "/pdin" in paths:
            self.entered.set()
            self.release.wait(timeout=8)
        payload = json.dumps({"code": 200, "cid": request["cid"],
                              "data": {path: {"code": 200, "data": values[path]} for path in paths}}).encode()
        try:
            self.send_response(200)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass  # Expected when the bounded client request times out.


def worker(cli, output, token):
    require(token and os.environ.pop(TOKEN_ENV, None) == token, "Launch through the isolated-console verifier")
    require(sys.stdin.buffer.readline() == b"job-ready\n", "Verifier job containment was not established")
    win = Windows()
    win.console({os.getpid()})
    # Re-enable CTRL+C inheritance before children. A custom worker callback is
    # not inherited, unlike SetConsoleCtrlHandler(NULL, TRUE)'s ignore attribute.
    win.checked(win.kernel.SetConsoleCtrlHandler(None, False))
    handler = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.DWORD)(lambda _event: True)
    win.checked(win.kernel.SetConsoleCtrlHandler(handler, True))
    base = ("version = 1\nname = 'Console interruption fixture'\ninput_bytes = 2\n"
            "[connection]\ngateway = 'simulator'\nport = 1\npoll_us = 1000\n"
            "[logging]\nmode = 'every_update'\nqueue_capacity = 128\nrotate_rows = 7\n")
    cases = {}

    profile = output / "rotation.toml"
    profile.write_bytes(base.encode())
    with Run(cli, profile, output / "ctrl-c-rotation", 0) as run:
        run.ready(lambda counts: counts["received"] >= 30)
        win.signal(run, 0)
        cases["ctrl_c_rotation"], report = run.finish()
        require(report["csv_parts"] > 1 and not report["counts"]["logger_overruns"], "Rotation fixture did not cover clean flush")

    profile = output / "pressure.toml"
    profile.write_bytes(base.replace("queue_capacity = 128", "queue_capacity = 16").encode())
    with Run(cli, profile, output / "repeated-break-pressure", 1, ["--simulate-writer-delay-ms", 25]) as run:
        run.ready(lambda counts: counts["received"] >= 80 and counts["logger_overruns"] > 0)
        win.signal(run, 1)
        time.sleep(.025)
        win.signal(run, 1)
        cases["repeated_break_backpressure"], report = run.finish()
        require(report["counts"]["logger_overruns"] > 0 and report["logger_metrics"]["queue_high_water_observed"] == 16,
                "Backpressure fixture did not fill the queue")

    server = ThreadingHTTPServer(("127.0.0.1", 0), HeldGateway)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    HeldGateway.entered.clear()
    HeldGateway.release.clear()
    HeldGateway.requests, HeldGateway.violations = [], []
    try:
        profile = output / "held-request.toml"
        text = base.replace("gateway = 'simulator'", "gateway = 'ifm_al1350'\nifm_mode = 3\ntimeout_ms = 1500\n"
                            f"endpoint = 'http://127.0.0.1:{server.server_port}'")
        profile.write_bytes(text.encode())
        with Run(cli, profile, output / "break-held-request", 1) as run:
            require(HeldGateway.entered.wait(timeout=5), "Loopback request did not enter the hold")
            started = time.monotonic()
            win.signal(run, 1)
            time.sleep(.075)
            require(run.process.poll() is None, "Request completed before its timeout")
            cases["break_held_request"], report = run.finish()
            cases["break_held_request"]["observed_stop_seconds"] = time.monotonic() - started
            require(len(HeldGateway.requests) == 2 and not HeldGateway.violations, "Unexpected gateway request")
            require(report["counts"]["transport_errors"] == report["counts"]["disconnects"] == 1
                    and report["counts"]["received"] == 0 and report.get("last_error"), "Timeout evidence differs")
            require(report["source_update_loss"] == "unknown" and "source_gaps_observed" not in report,
                    "Snapshot fixture invented source freshness")
    finally:
        HeldGateway.release.set()
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)

    profile = output / "writer-failure.toml"
    profile.write_bytes(base.replace("rotate_rows = 7", "rotate_rows = 5").encode())
    with Run(cli, profile, output / "break-writer-failure", 1, ["--simulate-writer-delay-ms", 100]) as run:
        obstacle = run.output / "capture-0002.csv"
        sentinel = b"Preserve this existing rotation destination.\n"
        with obstacle.open("xb") as stream:
            stream.write(sentinel)
        run.ready(lambda counts: counts["received"] >= 12)
        win.signal(run, 1)
        cases["break_writer_failure"], report = run.finish(expected=1)
        require(report.get("writer_error") and report["csv_rows_unconfirmed"] > 0 and report["csv_rows_flushed"] == 5,
                "Writer failure was hidden by interruption")
        require(obstacle.read_bytes() == sentinel, "Existing rotation destination was changed")

    win.console({os.getpid()})
    version = subprocess.check_output([str(cli), "--version"], env=environment(),
                                      creationflags=subprocess.CREATE_NO_WINDOW).decode().strip()
    receipt = {"format_version": 1, "cli_version": version, "cli_sha256": digest(cli), "cases": cases,
               "scope": "Windows console control events; synthetic/loopback input; no desktop input, screenshots or physical hardware",
               "console_isolation": "new hidden console; exact owned-PID checks before each event; kill-on-close job",
               "limitations": "No terminal keyboard routing, console-window close, OS shutdown, crash recovery, physical timing or visible GUI acceptance"}
    with (output / "receipt.json").open("xb") as stream:
        stream.write((json.dumps(receipt, indent=2) + "\n").encode())


def verify(cli, output):
    if os.name != "nt":
        raise ValueError("CLI console interruption checks require Windows")
    require(cli.is_file(), "CLI executable is missing")
    output.mkdir()  # Never reuse or overwrite a receipt/capture destination.
    win = Windows()
    job = win.job()
    process = None
    token = uuid.uuid4().hex
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = subprocess.SW_HIDE
    try:
        process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), str(cli), "--output", str(output),
                                    "--console-worker", token], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, env={**environment(), TOKEN_ENV: token}, startupinfo=startup,
                                   creationflags=subprocess.CREATE_NEW_CONSOLE)
        # The worker cannot create any test child until assignment succeeds.
        win.checked(win.kernel.AssignProcessToJobObject(job, wintypes.HANDLE(int(process._handle))))
        stdout, stderr = process.communicate(b"job-ready\n", timeout=55)
        require(process.returncode == 0, stderr.decode("utf-8", errors="replace")[:4000])
        require(not stdout and len(stderr) <= LIMIT, "Unexpected console worker output")
        return json.loads(read_text(output / "receipt.json"))
    finally:
        # Timeout/failure closes the job and kills only owned worker descendants.
        win.kernel.CloseHandle(job)
        if process is not None:
            try:
                # Assignment itself may have failed while the worker was gated.
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=5)
            finally:
                for stream in (process.stdin, process.stdout, process.stderr):
                    if stream and not stream.closed:
                        stream.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=lambda value: Path(value).resolve())
    parser.add_argument("--output", type=lambda value: Path(value).resolve())
    parser.add_argument("--console-worker", help=argparse.SUPPRESS)
    args = parser.parse_args()
    output = args.output or ROOT / ".local" / ("cli-shutdown-" + uuid.uuid4().hex[:8])
    if args.console_worker:
        worker(args.cli, output, args.console_worker)
    else:
        print(json.dumps(verify(args.cli, output), indent=2))


if __name__ == "__main__":
    main()
