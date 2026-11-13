"""Verify hidden native initialization/offscreen GUI painting; no desktop input or capture."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import uuid

ROOT = Path(__file__).resolve().parents[1]
CAPABILITY = "IOLINK_WORKBENCH_OFFSCREEN_SELF_TEST_V1"
PROFILES = {"bundled-simulator", "formats", "records", "arrays", "strings", "time", "values", "defaults"}


def unique_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate receipt key: {key}")
        result[key] = value
    return result


def invoke(gui, *args, expected=0):
    environment = {key: value for key, value in os.environ.items() if key.upper() != "IOLINK_IFM_PASSWORD"}
    completed = subprocess.run([str(gui), *map(str, args)], cwd=ROOT, env=environment,
                               capture_output=True, timeout=45, creationflags=subprocess.CREATE_NO_WINDOW)
    if completed.returncode != expected:
        raise RuntimeError(f"GUI check exited {completed.returncode}, expected {expected}: "
                           + completed.stderr.decode("utf-8", errors="replace")[:2000])
    return completed.stdout.decode("utf-8")


def verify(gui, output):
    if os.name != "nt":
        raise ValueError("Native GUI checks require Windows")
    binary = gui.read_bytes()
    # Older GUI binaries ignore arguments and open an ordinary window. Never launch
    # one unless its own offscreen-test capability marker is present.
    if CAPABILITY.encode("ascii") not in binary:
        raise ValueError("GUI binary lacks the offscreen-test capability; refusing to launch it")
    if output.exists():
        raise FileExistsError("Choose a new GUI receipt path")
    output.parent.mkdir(parents=True, exist_ok=True)
    version = invoke(gui, "--version").strip().split()[-1]
    invoke(gui, "--self-test", output)
    data = output.read_bytes()
    if data.startswith(b"\xef\xbb\xbf") or b"\r" in data or not data.endswith(b"\n"):
        raise ValueError("Receipt must be UTF-8 without BOM, LF, and a final newline")
    receipt = json.loads(data.decode("utf-8"), object_pairs_hook=unique_keys,
                         parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))
    assert receipt["capability"] == CAPABILITY and receipt["format_version"] == 1
    assert receipt["workbench_version"] == version and receipt["os"] == "windows" and receipt["arch"] == "x86_64"
    assert receipt["status"] == "pass" and receipt["error"] is None and receipt["background_attempts"] == 0
    native = receipt["native"]
    assert native["opengl_vendor"] and native["opengl_renderer"] and native["opengl_version"]
    assert not native["ordinary_frame_loop_entered"] and not native["pixel_readback_performed"]
    assert (native["framebuffer_width"], native["framebuffer_height"]) == (1130, 3200)
    cases = native["cases"]
    assert len(cases) == 24 and {(case["profile"], case["tab"]) for case in cases} == {(profile, tab) for profile in PROFILES for tab in ("setup", "monitor", "offline")}
    for case in cases:
        assert case["passes"] == 2 and case["shapes"] > 0 and case["meshes"] > 0 and case["triangles"] > 0
        assert (case["width"], case["height"]) == (1130, 3200)
    assert native["hidden_window_checks"] == 2 + 2 * sum(case["passes"] for case in cases)
    # Publication refuses a collision without launching the native initializer.
    invoke(gui, "--self-test", output, expected=1)
    assert output.read_bytes() == data
    invoke(gui, "--self-test", expected=2)
    return {"version": version, "gui_sha256": hashlib.sha256(binary).hexdigest(),
            "receipt": str(output), "receipt_sha256": hashlib.sha256(data).hexdigest(),
            "views": len(cases), "paint_passes": sum(case["passes"] for case in cases),
            "hidden_window_checks": native["hidden_window_checks"], "opengl_renderer": native["opengl_renderer"],
            "scope": receipt["scope"], "limitations": receipt["limitations"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gui", type=lambda value: Path(value).resolve())
    parser.add_argument("--output", type=lambda value: Path(value).resolve())
    args = parser.parse_args()
    output = args.output or ROOT / ".local" / ("gui-check-" + uuid.uuid4().hex[:8]) / "receipt.json"
    print(json.dumps(verify(args.gui, output), indent=2))


if __name__ == "__main__":
    main()
