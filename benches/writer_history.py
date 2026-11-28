"""Interleaved read comparison of the two exact archived writer benchmark binaries."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket

import http_load as load
import bench_affinity as affinity
from telemetry_http import mode_order, tcp_snapshot

RECORDS = {"before": load.ROOT / "docs/benchmarks/http-before-writer-2026-11-13.json",
           "writer": load.ROOT / "docs/benchmarks/http-writer-2026-11-13.json"}


def comparisons(rows):
    groups = {}
    for row in rows:
        group = groups.setdefault((row["clients"], row["repetition"]), {})
        if not row["valid"] or row["mode_id"] in group:
            raise ValueError("invalid_or_duplicate_member")
        group[row["mode_id"]] = row
    result = []
    for (clients, repetition), group in groups.items():
        if set(group) != set(range(4)):
            raise ValueError("incomplete_quartet")
        for name, left, right in [("writer_over_before_1", 0, 1), ("writer_over_before_2", 2, 3),
                                  ("before_AA", 0, 2), ("writer_AA", 1, 3)]:
            result.append({"clients": clients, "repetition": repetition, "comparison": name,
                "rps_right_over_left": group[right]["confirmed_ops_per_second"] / group[left]["confirmed_ops_per_second"],
                "p95_right_over_left": group[right]["success_latency"]["p95_ms"] / group[left]["success_latency"]["p95_ms"]})
    return result


def source_hash():
    digest = hashlib.sha256(load.fingerprint().encode())
    for path in [Path(__file__), load.ROOT / "benches/telemetry_http.py", *RECORDS.values()]:
        digest.update(path.read_bytes())
    return digest.hexdigest()


def run(args):
    fingerprint = source_hash()
    binaries = {"before": args.before.resolve(), "writer": args.writer.resolve()}
    artifacts = {}
    for role, binary in binaries.items():
        record = json.loads(RECORDS[role].read_bytes())
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
        if digest != record["environment"]["binary_sha256"]:
            raise ValueError("historical_binary_hash_mismatch_" + role)
        artifacts[role] = {"sha256": digest, "bytes": binary.stat().st_size,
                           "historical_record": str(RECORDS[role].relative_to(load.ROOT)),
                           "historical_source_sha256": record["environment"]["source_sha256"]}
    report = {"schema": 1, "environment": load.environment(binaries["writer"]),
              "measurement_source_sha256": fingerprint, "artifacts": artifacts,
              "parameters": {"clients": args.clients, "repeat": args.repeat, "seconds": args.seconds,
                  "warmup": args.warmup, "documents": 256, "document_bytes": 1024,
                  "server_cpus": args.server_cpus, "client_cpus": args.client_cpus,
                  "server_config": load.SERVER_CONFIG,
                  "validation": "full JSON on every read, warmup and after each row",
                  "load_model": "closed_loop; process/client; new TCP/request; read-only",
                  "listener": "same loopback port across fresh installations",
                  "order": "Williams 4 modes: before0,writer1,before2,writer3",
                  "logs": "original CLI options; no log-level override added to historical binaries"},
              "results": []}
    journal = args.output.with_suffix(".progress.jsonl")
    with os.fdopen(os.open(journal, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as progress:
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        for repetition in range(1, args.repeat + 1):
            for ci, clients in enumerate(args.clients):
                for mode in mode_order(repetition, ci):
                    role = "before" if mode % 2 == 0 else "writer"
                    before = tcp_snapshot()
                    machine_before = affinity.snapshot()
                    measured = load.benchmark(argparse.Namespace(
                        binary=binaries[role], output=args.output, clients=[clients], scenarios=["read"],
                        repeat=1, seconds=args.seconds, warmup=args.warmup, documents=256,
                        document_bytes=1024, port=port, verify_full_read=True,
                        server_cpus=args.server_cpus, client_cpus=args.client_cpus))
                    row, = measured["results"]
                    row.update(mode_id=mode, role=role, repetition=repetition,
                        kernel_tcp_before=before, kernel_tcp_after=tcp_snapshot(),
                        machine_before=machine_before, machine_after=affinity.snapshot(),
                        server_affinity_observed=measured["environment"]["server_affinity_observed"],
                        data_filesystem=measured["environment"]["data_filesystem"],
                        data_block_bytes=measured["environment"]["data_block_bytes"],
                        dataset_sha256=measured["parameters"]["initial_dataset_sha256"],
                        post_load_dataset_verified=measured["post_load_dataset_verified"],
                        binary_sha256=measured["environment"]["binary_sha256"])
                    row["valid"] = (measured["valid"] and row["confirmed"] == row["attempts"]
                                    and set(row["statuses"]) == {"200"}
                                    and row["post_load_dataset_verified"]
                                    and row["binary_sha256"] == artifacts[role]["sha256"])
                    if args.client_cpus is not None:
                        row["valid"] &= row["client_affinities"] == [sorted(args.client_cpus)]
                    report["results"].append(row)
                    progress.write(json.dumps(row) + "\n")
                    progress.flush()
                    print(json.dumps({"trial": len(report["results"]), "clients": clients,
                        "repetition": repetition, "role": role, "mode_id": mode,
                        "confirmed_rps": row["confirmed_ops_per_second"], "valid": row["valid"]}), flush=True)
    if source_hash() != fingerprint:
        raise RuntimeError("measurement_source_changed")
    if any(hashlib.sha256(binary.read_bytes()).hexdigest() != artifacts[role]["sha256"]
           for role, binary in binaries.items()):
        raise RuntimeError("binary_changed")
    report["valid"] = bool(report["results"]) and all(row["valid"] for row in report["results"])
    report["comparisons"] = comparisons(report["results"]) if report["valid"] else None
    return report


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--writer", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--clients", type=int, nargs="+", default=[1, 8, 32])
    parser.add_argument("--server-cpus", type=int, nargs="+")
    parser.add_argument("--client-cpus", type=int, nargs="+")
    parser.add_argument("--repeat", type=int, default=4)
    parser.add_argument("--seconds", type=float, default=3)
    parser.add_argument("--warmup", type=int, default=5)
    args = parser.parse_args()
    try:
        affinity.validate(args.server_cpus, args.client_cpus)
    except ValueError as error:
        parser.error(str(error))
    if (len(set(args.clients)) != len(args.clients) or any(not 1 <= c <= 32 for c in args.clients)
        or not 1 <= args.repeat <= 8 or not .1 <= args.seconds <= 10 or not 0 <= args.warmup <= 100
        or args.output.exists() or not args.output.parent.is_dir()):
        parser.error("invalid bounded comparison or output path")
    return args


if __name__ == "__main__":
    options = arguments()
    report = run(options)
    with os.fdopen(os.open(options.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    raise SystemExit(0 if report["valid"] else 1)
