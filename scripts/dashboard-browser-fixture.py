#!/usr/bin/env python3
"""Owned, disposable MCP server for browser QA. Never accepts user config or credentials."""
import importlib.util
import json
import os
from pathlib import Path
import socket
import signal
import subprocess
import sys
import time
import urllib.request


_closing = False

def close_mcp(client):
    global _closing
    _closing = True
    try:
        try:
            client.proc.stdin.close()
        except (BrokenPipeError, OSError):
            pass
        try:
            client.proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            client.proc.kill()
            client.proc.wait(timeout=2)
    finally:
        try:
            if client.proc.poll() is None:
                client.proc.kill()
                client.proc.wait(timeout=2)
        finally:
            client.log.close()
            _closing = False


def interrupt(_signal, _frame):
    if not _closing:
        raise KeyboardInterrupt


def main():
    signal.signal(signal.SIGTERM, interrupt)
    signal.signal(signal.SIGINT, interrupt)
    binary, output = Path(sys.argv[1]).resolve(strict=True), Path(sys.argv[2]).resolve()
    output.mkdir(parents=True, exist_ok=False)
    spec = importlib.util.spec_from_file_location('smoke', Path(__file__).with_name('local-memory-smoke.py'))
    smoke = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(smoke)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    config = output / 'fixture.toml'
    config.write_text('database_url = ' + json.dumps('sqlite://' + (output / 'synthetic.sqlite').as_posix()) +
                      '\n[model]\nprovider = "mock"\n[dashboard]\nenabled = true\nhost = "127.0.0.1"\nport = ' + str(port) +
                      '\nbase_path = "/qa/desk"\nevent_capacity = 200\nsse_enabled = true\nrequired = true\nopen_browser = false\n', encoding='utf-8')
    env = {key: value for key, value in os.environ.items() if not key.startswith('AGENT_LLM_MM_')}
    env['AGENT_LLM_MM_CONFIG'] = str(config)
    init = subprocess.run([str(binary), 'init'], env=env, capture_output=True, text=True, check=True, timeout=15)
    (output / 'init.json').write_text(init.stdout, encoding='utf-8')
    evidence = []
    class FixtureClient(smoke.Client):
        def __init__(self, *args):
            try:
                super().__init__(*args)
            except BaseException:
                if hasattr(self, 'proc'):
                    close_mcp(self)
                raise

        def close(self):
            close_mcp(self)

    client = FixtureClient(binary, config, evidence)
    def append(number):
        return client.tool('ingest_interaction', {'request_id':f'browser-{number}', 'event':{
            'owner':'World','namespace':'project/dashboard-qa' if number % 2 else 'project/dashboard-other',
            'kind':'Observation','summary':f'Synthetic browser record {number} · 日本語 · 中文'},
            'claim_drafts':[],'episode_reference':'episode:browser-qa'})
    try:
        url = f'http://127.0.0.1:{port}/qa/desk/'
        for _ in range(100):
            try:
                with urllib.request.urlopen(url + 'api/status', timeout=1) as response:
                    status = json.load(response)
                assert status['process_id'] == client.proc.pid, 'port must belong to our child'
                break
            except OSError:
                time.sleep(.05)
        else:
            raise RuntimeError('owned dashboard did not start')
        for number in range(28):
            append(number)
        snapshot = client.tool('build_self_snapshot', {'budget':1000,'namespace':'project/dashboard-qa'})
        client.tool('decide_with_snapshot', {'task':'synthetic browser fixture','action':'inspect locally','snapshot':snapshot['snapshot']})
        # Handler-level failure supplies a real failed record, without model access.
        try:
            client.request('tools/call', {'name':'search_memory','arguments':{
                'namespace':'project/dashboard-qa','recorded_after':'not-a-timestamp'}})
        except RuntimeError:
            pass  # Expected handler-level invalid-params response; retained in the synthetic transcript.
        else:
            raise AssertionError('invalid time must fail')
        print(json.dumps({'url':url,'pid':client.proc.pid,'records':28}), flush=True)
        for line in sys.stdin:
            command = json.loads(line)
            if command['action'] == 'stop':
                break
            if command['action'] == 'append':
                append(command['number'])
                print(json.dumps({'ok':True}), flush=True)
    finally:
        client.close()
        (output / 'mcp-transcript.json').write_text(json.dumps(evidence,ensure_ascii=False,indent=2),encoding='utf-8')


if __name__ == '__main__':
    main()
