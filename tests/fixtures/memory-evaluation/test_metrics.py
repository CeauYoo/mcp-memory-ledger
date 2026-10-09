"""Pure offline regression tests for evaluation definitions, not retrieval outcomes."""
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("evaluation", ROOT / "scripts/evaluate-memory-loop.py")
evaluation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evaluation)


def load_fixture(name):
    return json.loads((Path(__file__).parent / name).read_text(encoding="utf-8"))


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
        fixture = load_fixture("tasks.json")
        tasks = fixture["tasks"]
        self.assertEqual(len(tasks), 10)
        self.assertEqual(len({t["id"] for t in tasks}), 10)
        self.assertEqual({t["language"] for t in tasks}, {"en", "zh"})
        self.assertEqual(sum(t["stratum"] == "unsupported_paraphrase" for t in tasks), 2)
        matrix = load_fixture("query-scenarios-v1.json")["cases"]
        self.assertEqual(len(matrix), 50)
        for task in tasks:
            cases = [c for c in matrix if c["task_id"] == task["id"]]
            self.assertEqual(len({c["query"] for c in cases}), 5)

    def test_multilingual_fixtures_with_non_utf8_default_encoding(self):
        # Simulate Windows' legacy default without relying on installed locales
        # or Python's UTF-8 mode. Explicit encodings must still pass through.
        def default_encoding(encoding, *args):
            return "cp1252" if encoding is None else encoding

        with patch("io.text_encoding", side_effect=default_encoding):
            # Prove this simulation would reproduce the original failure.
            with self.assertRaises(UnicodeDecodeError):
                (Path(__file__).parent / "tasks.json").read_text()
            for name in ("tasks.json", "query-scenarios-v1.json"):
                with self.subTest(fixture=name):
                    expected = json.loads((Path(__file__).parent / name).read_bytes())
                    self.assertEqual(load_fixture(name), expected)


if __name__ == "__main__":
    unittest.main()
