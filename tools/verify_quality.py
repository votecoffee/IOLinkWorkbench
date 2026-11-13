"""Offline saved-quality checks with parser-based CSV edits on private copies only."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import runpy
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]
helper = runpy.run_path(str(ROOT / "tools/verify_interface_review.py"))
run, files, copy_capture, patch = (helper[key] for key in ("run", "files", "copy_capture", "patch"))
root_patch = runpy.run_path(str(ROOT / "tools/verify_comparison.py"))["root_patch"]
QUALITY = ("invalid", "validity_unknown", "stale", "freshness_unknown", "duplicate", "out_of_order")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def contradict_stale_age(directory, threshold):
    for path in sorted(directory.glob("capture-*.csv")):
        with path.open(encoding="utf-8", newline="") as stream:
            records = list(csv.reader(stream, strict=True))
        header = records[0]
        assert all(len(row) == len(header) for row in records)
        age, stale, validity = (header.index(key) for key in ("source_age_us", "stale", "validity"))
        values = header.index("raw_hex") + 1
        for row in records[1:]:
            if row[validity] != "valid" or row[stale] != "false" or not row[age] or int(row[age]) > threshold:
                continue
            before = [item.copy() for item in records]
            row[stale] = "true"
            row[values:] = [""] * (len(row) - values)
            # Quote every cell so embedded CR/LF remains cell data; records use LF.
            with path.open("w", encoding="utf-8", newline="") as stream:
                csv.writer(stream, lineterminator="\n", quoting=csv.QUOTE_ALL).writerows(records)
            with path.open(encoding="utf-8", newline="") as stream:
                assert list(csv.reader(stream, strict=True)) == records
            assert all(old[:stale] == new[:stale] and old[stale + 1:values] == new[stale + 1:values] for old, new in zip(before, records))
            return path
    raise AssertionError("first capture needs a valid, explicitly aged not-stale row")


def verify(cli, captures, output):
    output.mkdir()
    originals = {p: digest(p) for capture in captures for p in files(capture)}
    checked = []
    for capture in captures:
        checked.append(tomllib.loads(run(cli, "verify-capture", capture))["rows_checked"])
    baseline = output / "baseline"
    copy_capture(captures[0], baseline)
    run(cli, "review-capture", baseline, "--buckets", 2)
    original = tomllib.loads((baseline / "capture.toml").read_bytes().decode())
    assert original["counts"]["invalid"] > 0
    assert tomllib.loads(run(cli, "verify-capture", baseline))["invalid_rows"] > 0
    edited_csv = None
    for number, name in enumerate(("invalid-total", "classification", "threshold", "stale-age")):
        case = output / name
        copy_capture(baseline, case)
        report = case / "capture.toml"
        if name == "invalid-total":
            patch(report, "counts", "invalid", 0)
        elif name == "classification":
            patch(report, "counts", "duplicate", original["counts"]["duplicate"] + 1)
        elif name == "threshold":
            root_patch(report, "stale_threshold_us", original["stale_threshold_us"] + 1)
        else:
            edited_csv = contradict_stale_age(case, original["stale_threshold_us"])
            patch(report, "counts", "stale", original["counts"]["stale"] + 1)
        joined, exported = output / f"join-{number}.csv", output / f"export-{number}"
        for args in (("verify-capture", case, "--joined-csv", joined), ("review-interfaces", case), ("review-capture", case), ("compare-captures", baseline, case), ("compare-captures", case, baseline), ("redecode-capture", case, case / "profile.toml", exported)):
            assert not run(cli, *args, expected=1)
        assert not joined.exists() and not exported.exists()
    legacy = output / "missing-quality-metrics"
    copy_capture(baseline, legacy)
    for key in QUALITY:
        patch(legacy / "capture.toml", "counts", key, None)
    assert tomllib.loads(run(cli, "verify-capture", legacy)) == tomllib.loads(run(cli, "verify-capture", baseline))
    assert all(digest(p) == value for p, value in originals.items())
    receipt = {"scope": "offline recorded-quality consistency only; no physical validity or timing qualification", "source_rows_verified": checked, "four_corruptions_rejected_by_six_paths": True, "missing_quality_metrics_not_zero": True, "csv_cells_encoding_and_lf_records_preserved": True, "edited_csv": str(edited_csv.resolve()), "failed_outputs_not_published": True, "originals_preserved": True}
    (output / "receipt.json").write_bytes((json.dumps(receipt, indent=2) + "\n").encode())
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("captures", type=Path, nargs="+")
    args = parser.parse_args()
    destination = ROOT / ".local" / ("verify-quality-" + uuid.uuid4().hex[:8])
    print(json.dumps(verify(args.cli.resolve(), [p.resolve() for p in args.captures], destination), indent=2))
