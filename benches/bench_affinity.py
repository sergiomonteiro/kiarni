"""Optional Linux affinity for benchmark processes only; no host settings changed."""
import os
from pathlib import Path
import sys


def validate(server, clients, allowed=None):
    if server is None and clients is None:
        return
    if not server or not clients:
        raise ValueError("both_cpu_sets_required")
    if allowed is None:
        if not hasattr(os, "sched_setaffinity"):
            raise ValueError("affinity_not_supported")
        allowed = os.sched_getaffinity(0)
    for values in (server, clients):
        if (len(values) != len(set(values)) or any(type(v) is not int or v < 0 for v in values)
            or not set(values) <= set(allowed)):
            raise ValueError("invalid_or_unavailable_cpu")
    if set(server) & set(clients):
        raise ValueError("overlapping_cpu_sets")


def apply(cpus):
    if cpus is not None:
        os.sched_setaffinity(0, cpus)
        if set(os.sched_getaffinity(0)) != set(cpus):
            raise RuntimeError("affinity_not_applied")


def observed(pid=0):
    return sorted(os.sched_getaffinity(pid)) if hasattr(os, "sched_getaffinity") else None


def snapshot():
    result = {"loadavg": list(os.getloadavg()) if hasattr(os, "getloadavg") else None,
              "cpus": {}}
    for cpu in observed() or []:
        base = Path(f"/sys/devices/system/cpu/cpu{cpu}")
        values = {}
        for name in ("topology/core_id", "topology/physical_package_id", "topology/thread_siblings_list",
                     "cpufreq/scaling_governor", "cpufreq/scaling_cur_freq"):
            try:
                values[name] = (base / name).read_text().strip()
            except OSError:
                pass
        result["cpus"][str(cpu)] = values
    return result


if __name__ == "__main__":
    # Apply before exec so every Rust thread inherits the requested mask.
    if len(sys.argv) < 3:
        raise SystemExit("cpu_list_and_executable_required")
    apply([int(value) for value in sys.argv[1].split(",")])
    os.execv(sys.argv[2], sys.argv[2:])
