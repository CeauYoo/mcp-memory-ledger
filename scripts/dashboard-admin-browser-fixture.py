#!/usr/bin/env python3
"""Disposable real-core owner UI fixture. Only synthetic credentials/data, no models."""
import importlib.util
import json
from pathlib import Path
import signal
import socket
import sys
import time
import urllib.request

CAPABILITY = 'synthetic-admin-browser-fixture-only'


def main():
    spec = importlib.util.spec_from_file_location('verify_core', Path(__file__).with_name('verify-local-core.py'))
    verify = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(verify)
    binary, output = verify.validate_paths(Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve())
    output.mkdir(mode=0o700)
    evidence = {'processes': [], 'readiness': [], 'rpc_calls': [], 'checks': []}
    processes = verify.Processes(binary, output, evidence)
    def interrupt(_signal, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupt)
    signal.signal(signal.SIGINT, interrupt)
    try:
        for name in ('bin', 'home', 'tmp', 'logs', 's', 'workspace'):
            (output / name).mkdir(mode=0o700)
        config, database, registry, address = [output / name for name in ('mock.toml', 'synthetic.sqlite', 'registry.json', 's/c.sock')]
        verify.write_mock_config(config, database)
        processes.command('initialize', ['init'], config)
        workspace = processes.command('register', ['workspace', '--registry', registry, 'register', '--root', output / 'workspace'])
        token = output / 'synthetic-capability'
        token.write_text(CAPABILITY, encoding='utf-8')
        token.chmod(0o600)
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', 0))
            port = probe.getsockname()[1]
        core = processes.spawn('core', 'core', ['core', '--socket', address, '--registry', registry, '--admin-bind', f'127.0.0.1:{port}', '--admin-token-file', token], config)
        verify.wait_ready(core, address, evidence)
        url = f'http://127.0.0.1:{port}/'
        for attempt in range(100):
            try:
                request = urllib.request.Request(url + 'api/health', headers={'Authorization': 'Bearer ' + CAPABILITY})
                with urllib.request.urlopen(request, timeout=1) as response:
                    verify.require(response.status == 200, 'admin_fixture_health_failed')
                break
            except OSError:
                verify.require(core.process.poll() is None, 'admin_fixture_core_exited')
                time.sleep(.05)
        else:
            raise verify.VerificationError('admin_fixture_health_timeout')
        bridge = processes.spawn('bridge', 'bridge', ['bridge', '--socket', address, '--workspace-id', workspace['id'], '--workspace-root', output / 'workspace', '--client-label', 'browser-fixture'], pipe=True)
        client = verify.McpClient(bridge, output, evidence)
        client.initialize()
        client.tool('recall_memory', {'query': 'SYNTHETIC_BROWSER_PROBE', 'limit': 10})
        print(json.dumps({'url': url, 'pid': core.process.pid}), flush=True)
        for line in sys.stdin:
            if json.loads(line).get('action') == 'stop':
                break
    finally:
        cleanup = processes.cleanup()
        verify.write_json(output / 'cleanup.json', cleanup)
        verify.write_json(output / 'evidence.json', evidence)
        (output / 'synthetic-capability').unlink(missing_ok=True)


if __name__ == '__main__':
    main()
