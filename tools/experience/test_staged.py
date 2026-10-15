"""Offline-only coverage for staged budgets, failure gates and narrow prose checks."""
import json
from pathlib import Path
import tempfile
import unittest
import urllib.request
import urllib.error
from unittest.mock import Mock,patch
import answer_checks
import network
import protocol
import run
import staged
from live_monitor import LiveMonitor,projection
from catalog import SCENARIOS
from test_experience import FAKE_KEY,fixture,response_for

class StagedTests(unittest.TestCase):
    def test_live_monitor_dynamic_state_and_no_private_files_or_foreign_host(self):
        report={'status':'calling_model','phase':'targeted','attempts':[],
            'totals':{'reserved_physical_attempts':0,'unexpected_private_field':FAKE_KEY},'private_key':FAKE_KEY,
            'scenarios':[{'id':'S16','title':'Synthetic','turns':[{'number':1,'status':'calling_model','assistant_text':FAKE_KEY}]}]}
        with LiveMonitor() as first,LiveMonitor() as second:
            self.assertNotEqual(first.url,second.url)
            first.publish('run',report)
            with urllib.request.urlopen(first.url+'api/status') as stream:before=json.load(stream)
            self.assertEqual(before['turns'][0]['status'],'calling_model');self.assertNotIn(FAKE_KEY,json.dumps(before))
            report['attempts'].append({});report['scenarios'][0]['turns'][0]['status']='objective_check_failed'
            first.publish('run',report)
            with urllib.request.urlopen(first.url+'api/status') as stream:after=json.load(stream)
            self.assertEqual(after['attempts'],1);self.assertEqual(after['turns'][0]['status'],'objective_check_failed')
            with urllib.request.urlopen(first.url) as stream:page=stream.read().decode()
            self.assertIn('setInterval(refresh,1000)',page)
            for path in ['diagnostics','../../memory.sqlite']:
                with self.assertRaises(urllib.error.HTTPError) as error:urllib.request.urlopen(first.url+path)
                self.assertEqual(error.exception.code,404)
                error.exception.close()
            request=urllib.request.Request(first.url+'api/status',headers={'Host':'untrusted.invalid'})
            with self.assertRaises(urllib.error.HTTPError) as error:urllib.request.urlopen(request)
            self.assertEqual(error.exception.code,403)
            error.exception.close()
            first.publish('plan',{'status':'interrupted_or_local_failure_no_resume',
                'total_physical_attempts':None,'total_explicit_retries':None,
                'accounting_complete':False,'known_physical_attempts_lower_bound':1})
            with urllib.request.urlopen(first.url+'api/status') as stream:unknown=json.load(stream)
            self.assertIsNone(unknown['attempts'])
            self.assertEqual(unknown['phase_attempts'],1)
            self.assertFalse(unknown['totals']['accounting_complete'])
            self.assertEqual(unknown['totals']['known_physical_attempts_lower_bound'],1)
            usage=unknown['totals']['usage']['input_tokens']
            self.assertFalse(usage['complete_accounting'])
            self.assertIsNone(usage['unknown_attempts'])
            self.assertTrue(usage['known_sum_is_lower_bound'])

    def test_targeted_scope_and_shared_budget(self):
        cases=staged.targeted_scenarios()
        self.assertEqual([c['id'] for c in cases],['S16','S19','S23','S25'])
        self.assertEqual(sum(len(c['turns']) for c in cases),12)
        self.assertEqual(staged.next_limits([],12),(22,10))
        first={'attempts':[{'retry':False}]*12+[{'retry':True}]*7}
        self.assertEqual(staged.next_limits([first],90),(93,3))
        first['attempts'] += [{'retry':True}]*3
        self.assertEqual(staged.next_limits([first],90),(90,0))
        first['attempts'].append({'retry':True})
        with self.assertRaises(RuntimeError):staged.next_limits([first],90)

    def test_old_past_plan_is_detected_without_changing_decision_or_sources(self):
        item=SCENARIOS[15]['turns'][0]
        original='现在—2026-09-08：确认交付基线；2026-09-09—2026-10-08：制定执行计划。'
        text=json.dumps({'answer':original,'decisions':item['expected'],'memory_audit':[]})
        result=protocol.evaluate(text,item,{'records':[]})
        self.assertTrue(result['checks']['decision:delivery_date'])
        self.assertFalse(result['body_checks']['no_explicit_date_before_planning_start'])
        self.assertEqual(result['status'],'objective_check_failed')
        checks=answer_checks.check_answer('2026年10月14日至2026-12-08准备并交付。',item['body_rules'])
        self.assertTrue(all(checks.values()))
        self.assertFalse(answer_checks.check_answer('2026-13-44',item['body_rules'])['planning_dates_parseable'])
        self.assertEqual(answer_checks.check_answer(original,{}),{})

    def test_input_cap_stops_before_reservation_or_dispatch(self):
        reserve=Mock();finish=Mock()
        with tempfile.TemporaryDirectory() as tmp:
            provider=network.Provider(Path(tmp),{},FAKE_KEY,reserve,finish)
            provider.opener.open=Mock()
            with self.assertRaises(network.InputLimitExceeded):provider.call({'text':'中'*network.MAX_INPUT_BYTES},'oversized')
            reserve.assert_not_called();finish.assert_not_called();provider.opener.open.assert_not_called()

    def test_real_mcp_staged_success_is_102_offline_requests(self):
        cases=staged.targeted_scenarios()+SCENARIOS;items=[t for s in cases for t in s['turns']]
        with tempfile.TemporaryDirectory() as tmp,patch.object(run,'ROOT',Path(tmp)),fixture(items) as (target,calls),LiveMonitor() as monitor:
            report,folder=staged.execute_staged(FAKE_KEY,offline=True,target=target,progress=lambda _:None,observer=monitor.publish)
            self.assertEqual(len(calls),102)
            self.assertEqual(report['total_physical_attempts'],102)
            self.assertEqual([p['physical_attempts'] for p in report['phases']],[12,90])
            self.assertEqual(report['status'],'completed_review_required')
            self.assertTrue(report['automatic_gate_is_not_semantic_acceptance'])
            self.assertNotIn(FAKE_KEY,(folder/'summary.json').read_text(encoding='utf-8'))
            with urllib.request.urlopen(monitor.url+'api/status') as stream:status=json.load(stream)
            self.assertEqual(status['attempts'],102)
            self.assertEqual(status['phase_attempts'],90)
            self.assertTrue(status['totals']['usage']['input_tokens']['complete_accounting'])
            self.assertGreater(status['totals']['usage']['input_tokens']['known_sum'],0)
            self.assertEqual(status['status'],'completed_review_required')
            self.assertEqual(len(status['turns']),90)

    def test_targeted_wrong_source_stops_full_phase(self):
        cases=staged.targeted_scenarios();items=[t for s in cases for t in s['turns']]
        def answer(body,n):
            obj=response_for(body,items[n-1],n)
            if n==4:
                value=json.loads(obj['output'][0]['content'][0]['text'])
                value['memory_audit'][0]['source_event_ids']=['event:synthetic-wrong-source']
                obj['output'][0]['content'][0]['text']=json.dumps(value)
            return 200,obj
        with tempfile.TemporaryDirectory() as tmp,patch.object(run,'ROOT',Path(tmp)),fixture(handler=answer) as (target,calls):
            report,_=staged.execute_staged(FAKE_KEY,offline=True,target=target,progress=lambda _:None)
            self.assertEqual(len(calls),4)
            self.assertEqual(report['status'],'targeted_gate_failed_full_not_started')
            self.assertEqual(len(report['phases']),1)

    def test_child_finalization_failure_keeps_location_and_unknown_accounting(self):
        def fail(key,**kwargs):
            path=run.ROOT/'runs/offline-finalization-fixture';path.mkdir(parents=True)
            kwargs['on_start'](path)
            (path/'summary.json').write_text(json.dumps({'attempts':[{'retry':False}]}), encoding='utf-8')
            raise OSError('synthetic report persistence failure after dispatch')
        with tempfile.TemporaryDirectory() as tmp,patch.object(run,'ROOT',Path(tmp)),patch.object(run,'execute',side_effect=fail):
            report,_=staged.execute_staged(FAKE_KEY,offline=True,progress=lambda _:None)
            self.assertIsNone(report['total_physical_attempts'])
            self.assertFalse(report['accounting_complete'])
            self.assertEqual(report['known_physical_attempts_lower_bound'],1)
            self.assertEqual(report['phases'][0]['run_id'],'offline-finalization-fixture')
            self.assertEqual(len(report['phases']),1)

    def test_auth_failure_stops_after_one_attempt(self):
        with tempfile.TemporaryDirectory() as tmp,patch.object(run,'ROOT',Path(tmp)),fixture(handler=lambda b,n:(401,{})) as (target,calls):
            report,_=staged.execute_staged(FAKE_KEY,offline=True,target=target,progress=lambda _:None)
            self.assertEqual(len(calls),1)
            self.assertEqual(report['status'],'targeted_gate_failed_full_not_started')

if __name__=='__main__':unittest.main()
