"""Pure offline checks for the opt-in loopback dashboard measurement contract."""
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import MagicMock, patch
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("capacity", ROOT / "scripts/benchmark-memory-capacity.py")
capacity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capacity)


class DashboardCapacityTests(unittest.TestCase):
    def test_config_is_mock_required_loopback_without_browser(self):
        config = capacity.dashboard_config(Path("/tmp/synthetic.sqlite"), 43210)
        for expected in ['provider = "mock"', 'host = "127.0.0.1"', 'port = 43210',
                         'required = true', 'open_browser = false', 'sse_enabled = false']:
            self.assertIn(expected, config)
        for port in [0, -1, 65536, "43210"]:
            with self.assertRaises(ValueError):
                capacity.dashboard_config(Path("synthetic.sqlite"), port)

    def test_unlisted_endpoints_are_rejected_before_connecting(self):
        with patch.object(capacity.http.client, "HTTPConnection") as connection:
            for path in ["https://example.com/", "/api/events/stream", "/api/health"]:
                with self.assertRaises(ValueError):
                    capacity.dashboard_get(43210, path)
            connection.assert_not_called()

    def test_get_is_direct_loopback_and_records_exact_bytes(self):
        value = {"runtime": {"provider": "mock", "read_only": True}}
        raw = json.dumps(value).encode("utf-8")
        response = MagicMock(status=200)
        response.read.return_value = raw
        with patch.object(capacity.http.client, "HTTPConnection") as constructor:
            connection = constructor.return_value
            connection.getresponse.return_value = response
            row, actual = capacity.dashboard_get(43210, "/api/summary")
            constructor.assert_called_once_with("127.0.0.1", 43210, timeout=30)
            connection.request.assert_called_once_with("GET", "/api/summary")
            connection.close.assert_called_once()
            self.assertEqual(actual, value)
            self.assertEqual(row["response_bytes"], len(raw))
            self.assertEqual(row["status"], 200)

    def test_redirect_is_error_without_following(self):
        response = MagicMock(status=302)
        response.read.return_value = b""
        with patch.object(capacity.http.client, "HTTPConnection") as constructor:
            constructor.return_value.getresponse.return_value = response
            with self.assertRaises(RuntimeError):
                capacity.dashboard_get(43210, "/api/summary")
            constructor.return_value.request.assert_called_once()
            constructor.return_value.close.assert_called_once()

    def test_foreign_scope_fails_measurement(self):
        response = MagicMock(status=200)
        response.read.return_value = b'[{"namespace":"project/foreign","read_only":true}]'
        with patch.object(capacity.http.client, "HTTPConnection") as constructor:
            constructor.return_value.getresponse.return_value = response
            with self.assertRaises(AssertionError):
                capacity.dashboard_get(43210, capacity.DASHBOARD_HTTP_PATHS[2])

    def test_reservation_probe_rolls_back_without_changing_source(self):
        import sqlite3
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "probe.sqlite"
            with sqlite3.connect(database) as connection:
                connection.execute("CREATE TABLE facts(value TEXT)")
                connection.execute("INSERT INTO facts VALUES ('preserved')")
            before = database.read_bytes()
            result = capacity.measure_reservation_acquisition(database, .01)
            self.assertEqual(result["result"], "acquired_then_rolled_back")
            self.assertGreaterEqual(result["acquisition_elapsed_ms"], 0)
            self.assertGreaterEqual(result["held_after_contender_ready_ms"], 0)
            self.assertEqual(database.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
