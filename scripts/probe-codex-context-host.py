#!/usr/bin/env python3
"""Opt-in public Codex host probe. No turn/model request or automatic hook trust.

Requires an already installed, explicitly chosen Codex executable. Does not install
software. The isolated home is retained beside the evidence for inspection. Never
reads the user's home, credentials, history, or private transcript files.
"""
import argparse
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--codex", required=True, help="Absolute installed official Codex executable")
    parser.add_argument("--output", required=True, help="New evidence JSON file; never overwritten")
    args = parser.parse_args()
    executable = Path(args.codex)
    output = Path(args.output)
    if not executable.is_absolute() or not output.is_absolute():
        parser.error("both paths must be absolute")
    if output.exists():
        parser.error("output already exists")
    base = Path(tempfile.mkdtemp(prefix="codex-context-host-", dir=output.parent))
    home, config = base / "home", base / "config"
    home.mkdir(mode=0o700)
    config.mkdir(mode=0o700)
    # Version/schema commands and app-server run only with the synthetic HOME.
    environment = {"PATH": os.environ.get("PATH", ""), "HOME": str(home), "CODEX_HOME": str(config)}
    if os.name == "nt":
        for key in ("SYSTEMROOT", "WINDIR", "PATHEXT"):
            if key in os.environ:
                environment[key] = os.environ[key]
        environment["USERPROFILE"] = str(home)
    version = subprocess.run([str(executable), "--version"], env=environment, cwd=base,
                             capture_output=True, text=True, timeout=10, check=True).stdout.strip()
    schema_dir = base / "schema"
    subprocess.run([str(executable), "app-server", "generate-json-schema", "--experimental", "--out", str(schema_dir)],
                   env=environment, cwd=base, capture_output=True, timeout=30, check=True)
    schema = json.loads((schema_dir / "v2/HooksListResponse.json").read_text())
    spill_schema = schema["definitions"]["HookMetadata"]["properties"]["additionalContextLimit"]
    if os.name == "nt":
        # Do not invent the hook shell contract. Configuration remains untrusted.
        command = "echo SYNTHETIC_UNTRUSTED_PROBE"
    else:
        command = "printf '%s' '{\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\"additionalContext\":\"SYNTHETIC_CONTEXT_PROBE\"}}'"
    (config / "hooks.json").write_text(json.dumps({"hooks": {"SessionStart": [{"matcher": "^(startup|resume|compact|fork)$", "hooks": [{"type": "command", "command": command, "additionalContextLimit": 0, "timeout": 3}]}]}}))
    provider = '{name="Offline probe",base_url="http://127.0.0.1:9/v1",wire_api="responses",requires_openai_auth=false}'
    process = subprocess.Popen([str(executable), "-c", 'model_provider="probe"', "-c", f"model_providers.probe={provider}", "-c", 'model="probe-no-inference"', "app-server", "--stdio"],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               env=environment, cwd=base, text=True)
    inbox = queue.Queue(maxsize=512)
    records, requests = [], []

    def reader(stream, label):
        for line in iter(lambda: stream.readline(65537), ""):
            if len(line) > 65536:
                inbox.put({"stream": label, "line": "OUTPUT_LIMIT_REACHED"})
                break
            inbox.put({"stream": label, "line": line.rstrip()})

    for stream, label in ((process.stdout, "stdout"), (process.stderr, "stderr")):
        threading.Thread(target=reader, args=(stream, label), daemon=True).start()

    def send(message):
        assert message["method"] in ("initialize", "initialized", "hooks/list", "thread/start")
        requests.append(message)
        process.stdin.write(json.dumps(message) + "\n")
        process.stdin.flush()

    def collect(seconds):
        until = time.monotonic() + seconds
        while time.monotonic() < until:
            try:
                records.append(inbox.get(timeout=max(0.01, until - time.monotonic())))
                if len(records) >= 512:
                    raise RuntimeError("probe event limit exceeded")
            except queue.Empty:
                break

    failure = None
    try:
        send({"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "ledger_context_probe", "version": "0.1"}, "capabilities": {"experimentalApi": True}}})
        collect(2)
        if process.poll() is None:
            send({"method": "initialized", "params": {}})
            send({"id": 2, "method": "hooks/list", "params": {"cwds": [str(base)]}})
            collect(2)
            send({"id": 3, "method": "thread/start", "params": {"cwd": str(base), "approvalPolicy": "never", "sandbox": "read-only", "ephemeral": True}})
            collect(5)
    except (OSError, RuntimeError) as error:
        failure = str(error)
    finally:
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=3)
    evidence = {"version": version, "isolated_home": str(home), "schema_additional_context_limit": spill_schema,
                "requests": requests, "events": records, "failure": failure,
                "model_turn_requested": False, "trusted_automatically": False,
                "private_transcript_read": False, "delivery_acceptance": "not_verified",
                "scope": "schema and untrusted config parsing only; never count this as lifecycle injection"}
    with output.open("x") as file:
        json.dump(evidence, file, indent=2)
    print(str(output))


if __name__ == "__main__":
    main()
