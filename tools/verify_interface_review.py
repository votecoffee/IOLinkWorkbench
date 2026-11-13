"""Offline saved-interface review acceptance. Source captures are copied and never modified."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]
COMPONENTS = ("local_api", "mqtt", "shared_memory")


def run(cli, *args, expected=0):
    result = subprocess.run([str(cli), *map(str, args)], capture_output=True, text=True, encoding="utf-8", timeout=20)
    assert result.returncode == expected, (args, result.stdout, result.stderr)
    return result.stdout


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def files(directory):
    return [directory / "capture.toml", directory / "profile.toml", *sorted(directory.glob("capture-*.csv"))]


def copy_capture(source, destination):
    destination.mkdir()
    for path in files(source):
        shutil.copyfile(path, destination / path.name)


def patch(path, section, key, value):
    data = path.read_bytes()
    marker = f"[{section}]\n".encode()
    assert data.count(marker) == 1, (path, section)
    first = data.index(marker) + len(marker)
    last = data.find(b"\n[", first)
    last = len(data) if last < 0 else last
    contents = data[first:last]
    expression = rb"(?m)^" + re.escape(key.encode()) + rb" = [^\r\n]*\n?"
    replacement = b"" if value is None else f"{key} = {json.dumps(value, ensure_ascii=False)}\n".encode("utf-8")
    contents, changed = re.subn(expression, lambda _: replacement, contents, count=1)
    if not changed:
        assert value is not None, (section, key)
        contents = replacement + contents
    path.write_bytes(data[:first] + contents + data[last:])


def inspect(cli, capture):
    data = tomllib.loads((capture / "capture.toml").read_text(encoding="utf-8"))
    reviewed = tomllib.loads(run(cli, "review-interfaces", capture))
    assert reviewed["format_version"] == 1
    assert reviewed["capture"] == tomllib.loads(run(cli, "verify-capture", capture))
    assert reviewed["capture_report_sha256"] == digest(capture / "capture.toml")
    assert reviewed["saved_profile_sha256"] == digest(capture / "profile.toml")
    for component in COMPONENTS:
        if component in data:
            assert reviewed["interfaces"][component + "_status"] == "recorded"
            assert reviewed["interfaces"][component] == data[component]
        else:
            assert reviewed["interfaces"][component + "_status"] == "not_recorded"
            assert component not in reviewed["interfaces"]
    expected = all(key in data["counts"] for key in ("received", "disconnects")) and any(key in data for key in COMPONENTS)
    assert reviewed["interfaces"]["offer_counts_checked_against_acquisition"] == expected
    context = reviewed["acquisition"]
    assert context["recorded_logger_overruns"] == data["counts"]["logger_overruns"]
    for key in ("host_poll_start_spacing", "host_receive_spacing", "host_new_update_spacing", "gateway_update_spacing", "logger_metrics"):
        assert context["recorded_timing"].get(key) == data.get(key)
    assert reviewed["capture"]["recorded_source_update_loss"] == data["source_update_loss"]
    return data, reviewed


def verify(cli, captures, samples, output):
    output.mkdir()
    source_hashes = {path: digest(path) for capture in captures for path in files(capture)}
    checked = []
    all_components = None
    for index, source in enumerate(captures):
        target = output / f"capture-{index:02}"
        copy_capture(source, target)
        data, reviewed = inspect(cli, target)
        checked.append({"rows": reviewed["capture"]["rows_checked"], "components": [key for key in COMPONENTS if key in data]})
        if all(key in data for key in COMPONENTS):
            all_components = target
        # The field-review path retains the same exact recorded component summaries.
        profile = tomllib.loads((target / "profile.toml").read_text(encoding="utf-8"))
        if not profile.get("logging", {}).get("raw_only", False):
            signal = tomllib.loads(run(cli, "review-capture", target, "--buckets", 2))
            if any(key in data for key in COMPONENTS):
                assert signal["interfaces"] == reviewed["interfaces"]
            else:
                assert "interfaces" not in signal
    assert all_components is not None, "supply a capture containing all three interface sections"

    offline = output / "offline-destinations"
    copy_capture(all_components, offline)
    report = offline / "capture.toml"
    patch(report, "local_api", "url", "http://127.0.0.1:9")
    patch(report, "mqtt", "broker", "127.0.0.1:9")
    patch(report, "shared_memory", "name", "absent_offline_" + uuid.uuid4().hex)
    edited = output / "unreachable-profile.toml"
    run(cli, "set-connection", offline / "profile.toml", edited, "--gateway", "ifm-al1350", "--endpoint", "http://127.0.0.1:9")
    shutil.copyfile(edited, offline / "profile.toml")
    before = {path: digest(path) for path in files(offline)}
    data, reviewed = inspect(cli, offline)
    assert reviewed["acquisition"]["configured_gateway"] == "ifm_al1350"
    assert all(digest(path) == saved for path, saved in before.items())
    codes = {entry["code"]: entry for entry in reviewed["interfaces"]["findings"]}
    assert codes["mqtt_coalesced"]["category"] == "intentional_selection"
    assert codes["shm_busy_skips"]["category"] == "interface_loss"
    assert codes["shm_final_state_unconfirmed"]["category"] == "unconfirmed"
    assert all(code in codes for code in ("api_client_delivery_unknown", "mqtt_subscriber_delivery_unknown", "shm_reader_delivery_unknown"))

    legacy = output / "missing-acquisition-counters"
    copy_capture(all_components, legacy)
    patch(legacy / "capture.toml", "counts", "received", None)
    patch(legacy / "capture.toml", "counts", "disconnects", None)
    _, reviewed = inspect(cli, legacy)
    assert not reviewed["interfaces"]["offer_counts_checked_against_acquisition"]
    assert "recorded_received" not in reviewed["acquisition"] and "recorded_disconnects" not in reviewed["acquisition"]

    for component in COMPONENTS:
        wrong = output / ("balanced-wrong-" + component)
        copy_capture(all_components, wrong)
        data = tomllib.loads((wrong / "capture.toml").read_text(encoding="utf-8"))
        metrics = data[component]
        patch(wrong / "capture.toml", component, "events_offered", metrics["events_offered"] + 1)
        patch(wrong / "capture.toml", component, "queue_dropped", metrics["queue_dropped"] + 1)
        assert not run(cli, "review-interfaces", wrong, expected=1)
        assert not run(cli, "verify-capture", wrong, expected=1)

    corrupt = output / "corrupt-csv"
    copy_capture(all_components, corrupt)
    next(corrupt.glob("capture-*.csv")).write_bytes(b"corrupt\n")
    assert not run(cli, "review-interfaces", corrupt, expected=1)

    raw = output / "raw-only"
    run(cli, "capture", samples / "raw-only.toml", raw, "--seconds", .25)
    _, raw_review = inspect(cli, raw)
    assert all(raw_review["interfaces"][key + "_status"] == "not_recorded" for key in COMPONENTS)
    assert not run(cli, "review-capture", raw, expected=1)
    # Comments are part of the metadata receipt, not normalized away by TOML parsing.
    before_hash = raw_review["saved_profile_sha256"]
    path = raw / "profile.toml"
    path.write_bytes(b"# Offline receipt checks original bytes\n" + path.read_bytes())
    _, commented = inspect(cli, raw)
    assert commented["saved_profile_sha256"] != before_hash
    assert commented["capture"] == raw_review["capture"]
    assert all(digest(path) == saved for path, saved in source_hashes.items())
    receipt = {"scope": "offline saved-file consistency only; no physical/network/IPC acceptance", "version": run(cli, "--version").strip(), "captures": checked,
               "metadata_hashes_match": True, "all_original_capture_hashes_preserved": True, "unreachable_gateway_broker_and_absent_mapping_reviewed": True,
               "three_balanced_but_wrong_offer_totals_rejected": True, "missing_counters_remain_unavailable": True, "raw_only_review_without_typed_field": True,
               "coalescing_retention_and_unconfirmed_delivery_kept_separate": True}
    (output / "receipt.json").write_bytes((json.dumps(receipt, indent=2) + "\n").encode("utf-8"))
    print(json.dumps(receipt, indent=2))
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=lambda value: Path(value).resolve())
    parser.add_argument("captures", nargs="+", type=lambda value: Path(value).resolve())
    parser.add_argument("--samples", type=Path, default=ROOT / "samples")
    args = parser.parse_args()
    target = ROOT / ".local" / ("interface-review-" + uuid.uuid4().hex[:8])
    verify(args.cli, args.captures, args.samples.resolve(), target)
    print(f"Evidence directory: {target}")
