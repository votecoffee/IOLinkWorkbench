"""Saved timing validation across offline entry points; original bundles stay unchanged."""
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
run, files, copy_capture, patch = (helper[key] for key in ("run", "files", "copy_capture", "patch"))
SECTIONS = ("host_poll_start_spacing", "host_receive_spacing", "host_new_update_spacing", "gateway_update_spacing")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def remove_section(path, section):
    data, count = re.subn(rb"(?ms)^\[" + re.escape(section.encode()) + rb"\]\n.*?(?=^\[|\Z)", b"", path.read_bytes())
    assert count == 1, section
    path.write_bytes(data)


def verify(cli, captures, output):
    output.mkdir()
    originals = {p: digest(p) for capture in captures for p in files(capture)}
    checked = []
    for source in captures:
        reviewed = tomllib.loads(run(cli, "review-interfaces", source))
        data = tomllib.loads((source / "capture.toml").read_bytes().decode())
        for key in SECTIONS:
            assert reviewed["acquisition"]["recorded_timing"].get(key) == data.get(key)
        checked.append(reviewed["capture"]["rows_checked"])
    baseline = output / "baseline"
    copy_capture(captures[0], baseline)
    original = tomllib.loads((baseline / "capture.toml").read_bytes().decode())
    assert all(key in original for key in SECTIONS), "first fixture must have all four timing domains"
    assert not tomllib.loads((baseline / "profile.toml").read_bytes().decode())["logging"].get("raw_only", False)
    rejected = 0
    for section in SECTIONS:
        for key, value in (("count", 0), ("p50_us", 2**63 - 1), ("mean_hz", -1.0), ("mean_hz", 0.0), ("mean_hz", 1e300)):
            case = output / f"bad-{rejected:02}"
            copy_capture(baseline, case)
            patch(case / "capture.toml", section, key, value)
            joined, exported = output / f"join-{rejected}.csv", output / f"export-{rejected}"
            for args in (("verify-capture", case, "--joined-csv", joined), ("review-interfaces", case), ("review-capture", case), ("compare-captures", baseline, case), ("compare-captures", case, baseline), ("redecode-capture", case, case / "profile.toml", exported)):
                assert not run(cli, *args, expected=1)
            assert not joined.exists() and not exported.exists()
            rejected += 1
    for section in SECTIONS[:2]:
        case = output / f"count-{section}"
        copy_capture(baseline, case)
        patch(case / "capture.toml", section, "count", original[section]["count"] + 1)
        assert not run(cli, "verify-capture", case, expected=1)
    missing_counts = output / "legacy-counts"
    copy_capture(baseline, missing_counts)
    for key in ("poll_attempts", "received", "new_updates_observed", "disconnects"):
        patch(missing_counts / "capture.toml", "counts", key, None)
    run(cli, "verify-capture", missing_counts)
    for section in SECTIONS:
        remove_section(missing_counts / "capture.toml", section)
    legacy = tomllib.loads(run(cli, "review-interfaces", missing_counts))
    assert all(key not in legacy["acquisition"]["recorded_timing"] for key in SECTIONS)
    # Reported source clocks are independent; a source interval can exceed the host window.
    independent = output / "independent-clock"
    copy_capture(baseline, independent)
    gap = original["elapsed_us"] + 1_000_000
    for key, value in (("count", 1), ("p50_us", gap), ("p95_us", gap), ("p99_us", gap), ("longest_us", gap), ("mean_hz", 1_000_000 / gap)):
        patch(independent / "capture.toml", "gateway_update_spacing", key, value)
    run(cli, "verify-capture", independent)
    for key, value in (("p50_us", gap), ("p95_us", gap), ("p99_us", gap), ("longest_us", gap), ("mean_hz", 1_000_000 / gap)):
        patch(independent / "capture.toml", "host_receive_spacing", key, value)
    assert not run(cli, "verify-capture", independent, expected=1)
    # Existing snapshot bundles remain host polls and cannot acquire source cadence by editing.
    snapshot_cases = 0
    for index, source in enumerate(captures):
        data = tomllib.loads((source / "capture.toml").read_bytes().decode())
        if data.get("last_status", {}).get("freshness") != "snapshots":
            continue
        case = output / f"snapshot-{index}"
        copy_capture(source, case)
        report = case / "capture.toml"
        assert not any(section in data for section in SECTIONS[2:])
        report.write_bytes(report.read_bytes() + b"\n[gateway_update_spacing]\ncount = 1\np50_us = 1000\np95_us = 1000\np99_us = 1000\nlongest_us = 1000\nmean_hz = 1000.0\n")
        assert not run(cli, "verify-capture", case, expected=1)
        snapshot_cases += 1
    assert snapshot_cases > 0, "supply an actual synthetic snapshot bundle"
    assert all(digest(p) == value for p, value in originals.items())
    receipt = {"scope": "offline recorded timing consistency, not physical timing or source authentication", "source_rows_verified": checked, "invalid_aggregate_cases_rejected_by_six_entry_points": rejected, "observation_counts_crosschecked": True, "missing_legacy_data_unavailable": True, "independent_source_clock_preserved": True, "snapshot_source_cadence_rejected": snapshot_cases, "failed_outputs_not_published": True, "originals_preserved": True}
    (output / "receipt.json").write_bytes((json.dumps(receipt, indent=2) + "\n").encode())
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("captures", type=Path, nargs="+")
    args = parser.parse_args()
    destination = ROOT / ".local" / ("verify-timing-" + uuid.uuid4().hex[:8])
    print(json.dumps(verify(args.cli.resolve(), [p.resolve() for p in args.captures], destination), indent=2))
