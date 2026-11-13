"""Build and package the portable Windows x64 workbench from a clean checkout."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"
EXES = {"iolink-workbench.exe": 2, "iolink-cli.exe": 3}
DOCS = ["README.md", "ROADMAP.md", "LICENSE", "AGENTS.md",
        "docs/OPERATIONS.md", "docs/DEVELOPMENT.md", "docs/INTERFACES.md", "docs/LOCAL_API.md",
        ".agents/skills/iolink-workbench/SKILL.md", "samples/simulator.toml",
        "samples/ifm-al1350.toml", "samples/raw-only.toml", "samples/redecode.toml", "samples/sensor.xml", "samples/sensor-formats.xml", "samples/sensor-record.xml", "samples/sensor-array.xml",
        "samples/synthetic-standard-definitions.xml", "samples/synthetic-standard-definitions-de.xml", "samples/synthetic-value-standard-definitions.xml", "samples/sensor-standard-values.xml", "samples/sensor-defaults.xml", "samples/sensor-time.xml", "samples/sensor-strings.xml", "samples/sensor-values.xml", "samples/sensor-checksum.xml", "samples/sensor-checksum-de.xml", "samples/sensor-communication.xml", "samples/sensor-de.xml", "samples/synthetic-unit-definitions.xml"]
NOTICE = re.compile(r"(^|[-_.])(licen[cs]e|copying|copyright|notice|ofl|ufl|unlicense)([-_.]|$)", re.I)
UPSTREAM = {
    **{(name, "0.31.1"): "egui" for name in
       ("ecolor", "eframe", "egui", "egui-winit", "egui_glow", "emath", "epaint", "epaint_default_fonts")},
    ("clipboard-win", "5.4.3"): "clipboard-win",
    ("gl_generator", "0.14.0"): "gl-rs",
    ("khronos_api", "3.1.0"): "gl-rs",
    ("profiling", "1.0.18"): "profiling",
}


def output(*command):
    return subprocess.check_output(command, cwd=ROOT)


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def pe_imports(data, subsystem):
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    machine, sections = struct.unpack_from("<HH", data, pe + 4)
    opt = pe + 24
    opt_size = struct.unpack_from("<H", data, pe + 20)[0]
    if (data[:2] != b"MZ" or data[pe:pe + 4] != b"PE\0\0" or machine != 0x8664
            or struct.unpack_from("<H", data, opt)[0] != 0x20B
            or struct.unpack_from("<H", data, opt + 68)[0] != subsystem):
        raise ValueError("Expected a Windows x64 PE with the requested GUI or console subsystem")
    table = [struct.unpack_from("<III", data, opt + opt_size + 40 * i + 12) for i in range(sections)]

    def file_offset(rva):
        for address, size, start in table:
            if address <= rva < address + size:
                return start + rva - address
        raise ValueError("PE import RVA lies outside the file sections")

    rva, length = struct.unpack_from("<II", data, opt + 120)
    start = file_offset(rva)
    names = []
    for offset in range(start, start + length, 20):
        descriptor = struct.unpack_from("<IIIII", data, offset)
        if not any(descriptor):
            break
        name = file_offset(descriptor[3])
        dll = data[name:data.index(0, name, name + 256)].decode("ascii").lower()
        if dll.startswith(("vcruntime", "msvcp", "msvcr", "ucrtbase", "api-ms-win-crt-")):
            raise ValueError(f"Unexpected dynamic C runtime import: {dll}")
        names.append(dll)
    return sorted(set(names))


def notices(metadata):
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = metadata["resolve"]["root"]
    reachable = set()
    pending = [root]
    while pending:
        item = pending.pop()
        if item in reachable:
            continue
        reachable.add(item)
        pending.extend(dep["pkg"] for dep in nodes[item]["deps"]
                       if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]))
    reachable.remove(root)
    index = ["# Third-party notices", "",
             "Windows Cargo dependency graph, including build dependencies.", "",
             "| Package | Declared license | Notices |", "|---|---|---|"]
    groups = set()
    for pkg in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if pkg["id"] not in reachable:
            continue
        key = pkg["name"], pkg["version"]
        name = "-".join(key)
        base = Path(pkg["manifest_path"]).parent
        files = {p for p in base.rglob("*") if p.is_file() and NOTICE.search(p.name)}
        if pkg["license_file"]:
            files.add(base / pkg["license_file"])
        locations = []
        for file in sorted(files):
            yield f"third-party/{name}/{file.relative_to(base).as_posix()}", file.read_bytes()
        if files:
            locations.append(f"[Cargo archive]({name}/)")
        if group := UPSTREAM.get(key):
            groups.add(group)
            locations.append(f"[Pinned upstream files](upstream/{group}/)")
        if not locations:
            raise ValueError(f"No license text for {name}. Review the pinned upstream package.")
        index.append(f"| {name} | {pkg['license'] or 'See notices'} | {' and '.join(locations)} |")
    for group in sorted(groups):
        base = ROOT / "tools" / "notices" / group
        if not base.is_dir():
            raise ValueError(f"Missing pinned license directory {group}")
        for file in sorted(base.iterdir()):
            yield f"third-party/upstream/{group}/{file.name}", file.read_bytes()
    yield "third-party/upstream/SOURCES.md", (ROOT / "tools/notices/SOURCES.md").read_bytes()
    rust = Path(output("rustc", "--print", "sysroot").decode().strip()) / "share/doc/rust"
    yield "third-party/rust/COPYRIGHT-library.html", (rust / "COPYRIGHT-library.html").read_bytes()
    for file in sorted((rust / "licenses").glob("*.txt")):
        yield f"third-party/rust/licenses/{file.name}", file.read_bytes()
    index.extend(["", "[Rust standard library notices](rust/COPYRIGHT-library.html)"])
    yield "third-party/INDEX.md", ("\n".join(index) + "\n").encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output_dir", type=lambda p: Path(p).resolve())
    args = parser.parse_args()
    if os.name != "nt":
        raise ValueError("Build on Windows with the x64 MSVC Rust toolchain")
    if output("git", "status", "--porcelain").strip():
        raise ValueError("Package from a clean source checkout")
    commit = output("git", "rev-parse", "HEAD").decode().strip()
    subprocess.run(["cargo", "build", "--locked", "--release", "--target", TARGET], cwd=ROOT, check=True)
    metadata = json.loads(output("cargo", "metadata", "--locked", "--format-version", "1",
                                 "--filter-platform", TARGET))
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    basename = f"IOLinkWorkbench-{version}-windows-x64"
    args.output_dir.mkdir(parents=True, exist_ok=True)
    archive = args.output_dir / (basename + ".zip")
    checksum = args.output_dir / (archive.name + ".sha256")
    if archive.exists() or checksum.exists():
        raise FileExistsError(f"Use a new output directory; {archive.name} or its checksum exists")

    files = {path: (ROOT / path).read_bytes() for path in DOCS}
    manifest = {"commit": commit, "version": version, "target": TARGET,
                "toolchain": output("rustc", "--version").decode().strip(), "executables": {}}
    for exe, subsystem in EXES.items():
        data = (Path(metadata["target_directory"]) / TARGET / "release" / exe).read_bytes()
        manifest["executables"][exe] = {"sha256": sha256(data), "direct_dll_imports": pe_imports(data, subsystem)}
        files[exe] = data
    files.update(notices(metadata))
    files["build.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    with zipfile.ZipFile(archive, "x", zipfile.ZIP_DEFLATED) as bundle:
        for name, data in sorted(files.items()):
            bundle.writestr(f"{basename}/{name}", data)
    with zipfile.ZipFile(archive) as bundle:
        if bundle.testzip() is not None:
            raise ValueError("ZIP integrity check failed")
    with checksum.open("x", encoding="ascii") as receipt:
        receipt.write(f"{sha256(archive.read_bytes())}  {archive.name}\n")
    print(archive)


if __name__ == "__main__":
    main()
