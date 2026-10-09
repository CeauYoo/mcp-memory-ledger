"""Pure offline regression tests for evaluation definitions, not retrieval outcomes."""
import importlib.util
from pathlib import Path
import sys
import unittest
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("evaluation", ROOT / "scripts/evaluate-memory-loop.py")
evaluation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evaluation)


class MetricsTests(unittest.TestCase):
    def test_recall_denominator_rank_and_stale_use(self):
        task = {"id": "test", "language": "en", "stratum": "literal", "namespace": "project/a", "subject": "x", "before": "old", "after": "new"}
        records = [{"record": {"id": "noise", "namespace": "project/a", "provenance": {"ref": 1}}},
                   {"record": {"id": "old", "namespace": "project/a", "subject": "x", "object": "old", "provenance": {"ref": 1}}},
                   {"record": {"id": "new", "namespace": "project/b", "subject": "x", "object": "new"}}]
        row = evaluation.metrics(task, records, ["old", "new"], {"old"}, 12.5, 123, 2)
        self.assertEqual(row["recall_at_k"], .5)
        self.assertEqual(row["reciprocal_rank"], .5)
        self.assertEqual(row["stale_claim_exposure_count"], 1)
        self.assertTrue(row["stale_conclusion_used_proxy"])
        self.assertFalse(row["exact_answer_proxy"])
        self.assertEqual(row["scope_leak_count"], 1)
        self.assertEqual(row["provenance_missing_count"], 1)
        self.assertIsNone(row["token_cost"])
        self.assertIsNone(row["model_task_success"])

    def test_fixture_ids_and_fixed_strata(self):
        import json
        fixture = json.loads((Path(__file__).parent / "tasks.json").read_text())
        tasks = fixture["tasks"]
        self.assertEqual(len(tasks), 10)
        self.assertEqual(len({t["id"] for t in tasks}), 10)
        self.assertEqual({t["language"] for t in tasks}, {"en", "zh"})
        self.assertEqual(sum(t["stratum"] == "unsupported_paraphrase" for t in tasks), 2)
        matrix = json.loads((Path(__file__).parent / "query-scenarios-v1.json").read_text())["cases"]
        self.assertEqual(len(matrix), 50)
        for task in tasks:
            cases = [c for c in matrix if c["task_id"] == task["id"]]
            self.assertEqual(len({c["query"] for c in cases}), 5)


if __name__ == "__main__":
    unittest.main()
