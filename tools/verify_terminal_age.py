"""Offline terminal receive-age consistency using complete, filtered, dropped and snapshot captures."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]
helpers = runpy.run_path(str(ROOT / "tools/verify_interface_review.py"))
files, copy_capture = helpers["files"], helpers["copy_capture"]
root_patch = runpy.run_path(str(ROOT / "tools/verify_comparison.py"))["root_patch"]
AGE = "time_since_last_receive_us"


def run(cli, *args, expected=0, error_contains=None):
    result = subprocess.run([str(cli), *map(str, args)], capture_output=True, text=True, encoding="utf-8",
                            timeout=20, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
                            env={k: v for k, v in os.environ.items() if k.upper() != "IOLINK_IFM_PASSWORD"})
    assert result.returncode == expected, (args, result.stdout, result.stderr)
    if error_contains is not None:
        assert error_contains in result.stderr, result.stderr
    return result.stdout


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def report(path):
    return tomllib.loads((path / "capture.toml").read_text(encoding="utf-8"))


def last_receive_row(path):
    last = None
    for part in sorted(path.glob("capture-*.csv"), key=lambda p: int(p.stem.removeprefix("capture-"))):
        with part.open(encoding="utf-8", newline="") as stream:
            rows = csv.DictReader(stream, strict=True)
            for row in rows:
                assert None not in row and all(value is not None for value in row.values())
                last = int(row["host_elapsed_us"])
    assert last is not None
    return last


def metric(data):
    return next(m for m in data["metrics"] if m["domain"] == "acquisition" and m["name"] == "terminal_receive_age")


def verify(cli, captures, output):
    assert len(captures) == 4, "provide complete, filtered, dropped-row and snapshot captures"
    output.mkdir()
    originals = {p: digest(p) for source in captures for p in files(source)}
    checked = [tomllib.loads(run(cli, "verify-capture", source))["rows_checked"] for source in captures]
    data = [report(source) for source in captures]
    assert data[0]["counts"]["received"] == data[0]["counts"]["selected"] and data[0]["counts"]["logger_overruns"] == 0
    assert data[1]["counts"]["selected"] < data[1]["counts"]["received"]
    assert data[2]["counts"]["logger_overruns"] > 0
    assert data[3]["last_status"]["freshness"] == "snapshots"
    assert all(AGE in item for item in data), "all fixtures need a recorded terminal receive age"
    baseline = output / "complete"
    copy_capture(captures[0], baseline)
    original = data[0]
    last = last_receive_row(baseline)
    assert last > 0
    run(cli, "review-capture", baseline, "--buckets", 2)
    rejected = 0
    for name in ("beyond-window", "before-csv-row", "underreported-complete-tail"):
        case = output / name
        copy_capture(baseline, case)
        metadata = case / "capture.toml"
        if name == "beyond-window":
            root_patch(metadata, AGE, original["elapsed_us"] + 1)
        elif name == "before-csv-row":
            root_patch(metadata, AGE, original["elapsed_us"] - last + 1)
        else:
            root_patch(metadata, "elapsed_us", original["elapsed_us"] + 1000)
        joined, exported = case / "must-not-publish.csv", output / (name + "-must-not-export")
        for args in (("verify-capture", case), ("verify-capture", case, "--joined-csv", joined),
                     ("review-interfaces", case), ("review-capture", case, "--buckets", 2),
                     ("compare-captures", case, case),
                     ("redecode-capture", case, case / "profile.toml", exported)):
            assert not run(cli, *args, expected=1, error_contains=AGE)
        assert not joined.exists() and not exported.exists()
        rejected += 1

    rounding = output / "rounding"
    copy_capture(baseline, rounding)
    end = original["elapsed_us"] + 2
    root_patch(rounding / "capture.toml", "elapsed_us", end)
    for age in (end - last, end - last - 1):
        root_patch(rounding / "capture.toml", AGE, age)
        reviewed = tomllib.loads(run(cli, "review-interfaces", rounding))
        assert reviewed["acquisition"]["recorded_timing"][AGE] == age
        signal = tomllib.loads(run(cli, "review-capture", rounding, "--buckets", 2))
        assert signal["signal"]["recorded_timing"][AGE] == age

    legacy = output / "missing-age"
    copy_capture(baseline, legacy)
    root_patch(legacy / "capture.toml", AGE, None)
    reviewed = tomllib.loads(run(cli, "review-interfaces", legacy))
    assert AGE not in reviewed["acquisition"]["recorded_timing"]
    assert "magnitude" not in metric(tomllib.loads(run(cli, "compare-captures", legacy, legacy)))

    for index in (1, 2):
        partial = output / f"partial-{index}"
        copy_capture(captures[index], partial)
        root_patch(partial / "capture.toml", "elapsed_us", data[index]["elapsed_us"] + 1_000_000)
        reviewed = tomllib.loads(run(cli, "review-interfaces", partial))
        assert reviewed["acquisition"]["recorded_timing"][AGE] == data[index][AGE]
    snapshot = tomllib.loads(run(cli, "review-interfaces", captures[3]))
    assert snapshot["acquisition"]["recorded_timing"][AGE] == data[3][AGE]
    assert snapshot["capture"]["recorded_source_update_loss"] == "unknown"

    other = output / "comparison-right"
    copy_capture(output / "partial-1", other)
    root_patch(other / "capture.toml", AGE, 0)
    compared = tomllib.loads(run(cli, "compare-captures", output / "partial-1", other))
    assert metric(compared)["magnitude"] == data[1][AGE]
    root_patch(other / "capture.toml", "elapsed_us", data[1]["elapsed_us"] + 1_000_001)
    compared = tomllib.loads(run(cli, "compare-captures", output / "partial-1", other))
    assert "magnitude" not in metric(compared)

    exported = output / "valid-export"
    run(cli, "redecode-capture", baseline, baseline / "profile.toml", exported)
    run(cli, "verify-export", exported)
    run(cli, "verify-export", exported, "--source", baseline)
    rehashed = output / "rehashed-export"
    shutil.copytree(exported, rehashed)
    saved_report = rehashed / "source-capture.toml"
    root_patch(saved_report, "elapsed_us", original["elapsed_us"] + 1000)
    new_bytes = saved_report.read_bytes()
    receipt_path = rehashed / "receipt.toml"
    receipt_bytes = receipt_path.read_bytes()
    inventory = tomllib.loads(receipt_bytes.decode("utf-8"))
    for group, name in (("source_files", "capture.toml"), ("exported_files", "source-capture.toml")):
        item = next(entry for entry in inventory[group] if entry["name"] == name)
        old = f'name = "{name}"\nbytes = {item["bytes"]}\nsha256 = "{item["sha256"]}"'.encode()
        new = f'name = "{name}"\nbytes = {len(new_bytes)}\nsha256 = "{hashlib.sha256(new_bytes).hexdigest()}"'.encode()
        assert receipt_bytes.count(old) == 1
        receipt_bytes = receipt_bytes.replace(old, new)
    receipt_path.write_bytes(receipt_bytes)
    assert not run(cli, "verify-export", rehashed, expected=1, error_contains=AGE)
    assert all(digest(p) == before for p, before in originals.items())
    receipt = {"scope": "offline terminal host receive-age consistency, not source age/loss or physical timing",
               "source_rows_verified": checked, "invalid_terminal_cases_rejected_by_six_entry_points": rejected,
               "microsecond_rounding_preserved": True, "missing_age_unavailable": True,
               "filtered_and_dropped_csv_not_used_as_final_receive": True, "snapshot_source_loss_stays_unknown": True,
               "terminal_delta_requires_equal_duration": True, "rehashed_local_export_rejected": True,
               "originals_preserved": True, "failed_outputs_not_published": True}
    with (output / "receipt.json").open("xb") as stream:
        stream.write((json.dumps(receipt, indent=2) + "\n").encode())
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=lambda p: Path(p).resolve())
    parser.add_argument("captures", type=lambda p: Path(p).resolve(), nargs=4,
                        metavar=("COMPLETE", "FILTERED", "DROPPED", "SNAPSHOT"))
    args = parser.parse_args()
    output = ROOT / ".local" / ("verify-terminal-age-" + uuid.uuid4().hex[:8])
    print(json.dumps(verify(args.cli, args.captures, output), indent=2))
