"""Closed-loop HTTP benchmark of an isolated Kiarni server, using only stdlib."""

import argparse
from collections import Counter, defaultdict
from concurrent.futures import ProcessPoolExecutor
from datetime import datetime, timezone
import hashlib
import http.client
import json
import math
import multiprocessing
import os
from pathlib import Path
import platform
import signal
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
CONTEXT = None
BARRIER = None
SERVER_CONFIG = {
    "max_requests": 32, "max_streams": 8, "feed_queue": 4,
    "max_document_bytes": 1048576, "max_query_bytes": 16384,
    "max_result_bytes": 4194304, "read_timeout_ms": 5000,
    "write_timeout_ms": 5000, "heartbeat_ms": 15000,
}


def encoded(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()


def document(index, revision, size):
    value = {
        "_id": f"d{index:06d}",
        "score": index,
        "group": "even" if index % 2 == 0 else "odd",
        "revision": revision,
        "padding": "",
    }
    value["padding"] = "x" * (size - len(encoded(value)))
    result = encoded(value)
    if len(result) != size:
        raise RuntimeError("document_size_too_small")
    return result


def exchange(address, token, method, path, body=None):
    connection = http.client.HTTPConnection(*address, timeout=5)
    headers = {"Authorization": f"Bearer {token}"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    try:
        connection.request(method, path, body=body, headers=headers)
        response = connection.getresponse()
        data = response.read(4 * 1024 * 1024 + 1)
        if len(data) > 4 * 1024 * 1024:
            return response.status, "oversized_response", None
        value = json.loads(data) if data else None
        code = value.get("error", {}).get("code") if isinstance(value, dict) else None
        return response.status, code, value
    except (OSError, http.client.HTTPException) as error:
        return 0, type(error).__name__, None
    except (ValueError, UnicodeError):
        return 0, "invalid_json_response", None
    finally:
        connection.close()


def worker_init(context, barrier):
    global CONTEXT, BARRIER
    CONTEXT, BARRIER = context, barrier


def operation(client, step):
    setting = CONTEXT
    scenario = setting["scenario"]
    kind = "write" if scenario == "mixed" and step % 5 == 0 else scenario
    if kind == "mixed":
        kind = "read"
    index = client if kind == "write" else (client * 97 + step) % setting["documents"]
    path = f"/v1/collections/bench/documents/d{index:06d}"
    revision = setting["round"] * 1_000_000 + step
    if kind == "write":
        method, body = "PUT", document(index, revision, setting["document_bytes"])
    elif kind == "query":
        method, path, body = "POST", "/v1/collections/bench/query", setting["query"]
    else:
        method, body = "GET", None
    started = time.perf_counter_ns()
    status, code, value = exchange(setting["address"], setting["token"], method, path, body)
    if status == 200:
        if kind == "query":
            valid = value == setting["expected_query"]
        else:
            valid = isinstance(value, dict) and value.get("_id") == f"d{index:06d}"
            if kind == "write":
                valid = valid and value.get("revision") == revision
        if not valid:
            code = "invalid_success_response"
    return kind, status, code, time.perf_counter_ns() - started, revision


def worker(client):
    for step in range(CONTEXT["warmup"]):
        operation(client, step)
    BARRIER.wait(timeout=30)
    started = time.perf_counter_ns()
    cpu_started = time.process_time()
    deadline = started + int(CONTEXT["seconds"] * 1_000_000_000)
    counts = Counter()
    codes = Counter()
    by_operation = defaultdict(Counter)
    success_ns, failed_ns = [], []
    last_ack = None
    uncertain_writes = 0
    step = CONTEXT["warmup"]
    while time.perf_counter_ns() < deadline and step < 100000:
        kind, status, code, elapsed, revision = operation(client, step)
        key = str(status) if status else "transport_error"
        counts[key] += 1
        by_operation[kind][key] += 1
        if code:
            codes[code] += 1
        if status == 200 and code is None:
            success_ns.append(elapsed)
            if kind == "write":
                last_ack = revision
        else:
            failed_ns.append(elapsed)
        if kind == "write" and (status == 0 or (status >= 500 and code != "write_busy")):
            uncertain_writes += 1
        step += 1
    return {
        "started_ns": started,
        "ended_ns": time.perf_counter_ns(),
        "cpu_seconds": time.process_time() - cpu_started,
        "counts": dict(counts),
        "codes": dict(codes),
        "operations": {key: dict(value) for key, value in by_operation.items()},
        "success_ns": success_ns,
        "failed_ns": failed_ns,
        "last_ack": last_ack,
        "uncertain_writes": uncertain_writes,
        "client": client,
        "operation_cap_hit": step >= 100000,
    }


def percentiles(samples):
    if not samples:
        return None
    samples = sorted(samples)
    return {
        name: round(samples[math.ceil(len(samples) * quantile) - 1] / 1_000_000, 4)
        for name, quantile in [("p50_ms", 0.50), ("p95_ms", 0.95), ("p99_ms", 0.99), ("max_ms", 1)]
    }


def control(address, token, method, path, body=None):
    status, code, value = exchange(address, token, method, path, body)
    if status not in (200, 201, 204) or code is not None:
        raise RuntimeError(f"control_request_failed_status_{status}")
    return value


def run_round(context):
    count = context["clients"]
    spawn = multiprocessing.get_context("spawn")
    barrier = spawn.Barrier(count)
    with ProcessPoolExecutor(
        max_workers=count, mp_context=spawn,
        initializer=worker_init, initargs=(context, barrier),
    ) as pool:
        outcomes = list(pool.map(worker, range(count), chunksize=1))
    return summarize(context, outcomes)


def summarize(context, outcomes):
    count = context["clients"]
    elapsed = (max(value["ended_ns"] for value in outcomes) -
               min(value["started_ns"] for value in outcomes)) / 1_000_000_000
    statuses, codes = Counter(), Counter()
    operations = defaultdict(Counter)
    success, failed = [], []
    for value in outcomes:
        statuses.update(value["counts"])
        codes.update(value["codes"])
        success.extend(value["success_ns"])
        failed.extend(value["failed_ns"])
        for kind, counts in value["operations"].items():
            operations[kind].update(counts)
        if value["last_ack"] is not None:
            persisted = control(context["address"], context["token"], "GET",
                                f'/v1/collections/bench/documents/d{value["client"]:06d}')
            if persisted["revision"] < value["last_ack"]:
                raise RuntimeError("acknowledged_write_not_visible")
    return {
        "scenario": context["scenario"],
        "clients": count,
        "repetition": context["repetition"],
        "elapsed_seconds": round(elapsed, 6),
        "attempts": sum(statuses.values()),
        "confirmed": len(success),
        "attempted_ops_per_second": round(sum(statuses.values()) / elapsed, 3),
        "confirmed_ops_per_second": round(len(success) / elapsed, 3),
        "success_latency": percentiles(success),
        "failure_latency": percentiles(failed),
        "statuses": dict(statuses),
        "error_codes": dict(codes),
        "operations": {key: dict(value) for key, value in operations.items()},
        "client_cpu_seconds": round(sum(value["cpu_seconds"] for value in outcomes), 6),
        "uncertain_write_outcomes": sum(value["uncertain_writes"] for value in outcomes),
        "acknowledged_writes_verified": True,
        "operation_cap_hit": any(value["operation_cap_hit"] for value in outcomes),
    }


def command_output(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def fingerprint():
    digest = hashlib.sha256()
    files = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", Path(__file__).resolve()]
    for folder in ["apps", "crates"]:
        files.extend((ROOT / folder).rglob("*.rs"))
        files.extend((ROOT / folder).rglob("Cargo.toml"))
    for path in sorted(set(files)):
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def environment(binary):
    cpu, memory = None, None
    if Path("/proc/cpuinfo").exists():
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    if Path("/proc/meminfo").exists():
        line = Path("/proc/meminfo").read_text().splitlines()[0]
        memory = int(line.split()[1]) * 1024
    return {
        "utc": datetime.now(timezone.utc).isoformat(),
        "os": platform.system(),
        "kernel": platform.release(),
        "machine": platform.machine(),
        "cpu_model": cpu,
        "logical_cpus": os.cpu_count(),
        "cpu_affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "memory_total_bytes": memory,
        "rustc": command_output("rustc", "--version"),
        "cargo": command_output("cargo", "--version"),
        "python": platform.python_version(),
        "git_head": command_output("git", "rev-parse", "HEAD"),
        "workspace_dirty": bool(command_output("git", "status", "--porcelain")),
        "source_sha256": fingerprint(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    }

def filesystem(path):
    if not Path("/proc/mounts").exists():
        return None
    selected, kind = "", None
    for line in Path("/proc/mounts").read_text().splitlines():
        fields = line.split()
        mount = fields[1].replace("\\040", " ")
        if path.is_relative_to(Path(mount)) and len(mount) >= len(selected):
            selected, kind = mount, fields[2]
    return kind


def benchmark(args):
    binary = args.binary.resolve()
    report = {
        "schema": 1,
        "environment": environment(binary),
        "parameters": {
            "clients": args.clients, "repetitions": args.repeat,
            "seconds_per_round": args.seconds, "warmup_attempts_per_client": args.warmup,
            "documents": args.documents, "document_bytes": args.document_bytes,
            "scenarios": args.scenarios,
            "mixed_write_fraction": 0.2, "server_config": SERVER_CONFIG,
            "profile": "release", "connection_per_request": True,
            "load_model": "closed_loop", "retries": 0,
        },
        "results": [],
    }
    with tempfile.TemporaryDirectory(prefix="kiarni-http-bench-") as temporary:
        directory = Path(temporary)
        report["environment"]["data_filesystem"] = filesystem(directory)
        report["environment"]["data_block_bytes"] = os.statvfs(directory).f_frsize
        root = directory / "data"
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            address = reservation.getsockname()
        env = {key: value for key, value in os.environ.items() if not key.startswith("KIARNI_")}
        with (directory / "server.log").open("xb") as log:
            child = subprocess.Popen(
                [str(binary), "serve", "--data-dir", str(root), "--bind",
                 f"{address[0]}:{address[1]}", *[
                     part for key, value in SERVER_CONFIG.items()
                     for part in ("--" + key.replace("_", "-"), str(value))
                 ]],
                cwd=directory, env=env, stdout=log, stderr=log,
            )
            try:
                deadline = time.monotonic() + 10
                token = None
                while time.monotonic() < deadline:
                    if child.poll() is not None:
                        raise RuntimeError("server_startup_failed")
                    paths = list((directory / "data.credentials").glob("token-*.txt"))
                    if len(paths) == 1:
                        token = paths[0].read_text()
                        if exchange(address, token, "GET", "/v1/health/ready")[0] == 200:
                            break
                    time.sleep(0.01)
                else:
                    raise RuntimeError("server_startup_timeout")
                control(address, token, "POST", "/v1/collections", encoded({"name": "bench"}))
                dataset_hash = hashlib.sha256()
                for index in range(args.documents):
                    body = document(index, 0, args.document_bytes)
                    dataset_hash.update(body)
                    control(address, token, "POST", "/v1/collections/bench/documents", body)
                report["parameters"]["initial_dataset_sha256"] = dataset_hash.hexdigest()
                selected = [index for index in reversed(range(args.documents))
                            if index % 2 == 0 and index >= args.documents // 2][:10]
                ast = {
                    "from": "bench", "limit": 10,
                    "filter": {"op": "all", "args": [
                        {"op": "eq", "field": "group", "value": "even"},
                        {"op": "gte", "field": "score", "value": args.documents // 2},
                    ]},
                    "sort": [{"field": "score", "direction": "desc"}], "project": ["score"],
                }
                expected = {"documents": [{"_id": f"d{index:06d}", "score": index}
                                          for index in selected], "count": len(selected)}
                round_id = 0
                for repetition in range(1, args.repeat + 1):
                    for clients in args.clients:
                        for scenario in report["parameters"]["scenarios"]:
                            round_id += 1
                            row = run_round({
                                "address": address, "token": token, "clients": clients,
                                "scenario": scenario, "round": round_id, "repetition": repetition,
                                "warmup": args.warmup, "seconds": args.seconds,
                                "documents": args.documents, "document_bytes": args.document_bytes,
                                "query": encoded(ast), "expected_query": expected,
                            })
                            report["results"].append(row)
                            print(json.dumps({
                                "scenario": scenario, "clients": clients, "repetition": repetition,
                                "confirmed_ops_per_second": row["confirmed_ops_per_second"],
                                "statuses": row["statuses"],
                            }), flush=True)
                control(address, token, "GET", "/v1/health/ready")
                if token in json.dumps(report):
                    raise RuntimeError("secret_in_report")
            finally:
                if child.poll() is None:
                    child.send_signal(signal.SIGTERM)
                try:
                    status = child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
                    raise RuntimeError("graceful_shutdown_failed")
                if status != 0:
                    raise RuntimeError("server_exit_failed")
            for line in (directory / "server.log").read_text().splitlines():
                if line.startswith("KIARNI_PROFILE "):
                    report["server_profile"] = json.loads(line.removeprefix("KIARNI_PROFILE "))
            if token in json.dumps(report):
                raise RuntimeError("secret_in_report")
    report["valid"] = all(
        row["confirmed"] > 0 and not row["operation_cap_hit"]
        and not (set(row["statuses"]) - {"200", "503", "transport_error"})
        and "invalid_success_response" not in row["error_codes"]
        and "invalid_json_response" not in row["error_codes"]
        and "oversized_response" not in row["error_codes"]
        for row in report["results"]
    )
    return report


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/kiarni")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--clients", nargs="+", type=int, default=[1, 8, 32])
    parser.add_argument("--scenarios", nargs="+", choices=["read", "write", "query", "mixed"],
                        default=["read", "write", "query", "mixed"])
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--seconds", type=float, default=2)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--documents", type=int, default=256)
    parser.add_argument("--document-bytes", type=int, default=1024)
    args = parser.parse_args()
    if len(set(args.scenarios)) != len(args.scenarios):
        parser.error("scenarios must be distinct")
    if not args.clients or len(set(args.clients)) != len(args.clients) or any(not 1 <= n <= 32 for n in args.clients):
        parser.error("clients must be distinct integers from 1 to 32")
    if not 1 <= args.repeat <= 10 or not 0.1 <= args.seconds <= 60 or not 0 <= args.warmup <= 1000:
        parser.error("invalid repetition, duration or warmup")
    if not max(args.clients) <= args.documents <= 10000 or not 128 <= args.document_bytes <= 1048576:
        parser.error("invalid dataset dimensions")
    if args.documents * args.document_bytes > 8 * 1024 * 1024:
        parser.error("dataset exceeds the initial query scan budget")
    if args.output.exists():
        parser.error("output already exists")
    if not args.output.parent.is_dir():
        parser.error("output parent directory does not exist")
    return args


if __name__ == "__main__":
    options = arguments()
    measured = benchmark(options)
    # Exclusive creation preserves earlier benchmark artifacts.
    with os.fdopen(os.open(options.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
        json.dump(measured, output, indent=2)
        output.write("\n")
    raise SystemExit(0 if measured["valid"] else 1)
