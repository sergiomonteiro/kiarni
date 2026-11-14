import json
import unittest

from http_load import document, percentiles, summarize


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


if __name__ == "__main__":
    unittest.main()
