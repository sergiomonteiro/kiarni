import json
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import Mock, patch
from types import SimpleNamespace

from http_load import document, percentiles, summarize
import http_load
import index_http
import telemetry_http
import writer_history
import bench_affinity


class MeasurementTests(unittest.TestCase):
    def test_rust_driver_receives_secret_only_via_stdin_and_records_whole_process_cpu(self):
        context = {"scenario":"read", "expected_documents":[{"_id":"d000000"}],
                   "address":("127.0.0.1",1234), "token":"fixture-private-token",
                   "clients":1, "warmup":5, "seconds":1, "repetition":1}
        output = {"clients":1,"attempts":1,"confirmed":1,"statuses":{"200":1},
                  "client_affinity":None,"valid":True}
        with patch.object(http_load.subprocess,"run",return_value=SimpleNamespace(returncode=0,stdout=json.dumps(output).encode())) as run, \
             patch.object(http_load.resource,"getrusage",side_effect=[SimpleNamespace(ru_utime=1,ru_stime=1),SimpleNamespace(ru_utime=2,ru_stime=1.5)]):
            row = http_load.rust_round(context, Path("fixture-driver"))
        self.assertEqual(row["client_cpu_seconds"],1.5)
        self.assertNotIn(context["token"], str(run.call_args.args))
        self.assertEqual(json.loads(run.call_args.kwargs["input"])["token"],context["token"])
        self.assertNotIn(context["token"],json.dumps(row))

    def test_aa_campaign_labels_do_not_claim_two_different_binaries(self):
        rows=[dict(clients=8,repetition=1,mode_id=i,valid=True,
                   role="writer",binary_sha256="fixture",confirmed_ops_per_second=100+i,
                   success_latency={"p95_ms":1}) for i in range(4)]
        pairs=writer_history.comparisons(rows,aa_only=True)
        self.assertEqual([r["comparison"] for r in pairs],["same_binary_AA_01","same_binary_AA_23"])
        for key,value in [("role","before"),("binary_sha256","different")]:
            with self.assertRaises(ValueError):
                writer_history.comparisons([dict(rows[0],**{key:value}),*rows[1:]],aa_only=True)

    def test_affinity_requires_two_available_disjoint_masks(self):
        bench_affinity.validate(None, None, allowed={0, 1, 2, 3})
        bench_affinity.validate([0, 1], [2, 3], allowed={0, 1, 2, 3})
        for server, clients in [([0], None), ([], [1]), ([0], [0]), ([4], [1]), ([0, 0], [1])]:
            with self.assertRaises(ValueError):
                bench_affinity.validate(server, clients, allowed={0, 1, 2, 3})

    @unittest.skipUnless(hasattr(os, "sched_setaffinity"), "Linux affinity only")
    def test_affinity_is_inherited_through_exec_and_threads_without_pinning_parent(self):
        initial = os.sched_getaffinity(0)
        cpu = min(initial)
        script = ("import json,os,threading; a=[sorted(os.sched_getaffinity(0))]; "
                  "t=threading.Thread(target=lambda:a.append(sorted(os.sched_getaffinity(0)))); "
                  "t.start(); t.join(); print(json.dumps(a))")
        result = subprocess.check_output([sys.executable, str(Path(bench_affinity.__file__)),
                                          str(cpu), sys.executable, "-c", script], text=True)
        self.assertEqual(json.loads(result), [[cpu], [cpu]])
        self.assertEqual(os.sched_getaffinity(0), initial)

    def test_full_read_validation_rejects_wrong_payload_with_matching_id(self):
        expected = json.loads(document(0, 0, 128))
        context = {"scenario": "read", "documents": 1, "round": 1,
                   "expected_documents": [expected], "address": None, "token": "fixture"}
        with patch.object(http_load, "CONTEXT", context), \
             patch.object(http_load, "exchange", return_value=(200, None, dict(expected, revision=1))):
            result = http_load.operation(0, 0)
        self.assertEqual(result[2], "invalid_success_response")

    def test_historical_pairs_preserve_direction_and_reject_incomplete_data(self):
        rows = [dict(clients=8, repetition=1, mode_id=i, valid=True,
                     confirmed_ops_per_second=rps, success_latency={"p95_ms": p95})
                for i, rps, p95 in [(0, 100, 1), (1, 95, 2), (2, 102, 1), (3, 96, 2)]]
        comparisons = writer_history.comparisons(rows)
        self.assertEqual(comparisons[0]["rps_right_over_left"], .95)
        self.assertEqual(comparisons[2]["rps_right_over_left"], 1.02)
        self.assertEqual(comparisons[0]["p95_right_over_left"], 2)
        for invalid in (rows[:-1], rows + [rows[0]], [dict(rows[0], valid=False), *rows[1:]]):
            with self.assertRaises(ValueError):
                writer_history.comparisons(invalid)

    def test_exact_document_size_and_revision(self):
        for size in [128, 1024]:
            payload = document(9999, 360000000005, size)
            self.assertEqual(len(payload), size)
            value = json.loads(payload)
            self.assertEqual(value["_id"], "d009999")
            self.assertEqual(value["revision"], 360000000005)

    def test_nearest_rank_percentiles_and_no_success_sample(self):
        self.assertIsNone(percentiles([]))
        self.assertEqual(
            percentiles([number * 1000000 for number in range(1, 101)]),
            {"p50_ms": 50.0, "p95_ms": 95.0, "p99_ms": 99.0, "max_ms": 100.0},
        )

    def test_rejections_are_not_confirmed_throughput_or_success_latency(self):
        result = summarize(
            {"clients": 1, "scenario": "write", "repetition": 1},
            [{
                "started_ns": 1000000000, "ended_ns": 2000000000,
                "cpu_seconds": 0.01,
                "counts": {"200": 1, "503": 1, "transport_error": 1},
                "codes": {"write_busy": 1, "ConnectionResetError": 1},
                "operations": {"write": {"200": 1, "503": 1, "transport_error": 1}},
                "success_ns": [20000000], "failed_ns": [1000000, 2000000],
                "last_ack": None, "client": 0,
                "uncertain_writes": 1, "operation_cap_hit": False,
            }],
        )
        self.assertEqual(result["attempts"], 3)
        self.assertEqual(result["confirmed_ops_per_second"], 1)
        self.assertEqual(result["attempted_ops_per_second"], 3)
        self.assertEqual(result["success_latency"]["p50_ms"], 20)
        self.assertEqual(result["uncertain_write_outcomes"], 1)


