"""Bounded four-mode HTTP comparison of request logs and telemetry readers."""
import argparse
from collections import Counter
from concurrent.futures import ProcessPoolExecutor
import hashlib
import http.client
import json
import multiprocessing
import os
from pathlib import Path
import socket
import sys
import tempfile
import time

import http_load as load

sys.path.insert(0, str(load.ROOT / "scripts"))
from release_smoke import Server, credentials

MODES = [("off", False), ("info", False), ("off", True), ("info", True)]
CONTEXT = BARRIER = None
CAP = 100_000


def init(context, barrier):
    global CONTEXT, BARRIER
    CONTEXT, BARRIER = context, barrier


def acceptable(scenario, index, status, code, value):
    if scenario == "missing":
        return (status == 404 and code == "document_not_found"
                and isinstance(value.get("request_id"), str))
    return status == 200 and code is None and value == CONTEXT["expected"][index]


def foreground(client):
    ctx = CONTEXT
    def request(step):
        index = (client * 97 + step) % ctx["documents"]
        suffix = "absent" if ctx["scenario"] == "missing" else f"d{index:06d}"
        start = time.perf_counter_ns()
        status, code, value = load.exchange(ctx["address"], ctx["token"], "GET",
            "/v1/collections/bench/documents/" + suffix)
        ok = acceptable(ctx["scenario"], index, status, code, value)
        return status, code, time.perf_counter_ns() - start, ok
    for step in range(ctx["warmup"]):
        if not request(step)[3]:
            raise RuntimeError("invalid_warmup_response")
    BARRIER.wait(timeout=30)
    start = time.perf_counter_ns()
    deadline = start + int(ctx["seconds"] * 1e9)
    statuses, codes = Counter(), Counter()
    good, bad = [], []
    step = 0
    while time.perf_counter_ns() < deadline and step < CAP:
        status, code, elapsed, ok = request(step)
        statuses[str(status) if status else "transport_error"] += 1
        if code:
            codes[code] += 1
        (good if ok else bad).append(elapsed)
        step += 1
    return {"started_ns": start, "ended_ns": time.perf_counter_ns(),
            "statuses": dict(statuses), "codes": dict(codes),
            "good": good, "bad": bad, "cap_hit": step >= CAP}


def recent_valid(value):
    return (isinstance(value, dict) and value.get("capacity") == 8
            and isinstance(value.get("errors"), list) and len(value["errors"]) <= 8
            and isinstance(value.get("dropped_total"), str)
            and value["dropped_total"].isascii() and value["dropped_total"].isdigit())


def reader(_):
    ctx = CONTEXT
    errors = []
    def collect():
        samples = []
        for endpoint in ("/v1/metrics", "/v1/errors/recent"):
            start = time.perf_counter_ns()
            try:
                status, body = server.request(endpoint, ctx["token"])
                if endpoint.endswith("metrics"):
                    valid = (status == 200 and b"kiarni_http_requests_total{" in body
                             and b"kiarni_storage_" in body)
                else:
                    valid = status == 200 and recent_valid(json.loads(body))
                if not valid:
                    errors.append({"endpoint": endpoint, "status": status, "error": "invalid_response"})
            except (OSError, http.client.HTTPException, ValueError) as error:
                body, valid = b"", False
                errors.append({"endpoint": endpoint, "error": type(error).__name__,
                               "errno": getattr(error, "errno", None)})
            samples.append((endpoint, time.perf_counter_ns() - start, valid, len(body)))
        return samples
    # Only the request helper is used; no child process is started here.
    server = Server(None, None, None)
    server.port = ctx["address"][1]
    if not all(item[2] for item in collect()):
        raise RuntimeError("invalid_observer_warmup")
    BARRIER.wait(timeout=30)
    start = time.perf_counter()
    deadline, tick = start + ctx["seconds"], start
    samples, late_cycles = [], 0
    while time.perf_counter() < deadline:
        collected = collect()
        samples.extend(collected)
        if not all(item[2] for item in collected):
            break
        tick += 1 / ctx["reader_hz"]
        now = time.perf_counter()
        if now > tick:
            late_cycles += 1
            tick = now + 1 / ctx["reader_hz"]
        time.sleep(max(0, min(tick, deadline) - now))
    return {"valid": bool(samples) and all(item[2] for item in samples),
            "errors": errors,
            "late_cycles": late_cycles,
            "endpoints": {endpoint: {"requests": len(items := [s for s in samples if s[0] == endpoint]),
                          "bytes": sum(s[3] for s in items),
                          "latency": load.percentiles([s[1] for s in items])}
                          for endpoint in ("/v1/metrics", "/v1/errors/recent")}}


