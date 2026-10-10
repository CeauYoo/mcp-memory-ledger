#!/usr/bin/env python3
"""Fail-closed PowerShell wrapper contract tests, using only Python's stdlib.

Run on Windows with PowerShell 7 and the repository's pinned Rust toolchain.
--allow-non-windows permits supplemental PowerShell testing on other hosts; it
never labels that run as Windows runtime evidence. No provider calls are made.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest


ROOT = Path(__file__).resolve().parent.parent
WRAPPER = ROOT / "scripts" / "agent-llm-mm.ps1"
PWSH = shutil.which("pwsh")
ENV = {key: value for key, value in os.environ.items()
       if not key.upper().startswith("AGENT_LLM_MM_")}
ENV.update(CARGO_TERM_COLOR="never", RUST_LOG="agent_llm_mm=debug")


def ps_quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def sqlite_url(path):
    return "sqlite://" + path.as_posix()


def spawn(command, **kwargs):
    return subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True, encoding="utf-8",
                            start_new_session=os.name != "nt",
                            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0,
                            **kwargs)


def terminate_tree(process):
    if process.poll() is not None:
        return
    if os.name == "nt":
        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                       capture_output=True, check=False, timeout=15)
    else:
        os.killpg(process.pid, signal.SIGKILL)
    process.wait(timeout=15)


def run(command, *, cwd, env=ENV, timeout=120):
    process = spawn(command, cwd=cwd, env=env)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        terminate_tree(process)
        process.communicate(timeout=15)
        raise RuntimeError(f"command timed out after {timeout}s: {command}") from None
    return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


class WrapperTests(unittest.TestCase):
    def setUp(self):
        # Inside target so a repository-relative path can be tested without
        # touching any real local configuration or default user database.
        (ROOT / "target").mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="windows-wrapper-", dir=ROOT / "target")
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        self.caller = self.work / "caller directory"
        self.caller.mkdir()
        self.fixture = self.work / "fixture repo [literal]"
        (self.fixture / "scripts").mkdir(parents=True)
        (self.fixture / "examples").mkdir()
        self.fixture_wrapper = self.fixture / "scripts" / WRAPPER.name
        shutil.copy2(WRAPPER, self.fixture_wrapper)
        self.example = (ROOT / "examples" / "agent-llm-mm.dev.example.toml").read_bytes()
        (self.fixture / "examples" / "agent-llm-mm.dev.example.toml").write_bytes(self.example)

    def invoke(self, *args, fixture=False, env=None):
        return run([PWSH, "-NoProfile", "-NonInteractive", "-File",
                    str(self.fixture_wrapper if fixture else WRAPPER), *map(str, args)],
                   cwd=self.caller, env=ENV if env is None else env)

    def success(self, result):
        self.assertEqual(result.returncode, 0,
                         f"command={result.args}\nstdout={result.stdout}\nstderr={result.stderr}")
        return result

    def report(self, *args, **kwargs):
        # json.loads rejects any wrapper, Cargo, or tracing chatter on stdout.
        return json.loads(self.success(self.invoke(*args, **kwargs)).stdout)

    def config(self, name="config.toml", database=None):
        path = self.work / name
        database = database or self.work / (path.stem + ".sqlite")
        path.write_text('transport = "stdio"\ndatabase_url = ' + json.dumps(sqlite_url(database)) +
                        '\n[model]\nprovider = "mock"\n[dashboard]\nenabled = false\n'
                        '[daemon]\nenabled = false\n', encoding="utf-8")
        return path, database

    def test_bootstrap_default_from_external_directory(self):
        result = self.success(self.invoke("bootstrap-local", fixture=True))
        target = self.fixture / "agent-llm-mm.local.toml"
        self.assertEqual(target.read_bytes(), self.example)
        self.assertFalse((self.caller / target.name).exists())
        self.assertIn("Next commands:", result.stdout)
        self.assertIn("doctor --read-only", result.stdout)
        self.assertEqual(sorted(path.name for path in self.fixture.iterdir()),
                         ["agent-llm-mm.local.toml", "examples", "scripts"])

    def test_bootstrap_relative_literal_path_and_no_overwrite(self):
        relative = Path("config [literal] 'quoted' 中文.toml")
        result = self.success(self.invoke("bootstrap-local", relative, fixture=True))
        target = self.fixture / relative
        self.assertEqual(target.read_bytes(), self.example)
        self.assertIn("''quoted''", result.stdout)
        self.assertFalse((self.caller / relative).exists())
        target.write_text("keep exactly this config", encoding="utf-8")
        refused = self.invoke("bootstrap-local", relative, fixture=True)
        self.assertEqual(refused.returncode, 1)
        self.assertEqual(refused.stdout, "")
        self.assertIn("refusing to overwrite", refused.stderr)
        self.assertEqual(target.read_text(encoding="utf-8"), "keep exactly this config")

    def test_bootstrap_absolute_path_and_missing_parent(self):
        target = self.work / "absolute config.toml"
        self.success(self.invoke("bootstrap-local", target, fixture=True))
        self.assertEqual(target.read_bytes(), self.example)
        missing = self.work / "missing parent" / "config.toml"
        result = self.invoke("bootstrap-local", missing, fixture=True)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertIn("parent directory does not exist", result.stderr)
        self.assertFalse(missing.parent.exists())

    def test_invalid_arguments_fail_before_launching_cargo(self):
        cases = [
            ("nope",), ("SERVE",), ("doctor", "--unknown"), ("doctor", "--READ-ONLY"),
            ("serve", "config", "extra"), ("init", "config", "extra"),
            ("migrate", "config", "extra"), ("bootstrap-local", "config", "extra"),
            ("doctor", "config", "extra"),
            ("doctor", "--read-only", "config", "extra"),
            ("serve", "config", "extra", "one-more"),
        ]
        for args in cases:
            with self.subTest(args=args):
                result = self.invoke(*args, fixture=True)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertTrue("unsupported" in result.stderr or "too many arguments" in result.stderr)

    def test_config_literal_relative_path_and_explicit_precedence(self):
        config, database = self.config("config [literal] 'quoted' 中文.toml")
        other, untouched = self.config("inherited.toml")
        env = dict(ENV, AGENT_LLM_MM_CONFIG=str(other))
        report = self.report("doctor", config.relative_to(ROOT), env=env)
        self.assertEqual(report["database_url"], sqlite_url(database))
        self.assertEqual(report["database_lifecycle"]["status"], "missing")
        self.assertFalse(database.exists())
        self.assertFalse(untouched.exists())

    def test_environment_config_and_database_override_are_forwarded(self):
        config, original = self.config()
        selected = self.work / "environment-selected.sqlite"
        env = dict(ENV, AGENT_LLM_MM_CONFIG=str(config),
                   AGENT_LLM_MM_DATABASE_URL=sqlite_url(selected))
        report = self.report("doctor", "--read-only", env=env)
        self.assertEqual(report["database_url"], sqlite_url(selected))
        self.assertEqual(report["provider"], "mock")
        self.assertFalse(selected.exists())
        self.assertFalse(original.exists())

    def test_missing_and_invalid_configs_fail_without_stdout_or_database(self):
        config, database = self.config()
        for path in [self.work / "missing.toml", config]:
            if path == config:
                path.write_text("invalid = [", encoding="utf-8")
            with self.subTest(path=path):
                result = self.invoke("doctor", "--read-only", path)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, "")
                self.assertTrue(result.stderr.strip())
                self.assertFalse(database.exists())

    def test_readonly_init_migrate_and_duplicate_init_lifecycle(self):
        config, database = self.config()
        before = self.report("doctor", config)
        self.assertEqual(before["database_lifecycle"]["status"], "missing")
        self.assertFalse(database.exists())
        serve = self.invoke("serve", config)
        self.assertNotEqual(serve.returncode, 0)
        self.assertEqual(serve.stdout, "")
        self.assertFalse(database.exists(), "serve must never bootstrap")
        missing_migration = self.invoke("migrate", config)
        self.assertNotEqual(missing_migration.returncode, 0)
        self.assertFalse(database.exists())
        initialized = self.report("init", config)
        self.assertEqual(initialized["operation"], "init")
        self.assertEqual(initialized["status"], "current")
        original = database.read_bytes()
        refused = self.invoke("init", config)
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("refuses to overwrite", refused.stderr)
        self.assertEqual(database.read_bytes(), original)
        healthy = self.report("doctor", "--read-only", config)
        self.assertEqual(healthy["status"], "ok")
        self.assertEqual(database.read_bytes(), original, "doctor must be read-only")
        migrated = self.report("migrate", config)
        self.assertEqual(migrated["status"], "current")
        self.assertIsNone(migrated["backup_path"], "current schema needs no migration")

    def test_legacy_migration_keeps_backup_and_existing_record(self):
        config, database = self.config("legacy.toml")
        with sqlite3.connect(database) as connection:
            connection.executescript("""
                CREATE TABLE events (event_id TEXT PRIMARY KEY, recorded_at TEXT NOT NULL,
                  owner TEXT NOT NULL, namespace TEXT, kind TEXT NOT NULL, summary TEXT NOT NULL);
                INSERT INTO events VALUES ('wrapper-legacy', '2026-01-01T00:00:00Z',
                  'self', 'self', 'observation', 'preserve this legacy record');
            """)
        original = database.read_bytes()
        before = self.report("doctor", "--read-only", config)
        self.assertEqual(before["database_lifecycle"]["status"], "migration_required")
        self.assertEqual(database.read_bytes(), original)
        migrated = self.report("migrate", config)
        self.assertEqual(migrated["status"], "current")
        self.assertTrue(migrated["preserved_row_counts"])
        backup = Path(migrated["backup_path"])
        self.assertTrue(backup.is_file())
        # Backup is produced by VACUUM INTO, so validate logical data, not bytes.
        for path in [backup, database]:
            with sqlite3.connect(path) as connection:
                self.assertEqual(connection.execute(
                    "SELECT summary FROM events WHERE event_id='wrapper-legacy'").fetchone(),
                    ("preserve this legacy record",))
        self.assertEqual(self.report("doctor", config)["status"], "ok")

    def test_explicit_doctor_bootstrap(self):
        config, database = self.config()
        report = self.report("doctor", "--allow-bootstrap", config)
        self.assertEqual(report["status"], "ok")
        self.assertTrue(report["database_lifecycle"]["bootstrap_performed"])
        self.assertTrue(database.is_file())

    def test_caller_state_and_native_exit_code_are_preserved(self):
        config, _ = self.config("config [literal].toml")
        probe = self.work / "native-probe.py"
        probe.write_text(
            "import json, os, sys\n"
            "print(json.dumps({'args': sys.argv[1:], 'cwd': os.getcwd(), "
            "'config': os.environ.get('AGENT_LLM_MM_CONFIG'), 'path': os.environ['PATH']}))\n"
            "print('intentional native failure', file=sys.stderr)\n"
            "sys.exit(37)\n", encoding="utf-8")
        for inherited in [None, "original caller config"]:
            with self.subTest(inherited=inherited):
                driver = self.work / "invoke-in-session.ps1"
                driver.write_text(
                    "$ErrorActionPreference = 'Stop'\n"
                    "$PSNativeCommandUseErrorActionPreference = $true\n"
                    f"function cargo {{ & {ps_quote(sys.executable)} {ps_quote(probe)} @args }}\n"
                    f"& {ps_quote(WRAPPER)} doctor --read-only {ps_quote(config)}\n"
                    "$code = $LASTEXITCODE\n"
                    "[ordered]@{exit_code = $code; cwd = (Get-Location).Path; "
                    "config = $env:AGENT_LLM_MM_CONFIG; "
                    "config_present = (Test-Path Env:AGENT_LLM_MM_CONFIG); path = $env:PATH; "
                    "native_error_preference = $PSNativeCommandUseErrorActionPreference} | "
                    "ConvertTo-Json -Compress\nexit 0\n", encoding="utf-8")
                env = dict(ENV)
                if inherited is not None:
                    env["AGENT_LLM_MM_CONFIG"] = inherited
                result = self.success(run([PWSH, "-NoProfile", "-NonInteractive", "-File", str(driver)],
                                          cwd=self.caller, env=env))
                native, caller = map(json.loads, result.stdout.splitlines())
                self.assertEqual(native["args"], ["run", "--quiet", "--bin", "agent_llm_mm",
                                                  "--", "doctor", "--read-only"])
                self.assertEqual(Path(native["cwd"]), ROOT)
                self.assertEqual(Path(native["config"]), config)
                self.assertEqual(caller["exit_code"], 37)
                self.assertEqual(Path(caller["cwd"]), self.caller)
                self.assertEqual(caller["config"], inherited)
                self.assertEqual(caller["config_present"], inherited is not None)
                self.assertEqual(caller["path"], native["path"])
                self.assertEqual(caller["path"], env["PATH"])
                self.assertTrue(caller["native_error_preference"])
                self.assertIn("intentional native failure", result.stderr)

    def test_stdio_default_and_explicit_serve_keep_stdout_json_only(self):
        config, _ = self.config()
        self.report("init", config)
        for arguments in [[], ["serve", str(config)]]:
            with self.subTest(arguments=arguments):
                env = dict(ENV, AGENT_LLM_MM_CONFIG=str(config))
                process = spawn([PWSH, "-NoProfile", "-NonInteractive", "-File", str(WRAPPER),
                                 *arguments], cwd=self.caller, env=env)
                lines = queue.Queue()
                stderr = []

                def read_stdout():
                    for line in process.stdout:
                        lines.put(line)
                    lines.put(None)

                def read_stderr():
                    stderr.append(process.stderr.read())

                reader = threading.Thread(target=read_stdout, daemon=True)
                errors = threading.Thread(target=read_stderr, daemon=True)
                reader.start()
                errors.start()

                def send(message):
                    process.stdin.write(json.dumps(message, ensure_ascii=False) + "\n")
                    process.stdin.flush()

                def request(identifier, method, params):
                    send({"jsonrpc": "2.0", "id": identifier, "method": method, "params": params})
                    line = lines.get(timeout=60)
                    self.assertIsNotNone(line, "stdio closed before a response")
                    response = json.loads(line)
                    self.assertEqual(response["id"], identifier)
                    self.assertNotIn("error", response)
                    return response["result"]

                try:
                    initialized = request(1, "initialize", {"protocolVersion": "2025-03-26",
                        "capabilities": {}, "clientInfo": {"name": "windows-wrapper-test", "version": "1"}})
                    self.assertIn("serverInfo", initialized)
                    send({"jsonrpc": "2.0", "method": "notifications/initialized"})
                    tools = request(2, "tools/list", {})
                    self.assertIn("recall_memory", {item["name"] for item in tools["tools"]})
                    process.stdin.close()
                    self.assertEqual(process.wait(timeout=30), 0)
                    reader.join(timeout=5)
                    errors.join(timeout=5)
                    while True:
                        line = lines.get(timeout=5)
                        if line is None:
                            break
                        json.loads(line)
                    self.assertIn("CLI command selected", "".join(stderr))
                finally:
                    terminate_tree(process)
                    for stream in [process.stdin, process.stdout, process.stderr]:
                        stream.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-non-windows", action="store_true")
    args = parser.parse_args()
    if os.name != "nt" and not args.allow_non_windows:
        parser.error("Windows is required; use --allow-non-windows only for supplemental checks")
    if not PWSH or not shutil.which("cargo"):
        parser.error("PowerShell 7 (pwsh) and cargo are required; tests are never silently skipped")
    version = run([PWSH, "-NoProfile", "-NonInteractive", "-Command", "$PSVersionTable.PSVersion.ToString()"],
                  cwd=ROOT)
    if version.returncode or int(version.stdout.strip().split(".")[0]) < 7:
        parser.error("PowerShell 7 or newer is required")
    build = run([shutil.which("cargo"), "build", "--quiet", "--bin", "agent_llm_mm"], cwd=ROOT, timeout=900)
    if build.returncode:
        print(build.stderr, file=sys.stderr)
        return build.returncode
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(WrapperTests))
    print(json.dumps({"kind": "powershell_wrapper_contract", "status": "passed" if result.wasSuccessful() else "failed",
                      "host_os": sys.platform, "windows_runtime_evidence": os.name == "nt" and result.wasSuccessful(),
                      "powershell_version": version.stdout.strip(), "tests_run": result.testsRun,
                      "wrapper_sha256": hashlib.sha256(WRAPPER.read_bytes()).hexdigest(),
                      "provider": "mock", "remote_model_calls": 0}))
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