class IndexedMeasurementTests(unittest.TestCase):
    def test_distribution_thresholds_and_exact_sizes(self):
        for count, size in [(128, 128), (1024, 256), (4096, 1024)]:
            counts = dict(index_http.distribution(count))
            self.assertEqual(sum(counts.values()), count)
            self.assertEqual(counts["edge"], count // 8)
            self.assertEqual(counts["over"], count // 8 + 1)
            for layout in ["contiguous", "spread"]:
                actual = {}
                for i in range(count):
                    tag = index_http.tag_for(i, count, layout)
                    actual[tag] = actual.get(tag, 0) + 1
                self.assertEqual(actual, counts)
            for tag in [None, "changed0", "changed1", "churn"]:
                body = index_http.document(0, 123456789, count, size, tag, tag == "churn")
                self.assertEqual(len(body), size)
                self.assertEqual(json.loads(body)["revision"], 123456789)
        with self.assertRaises(ValueError):
            index_http.tag_for(-1, 128)
        for count in [128, 194, 1024]:
            self.assertEqual(len({i * index_http.spread_stride(count) % count for i in range(count)}), count)

    def test_read_fixture_and_expected_plan(self):
        ast, expected = index_http.query_case("edge", 128)
        self.assertEqual(expected["count"], 10)
        scores = [value["score"] for value in expected["documents"]]
        self.assertEqual(scores, sorted(scores, reverse=True))
        self.assertEqual(ast["filter"]["value"], "edge")
        self.assertEqual(index_http.expected_access("edge", True), "secondary")
        for case in ["over", "dense", "compound"]:
            self.assertEqual(index_http.expected_access(case, True), "scan")
        self.assertEqual(index_http.expected_access("id", True), "id")
        self.assertEqual(index_http.query_case("absent", 128)[1]["count"], 0)

    def test_failures_busy_and_unknown_writes_are_not_confirmations(self):
        self.assertEqual(index_http.classification("put", 200, None, True), (True, False, False))
        self.assertEqual(index_http.classification("insert", 201, None, True), (True, False, False))
        self.assertEqual(index_http.classification("delete", 204, None, True), (True, False, False))
        self.assertEqual(index_http.classification("put", 503, "write_busy", False), (False, True, False))
        self.assertEqual(index_http.classification("query", 503, "service_unavailable", False), (False, True, False))
        for status, code, correct in [(0, "timeout", False), (503, "storage_unavailable", False), (200, None, False)]:
            self.assertEqual(index_http.classification("put", status, code, correct), (False, False, True))

    def test_summary_requires_complete_valid_pairs_and_uses_paired_ratios(self):
        self.assertEqual(index_http.pair_order(1, 0, 0), list(reversed(index_http.pair_order(2, 0, 0))))
        def row(repetition, indexed, rps, p50):
            return {"documents": 128, "document_bytes": 128, "clients": 1, "case": "needle",
                    "repetition": repetition, "indexed": indexed, "confirmed_ops_per_second": rps,
                    "success_latency": {"p50_ms": p50, "p95_ms": p50}, "confirmed": 100, "valid": True}
        rows = [row(1, False, 100, 10), row(1, True, 200, 5),
                row(2, True, 30, 50), row(2, False, 10, 100)]
        summary = index_http.paired_summary(rows)[0]
        self.assertEqual(summary["median_throughput_index_over_plain"], 2.5)
        self.assertEqual(summary["median_p50_index_over_plain"], 0.5)
        for invalid in [rows[:-1], rows + [rows[0]], [dict(rows[0], valid=False), rows[1]]]:
            with self.assertRaises(ValueError):
                index_http.paired_summary(invalid)


class TelemetryMeasurementTests(unittest.TestCase):
    def test_reader_timeout_is_reported_without_retry_or_lost_result(self):
        server = Mock()
        recent = b'{"capacity":8,"errors":[],"dropped_total":"0"}'
        server.request.side_effect = [
            (200, b"kiarni_http_requests_total{ kiarni_storage_"), (200, recent),
            TimeoutError("fixture"), (200, recent),
        ]
        ctx = {"address": ("127.0.0.1", 1), "token": "fixture", "seconds": .1, "reader_hz": 2}
        with patch.object(telemetry_http, "Server", return_value=server), \
             patch.object(telemetry_http, "CONTEXT", ctx), \
             patch.object(telemetry_http, "BARRIER", Mock()):
            result = telemetry_http.reader(0)
        self.assertFalse(result["valid"])
        self.assertEqual(result["errors"][0]["error"], "TimeoutError")
        self.assertEqual(server.request.call_count, 4)

    def test_post_load_failure_is_not_retried_or_hidden(self):
        server = Mock()
        payload = document(0, 0, 128)
        server.request.return_value = (200, payload)
        self.assertEqual(telemetry_http.verify_dataset(server, "fixture", [payload]), (True, None))
        server.reset_mock()
        server.request.side_effect = ConnectionResetError(104, "fixture")
        valid, error = telemetry_http.verify_dataset(server, "fixture", [payload])
        self.assertFalse(valid)
        self.assertEqual(error, {"index": 0, "error": "ConnectionResetError", "errno": 104})
        server.request.assert_called_once()

    def test_expected_errors_are_distinct_from_success_and_transport_failure(self):
        response = {"error": {"code": "document_not_found"}, "request_id": "req_fixture"}
        self.assertTrue(telemetry_http.acceptable("missing", 0, 404, "document_not_found", response))
        for status, code, body in [(0, "TimeoutError", None), (503, "service_unavailable", {}),
                                   (404, "collection_not_found", response), (200, None, {})]:
            self.assertFalse(telemetry_http.acceptable("missing", 0, status, code, body))
        outcome = {"started_ns": 1_000_000_000, "ended_ns": 2_000_000_000,
                   "statuses": {"404": 1, "transport_error": 1},
                   "codes": {"document_not_found": 1, "TimeoutError": 1},
                   "good": [2_000_000], "bad": [5_000_000], "cap_hit": False}
        row = telemetry_http.aggregate([outcome])
        self.assertEqual(row["expected_rps"], 1)
        self.assertEqual(row["expected_latency"]["p95_ms"], 2)
        self.assertFalse(row["valid"])

    def test_balanced_modes_and_complete_comparisons(self):
        orders = [telemetry_http.mode_order(r, 0) for r in range(1, 5)]
        for position in range(4):
            self.assertEqual({order[position] for order in orders}, set(range(4)))
        edges = [(a, b) for order in orders for a, b in zip(order, order[1:])]
        self.assertEqual(len(set(edges)), 12)
        rows = [dict(clients=1, scenario="read", repetition=1, log_level=level,
                     readers=readers, expected_rps=100 * (i + 1), valid=True)
                for i, (level, readers) in enumerate(telemetry_http.MODES)]
        ratios = telemetry_http.comparisons(rows)
        self.assertEqual(ratios[0]["rps_right_over_left"], 2)
        for invalid in (rows[:-1], rows + [rows[0]], [dict(rows[0], valid=False), *rows[1:]]):
            with self.assertRaises(ValueError):
                telemetry_http.comparisons(invalid)

    def test_recent_history_shape_and_limit(self):
        self.assertTrue(telemetry_http.recent_valid({"capacity": 8, "errors": [], "dropped_total": "10"}))
        for value in ({}, {"capacity": 8, "errors": [{}] * 9, "dropped_total": "0"},
                      {"capacity": 8, "errors": [], "dropped_total": "-1"}):
            self.assertFalse(telemetry_http.recent_valid(value))


if __name__ == "__main__":
    unittest.main()
