"""Offline contracts only: no process, socket, model, or credential access."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import Mock

SPEC = importlib.util.spec_from_file_location('probe', Path(__file__).resolve().parents[1] / 'scripts/probe-codex-native-host.py')
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


class ProbeContracts(unittest.TestCase):
    def test_exact_version_pin(self):
        probe.require_pinned_version('codex-cli 0.159.2')
        for version in ['codex-cli 0.159.3', 'codex-cli 0.159.2-dev', '0.159.2', '']:
            with self.assertRaises(probe.Blocked):
                probe.require_pinned_version(version)

    def test_initialize_failure_closes_process(self):
        for method in ['call', 'send']:
            rpc = Mock()
            getattr(rpc, method).side_effect = KeyboardInterrupt()
            with self.assertRaises(KeyboardInterrupt):
                probe.initialize_host(rpc)
            rpc.close.assert_called_once()

    def test_initialize_success_leaves_process_open(self):
        rpc = Mock()
        self.assertIs(probe.initialize_host(rpc), rpc)
        rpc.close.assert_not_called()

    def test_actual_sse_matches_pinned_official_minimal_fixture(self):
        wire = probe.fixture_sse().decode()
        blocks = wire.strip().split('\n\n')
        events = [json.loads(block.split('\ndata: ', 1)[1]) for block in blocks]
        self.assertEqual([e['type'] for e in events], ['response.created', 'response.completed'])
        self.assertEqual(events[0]['response']['id'], events[1]['response']['id'])
        self.assertEqual(events[1]['response']['usage'], {
            'input_tokens': 0, 'input_tokens_details': None, 'output_tokens': 0,
            'output_tokens_details': None, 'total_tokens': 0})
        for block, event in zip(blocks, events):
            self.assertTrue(block.startswith('event: ' + event['type'] + '\n'))

    def test_marker_must_be_in_request_input(self):
        self.assertTrue(probe.request_has_requirement(b'{"input":[{"text":"REQUIRED"}]}', 'REQUIRED'))
        for body in [b'{"metadata":"REQUIRED"}', b'not json', b'[]', b'{"input":"older"}']:
            self.assertFalse(probe.request_has_requirement(body, 'REQUIRED'))

    def test_hook_requires_matching_scope_status_and_context_entry(self):
        event = {'method': 'hook/completed', 'params': {'threadId': 'thread', 'run': {
            'status': 'completed', 'eventName': 'sessionStart', 'sourcePath': '/fixture/hooks.json',
            'entries': [{'kind': 'context', 'text': 'REQUIRED'}]}}}
        def seen(value):
            return probe.has_context_event([value], 'thread', '/fixture/hooks.json', 'REQUIRED')
        self.assertTrue(seen(event))
        for key, value in [('status', 'failed'), ('eventName', 'stop'), ('sourcePath', '/other')]:
            wrong = copy.deepcopy(event)
            wrong['params']['run'][key] = value
            self.assertFalse(seen(wrong))
        wrong = copy.deepcopy(event)
        wrong['params']['threadId'] = 'other-thread'
        self.assertFalse(seen(wrong))
        wrong = copy.deepcopy(event)
        wrong['params']['run']['entries'][0]['kind'] = 'warning'
        self.assertFalse(seen(wrong))

    def test_all_four_observations_required(self):
        result = dict.fromkeys(('startup_hook_context_event', 'resume_hook_context_event',
            'startup_context_in_request', 'resume_latest_context_in_request'), True)
        self.assertTrue(probe.full_chain_verified(result))
        for key in result:
            wrong = dict(result, **{key: False})
            self.assertFalse(probe.full_chain_verified(wrong))
        self.assertFalse(probe.full_chain_verified({}))

    def test_failed_turn_does_not_count_as_success(self):
        rpc = probe.Rpc.__new__(probe.Rpc)
        rpc.call = Mock(return_value={'turn': {'id': 'current'}})
        rpc.events = [{'method': 'turn/completed', 'params': {'threadId': 'thread',
                       'turn': {'id': 'current', 'status': 'failed'}}}]
        with self.assertRaisesRegex(probe.Blocked, 'synthetic_turn_failed'):
            rpc.turn('thread')

    def test_unrelated_turn_completion_is_ignored(self):
        rpc = probe.Rpc.__new__(probe.Rpc)
        rpc.call = Mock(return_value={'turn': {'id': 'current'}})
        rpc.events = [{'method': 'turn/completed', 'params': {'threadId': 'thread',
                       'turn': {'id': 'previous', 'status': 'completed'}}}]
        rpc.next = Mock(side_effect=probe.Blocked('host_timeout'))
        with self.assertRaisesRegex(probe.Blocked, 'host_timeout'):
            rpc.turn('thread')
        rpc.next.assert_called_once()


if __name__ == '__main__':
    unittest.main()
