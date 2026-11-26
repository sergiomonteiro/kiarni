import json
import unittest

from http_load import document, percentiles, summarize
import index_http
import telemetry_http


class MeasurementTests(unittest.TestCase):
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