def aggregate(outcomes):
    statuses, codes = Counter(), Counter()
    good, bad = [], []
    for item in outcomes:
        statuses.update(item["statuses"])
        codes.update(item["codes"])
        good.extend(item["good"])
        bad.extend(item["bad"])
    elapsed = (max(i["ended_ns"] for i in outcomes) - min(i["started_ns"] for i in outcomes)) / 1e9
    return {"elapsed_seconds": elapsed, "attempts": sum(statuses.values()),
            "expected_responses": len(good), "unexpected_responses": len(bad),
            "expected_rps": len(good) / elapsed,
            "expected_latency": load.percentiles(good), "unexpected_latency": load.percentiles(bad),
            "statuses": dict(statuses), "error_codes": dict(codes),
            "valid": bool(good) and not bad and not any(i["cap_hit"] for i in outcomes)}


def mode_order(repetition, cell):
    # Williams design: four repetitions balance positions and immediate predecessors.
    shift = (repetition - 1 + cell) % 4
    return [(mode + shift) % 4 for mode in (0, 1, 3, 2)]


def comparisons(rows):
    groups = {}
    for row in rows:
        key = (row["clients"], row["scenario"], row["repetition"])
        modes = groups.setdefault(key, {})
        mode = (row["log_level"], row["readers"])
        if mode in modes or not row["valid"]:
            raise ValueError("duplicate_or_invalid_mode")
        modes[mode] = row
    result = []
    for (clients, scenario, repetition), modes in groups.items():
        if set(modes) != set(MODES):
            raise ValueError("incomplete_comparison")
        for label, left, right in [
            ("logs_without_readers", MODES[0], MODES[1]),
            ("logs_with_readers", MODES[2], MODES[3]),
            ("readers_logs_off", MODES[0], MODES[2]),
            ("readers_logs_info", MODES[1], MODES[3]),
        ]:
            result.append({"clients": clients, "scenario": scenario, "repetition": repetition,
                           "comparison": label,
                           "rps_right_over_left": modes[right]["expected_rps"] / modes[left]["expected_rps"]})
    return result


def source_hash():
    digest = hashlib.sha256(load.fingerprint().encode())
    for path in (Path(__file__), load.ROOT / "scripts/release_smoke.py"):
        digest.update(path.read_bytes())
    return digest.hexdigest()


def tcp_snapshot():
    """Global Linux counters are context, not attribution to this server."""
    result = {}
    path = Path("/proc/net/netstat")
    if path.exists():
        lines = path.read_text().splitlines()
        for header, values in zip(lines[::2], lines[1::2]):
            if header.startswith("TcpExt:") and values.startswith("TcpExt:"):
                counters = dict(zip(header.split()[1:], values.split()[1:]))
                result["counters"] = {k: int(counters[k]) for k in
                    ("TCPTimeWaitOverflow", "TW", "TWRecycled", "ListenOverflows", "ListenDrops", "TCPTimeouts")
                    if k in counters}
    for name, relative in [("max_timewait", "ipv4/tcp_max_tw_buckets"),
                           ("timewait_reuse", "ipv4/tcp_tw_reuse"),
                           ("ephemeral_port_range", "ipv4/ip_local_port_range"),
                           ("conntrack_count", "netfilter/nf_conntrack_count"),
                           ("conntrack_max", "netfilter/nf_conntrack_max")]:
        path = Path("/proc/sys/net") / relative
        try:
            result[name] = path.read_text().strip()
        except OSError:
            pass
    path = Path("/proc/net/sockstat")
    if path.exists():
        for line in path.read_text().splitlines():
            if line.startswith("TCP:"):
                parts = line.split()[1:]
                result["tcp_sockets"] = {k: int(v) for k, v in zip(parts[::2], parts[1::2])}
    return result


