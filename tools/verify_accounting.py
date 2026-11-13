"""Offline selection, reconnect and complete-CSV source-gap checks on copied evidence."""
import argparse
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


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def csv_gap_total(directory):
    import csv
    gaps, previous = 0, None
    for path in sorted(directory.glob("capture-*.csv")):
        with path.open(encoding="utf-8", newline="") as stream:
            for row in csv.DictReader(stream):
                if not row["source_counter"]:
                    continue
                generation, counter = int(row["generation"]), int(row["source_counter"])
                if previous is not None and generation == previous[0]:
                    gaps += ((counter - previous[1]) % 2**32) - 1
                previous = generation, counter
    return gaps


def verify(cli, captures, output):
    output.mkdir()
    originals = {p: digest(p) for capture in captures for p in files(capture)}
    checked, filtered, dropped, snapshots = [], 0, 0, 0
    for capture in captures:
        data = tomllib.loads((capture / "capture.toml").read_bytes().decode())
        profile = tomllib.loads((capture / "profile.toml").read_bytes().decode())
        reviewed = tomllib.loads(run(cli, "review-interfaces", capture))
        checked.append(reviewed["capture"]["rows_checked"])
        if profile["logging"]["mode"] == "on_change" and data["counts"].get("unchanged", 0) > 0:
            assert csv_gap_total(capture) > data["source_gaps_observed"]
            filtered += 1
        if data["counts"]["logger_overruns"] > 0 and data.get("source_gaps_observed") is not None:
            assert csv_gap_total(capture) > data["source_gaps_observed"]
            dropped += 1
        if data.get("last_status", {}).get("freshness") == "snapshots":
            assert "recorded_source_gaps" not in reviewed["capture"]
            snapshots += 1
    assert filtered and dropped and snapshots, "include on-change, dropped-row and snapshot fixtures"
    baseline = output / "baseline"
    copy_capture(captures[0], baseline)
    run(cli, "review-capture", baseline, "--buckets", 2)  # First fixture needs typed fields.
    original = tomllib.loads((baseline / "capture.toml").read_bytes().decode())
    counts = original["counts"]
    assert original["last_status"]["freshness"] == "counter32" and original["ended_connected"]
    assert counts["logger_overruns"] == counts["unchanged"] == 0
    assert counts["new_updates_observed"] == counts["selected"]
    assert csv_gap_total(baseline) == original["source_gaps_observed"]
    for number, name in enumerate(("end-state", "unchanged", "source-gaps", "eligible-inputs")):
        case = output / name
        copy_capture(baseline, case)
        report = case / "capture.toml"
        if name == "end-state":
            root_patch(report, "ended_connected", False)
        elif name == "unchanged":
            patch(report, "counts", "unchanged", 1)
        elif name == "source-gaps":
            root_patch(report, "source_gaps_observed", original["source_gaps_observed"] + 1)
        else:
            patch(report, "counts", "new_updates_observed", counts["new_updates_observed"] + 1)
        joined, exported = output / f"join-{number}.csv", output / f"export-{number}"
        for args in (("verify-capture", case, "--joined-csv", joined), ("review-interfaces", case), ("review-capture", case), ("compare-captures", baseline, case), ("compare-captures", case, baseline), ("redecode-capture", case, case / "profile.toml", exported)):
            assert not run(cli, *args, expected=1)
        assert not joined.exists() and not exported.exists()
    legacy = output / "missing-population"
    copy_capture(baseline, legacy)
    for key in ("received", "new_updates_observed", "poll_attempts", "unchanged", "disconnects"):
        patch(legacy / "capture.toml", "counts", key, None)
    root_patch(legacy / "capture.toml", "ended_connected", None)
    root_patch(legacy / "capture.toml", "source_gaps_observed", original["source_gaps_observed"] + 1)
    legacy_review = tomllib.loads(run(cli, "review-interfaces", legacy))
    assert legacy_review["capture"]["recorded_source_gaps"] == original["source_gaps_observed"] + 1
    assert "recorded_received" not in legacy_review["acquisition"]
    assert all(digest(p) == value for p, value in originals.items())
    receipt = {"scope": "offline recorded accounting consistency, not physical update-loss qualification", "source_rows_verified": checked, "four_corruptions_rejected_by_six_paths": True, "on_change_and_writer_loss_not_source_loss": True, "snapshot_source_loss_unknown": True, "missing_populations_not_assumed_complete": True, "failed_outputs_not_published": True, "originals_preserved": True}
    (output / "receipt.json").write_bytes((json.dumps(receipt, indent=2) + "\n").encode())
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("captures", type=Path, nargs="+")
    args = parser.parse_args()
    destination = ROOT / ".local" / ("verify-accounting-" + uuid.uuid4().hex[:8])
    print(json.dumps(verify(args.cli.resolve(), [p.resolve() for p in args.captures], destination), indent=2))
