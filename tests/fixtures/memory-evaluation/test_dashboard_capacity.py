"""Offline dashboard contract and deterministic SQLite resource-lifetime checks."""
from contextlib import closing, contextmanager
import importlib.util
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import MagicMock, patch
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("capacity", ROOT / "scripts/benchmark-memory-capacity.py")
capacity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capacity)


class TrackedConnection(sqlite3.Connection):
    """Keep handles reachable so garbage collection cannot hide a missing close."""
    closed = False
    fail_statement = None
    in_transaction_on_close = None

    def execute(self, statement, *args, **kwargs):
        result = super().execute(statement, *args, **kwargs)
        if self.fail_statement and statement.startswith(self.fail_statement):
            raise RuntimeError("injected SQLite statement failure")
        return result

    def close(self):
        self.in_transaction_on_close = self.in_transaction
        super().close()
        self.closed = True


@contextmanager
def tracked_connections(fail_at=None, fail_statement=None):
    connect = sqlite3.connect
    connections = []

    def tracked_connect(*args, **kwargs):
        connection = connect(*args, **kwargs, factory=TrackedConnection)
        if len(connections) == fail_at:
            connection.fail_statement = fail_statement
        connections.append(connection)
        return connection

    with patch.object(capacity.sqlite3, "connect", side_effect=tracked_connect):
        yield connections


class DashboardCapacityTests(unittest.TestCase):
    def assert_connections_closed(self, connections, expected_count):
        self.assertEqual(len(connections), expected_count)
        for connection in connections:
            self.assertTrue(connection.closed, "SQLite handle must close before returning")
            self.assertFalse(connection.in_transaction_on_close,
                             "transaction context must finish before closing the handle")

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
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "probe.sqlite"
            with closing(sqlite3.connect(database)) as connection, connection:
                connection.execute("CREATE TABLE facts(value TEXT)")
                connection.execute("INSERT INTO facts VALUES ('preserved')")
            before = database.read_bytes()
            with tracked_connections() as connections:
                result = capacity.measure_reservation_acquisition(database, .01)
            self.assert_connections_closed(connections, 2)
            self.assertEqual(result["result"], "acquired_then_rolled_back")
            self.assertGreaterEqual(result["acquisition_elapsed_ms"], 0)
            self.assertGreaterEqual(result["held_after_contender_ready_ms"], 0)
            self.assertEqual(database.read_bytes(), before)

    def test_reservation_probe_closes_and_rolls_back_on_statement_failure(self):
        for failing_connection in (0, 1):
            with self.subTest(failing_connection=failing_connection):
                with tempfile.TemporaryDirectory() as directory:
                    database = Path(directory) / "probe.sqlite"
                    with closing(sqlite3.connect(database)) as connection, connection:
                        connection.execute("CREATE TABLE facts(value TEXT)")
                        connection.execute("INSERT INTO facts VALUES ('preserved')")
                    before = database.read_bytes()
                    with tracked_connections(failing_connection, "BEGIN IMMEDIATE") as connections:
                        with self.assertRaisesRegex(RuntimeError, "injected SQLite statement failure"):
                            capacity.measure_reservation_acquisition(database, .001)
                    self.assert_connections_closed(connections, failing_connection + 1)
                    self.assertEqual(database.read_bytes(), before)

    def test_reservation_probe_releases_holder_when_hold_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "probe.sqlite"
            with tracked_connections() as connections:
                with patch.object(capacity.time, "sleep", side_effect=RuntimeError("injected hold failure")):
                    with self.assertRaisesRegex(RuntimeError, "injected hold failure"):
                        capacity.measure_reservation_acquisition(database, .001)
            self.assert_connections_closed(connections, 2)

    def test_seed_commits_or_rolls_back_before_closing(self):
        for fail_statement in (None, "INSERT INTO evidence_links"):
            with self.subTest(fail_statement=fail_statement):
                with tempfile.TemporaryDirectory() as directory:
                    database = Path(directory) / "seed.sqlite"
                    with closing(sqlite3.connect(database)) as connection, connection:
                        connection.executescript("""
                            CREATE TABLE events(event_id, recorded_at, owner, namespace, kind, summary);
                            CREATE TABLE claims(claim_id, owner, namespace, subject, predicate, object, mode, status);
                            CREATE TABLE evidence_links(claim_id, event_id);
                            CREATE TABLE episode_events(episode_reference, event_id);
                        """)
                    with tracked_connections(0, fail_statement) as connections:
                        if fail_statement:
                            with self.assertRaisesRegex(RuntimeError, "injected SQLite statement failure"):
                                capacity.seed(database, 2)
                        else:
                            capacity.seed(database, 2)
                    self.assert_connections_closed(connections, 1)
                    with closing(sqlite3.connect(database)) as connection, connection:
                        for table in ("events", "claims", "evidence_links", "episode_events"):
                            self.assertEqual(connection.execute(f"SELECT count(*) FROM {table}").fetchone()[0],
                                             0 if fail_statement else 1)

    def test_query_plans_close_on_success_and_failure(self):
        for fail_statement in (None, "EXPLAIN QUERY PLAN"):
            with self.subTest(fail_statement=fail_statement):
                with tempfile.TemporaryDirectory() as directory:
                    database = Path(directory) / "plans.sqlite"
                    with closing(sqlite3.connect(database)) as connection, connection:
                        connection.execute("CREATE TABLE claims(claim_id, owner, namespace, status, subject)")
                    with tracked_connections(0, fail_statement) as connections:
                        if fail_statement:
                            with self.assertRaisesRegex(RuntimeError, "injected SQLite statement failure"):
                                capacity.plans(database, "needle")
                        else:
                            self.assertEqual(len(capacity.plans(database, "needle")), 1)
                    self.assert_connections_closed(connections, 1)


if __name__ == "__main__":
    unittest.main()
