"""Paired secondary-index measurements through the real Kiarni HTTP/CLI paths."""
import argparse
from collections import Counter, defaultdict
from concurrent.futures import ProcessPoolExecutor
import hashlib
from functools import lru_cache
import json
import multiprocessing
from math import gcd
import os
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time

import http_load as load

sys.path.insert(0, str(load.ROOT / "scripts"))
from release_smoke import Server, credentials

CASES = ["absent", "needle", "sparse", "medium", "edge", "over", "dense", "compound",
         "id", "put_indexed", "put_other", "churn"]
WRITES = {"put_indexed", "put_other", "churn"}
CONFIG = dict(load.SERVER_CONFIG, heartbeat_ms=1000, log_level="off")
LIMIT = 100_000
CONTEXT = BARRIER = None


def distribution(count):
    sizes = [("needle", 1), ("sparse", count // 64), ("medium", count // 16),
             ("edge", count // 8), ("over", count // 8 + 1)]
    return sizes + [("dense", count - sum(size for _, size in sizes))]


@lru_cache(maxsize=16)
def spread_stride(count):
    if count < 1:
        raise ValueError("invalid_document_count")
    stride = 97
    while gcd(stride, count) != 1:
        stride += 2
    return stride


def tag_for(index, count, layout="contiguous"):
    if index < 0:
        raise ValueError("invalid_document_index")
    if index >= count:
        raise ValueError("invalid_document_index")
    if layout == "spread":
        index = index * spread_stride(count) % count
    for tag, size in distribution(count):
        if index < size:
            return tag
        index -= size
    raise ValueError("invalid_document_index")


def document(index, revision, count, size, tag=None, churn=False, layout="contiguous"):
    value = {"_id": f'{"w" if churn else "d"}{index:06d}', "score": index,
             "tag": tag if tag is not None else tag_for(index, count, layout),
             "revision": revision, "padding": ""}
    padding = size - len(load.encoded(value))
    if padding < 0:
        raise ValueError("document_size_too_small")
    value["padding"] = "x" * padding
    result = load.encoded(value)
    if len(result) != size:
        raise ValueError("incorrect_document_size")
    return result


def query_case(case, count, layout="contiguous"):
    ast = {"from": "bench", "limit": 10, "sort": [{"field": "score", "direction": "desc"}],
           "project": ["score"]}
    tag = "dense" if case == "compound" else case
    if case == "id":
        ast["id"] = f"d{count // 2:06d}"
        selected = [count // 2]
    else:
        predicate = {"op": "eq", "field": "tag", "value": tag}
        ast["filter"] = ({"op": "all", "args": [predicate,
                          {"op": "gte", "field": "score", "value": 0}]}
                         if case == "compound" else predicate)
        selected = [i for i in reversed(range(count)) if tag_for(i, count, layout) == tag][:10]
    expected = {"count": len(selected), "documents": [{"_id": f"d{i:06d}", "score": i} for i in selected]}
    return ast, expected


def expected_access(case, indexed):
    if case == "id":
        return "id"
    if indexed and case in {"absent", "needle", "sparse", "medium", "edge"}:
        return "secondary"
    return "scan"


def pair_order(repetition, client_index, case_index):
    return [False, True] if (repetition + client_index + case_index) % 2 else [True, False]


def classification(kind, status, code, correct):
    expected = {"query": 200, "put": 200, "insert": 201, "delete": 204}[kind]
    confirmed = status == expected and code is None and correct
    busy = status == 503 and code in {"write_busy", "service_unavailable"}
    uncertain = kind != "query" and not busy and (status == 0 or status >= 500 or
                                                 (status == expected and not confirmed))
    return confirmed, busy, uncertain


def initialize(context, barrier):
    global CONTEXT, BARRIER
    CONTEXT, BARRIER = context, barrier


def attempt(client, step, state):
    context = CONTEXT
    case, count, size = context["case"], context["documents"], context["document_bytes"]
    revision = context["pair"] * 1_000_000 + step + 1
    if case not in WRITES:
        kind, method, path, body, expected = "query", "POST", "/v1/collections/bench/query", context["query"], context["expected"]
    elif case == "churn" and state["present"]:
        kind, method, path, body, expected = "delete", "DELETE", f"/v1/collections/bench/documents/w{client:06d}", None, None
    else:
        churn = case == "churn"
        tag = "churn" if churn else (("changed1" if state["expected"]["tag"] == "changed0" else "changed0")
                                     if case == "put_indexed" else None)
        body = document(client, revision, count, size, tag, churn, context["layout"])
        expected = json.loads(body)
        kind, method, path = (("insert", "POST", "/v1/collections/bench/documents") if churn else
                              ("put", "PUT", f"/v1/collections/bench/documents/d{client:06d}"))
    started = time.perf_counter_ns()
    status, code, value = load.exchange(context["address"], context["token"], method, path, body)
    correct = value == expected
    confirmed, busy, uncertain = classification(kind, status, code, correct)
    if status in (200, 201, 204) and not confirmed:
        code = code or "invalid_success_response"
    if confirmed and kind != "query":
        state["present"] = kind != "delete"
        state["expected"] = expected
    return kind, status, code, time.perf_counter_ns() - started, confirmed, busy, uncertain


def worker(client):
    context = CONTEXT
    state = {"present": context["case"] != "churn", "expected": None}
    if context["case"] in WRITES and context["case"] != "churn":
        state["expected"] = json.loads(document(client, 0, context["documents"], context["document_bytes"], layout=context["layout"]))
    for step in range(context["warmup"]):
        outcome = attempt(client, step, state)
        if not (outcome[4] or outcome[5]):
            BARRIER.abort()
            raise RuntimeError("warmup_failed")
    BARRIER.wait(timeout=30)
    started, cpu = time.perf_counter_ns(), time.process_time()
    deadline = started + int(context["seconds"] * 1e9)
    statuses, codes = Counter(), Counter()
    success, failed, kinds = [], [], defaultdict(lambda: {"success": [], "failed": []})
    uncertain, attempts, invalid = 0, 0, False
    while time.perf_counter_ns() < deadline and attempts < LIMIT:
        kind, status, code, elapsed, ok, busy, unknown = attempt(client, attempts + context["warmup"], state)
        statuses[str(status) if status else "transport_error"] += 1
        if code:
            codes[code] += 1
        (success if ok else failed).append(elapsed)
        kinds[kind]["success" if ok else "failed"].append(elapsed)
        uncertain += int(unknown)
        attempts += 1
        if not (ok or busy):
            invalid = True
            break  # Never repeat a request whose write outcome may be unknown.
    return {"client": client, "start": started, "end": time.perf_counter_ns(),
            "client_cpu_seconds": time.process_time() - cpu, "statuses": dict(statuses),
            "codes": dict(codes), "success": success, "failed": failed, "kinds": dict(kinds),
            "uncertain": uncertain, "invalid": invalid, "cap_hit": attempts == LIMIT, "state": state}


def metrics(server, token):
    status, data = server.request("/v1/metrics", token)
    if status != 200:
        raise RuntimeError("metrics_failed")
    values = {}
    for line in data.decode().splitlines():
        if not line or line.startswith("#"):
            continue
        name, value = line.split(" ", 1)
        if name.startswith(("kiarni_query_execution_total", "kiarni_storage_commits_total",
                            "kiarni_storage_group_receipts_total", "kiarni_storage_group_items_total")):
            values[name] = int(value)
    return values


def run_load(context, server, token):
    spawn = multiprocessing.get_context("spawn")
    barrier = spawn.Barrier(context["clients"] + 1)
    with ProcessPoolExecutor(max_workers=context["clients"], mp_context=spawn,
                             initializer=initialize, initargs=(context, barrier)) as pool:
        futures = [pool.submit(worker, client) for client in range(context["clients"])]
        deadline = time.monotonic() + 30
        while barrier.n_waiting != context["clients"]:
            if time.monotonic() > deadline or any(future.done() for future in futures):
                barrier.abort()
                raise RuntimeError("client_warmup_barrier_failed")
            time.sleep(0.005)
        before = metrics(server, token)
        barrier.wait(timeout=5)
        outcomes = [future.result(timeout=context["seconds"] + 15) for future in futures]
        after = metrics(server, token)
    elapsed = (max(row["end"] for row in outcomes) - min(row["start"] for row in outcomes)) / 1e9
    statuses, codes, kinds = Counter(), Counter(), defaultdict(lambda: {"success": [], "failed": []})
    success, failed = [], []
    for row in outcomes:
        statuses.update(row["statuses"])
        codes.update(row["codes"])
        success.extend(row["success"])
        failed.extend(row["failed"])
        for kind, samples in row["kinds"].items():
            for outcome in samples:
                kinds[kind][outcome].extend(samples[outcome])
    delta = {key: after[key] - value for key, value in before.items()}
    access = {name: delta[f'kiarni_query_execution_total{{access="{name}"}}'] for name in ["id", "scan", "secondary"]}
    path_valid = (all(value == (len(success) if name == expected_access(context["case"], context["indexed"]) else 0)
                      for name, value in access.items()) if context["case"] not in WRITES else sum(access.values()) == 0)
    report = {"case": context["case"], "indexed": context["indexed"], "clients": context["clients"],
              "trial": context["trial"], "pair": context["pair"],
              "repetition": context["repetition"], "documents": context["documents"], "document_bytes": context["document_bytes"],
              "elapsed_seconds": elapsed, "attempts": sum(statuses.values()), "confirmed": len(success),
              "confirmed_ops_per_second": len(success) / elapsed,
              "success_latency": load.percentiles(success), "failure_latency": load.percentiles(failed),
              "statuses": dict(statuses), "error_codes": dict(codes), "access": access,
              "server_counter_delta": delta, "client_cpu_seconds": sum(row["client_cpu_seconds"] for row in outcomes),
              "uncertain_write_outcomes": sum(row["uncertain"] for row in outcomes),
              "per_operation": {kind: {"confirmed": len(samples["success"]), "failed": len(samples["failed"]),
                               "success_latency": load.percentiles(samples["success"]), "failure_latency": load.percentiles(samples["failed"])}
                                for kind, samples in kinds.items()},
              "valid": bool(success) and path_valid and not any(row["invalid"] or row["cap_hit"] for row in outcomes)}
    return report, outcomes


def cli(binary, *args):
    started = time.perf_counter_ns()
    result = subprocess.run([str(binary), *map(str, args)], env={}, capture_output=True, timeout=60)
    if result.returncode:
        raise RuntimeError(f"control_cli_failed_{args[0]}")
    return (time.perf_counter_ns() - started) / 1e6


def server_for(binary, directory, root):
    return Server(binary, directory, root).start([
        part for key, value in CONFIG.items() if key != "heartbeat_ms"
        for part in ("--" + key.replace("_", "-"), str(value))])


def seed(binary, directory, count, size, layout):
    root = directory / "seed"
    server = server_for(binary, directory, root)
    try:
        token = next(iter(credentials(root)))
        load.control(("127.0.0.1", server.port), token, "POST", "/v1/collections", load.encoded({"name": "bench"}))
        digest = hashlib.sha256()
        for index in range(count):
            body = document(index, 0, count, size, layout=layout)
            digest.update(body)
            load.control(("127.0.0.1", server.port), token, "POST", "/v1/collections/bench/documents", body)
    finally:
        server.stop()
    cli(binary, "backup", "--data-dir", root, "--output", directory / "plain")
    build_ms = cli(binary, "index", "create", "--data-dir", root, "--collection", "bench", "--field", "tag")
    audit_ms = cli(binary, "index", "audit", "--data-dir", root)
    cli(binary, "backup", "--data-dir", root, "--output", directory / "indexed")
    return {"dataset_sha256": digest.hexdigest(), "distribution": dict(distribution(count)), "layout": layout,
            "offline_create_including_open_audit_ms": build_ms, "offline_audit_ms": audit_ms,
            "artifact_manifests": {mode: json.loads((directory / mode / "backup.json").read_text()) for mode in ["plain", "indexed"]},
            "artifact_logical_bytes": {mode: (directory / mode / "store/data.redb").stat().st_size for mode in ["plain", "indexed"]}}


def verify(server, token, context, outcomes):
    address = ("127.0.0.1", server.port)
    if context["case"] not in WRITES:
        status, code, value = load.exchange(address, token, "POST", "/v1/collections/bench/query", context["query"])
        return status == 200 and code is None and value == context["expected"]
    for row in outcomes:
        prefix = "w" if context["case"] == "churn" else "d"
        status, code, value = load.exchange(address, token, "GET", f'/v1/collections/bench/documents/{prefix}{row["client"]:06d}')
        expected = row["state"]
        if expected["present"]:
            if status != 200 or code is not None or value != expected["expected"]:
                return False
        elif status != 404 or code != "document_not_found":
            return False
    return True


def paired_summary(rows):
    pairs = defaultdict(dict)
    for row in rows:
        key = (row["documents"], row["document_bytes"], row["clients"], row["case"], row["repetition"])
        if row["indexed"] in pairs[key]:
            raise ValueError("duplicate_pair_member")
        pairs[key][row["indexed"]] = row
    groups = defaultdict(list)
    for key, pair in pairs.items():
        if set(pair) != {False, True} or not all(row["valid"] for row in pair.values()):
            raise ValueError("missing_or_invalid_pair")
        plain, indexed = pair[False], pair[True]
        groups[key[:4]].append({"repetition": key[4],
            "throughput_index_over_plain": indexed["confirmed_ops_per_second"] / plain["confirmed_ops_per_second"],
            "p50_index_over_plain": indexed["success_latency"]["p50_ms"] / plain["success_latency"]["p50_ms"],
            "p95_index_over_plain": indexed["success_latency"]["p95_ms"] / plain["success_latency"]["p95_ms"],
            "plain_confirmed": plain["confirmed"], "indexed_confirmed": indexed["confirmed"]})
    return [{"documents": key[0], "document_bytes": key[1], "clients": key[2], "case": key[3],
             "pairs": pairs, "median_throughput_index_over_plain": statistics.median(p["throughput_index_over_plain"] for p in pairs),
             "median_p50_index_over_plain": statistics.median(p["p50_index_over_plain"] for p in pairs),
             "median_p95_index_over_plain": statistics.median(p["p95_index_over_plain"] for p in pairs)}
            for key, pairs in sorted(groups.items())]


def source_hash():
    files = [Path(__file__), load.ROOT / "benches/http_load.py", load.ROOT / "scripts/release_smoke.py"]
    return hashlib.sha256((load.fingerprint() + "".join(hashlib.sha256(p.read_bytes()).hexdigest() for p in files)).encode()).hexdigest()


def cgroup_limits():
    result = []
    membership = Path("/proc/self/cgroup")
    if membership.exists():
        relative = next((line[3:] for line in membership.read_text().splitlines() if line.startswith("0::")), None)
        if relative is not None:
            root = Path("/sys/fs/cgroup")
            current = root / relative.lstrip("/")
            while current.is_relative_to(root):
                values = {name: (current / name).read_text().strip() for name in
                          ["cpu.max", "cpuset.cpus.effective", "memory.max"] if (current / name).is_file()}
                if values:
                    result.append(values)
                if current == root:
                    break
                current = current.parent
    return result or None


def benchmark(args):
    binary = args.binary.resolve(strict=True)
    fingerprint = source_hash()
    report = {"schema": 1, "environment": load.environment(binary),
              "measurement_source_sha256": fingerprint,
              "parameters": {"documents": args.documents, "document_bytes": args.document_bytes, "layout": args.layout,
                  "clients": args.clients, "cases": args.cases, "repetitions": args.repeat,
                  "seconds_per_round": args.seconds, "warmup_per_client": args.warmup, "server_config": CONFIG,
                  "load_model": "closed_loop; one new TCP connection/request; process/client",
                  "pair_order": "alternates each pair; fresh CLI restore/server per member",
                  "commit": "Immediate; normal bounded writer grouping; no subscribers",
                  "binary_profile": "release, default features; build command recorded in methodology",
                  "build_command": ["cargo", "build", "-p", "kiarni", "--release", "--locked", "--offline"],
                  "latency": "HTTP, decode and response validation; excludes payload construction",
                  "retry_policy": "no uncertain write retries; churn retains known state after busy",
                  "attempt_cap_per_client": LIMIT}, "results": []}
    with tempfile.TemporaryDirectory(prefix="kiarni-index-http-", dir=args.work_dir) as temporary:
        directory = Path(temporary)
        report["environment"].update(data_filesystem=load.filesystem(directory), data_block_bytes=os.statvfs(directory).f_frsize,
                                     cpu_load_before=list(os.getloadavg()) if hasattr(os, "getloadavg") else None,
                                     cgroup_v2_limits=cgroup_limits())
        report["seed"] = seed(binary, directory, args.documents, args.document_bytes, args.layout)
        trial = 0
        for repetition in range(1, args.repeat + 1):
            for client_index, clients in enumerate(args.clients):
                for case_index, case in enumerate(args.cases):
                    order = pair_order(repetition, client_index, case_index)
                    for indexed in order:
                        trial += 1
                        with tempfile.TemporaryDirectory(prefix="round-", dir=directory) as work:
                            root = Path(work) / "data"
                            cli(binary, "restore", "--from", directory / ("indexed" if indexed else "plain"), "--data-dir", root)
                            context = {"case": case, "indexed": indexed, "clients": clients,
                                       "repetition": repetition, "trial": trial, "pair": (trial + 1) // 2, "seconds": args.seconds,
                                       "documents": args.documents, "document_bytes": args.document_bytes, "warmup": args.warmup, "layout": args.layout}
                            if case not in WRITES:
                                ast, expected = query_case(case, args.documents, args.layout)
                                context.update(query=load.encoded(ast), expected=expected)
                            server = server_for(binary, Path(work), root)
                            try:
                                token = next(iter(credentials(root)))
                                context.update(address=("127.0.0.1", server.port), token=token)
                                before = (root / "store/data.redb").stat().st_size
                                row, outcomes = run_load(context, server, token)
                            finally:
                                server.stop()
                            row["file_logical_bytes"] = {"before": before, "after": (root / "store/data.redb").stat().st_size}
                            cli(binary, "index", "audit", "--data-dir", root)
                            server = server_for(binary, Path(work), root)
                            try:
                                row["post_restart_verified"] = verify(server, token, context, outcomes)
                            finally:
                                server.stop()
                            row["valid"] = row["valid"] and row["post_restart_verified"]
                            if token in json.dumps(row):
                                raise RuntimeError("secret_in_report")
                            report["results"].append(row)
                            print(json.dumps({"trial": trial, "case": case, "indexed": indexed, "clients": clients,
                                              "repetition": repetition, "confirmed_rps": round(row["confirmed_ops_per_second"], 2),
                                              "valid": row["valid"]}), flush=True)
        report["environment"]["cpu_load_after"] = list(os.getloadavg()) if hasattr(os, "getloadavg") else None
    if fingerprint != source_hash():
        raise RuntimeError("source_changed_during_measurement")
    report["valid"] = all(row["valid"] for row in report["results"])
    report["summary"] = paired_summary(report["results"]) if report["valid"] else None
    return report


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=load.ROOT / "target/release/kiarni")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, default=Path(tempfile.gettempdir()))
    parser.add_argument("--clients", type=int, nargs="+", default=[1, 8])
    parser.add_argument("--cases", nargs="+", choices=CASES, default=CASES)
    parser.add_argument("--documents", type=int, default=4096)
    parser.add_argument("--document-bytes", type=int, default=1024)
    parser.add_argument("--layout", choices=["contiguous", "spread"], default="spread")
    parser.add_argument("--seconds", type=float, default=2)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--warmup", type=int, default=5)
    args = parser.parse_args()
    if (len(set(args.clients)) != len(args.clients) or any(not 1 <= n <= 32 for n in args.clients)
        or len(set(args.cases)) != len(args.cases) or not 128 <= args.documents <= 9900
        or not 128 <= args.document_bytes <= 1048576 or (args.documents + max(args.clients)) * args.document_bytes > 8 * 1024 * 1024
        or not 0.1 <= args.seconds <= 30 or not 1 <= args.repeat <= 5 or not 0 <= args.warmup <= 100):
        parser.error("invalid bounded matrix")
    if args.output.exists() or not args.output.parent.is_dir() or not args.work_dir.is_dir():
        parser.error("new output in existing parent and existing work directory required")
    return args


if __name__ == "__main__":
    args = arguments()
    report = benchmark(args)
    with os.fdopen(os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    raise SystemExit(0 if report["valid"] else 1)
