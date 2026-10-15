#!/usr/bin/env python3
"""Local MCP process and file helpers. No external provider network calls."""
import hashlib
import json
from pathlib import Path
import queue
import subprocess
import threading
import time
import sys

ROOT = Path(__file__).resolve().parent
BINARY = ROOT.parents[1] / 'target/debug' / ('agent_llm_mm.exe' if sys.platform=='win32' else 'agent_llm_mm')
MODEL = 'gpt-5.6-luna'


def save(path, value):
    data = json.dumps(value, ensure_ascii=False, indent=2)
    temporary = path.with_name(path.name + '.tmp')
    with temporary.open('w', encoding='utf-8') as stream:
        stream.write(data)
        stream.flush()
        import os
        os.fsync(stream.fileno())
    temporary.chmod(0o600)
    temporary.replace(path)


def verify_binary():
    if not BINARY.is_file():raise RuntimeError('binary_missing_build_agent_llm_mm_first')
    return {'binary_sha256':hashlib.sha256(BINARY.read_bytes()).hexdigest(),
            'source_binding_verified':False,'binding_note':'Use a separately verified package receipt to establish build provenance.'}


def write_config(folder, database):
    path = folder / 'local-storage.toml'
    content = ('transport = "stdio"\ndatabase_url = ' + json.dumps('sqlite://' + database.as_posix()) +
               '\n[dashboard]\nenabled = false\n[daemon]\nenabled = false\n[model]\n')
    content += 'provider = "mock"\n'  # Storage only; real conversation uses the shared HTTP client.
    path.write_text(content, encoding='utf-8')
    path.chmod(0o600)
    return path


def child_environment(config):
    empty = config.parent / 'empty-path'
    empty.mkdir(exist_ok=True)
    return {'PATH': str(empty), 'HOME': str(config.parent), 'TMPDIR': str(config.parent),
            'AGENT_LLM_MM_CONFIG': str(config), 'RUST_LOG': 'error'}


class Client:
    def __init__(self, config):
        self.child = subprocess.Popen([str(BINARY), 'serve'], cwd=config.parent,
            env=child_environment(config), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, text=True, encoding='utf-8')
        self.lines = queue.Queue()
        self.counter = 0
        def read():
            for line in self.child.stdout:
                self.lines.put(line)
            self.lines.put(None)
        threading.Thread(target=read, daemon=True).start()

    def __enter__(self):
        try:
            self.request('initialize', {'protocolVersion': '2025-03-26', 'capabilities': {},
                         'clientInfo': {'name': 'user-operated-experience-30', 'version': '1'}})
            self.send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        except BaseException:
            self.__exit__()
            raise
        return self

    def __exit__(self, *unused):
        self.child.stdin.close()
        try:
            self.child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.child.kill()
            self.child.wait()
        self.child.stdout.close()

    def send(self, value):
        self.child.stdin.write(json.dumps(value) + '\n')
        self.child.stdin.flush()

    def request(self, method, params):
        self.counter += 1
        self.send({'jsonrpc': '2.0', 'id': self.counter, 'method': method, 'params': params})
        deadline = time.monotonic() + 55
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:raise RuntimeError('mcp_deadline_exceeded')
            line = self.lines.get(timeout=remaining)
            if line is None:
                raise RuntimeError('mcp_process_closed')
            response = json.loads(line)
            if response.get('id') == self.counter:
                break
        if 'error' in response:
            raise RuntimeError('mcp_protocol_error')
        return response['result']

    def tool(self, name, arguments):
        response = self.request('tools/call', {'name': name, 'arguments': arguments})
        if response.get('isError'):
            raise RuntimeError('mcp_tool_error')
        return response['structuredContent']
