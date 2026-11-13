"""Read-only Windows snapshot reference. A row is a reader observation, not a source update."""
import argparse
from contextlib import contextmanager
import ctypes as ct
from ctypes import wintypes as wt
import json
import os
import re
import struct
import time

MAP_BYTES = 1_048_576
HEADER = struct.Struct("<8sIIIIQIIQQQ")
PREFIX = "Local\\IOLinkWorkbench.Snapshot."


def kernel():
    if os.name != "nt":
        raise RuntimeError("Windows is required")
    api = ct.WinDLL("kernel32", use_last_error=True)
    signatures = {
        "OpenMutexW": ([wt.DWORD, wt.BOOL, wt.LPCWSTR], wt.HANDLE),
        "OpenFileMappingW": ([wt.DWORD, wt.BOOL, wt.LPCWSTR], wt.HANDLE),
        "MapViewOfFile": ([wt.HANDLE, wt.DWORD, wt.DWORD, wt.DWORD, ct.c_size_t], ct.c_void_p),
        "UnmapViewOfFile": ([ct.c_void_p], wt.BOOL),
        "CloseHandle": ([wt.HANDLE], wt.BOOL),
        "WaitForSingleObject": ([wt.HANDLE, wt.DWORD], wt.DWORD),
        "ReleaseMutex": ([wt.HANDLE], wt.BOOL),
        "QueryPerformanceCounter": ([ct.POINTER(ct.c_longlong)], wt.BOOL),
        "QueryPerformanceFrequency": ([ct.POINTER(ct.c_longlong)], wt.BOOL),
        "OpenProcess": ([wt.DWORD, wt.BOOL, wt.DWORD], wt.HANDLE),
        "GetProcessTimes": ([wt.HANDLE, *([ct.POINTER(wt.FILETIME)] * 4)], wt.BOOL),
    }
    for name, (args, result) in signatures.items():
        function = getattr(api, name)
        function.argtypes, function.restype = args, result
    return api


class Reader:
    def __init__(self, name):
        if not re.fullmatch(r"[A-Za-z0-9_.-]{1,64}", name):
            raise ValueError("invalid shared-memory suffix")
        self.api = kernel()
        self.mapping_name = PREFIX + name
        self.mutex = self.mapping = self.address = None
        try:
            self.mutex = self.api.OpenMutexW(0x100001, False, self.mapping_name + ".Mutex")
            if not self.mutex:
                raise ct.WinError(ct.get_last_error())
            self.mapping = self.api.OpenFileMappingW(4, False, self.mapping_name)
            if not self.mapping:
                raise ct.WinError(ct.get_last_error())
            self.address = self.api.MapViewOfFile(self.mapping, 4, 0, 0, MAP_BYTES)
            if not self.address:
                raise ct.WinError(ct.get_last_error())
        except Exception:
            self.close()
            raise

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def close(self):
        if self.address:
            self.api.UnmapViewOfFile(self.address)
            self.address = None
        for name in ("mapping", "mutex"):
            value = getattr(self, name)
            if value:
                self.api.CloseHandle(value)
                setattr(self, name, None)

    @contextmanager
    def lock(self, wait_ms=50):
        if not 0 <= wait_ms <= 1000:
            raise ValueError("wait must be 0–1000 ms")
        result = self.api.WaitForSingleObject(self.mutex, wait_ms)
        if result == 0x102:
            raise TimeoutError("snapshot mutex is busy")
        if result not in (0, 0x80):
            raise ct.WinError(ct.get_last_error())
        try:
            if result == 0x80:
                raise RuntimeError("abandoned mutex; reject this observation")
            yield
        finally:
            if not self.api.ReleaseMutex(self.mutex):
                raise ct.WinError(ct.get_last_error())

    def read(self, wait_ms=50, max_age_ms=1000):
        if not 1 <= max_age_ms <= 60000:
            raise ValueError("age limit must be 1–60000 ms")
        with self.lock(wait_ms):
            fields = HEADER.unpack(ct.string_at(self.address, HEADER.size))
            magic, version, header_size, map_size, length, sequence, pid, flags, ticks, frequency, created = fields
            if magic != b"IOLWSHM\0" or version != 1 or header_size != 64 or map_size != MAP_BYTES:
                raise ValueError("uninitialized or unsupported shared-memory header")
            if not 1 <= length <= MAP_BYTES - 64 or sequence < 1 or pid < 1 or flags & ~3 or frequency < 1 or created < 1:
                raise ValueError("invalid shared-memory bounds/metadata")
            payload_bytes = ct.string_at(self.address + 64, length)
        payload = json.loads(payload_bytes.decode("utf-8"))
        if payload.get("format_version") != 1 or payload.get("snapshot_sequence") != sequence or payload.get("sampling_active") != bool(flags & 1) or payload.get("gateway_connected_last_observed") != bool(flags & 2):
            raise ValueError("JSON/header mismatch")
        now, actual_frequency = ct.c_longlong(), ct.c_longlong()
        if not self.api.QueryPerformanceCounter(ct.byref(now)) or not self.api.QueryPerformanceFrequency(ct.byref(actual_frequency)):
            raise ct.WinError(ct.get_last_error())
        if actual_frequency.value != frequency or now.value < ticks:
            raise ValueError("inconsistent QPC clock")
        age = (now.value - ticks) * 1_000_000 // frequency
        process = self.api.OpenProcess(0x101000, False, pid)
        alive = None
        if process:
            try:
                creation, exit_time, system, user = (wt.FILETIME() for _ in range(4))
                if not self.api.GetProcessTimes(process, ct.byref(creation), ct.byref(exit_time), ct.byref(system), ct.byref(user)):
                    raise ct.WinError(ct.get_last_error())
                actual_creation = creation.dwLowDateTime | (creation.dwHighDateTime << 32)
                alive = self.api.WaitForSingleObject(process, 0) == 0x102 and actual_creation == created
            finally:
                self.api.CloseHandle(process)
        header = dict(zip(("abi_version", "payload_bytes", "publish_sequence", "publisher_pid", "flags", "published_qpc_ticks", "qpc_frequency", "process_creation_time"),
                          (version, length, sequence, pid, flags, ticks, frequency, created)))
        return {"format_version": 1, "mapping_name": self.mapping_name, "header": header, "publish_age_us": age,
                "publish_age_exceeds_limit": age > max_age_ms * 1000, "publisher_process_alive": alive, "payload": payload}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("name")
    parser.add_argument("--count", type=int, default=1)
    parser.add_argument("--interval-ms", type=int, default=10)
    parser.add_argument("--max-age-ms", type=int, default=1000)
    args = parser.parse_args()
    if not 1 <= args.count <= 1000 or not 1 <= args.interval_ms <= 60000:
        parser.error("count must be 1–1000 and interval 1–60000 ms")
    with Reader(args.name) as reader:
        for index in range(args.count):
            if index:
                time.sleep(args.interval_ms / 1000)
            print(json.dumps(reader.read(max_age_ms=args.max_age_ms), ensure_ascii=False))
