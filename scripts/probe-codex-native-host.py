#!/usr/bin/env python3
"""Explicit opt-in, synthetic native ledger + official Codex + loopback SSE probe.
Never persists raw requests, hook input, transcript, or RPC events. No external inference.
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import queue
import shlex
import subprocess
import tempfile
import threading
import time
import uuid


class Blocked(Exception):
    pass


def category(text):
    if 'app-server socket directory' in text:
        return 'host_sandbox_socket_directory'
    if 'Operation not permitted' in text or 'Permission denied' in text:
        return 'environment_permission_denied'
    return 'rpc_or_process_failure'


def require_pinned_version(version):
    if version != 'codex-cli 0.159.2':
        raise Blocked('unsupported_codex_version')


def initialize_host(rpc):
    try:
        rpc.call('initialize', {'clientInfo': {'name': 'synthetic_native_probe', 'version': '0.1'},
                                'capabilities': {'experimentalApi': True}})
        rpc.send('initialized', {})
        return rpc
    except BaseException:
        rpc.close()
        raise


def fixture_sse():
    # Matches fixed-tag core/tests/common/responses.rs sse_completed helpers.
    events = [{'type': 'response.created', 'response': {'id': 'resp_synthetic'}},
              {'type': 'response.completed', 'response': {'id': 'resp_synthetic', 'usage': {
                  'input_tokens': 0, 'input_tokens_details': None, 'output_tokens': 0,
                  'output_tokens_details': None, 'total_tokens': 0}}}]
    return ''.join('event: ' + e['type'] + '\ndata: ' + json.dumps(e) + '\n\n' for e in events).encode()


def request_has_requirement(body, marker):
    try:
        value = json.loads(body)
    except (ValueError, UnicodeDecodeError):
        return False
    return isinstance(value, dict) and marker in json.dumps(value.get('input', []))


def has_context_event(events, thread_id, hook_path, marker):
    for event in events:
        params = event.get('params', {})
        run = params.get('run', {})
        if (event.get('method') == 'hook/completed' and params.get('threadId') == thread_id
                and run.get('status') == 'completed' and run.get('eventName') == 'sessionStart'
                and run.get('sourcePath') == str(hook_path)):
            if any(e.get('kind') == 'context' and marker in e.get('text', '') for e in run.get('entries', [])):
                return True
    return False


def full_chain_verified(result):
    return all(result.get(key) is True for key in (
        'startup_hook_context_event', 'resume_hook_context_event',
        'startup_context_in_request', 'resume_latest_context_in_request'))


class Rpc:
    def __init__(self, argv, env, cwd):
        self.process = subprocess.Popen(argv, env=env, cwd=cwd, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.queue = queue.Queue(maxsize=256)
        self.events = []
        self.counter = 0
        def reader():
            for line in iter(lambda: self.process.stdout.readline(1048577), ''):
                if len(line) > 1048576:
                    self.queue.put({'oversize': True})
                    return
                try:
                    self.queue.put(json.loads(line), timeout=1)
                except (ValueError, queue.Full):
                    return
        threading.Thread(target=reader, daemon=True).start()

    def send(self, method, params):
        allowed = {'initialize', 'initialized', 'hooks/list', 'config/batchWrite',
                   'thread/start', 'thread/resume', 'turn/start'}
        if method not in allowed:
            raise Blocked('rpc_not_allowlisted')
        self.counter += 1
        value = {'id': self.counter, 'method': method, 'params': params}
        if method == 'initialized':
            value.pop('id')
        self.process.stdin.write(json.dumps(value) + '\n')
        self.process.stdin.flush()
        return self.counter

    def next(self, timeout):
        try:
            value = self.queue.get(timeout=timeout)
        except queue.Empty:
            raise Blocked('host_timeout')
        if value.get('oversize'):
            raise Blocked('host_output_limit')
        if len(self.events) >= 512:
            raise Blocked('host_event_limit')
        self.events.append(value)
        return value

    def call(self, method, params):
        request_id = self.send(method, params)
        end = time.monotonic() + 25
        while time.monotonic() < end:
            value = self.next(max(.01, end - time.monotonic()))
            if value.get('id') == request_id:
                if 'error' in value:
                    raise Blocked(category(json.dumps(value['error'])))
                return value.get('result', {})
        raise Blocked('host_timeout')

    def turn(self, thread_id):
        # No credentials, external provider, or real model. Only local SSE fixture.
        started = self.call('turn/start', {'threadId': thread_id, 'input': [
            {'type': 'text', 'text': 'Synthetic test: acknowledge the task context.', 'text_elements': []}]})
        turn_id = started['turn']['id']
        end = time.monotonic() + 25
        while True:
            completed = [e for e in self.events if e.get('method') == 'turn/completed'
                         and e.get('params', {}).get('threadId') == thread_id
                         and e.get('params', {}).get('turn', {}).get('id') == turn_id]
            if completed:
                if completed[-1]['params']['turn'].get('status') != 'completed':
                    raise Blocked('synthetic_turn_failed')
                return
            if time.monotonic() >= end:
                raise Blocked('turn_timeout')
            self.next(max(.01, end - time.monotonic()))

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=3)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--codex', required=True)
    parser.add_argument('--ledger', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--allow-synthetic-local-turn', action='store_true', required=True)
    args = parser.parse_args()
    codex, ledger, output = map(Path, (args.codex, args.ledger, args.output))
    if any(not p.is_absolute() for p in (codex, ledger, output)) or output.exists():
        parser.error('absolute executable/output paths and new output required')
    if os.name != 'posix':
        parser.error('this optional host probe is POSIX-only')
    # Short private socket paths; never use the user's HOME or modify sandbox settings.
    base = Path(tempfile.mkdtemp(prefix='ledger-host-')).resolve()
    home, config, project = (base / n for n in ('home', 'config', 'project'))
    for p in (home, config, project):
        p.mkdir(mode=0o700)
    env = {'PATH': os.defpath, 'HOME': str(home), 'CODEX_HOME': str(config),
           'TMPDIR': str(base), 'TOKIO_WORKER_THREADS': '2',
           'AGENT_LLM_MM_CONFIG': str(base / 'ledger.toml')}
    (base / 'ledger.toml').write_text('transport="stdio"\ndatabase_url=' +
        json.dumps('sqlite://' + str(base / 'ledger.sqlite')) + '\n[model]\nprovider="mock"\n')
    source = {'kind': 'caller_reported', 'reference': None}
    token = uuid.uuid4().hex
    markers = ['SYNTHETIC_INITIAL_' + token, 'SYNTHETIC_REVISED_' + token]
    result = {'version_pinned': False, 'local_fixture_only': True, 'raw_requests_saved': False,
              'private_transcript_read': False, 'model_understanding_verified': False,
              'fixture_trusted': False, 'startup_context_in_request': False,
              'resume_latest_context_in_request': False, 'startup_hook_context_event': False,
              'resume_hook_context_event': False, 'fixture_requests': 0,
              'full_native_chain_verified': False, 'failure_class': None}
    host = core = server = None
    core_errors = tempfile.TemporaryFile()
    fixture_observations = []
    phase = {'index': 0}

    def cli(*argv):
        done = subprocess.run([str(ledger), *map(str, argv)], env=env, cwd=project,
                              capture_output=True, timeout=20)
        if done.returncode:
            raise Blocked('ledger_' + category(done.stderr.decode(errors='replace')))
        try:
            return json.loads(done.stdout)
        except ValueError:
            return None

    def requirement(index):
        return {'requirement_id': 'required-' + str(index), 'kind': 'prohibition',
                'text': markers[index], 'state': 'reported', 'source': source, 'replaces': None}

    class Fixture(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass
        def do_POST(self):
            count = int(self.headers.get('Content-Length', '0'))
            if count <= 0 or count > 2 * 1024 * 1024 or self.path != '/v1/responses':
                self.send_error(400)
                return
            body = self.rfile.read(count)
            # Only booleans/counts are retained, never raw request contents.
            fixture_observations.append({'phase': phase['index'],
                                         'current_requirement': request_has_requirement(body, markers[phase['index']])})
            wire = fixture_sse()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(wire)))
            self.end_headers()
            self.wfile.write(wire)

    try:
        version = subprocess.run([str(codex), '--version'], env=env, cwd=project,
                                 capture_output=True, timeout=10, check=True).stdout.decode().strip()
        require_pinned_version(version)
        result['version_pinned'] = True
        result['ledger_sha256'] = hashlib.sha256(ledger.read_bytes()).hexdigest()
        cli('init')
        registry, socket, state = base / 'registry.json', base / 'c.sock', base / 'adapter.sqlite'
        workspace = cli('workspace', 'register', '--registry', registry, '--root', project)
        namespace = workspace['project_namespace']
        cli('task', 'create', '--json', json.dumps({'namespace': namespace, 'task_id': 'synthetic-task',
            'request_id': 'create', 'title': 'Synthetic continuity fixture', 'requirements': [requirement(0)],
            'source': source, 'reason': 'Local-only host integration test'}))
        core = subprocess.Popen([str(ledger), 'core', '--registry', str(registry), '--socket', str(socket)],
            env=env, cwd=project, stdout=subprocess.DEVNULL, stderr=core_errors)
        end = time.monotonic() + 10
        while not socket.exists():
            if core.poll() is not None or time.monotonic() > end:
                core_errors.seek(0)
                raise Blocked('local_core_' + category(core_errors.read(8192).decode(errors='replace')))
            time.sleep(.05)
        cli('codex-context', 'init', '--state', state)
        cli('codex-context', 'select-default', '--state', state, '--registry', registry, '--socket', socket,
            '--root', project, '--workspace-id', workspace['id'], '--task-id', 'synthetic-task')
        hook_args = [str(ledger), 'codex-context', 'hook', '--state', str(state), '--registry', str(registry),
                     '--socket', str(socket), '--max-bytes', '8192']
        command = shlex.join(hook_args)
        definition = {'hooks': {'SessionStart': [{'matcher': '^(startup|resume)$', 'hooks': [
            {'type': 'command', 'command': command, 'additionalContextLimit': 0, 'timeout': 8}]}]}}
        hook_path = config / 'hooks.json'
        hook_path.write_text(json.dumps(definition))
        definition_hash = hashlib.sha256(hook_path.read_bytes()).hexdigest()
        (config / 'config.toml').write_text('')
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        provider = '{name="Synthetic loopback",base_url="http://127.0.0.1:' + str(server.server_port) + '/v1",wire_api="responses",requires_openai_auth=false}'
        host_args = [str(codex), '-c', 'model_provider="fixture"', '-c', 'model_providers.fixture=' + provider,
                     '-c', 'model="synthetic-no-inference"', 'app-server', '--stdio']
        def start_host():
            return initialize_host(Rpc(host_args, env, project))
        host = start_host()
        def inspect(trust):
            groups = host.call('hooks/list', {'cwds': [str(project)]})['data']
            if len(groups) != 1 or groups[0].get('warnings') or groups[0].get('errors') or len(groups[0]['hooks']) != 1:
                raise Blocked('unexpected_hooks')
            h = groups[0]['hooks'][0]
            expected = {'sourcePath': str(hook_path), 'source': 'user', 'command': command,
                        'eventName': 'sessionStart', 'handlerType': 'command', 'matcher': '^(startup|resume)$',
                        'timeoutSec': 8, 'additionalContextLimit': 0, 'async': False, 'enabled': True,
                        'isManaged': False, 'pluginId': None, 'trustStatus': trust,
                        'key': str(hook_path) + ':session_start:0:0'}
            if any(h.get(k) != v for k, v in expected.items()):
                raise Blocked('fixture_definition_mismatch')
            if hashlib.sha256(hook_path.read_bytes()).hexdigest() != definition_hash or hashlib.sha256(ledger.read_bytes()).hexdigest() != result['ledger_sha256']:
                raise Blocked('fixture_files_changed')
            if not h['currentHash'].startswith('sha256:') or len(h['currentHash']) != 71:
                raise Blocked('invalid_definition_hash')
            return h
        h = inspect('untrusted')
        host.call('config/batchWrite', {'filePath': str(config / 'config.toml'), 'reloadUserConfig': True,
            'edits': [{'keyPath': 'hooks.state', 'mergeStrategy': 'upsert',
                       'value': {h['key']: {'trusted_hash': h['currentHash']}}}]})
        if inspect('trusted')['currentHash'] != h['currentHash']:
            raise Blocked('fixture_hash_changed')
        result['fixture_trusted'] = True
        def observe(index):
            return has_context_event(host.events, thread, hook_path, markers[index])
        thread = host.call('thread/start', {'cwd': str(project), 'approvalPolicy': 'never', 'sandbox': 'read-only'})['thread']['id']
        host.turn(thread)
        result['startup_hook_context_event'] = observe(0)
        result['startup_context_in_request'] = any(o['phase'] == 0 and o['current_requirement'] for o in fixture_observations)
        host.close()
        host = None
        cli('task', 'revise', '--json', json.dumps({'namespace': namespace, 'task_id': 'synthetic-task',
            'request_id': 'revise', 'expected_version': 1, 'changes': [{'operation': 'add', 'requirement': requirement(1)}],
            'source': source, 'reason': 'Synthetic requirement changed while host closed'}))
        phase['index'] = 1
        host = start_host()
        host.call('thread/resume', {'threadId': thread, 'cwd': str(project), 'approvalPolicy': 'never', 'sandbox': 'read-only'})
        host.turn(thread)
        result['resume_hook_context_event'] = observe(1)
        result['resume_latest_context_in_request'] = any(o['phase'] == 1 and o['current_requirement'] for o in fixture_observations)
        result['full_native_chain_verified'] = full_chain_verified(result)
    except Blocked as error:
        result['failure_class'] = str(error)
    except Exception as error:
        # Exception text can include raw RPC or filesystem context; retain only its class.
        result['failure_class'] = 'probe_exception_' + type(error).__name__
    finally:
        if host:
            host.close()
        if core:
            core.terminate()
            try:
                core.wait(timeout=3)
            except subprocess.TimeoutExpired:
                core.kill()
                core.wait(timeout=3)
        if server:
            server.shutdown()
            server.server_close()
        core_errors.close()
        result['fixture_requests'] = len(fixture_observations)
        with output.open('x') as file:
            json.dump(result, file, indent=2)
        print(json.dumps(result))
    return 0 if result['full_native_chain_verified'] else 2


if __name__ == '__main__':
    raise SystemExit(main())