def verify_dataset(server, token, dataset):
    """Never retry or hide a failed post-load check; preserve a bounded diagnostic."""
    for index, payload in enumerate(dataset):
        try:
            status, body = server.request(f"/v1/collections/bench/documents/d{index:06d}", token)
            if status != 200 or json.loads(body) != json.loads(payload):
                return False, {"index": index, "status": status, "error": "response_mismatch"}
        except (OSError, http.client.HTTPException, ValueError, UnicodeError) as error:
            return False, {"index": index, "error": type(error).__name__,
                           "errno": getattr(error, "errno", None)}
    return True, None


def run(args):
    binary = args.binary.resolve()
    fingerprint = source_hash()
    report = {"schema": 1, "environment": load.environment(binary),
              "measurement_source_sha256": fingerprint,
              "parameters": {**vars(args), "binary": str(binary), "output": str(args.output),
                  "modes": MODES, "load_model": "closed_loop; process/client; new TCP/request",
                  "sink": "temporary regular file on data filesystem, never a pipe",
                  "log_window": "entire server lifecycle including seed, warmup, verification and shutdown",
                  "recent_counter_window": "entire server lifecycle through post-load snapshot, including warmup",
                  "reader_schedule": "one process; both endpoints/cycle; skip missed ticks; no catch-up bursts",
                  "listener_policy": "one reserved loopback port reused across fresh server lifecycles; new TCP/request",
                  "server_config": dict(load.SERVER_CONFIG, heartbeat_ms=1000),
                  "build_command": "cargo build -p kiarni --release --locked --offline"},
              "results": []}
    report["parameters"].pop("expected", None)
    dataset = [load.document(i, 0, args.document_bytes) for i in range(args.documents)]
    report["dataset_sha256"] = hashlib.sha256(b"".join(dataset)).hexdigest()
    report["environment"]["cpu_load_before"] = list(os.getloadavg()) if hasattr(os, "getloadavg") else None
    with tempfile.TemporaryDirectory(prefix="kiarni-telemetry-") as work:
        directory = Path(work)
        report["environment"]["data_filesystem"] = load.filesystem(directory)
        report["environment"]["data_block_bytes"] = os.statvfs(directory).f_frsize
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        for repetition in range(1, args.repeat + 1):
            for ci, clients in enumerate(args.clients):
                for si, scenario in enumerate(args.scenarios):
                    for mode in mode_order(repetition, ci * len(args.scenarios) + si):
                        level, readers = MODES[mode]
                        with tempfile.TemporaryDirectory(dir=directory) as rowdir:
                            kernel_before = tcp_snapshot()
                            root = Path(rowdir) / "data"
                            server = Server(binary, Path(rowdir), root)
                            options = [part for k, v in load.SERVER_CONFIG.items() if k != "heartbeat_ms"
                                       for part in ("--" + k.replace("_", "-"), str(v))]
                            # Server helper supplies heartbeat=1000; there are no stream subscribers.
                            options.extend(["--log-level", level])
                            try:
                                server.start(options, port=port)
                                token, = credentials(root)
                                address = ("127.0.0.1", server.port)
                                load.control(address, token, "POST", "/v1/collections", load.encoded({"name": "bench"}))
                                for payload in dataset:
                                    load.control(address, token, "POST", "/v1/collections/bench/documents", payload)
                                ctx = {**vars(args), "expected": [json.loads(d) for d in dataset],
                                       "address": address, "token": token, "clients": clients, "scenario": scenario}
                                spawn = multiprocessing.get_context("spawn")
                                barrier = spawn.Barrier(clients + int(readers) + 1)
                                with ProcessPoolExecutor(max_workers=clients + int(readers), mp_context=spawn,
                                        initializer=init, initargs=(ctx, barrier)) as pool:
                                    futures = [pool.submit(foreground, i) for i in range(clients)]
                                    observer = pool.submit(reader, 0) if readers else None
                                    barrier.wait(timeout=30)
                                    row = aggregate([f.result() for f in futures])
                                    row["observer"] = observer.result() if observer else None
                                try:
                                    status, body = server.request("/v1/errors/recent", token)
                                    value = json.loads(body)
                                    row["recent_snapshot_valid"] = status == 200 and recent_valid(value)
                                    row["recent_dropped_total"] = value.get("dropped_total")
                                    row["recent_retained"] = len(value.get("errors", []))
                                except (OSError, http.client.HTTPException, ValueError) as error:
                                    row["recent_snapshot_valid"] = False
                                    row["recent_snapshot_error"] = {"error": type(error).__name__,
                                                                    "errno": getattr(error, "errno", None)}
                                row["valid"] &= row["recent_snapshot_valid"] and (not readers or row["observer"]["valid"])
                                # Read-only workload: verify every initial document after the measured window.
                                row["dataset_verified"], row["verification_error"] = verify_dataset(server, token, dataset)
                                row["valid"] &= row["dataset_verified"]
                                log = os.fdopen(os.dup(server.log.fileno()), "rb")
                            finally:
                                server.stop()
                            with log:
                                log.seek(0, 2)
                                row["log_bytes"] = log.tell()
                                if row["log_bytes"] > 128 * 1024 * 1024:
                                    raise RuntimeError("log_budget_exceeded")
                                log.seek(0)
                                logs, drops, malformed = 0, 0, 0
                                for line in log:
                                    try:
                                        record = json.loads(line)
                                    except (ValueError, UnicodeError):
                                        malformed += int(line.startswith(b"{"))
                                        continue
                                    if record.get("event") == "request_complete":
                                        logs += 1
                                        drops += record["dropped_before"]
                                row.update(log_records=logs, reported_log_drops=drops, malformed_log_lines=malformed)
                            row.update(log_level=level, readers=readers, clients=clients, scenario=scenario, repetition=repetition)
                            row.update(kernel_tcp_before=kernel_before, kernel_tcp_after=tcp_snapshot())
                            row["valid"] &= malformed == 0 and (logs == 0 if level == "off" else logs > 0)
                            if token in json.dumps(row):
                                raise RuntimeError("secret_in_report")
                            report["results"].append(row)
                            # Preserve completed rows if a later setup/control call aborts the run.
                            with args.output.with_suffix(".progress.jsonl").open("a") as journal:
                                journal.write(json.dumps(row) + "\n")
                            print(json.dumps({"trial": len(report["results"]), "scenario": scenario,
                                  "clients": clients, "log_level": level, "readers": readers,
                                  "rps": round(row["expected_rps"], 2), "valid": row["valid"]}), flush=True)
    report["environment"]["cpu_load_after"] = list(os.getloadavg()) if hasattr(os, "getloadavg") else None
    if source_hash() != fingerprint:
        raise RuntimeError("source_changed_during_measurement")
    report["valid"] = bool(report["results"]) and all(row["valid"] for row in report["results"])
    report["comparisons"] = comparisons(report["results"]) if report["valid"] else None
    return report


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=load.ROOT / "target/release/kiarni")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--clients", type=int, nargs="+", default=[1, 8, 32])
    parser.add_argument("--scenarios", choices=["read", "missing"], nargs="+", default=["read", "missing"])
    parser.add_argument("--repeat", type=int, default=4)
    parser.add_argument("--seconds", type=float, default=3)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--reader-hz", type=float, default=2)
    parser.add_argument("--documents", type=int, default=256)
    parser.add_argument("--document-bytes", type=int, default=1024)
    args = parser.parse_args()
    if (not args.clients or len(set(args.clients)) != len(args.clients) or any(not 1 <= c <= 32 for c in args.clients)
        or len(set(args.scenarios)) != len(args.scenarios) or not 1 <= args.repeat <= 8
        or not 0.1 <= args.seconds <= 10 or not 0 <= args.warmup <= 100
        or not 0.1 <= args.reader_hz <= 10 or not max(args.clients) <= args.documents <= 1024
        or not 128 <= args.document_bytes <= 4096 or args.output.exists() or not args.output.parent.is_dir()):
        parser.error("invalid bounded matrix or output path")
    return args


if __name__ == "__main__":
    options = arguments()
    journal = options.output.with_suffix(".progress.jsonl")
    descriptor = os.open(journal, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    os.close(descriptor)
    measured = run(options)
    with os.fdopen(os.open(options.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
        json.dump(measured, output, indent=2, default=str)
        output.write("\n")
    raise SystemExit(0 if measured["valid"] else 1)
