"""Offline recorded identity checks on private copies, never live endpoint verification."""
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


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify(cli, captures, output):
    output.mkdir()
    originals = {p: digest(p) for capture in captures for p in files(capture)}
    rows = []
    for capture in captures:
        data = tomllib.loads((capture / "capture.toml").read_bytes().decode())
        profile = tomllib.loads((capture / "profile.toml").read_bytes().decode())
        reviewed = tomllib.loads(run(cli, "review-interfaces", capture))
        observed = reviewed["identity"]
        assert observed.get("last_accepted") == data.get("last_status")
        assert observed.get("rejected") == data.get("rejected_status")
        assert observed.get("producer_check") == data.get("identity_check")
        assert observed["expected"] == profile.get("expected_identity", {})
        if not profile.get("logging", {}).get("raw_only", False):
            signal = tomllib.loads(run(cli, "review-capture", capture, "--buckets", 2))
            assert signal["identity"] == observed
        rows.append(reviewed["capture"]["rows_checked"])
    baseline = output / "baseline"
    copy_capture(captures[0], baseline)
    source = tomllib.loads((baseline / "capture.toml").read_bytes().decode())
    profile = tomllib.loads((baseline / "profile.toml").read_bytes().decode())
    assert "serial" not in profile.get("expected_identity", {}), "first fixture must have no serial pin"
    assert source["last_status"]["identity"].get("serial")
    for key, section, value in (("serial", "last_status.identity", "another synthetic sensor"), ("firmware", "last_status", "synthetic-other"), ("port", "last_status", 2)):
        changed = output / f"changed-{key}"
        copy_capture(baseline, changed)
        patch(changed / "capture.toml", section, key, value)
        compared = tomllib.loads(run(cli, "compare-captures", baseline, changed))
        assert compared["timing_context"] == "settings_differ"
        assert all("magnitude" not in m for m in compared["metrics"])
    missing = output / "missing-identity"
    copy_capture(baseline, missing)
    report = missing / "capture.toml"
    data, count = re.subn(rb"(?ms)^\[last_status\.identity\]\n.*?(?=^\[|\Z)", b"", report.read_bytes())
    assert count == 1
    report.write_bytes(data)
    reviewed = tomllib.loads(run(cli, "review-interfaces", missing))["identity"]
    assert reviewed["accepted_recheck"] == "recorded device identity unavailable"
    assert "identity" not in reviewed["last_accepted"]
    compared = tomllib.loads(run(cli, "compare-captures", baseline, missing))
    assert compared["timing_context"] == "settings_unavailable" and all("magnitude" not in m for m in compared["metrics"])
    contradiction = output / "contradiction"
    copy_capture(baseline, contradiction)
    patch(contradiction / "profile.toml", "expected_identity", "serial", "wrong configured serial")
    joined, exported = output / "must-not-publish.csv", output / "must-not-publish"
    for args in (("verify-capture", contradiction, "--joined-csv", joined), ("review-interfaces", contradiction), ("review-capture", contradiction), ("compare-captures", baseline, contradiction), ("redecode-capture", contradiction, contradiction / "profile.toml", exported)):
        assert not run(cli, *args, expected=1)
    assert not joined.exists() and not exported.exists()
    rejected = output / "rejected"
    copy_capture(baseline, rejected)
    assert "terminal_error" not in source and "rejected_status" not in source
    report = rejected / "capture.toml"
    report.write_bytes(b'terminal_error = "synthetic rejected reconnect"\n' + report.read_bytes() + b'\n[rejected_status]\nfreshness = "counter32"\nmodel = "Rejected synthetic master"\nfirmware = "synthetic-rejected"\nport = 1\n[rejected_status.identity]\nserial = "REJECTED-SENSOR"\n')
    reviewed = tomllib.loads(run(cli, "review-interfaces", rejected))
    assert reviewed["identity"]["last_accepted"]["identity"]["serial"] == source["last_status"]["identity"]["serial"]
    assert reviewed["identity"]["rejected"]["identity"]["serial"] == "REJECTED-SENSOR"
    assert reviewed["capture"]["capture_had_error"]
    offline = output / "unreachable-declaration"
    copy_capture(baseline, offline)
    patch(offline / "profile.toml", "connection", "gateway", "ifm_al1350")
    patch(offline / "profile.toml", "connection", "endpoint", "http://127.0.0.1:9")
    run(cli, "review-interfaces", offline)
    assert all(digest(p) == value for p, value in originals.items())
    receipt = {"scope": "offline saved identity observations only; no hardware, ISDU or live identity query", "source_rows_verified": rows, "expected_accepted_rejected_separate": True, "field_review_same_identity": True, "changed_observed_context_no_deltas": True, "missing_observations_not_fabricated": True, "accepted_pin_contradictions_rejected_without_outputs": True, "unreachable_saved_endpoint_not_followed": True, "originals_preserved": True}
    (output / "receipt.json").write_bytes((json.dumps(receipt, indent=2) + "\n").encode())
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("captures", type=Path, nargs="+")
    args = parser.parse_args()
    destination = ROOT / ".local" / ("verify-identity-" + uuid.uuid4().hex[:8])
    print(json.dumps(verify(args.cli.resolve(), [p.resolve() for p in args.captures], destination), indent=2))
