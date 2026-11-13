"""Offline two-capture comparison checks using copied synthetic/recorded bundles."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import runpy
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]
helper = runpy.run_path(str(ROOT / "tools/verify_interface_review.py"))
run = helper["run"]
files = helper["files"]
copy_capture = helper["copy_capture"]
patch = helper["patch"]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def root_patch(path, key, value):
    data = path.read_bytes()
    first_table = data.index(b"\n[")
    head, tail = data[:first_table], data[first_table:]
    expression = rb"(?m)^" + re.escape(key.encode()) + rb" = [^\r\n]*(?:\n|$)"
    replacement = b"" if value is None else f"{key} = {json.dumps(value)}\n".encode()
    head, count = re.subn(expression, lambda _: replacement, head, count=1)
    assert count == 1, key
    path.write_bytes(head.rstrip(b"\n") + b"\n" + tail)


def verify(cli, captures, output):
    output.mkdir()
    originals = {p: digest(p) for c in captures for p in files(c)}
    pairs = []

    def compare(left, right):
        data = tomllib.loads(run(cli, "compare-captures", left, right))
        assert data["format_version"] == 1
        for side, path in (("left", left), ("right", right)):
            assert data[side] == tomllib.loads(run(cli, "review-interfaces", path))
        for metric in data["metrics"]:
            if metric["assessment"] == "matching_recorded_context":
                assert metric["magnitude"] == abs(metric["right"] - metric["left"])
                expected = "equal" if metric["right"] == metric["left"] else "increase" if metric["right"] > metric["left"] else "decrease"
                assert metric["change"] == expected
            else:
                assert "magnitude" not in metric and metric["change"] == "not_compared"
        return data

    for index, source in enumerate(captures):
        target = output / f"capture-{index}"
        copy_capture(source, target)
        same = compare(target, target)
        assert same["left"]["capture_report_sha256"] == same["right"]["capture_report_sha256"]
        pairs.append({"rows": same["left"]["capture"]["rows_checked"], "timing_context": same["timing_context"]})
    source = output / "capture-0"
    other = output / "edited"
    copy_capture(source, other)
    baseline = compare(source, other)
    assert baseline["timing_context"] == "matching_recorded_context"
    # Comments change provenance hashes but not semantic settings or metrics.
    report = other / "capture.toml"
    report.write_bytes(b"# offline comparison fixture\n" + report.read_bytes())
    commented = compare(source, other)
    assert commented["settings"] == baseline["settings"] and commented["metrics"] == baseline["metrics"]
    assert commented["left"]["capture_report_sha256"] != commented["right"]["capture_report_sha256"]
    data = tomllib.loads(report.read_text(encoding="utf-8"))
    for key in ("p50_us", "p95_us", "p99_us", "longest_us"):
        patch(report, "host_receive_spacing", key, data["host_receive_spacing"][key] + 1000)
    shifted = compare(source, other)
    spacing = [m for m in shifted["metrics"] if m["domain"] == "host_receive" and m["unit"] == "us"]
    assert len(spacing) == 4 and all(m["change"] == "increase" and m["magnitude"] == 1000 for m in spacing)
    root_patch(report, "elapsed_us", data["elapsed_us"] + 1)
    longer = compare(source, other)
    assert longer["timing_context"] == "matching_recorded_context" and longer["count_context"] == "settings_differ"
    assert all("magnitude" not in m for m in longer["metrics"] if m["unit"] == "count")
    root_patch(report, "requested_poll_us", None)
    missing = compare(source, other)
    assert missing["timing_context"] == "settings_unavailable"
    assert all("magnitude" not in m for m in missing["metrics"])
    # The declared endpoint is intentionally unreachable; only local files are read.
    patch(other / "profile.toml", "connection", "gateway", "ifm_al1350")
    patch(other / "profile.toml", "connection", "endpoint", "http://127.0.0.1:9")
    offline = compare(source, other)
    assert offline["timing_context"] == "settings_differ"
    assert all("magnitude" not in m for m in offline["metrics"])
    # Invalid recorded timing must not produce a successful comparison.
    patch(report, "host_receive_spacing", "count", 0)
    run(cli, "compare-captures", source, other, expected=1)
    copy_capture(source, output / "corrupt")
    (output / "corrupt/capture-0001.csv").write_bytes(b"corrupt\n")
    run(cli, "compare-captures", source, output / "corrupt", expected=1)
    assert not run(cli, "compare-captures", source, output / "corrupt", expected=1)
    if len(captures) > 1:
        cross = compare(output / "capture-0", output / "capture-1")
        assert cross["left"]["capture"]["rows_checked"] > 0
    assert all(digest(p) == value for p, value in originals.items())
    receipt = {"scope": "offline saved-record comparison only; no physical or visible GUI qualification", "captures": pairs, "exact_reviews_and_hashes": True, "comments_semantics_preserved": True, "known_timing_shift_exact_delta": True, "missing_settings_no_delta": True, "different_duration_no_count_delta": True, "unreachable_declared_endpoint_offline": True, "corruption_no_successful_stdout": True, "originals_preserved": True}
    (output / "receipt.json").write_bytes((json.dumps(receipt, indent=2) + "\n").encode())
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("captures", type=Path, nargs="+")
    args = parser.parse_args()
    destination = ROOT / ".local" / ("verify-comparison-" + uuid.uuid4().hex[:8])
    print(json.dumps(verify(args.cli.resolve(), [p.resolve() for p in args.captures], destination), indent=2))
