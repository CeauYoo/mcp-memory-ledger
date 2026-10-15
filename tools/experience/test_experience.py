"""Synthetic-only offline regression. External provider credentials are never used."""
import contextlib
import copy
import hashlib
import http.server
import io
import json
import os
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import Mock,patch
import urllib.error

import run as app
import protocol
import network
import reporting
import ledger
import staged
from catalog import SCENARIOS,background,validate_catalog
from local_records import body_record,filtered_headers

FAKE_KEY='synthetic-only-credential-30-cases-X9r7'

def memory_from_request(body):
    text=body['input'][0]['content'][0]['text']
    return json.loads(text.split('当前可访问MCP记忆：\n',1)[1].split('\n本轮审计字段：',1)[0])

def response_for(body,item,ordinal=1,model='gpt-6-luna'):
    memory=memory_from_request(body)
    value={'answer':f'这是第{ordinal}轮的离线合成回答，用于验证实际前轮文本传递；不能算真实模型表现。',
           'decisions':item['expected'],'memory_audit':[{k:v for k,v in r.items() if k!='evidence'} for r in memory['records']],
           'uncertainties':[]}
    return {'status':'completed','model':model,'output':[{'type':'message','role':'assistant','status':'completed',
        'content':[{'type':'output_text','text':json.dumps(value,ensure_ascii=False)}]}],
        'usage':{'input_tokens':700,'output_tokens':160,'total_tokens':860,'output_tokens_details':{'reasoning_tokens':30}},
        'extra_local_evidence':{'keep':'full-fixture-marker','echo':FAKE_KEY}}

@contextlib.contextmanager
def fixture(items=None,handler=None):
    calls=[]
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self,*args):pass
        def do_POST(self):
            body=json.loads(self.rfile.read(int(self.headers['Content-Length'])));calls.append(body)
            if handler:status,value=handler(body,len(calls))
            else:status,value=200,response_for(body,items[len(calls)-1],len(calls))
            data=value if isinstance(value,bytes) else json.dumps(value,ensure_ascii=False).encode()
            self.send_response(status);self.send_header('Content-Length',str(len(data)))
            self.send_header('Set-Cookie','synthetic-private-cookie');self.end_headers();self.wfile.write(data)
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    try:yield f'http://127.0.0.1:{server.server_port}/v1/responses',calls
    finally:server.shutdown();server.server_close();thread.join()

def offline_execution(scenarios=SCENARIOS,handler=None):
    items=[t for s in scenarios for t in s['turns']]
    with fixture(items,handler) as (target,calls):
        report,folder=app.execute(FAKE_KEY,offline=True,target=target,scenarios=scenarios,progress=lambda _:None)
    return report,folder,calls

def rehearse():
    report,folder,calls=offline_execution()
    if len(calls)!=90 or report['status']!='completed_review_required':raise RuntimeError('offline_90_rounds_failed')
    if report['totals']['turn_status_counts']!={'objective_checks_passed_review_required':90}:raise RuntimeError('offline_objective_checks_failed')
    print('30场景/90轮离线彩排完成；真实MCP存储与合成模型编排通过。真实模型和自然答复质量未验证。')
    print('本机报告：'+str(folder/'report.html'))

class PureTests(unittest.TestCase):
    def test_controlled_decision_equivalences_and_contradictions(self):
        accepted=[('meal','纯素套餐（不含肉、蛋、奶）','纯素套餐'),
                  ('route','路线B','B'),('choice','供应商A','A'),('choice','方案C','C'),
                  ('tokyo_datetime','2026-11-11T00:30:00+09:00','2026-11-11T00:30')]
        rejected=[('meal','纯素套餐（含奶）','纯素套餐'),('meal','纯素套餐，但可以加蛋','纯素套餐'),
                  ('route','路线B或A','B'),('route','路线A','B'),('choice','供应商AB','A'),
                  ('choice','供应商A不符合','A'),('address','路线B','B'),
                  ('tokyo_datetime','2026-11-11T00:30:00+08:00','2026-11-11T00:30'),
                  ('tokyo_datetime','2026-11-10T00:30:00+09:00','2026-11-11T00:30'),
                  ('tokyo_datetime','2026-11-11T00:30:01+09:00','2026-11-11T00:30'),
                  ('tokyo_datetime','2026-11-31T00:30:00+09:00','2026-11-11T00:30')]
        for key,actual,expected in accepted:
            with self.subTest(key=key,actual=actual):self.assertTrue(protocol.decision_matches(key,actual,expected))
        for key,actual,expected in rejected:
            with self.subTest(key=key,actual=actual):self.assertFalse(protocol.decision_matches(key,actual,expected))

    def test_equivalent_decision_does_not_relax_claim_source_matching(self):
        record={'id':'claim:one','predicate':'route','value':'B','status':'Active','namespace':'project/a','source_event_ids':['event:one']}
        other={**record,'id':'claim:two','predicate':'date','value':'2026-11-11','source_event_ids':['event:two']}
        value={'answer':'请选择路线B。','decisions':{'route':'路线B'},'memory_audit':[copy.deepcopy(record)]}
        item={'expected':{'route':'B'}};memory={'records':[record,other]}
        result=protocol.evaluate(json.dumps(value),item,memory)
        self.assertEqual(result['status'],'objective_checks_passed_review_required')
        self.assertEqual(result['equivalent_decision_fields'],['route'])
        for field,bad in [('value','路线B'),('source_event_ids',['event:two'])]:
            altered=copy.deepcopy(value);altered['memory_audit'][0][field]=bad
            result=protocol.evaluate(json.dumps(altered),item,memory)
            self.assertTrue(result['checks']['decision:route'])
            self.assertFalse(result['checks']['cited_values_units_scope_status_and_source_match'])
            self.assertEqual(result['status'],'objective_check_failed')

    def test_catalog_distribution_and_measured_context(self):
        stats=validate_catalog();self.assertEqual(stats['scenarios'],30);self.assertEqual(stats['logical_requests'],90)
        self.assertEqual(len(stats['categories']),7)
        for s in SCENARIOS:
            doc=background(s);self.assertEqual(len(doc),s.get('context_chars',0))
            if len(doc)>=12000:
                for word in ('版本差异','历史回溯','相似实体','候选报价','退回意见','2026-','初始正式摘要'):
                    self.assertIn(word,doc)
        self.assertEqual(stats['output_caps'],[2304,4096,4608])

    def test_scoring_values_not_leaked_and_no_unified_old_input_cap(self):
        item=SCENARIOS[0]['turns'][0];modified=copy.deepcopy(item);modified['expected']['remaining_cny']=-9999
        memory={'namespace':'project/test','records':[]}
        first,metrics=protocol.request([],item['user'],memory,list(item['expected']),background(SCENARIOS[-1]),4608)
        second,_=protocol.request([],modified['user'],memory,list(modified['expected']),background(SCENARIOS[-1]),4608)
        self.assertEqual(first,second);self.assertNotIn('-9999',json.dumps(first))
        self.assertGreater(metrics['input_json_bytes'],8192);self.assertEqual(metrics['background_characters'],30000)
        self.assertFalse(metrics['context_silently_truncated']);self.assertEqual(first['max_output_tokens'],4608)

    def test_objective_pass_never_passes_natural_language(self):
        item={'expected':{'total':10}};memory={'namespace':'project/test','records':[]}
        text=json.dumps({'answer':'故意错误的正文：总额一百万元。','decisions':{'total':10},'memory_audit':[]})
        result=protocol.evaluate(text,item,memory)
        self.assertEqual(result['status'],'objective_checks_passed_review_required')
        self.assertEqual(result['natural_language_review'],'needs_human_review');self.assertFalse(result['user_experience_passed'])

    def test_typed_values_and_evidence_scope_status_not_just_id(self):
        record={'id':'claim:one','predicate':'budget_cny','value':100,'status':'Active','namespace':'project/a','source_event_ids':['event:one']}
        memory={'namespace':'project/a','records':[record]};item={'expected':{'budget_cny':100}}
        value={'answer':'一百元预算','decisions':{'budget_cny':100},'memory_audit':[record]}
        self.assertEqual(protocol.evaluate(json.dumps(value),item,memory)['status'],'objective_checks_passed_review_required')
        for field,replacement in [('value',101),('namespace','project/b'),('status','Superseded'),('predicate','budget_usd'),('source_event_ids',[])]:
            altered=copy.deepcopy(value);altered['memory_audit'][0][field]=replacement
            self.assertEqual(protocol.evaluate(json.dumps(altered),item,memory)['status'],'objective_check_failed')
        value['decisions']['budget_cny']=True
        self.assertEqual(protocol.evaluate(json.dumps(value),item,memory)['status'],'objective_check_failed')

    def test_partial_response_and_model_alias_are_different_outcomes(self):
        body={'status':'incomplete','model':'gpt-6-luna','usage':{'input_tokens':7,'output_tokens':8}}
        meta,text=protocol.project(body,200);self.assertEqual(meta['protocol_status'],'output_incomplete');self.assertIsNone(text)
        self.assertTrue(meta['model_label_warning']);self.assertFalse(meta['model_identity_authenticated'])
        self.assertEqual(meta['usage']['total_tokens'],None)

    def test_unknown_target_does_not_require_irrelevant_citations(self):
        record={'id':'one','predicate':'coffee','value':'tea','status':'Active','namespace':'project/a','source_event_ids':['event:one']}
        text=json.dumps({'answer':'没有预订号记录，无法确定。','decisions':{'booking_reference':None},'memory_audit':[]})
        result=protocol.evaluate(text,{'expected':{'booking_reference':None}},{'records':[record]})
        self.assertEqual(result['status'],'objective_checks_passed_review_required')

    def test_conflict_requires_citing_both_active_values(self):
        a={'id':'one','predicate':'warehouse','value':'north','status':'Active','namespace':'project/a','source_event_ids':['event:one']}
        b={**a,'id':'two','value':'south','source_event_ids':['event:two']}
        value={'answer':'需要澄清。','decisions':{'warehouse':None,'needs_clarification':True},'memory_audit':[a]}
        expected={'expected':{'warehouse':None,'needs_clarification':True}}
        self.assertEqual(protocol.evaluate(json.dumps(value),expected,{'records':[a,b]})['status'],'objective_check_failed')
        value['memory_audit'].append(b)
        self.assertEqual(protocol.evaluate(json.dumps(value),expected,{'records':[a,b]})['status'],'objective_checks_passed_review_required')

    def test_full_record_redaction_and_header_credentials(self):
        record=body_record(json.dumps({'message':FAKE_KEY,'keep':'full output'}).encode(),[FAKE_KEY],True)
        self.assertNotIn(FAKE_KEY,json.dumps(record));self.assertIn('full output',record['body_text_utf8'])
        headers=filtered_headers([('Authorization','Bearer '+FAKE_KEY),('Set-Cookie','secret'),('X-Debug',FAKE_KEY)],[FAKE_KEY])
        self.assertNotIn(FAKE_KEY,json.dumps(headers));self.assertNotIn('secret',json.dumps(headers))

    def test_model_credential_fragment_never_enters_safe_metadata(self):
        key='ABCdef1234567890qwerty'
        meta,_=protocol.project({'model':'gpt-'+key[3:15],'status':'completed','output':[]},200,[key])
        self.assertIsNone(meta['response_model']);self.assertNotIn(key[3:15],json.dumps(meta))

    def test_malformed_deep_and_huge_outputs_are_results_not_exceptions(self):
        meta,text=protocol.project({'status':'completed','output':[{'type':'message','role':'assistant','content':None}],
            'usage':{'input_tokens':10**1000}},200)
        self.assertEqual(meta['protocol_status'],'invalid_response');self.assertIsNone(text)
        self.assertIsNone(meta['usage']['input_tokens'])
        result=protocol.evaluate('['*2000+'0'+']'*2000,{'expected':{'x':1}},{'records':[]})
        self.assertEqual(result['status'],'invalid_audit_format')
        value={'answer':'test','decisions':{'x':1},'memory_audit':[{'id':[]}]}
        self.assertEqual(protocol.evaluate(json.dumps(value),{'expected':{'x':1}},{'records':[]})['status'],'objective_check_failed')

    def test_written_claim_is_checked_against_user_fixture_not_just_id(self):
        draft={'owner':'World','namespace':'project/a','subject':'scenario_memory','predicate':'budget_cny','object':'100','mode':'Observed'}
        record={**draft,'id':'unchanged-id','status':'Active','provenance':{'evidence_event_references':['event:one']}}
        ledger.validate_written(record,draft,'event:one')
        for field,bad in [('object','999'),('object','true'),('predicate','wrong'),('namespace','project/b'),('status','Superseded')]:
            changed={**record,field:bad}
            with self.assertRaises(RuntimeError):ledger.validate_written(changed,draft,'event:one')

    def test_inconsistent_completion_and_mixed_refusal_are_not_complete_answers(self):
        obj={'status':'completed','output':[{'type':'message','role':'assistant','content':[{'type':'output_text','text':'otherwise valid'}]}]}
        obj['incomplete_details']={'reason':'max_output_tokens'}
        meta,text=protocol.project(obj,200);self.assertEqual(meta['protocol_status'],'output_incomplete');self.assertIsNone(text)
        del obj['incomplete_details'];obj['output'][0]['content'].append({'type':'refusal','refusal':'declined'})
        meta,text=protocol.project(obj,200);self.assertEqual(meta['protocol_status'],'provider_refusal');self.assertIsNone(text)


class IntegrationTests(unittest.TestCase):
    def test_all_30_scenarios_90_actual_mcp_rounds_and_history(self):
        with tempfile.TemporaryDirectory() as temp,patch.object(app,'ROOT',Path(temp)):
            report,folder,calls=offline_execution()
            self.assertEqual(len(calls),90);self.assertEqual(report['status'],'completed_review_required')
            self.assertEqual(report['totals']['turn_status_counts'],{'objective_checks_passed_review_required':90})
            self.assertEqual(report['totals']['response_model_labels'],{'gpt-6-luna':90})
            self.assertFalse(report['totals']['user_experience_passed'])
            self.assertEqual(len(list((folder/'diagnostics').glob('call-*.json'))),180)
            self.assertEqual(len(list((folder/'turns').glob('*.json'))),90)
            self.assertEqual(json.loads((folder/'summary.json').read_text(encoding='utf-8')),report)
            for case,scenario in zip(report['scenarios'],SCENARIOS):
                for row,item in zip(case['turns'],scenario['turns']):
                    local=json.loads((folder/row['local_record']).read_text(encoding='utf-8'));body=local['request']
                    self.assertEqual(row['storage']['status'],'passed')
                    self.assertFalse(row['model']['model_identity_authenticated'])
                    if scenario.get('project_names'):
                        selected='foreign' if item.get('namespace')=='foreign' else 'base'
                        names=scenario['project_names'];memory=local['memory'];binding=memory['namespace_binding']
                        self.assertEqual(binding,{'namespace':memory['namespace'],'project_name':names[selected],
                            'source':'synthetic_fixture_scope_selection'})
                        self.assertNotIn(names['base' if selected=='foreign' else 'foreign'],json.dumps(memory,ensure_ascii=False))
                        if selected=='foreign':
                            self.assertTrue(all(names[selected] in e['summary'] for r in memory['records'] for e in r['evidence']))
                    if scenario['id']=='S16' and row['number']==1:
                        self.assertIn('2026-10-14',local['user'])
                        self.assertTrue(any(r['predicate']=='planning_start_date' and r['value']=='2026-10-14' for r in local['memory']['records']))
                    if item.get('restart'):
                        self.assertEqual(row['context']['history_messages'],0)
                        self.assertEqual(row['context']['background_characters'],0)
                        self.assertEqual([x['role'] for x in body['input']],['developer','user'])
                    elif row['number']>1:
                        prev=json.loads((folder/'turns'/f'{case["id"]}-{row["number"]-1:02d}.json').read_text(encoding='utf-8'))
                        assistant=[m['content'][0]['text'] for m in body['input'] if m['role']=='assistant']
                        self.assertIn(prev['assistant_text'],assistant)
                    for forbidden in scenario.get('forbidden_text',[]):self.assertNotIn(forbidden,json.dumps(body))
            for path in (folder/'diagnostics').glob('call-*.json'):
                content=path.read_text(encoding='utf-8');self.assertNotIn(FAKE_KEY,content);self.assertNotIn('synthetic-private-cookie',content)
                # Windows chmod only controls the read-only flag, not POSIX ACLs.
                if os.name=='nt':self.assertTrue(path.stat().st_mode & 0o200)
                else:self.assertEqual(path.stat().st_mode & 0o777,0o600)
            for name,digest in report['code_sha256'].items():
                self.assertEqual(hashlib.sha256((folder/'code'/name).read_bytes()).hexdigest(),digest)
            self.assertIn('待人工复核',(folder/'report.html').read_text(encoding='utf-8'))

    def test_bad_answer_is_carried_and_later_rounds_continue(self):
        items=SCENARIOS[0]['turns']
        def respond(body,n):
            if n==1:return 200,{'status':'completed','model':'gpt-6-luna','output':[{'type':'message','role':'assistant','content':[{'type':'output_text','text':'这是错误而未按审计格式输出的真实前轮替身。'}]}]}
            return 200,response_for(body,items[n-1],n)
        with tempfile.TemporaryDirectory() as temp,patch.object(app,'ROOT',Path(temp)):
            report,folder,calls=offline_execution([SCENARIOS[0]],respond)
            self.assertEqual(len(calls),3);self.assertEqual(report['scenarios'][0]['turns'][0]['status'],'invalid_audit_format')
            self.assertIn('这是错误而未按审计格式输出',calls[1]['input'][2]['content'][0]['text'])
            self.assertEqual(report['status'],'completed_with_failures_review_required')

    def test_auth_failure_stops_other_rounds_and_cases(self):
        with tempfile.TemporaryDirectory() as temp,patch.object(app,'ROOT',Path(temp)):
            report,folder,calls=offline_execution(SCENARIOS[:2],lambda body,n:(401,{'error':{'code':'invalid_api_key'}}))
            self.assertEqual(len(calls),1);self.assertEqual(report['status'],'authentication_or_authorization_stop')
            self.assertEqual(report['totals']['turn_status_counts'],{'http_error':1,'not_run':5})

    def test_wrapped_authentication_errors_stop_without_more_requests(self):
        for status in (200,400):
            with tempfile.TemporaryDirectory() as temp,patch.object(app,'ROOT',Path(temp)):
                report,folder,calls=offline_execution([SCENARIOS[0]],lambda body,n:(status,{'error':{'code':'invalid_api_key'}}))
                self.assertEqual(len(calls),1);self.assertEqual(report['status'],'authentication_or_authorization_stop')
                self.assertEqual(report['totals']['turn_status_counts']['not_run'],2)
        self.assertIsNone(network.error_kind(400,{'error':{'code':'invalid_request'}}))

    def test_storage_failure_blocks_only_dependent_scenario(self):
        original=app.Ledger.turn
        def broken(ledger,scenario,item):
            if scenario['id']=='S01':raise RuntimeError('synthetic storage failure')
            return original(ledger,scenario,item)
        with tempfile.TemporaryDirectory() as temp,patch.object(app,'ROOT',Path(temp)),patch.object(app.Ledger,'turn',broken):
            items=SCENARIOS[1]['turns']
            report,folder,calls=offline_execution(SCENARIOS[:2],lambda body,n:(200,response_for(body,items[n-1],n)))
            self.assertEqual(len(calls),3)
            self.assertEqual([r['status'] for r in report['scenarios'][0]['turns']],['storage_failed','blocked_storage_dependency','blocked_storage_dependency'])
            self.assertTrue(all(r['storage']['status']=='passed' for r in report['scenarios'][1]['turns']))

    def test_malformed_provider_and_audit_do_not_stop_later_rounds(self):
        items=SCENARIOS[0]['turns']
        def respond(body,n):
            value=response_for(body,items[n-1],n)
            if n==1:value['output'][0]['content']=None
            if n==2:
                audit=json.loads(value['output'][0]['content'][0]['text']);audit['memory_audit']=[{'id':[]}]
                value['output'][0]['content'][0]['text']=json.dumps(audit)
            return 200,value
        with tempfile.TemporaryDirectory() as temp,patch.object(app,'ROOT',Path(temp)):
            report,folder,calls=offline_execution([SCENARIOS[0]],respond)
            self.assertEqual(len(calls),3)
            self.assertEqual([t['status'] for t in report['scenarios'][0]['turns']],
                ['invalid_response','objective_check_failed','objective_checks_passed_review_required'])


class NetworkTests(unittest.TestCase):
    def client(self,folder,target,fail_reserve=False,fail_finish=False):
        attempts=[]
        def reserve(meta):
            if fail_reserve:raise OSError('synthetic disk fail')
            if len(attempts)>=100:raise network.BudgetExhausted()
            attempts.append(dict(meta));return len(attempts)
        def finish(meta):
            if fail_finish:raise OSError('synthetic disk fail')
            attempts[meta['call_number']-1].update(meta)
        return network.Provider(folder,{},FAKE_KEY,reserve,finish,target,sleep=lambda _:None),attempts

    def test_explicit_network_retry_budget_is_ten_and_total_is_one_hundred(self):
        with tempfile.TemporaryDirectory() as temp:
            client,attempts=self.client(Path(temp),'http://127.0.0.1:1/v1/responses')
            client.opener.open=Mock(side_effect=urllib.error.URLError('synthetic failure'))
            for number in range(90):client.call({'model':protocol.MODEL},f'turn-{number}')
            self.assertEqual(len(attempts),100);self.assertEqual(client.retry_count,10)
            self.assertEqual(client.opener.open.call_count,100)
            self.assertEqual(len(list(Path(temp).rglob('call-*.request.json'))),100)
            with self.assertRaises(network.BudgetExhausted):client.call({},'over-budget')
            self.assertEqual(client.opener.open.call_count,100)

    def test_retry_has_independent_records_then_quota_stops_without_retry(self):
        responses=[(503,{'error':{'type':'temporary'}}),(200,{'status':'completed','model':'gpt-6-luna','output':[]}),
                   (429,{'error':{'code':'insufficient_quota'}})]
        with tempfile.TemporaryDirectory() as temp,fixture(handler=lambda b,n:responses[n-1]) as (target,calls):
            client,attempts=self.client(Path(temp),target)
            result=client.call({},'round-1');self.assertEqual(len(calls),2)
            self.assertEqual(attempts[0]['logical_request_id'],attempts[1]['logical_request_id'])
            self.assertEqual([a['attempt_in_turn'] for a in attempts],[1,2])
            result=client.call({},'round-2');self.assertEqual(result['fatal'],'quota_stop')
            self.assertEqual(len(calls),3);self.assertFalse(result['metadata']['will_retry'])

    def test_reservation_failure_prevents_dispatch(self):
        with tempfile.TemporaryDirectory() as temp:
            client,_=self.client(Path(temp),'http://127.0.0.1:1',fail_reserve=True);client.opener.open=Mock()
            with self.assertRaises(network.RecordingFailure):client.call({},'round')
            client.opener.open.assert_not_called()

    def test_failed_attempt_summary_prevents_retry(self):
        with tempfile.TemporaryDirectory() as temp,fixture(handler=lambda b,n:(503,{})) as (target,calls):
            client,_=self.client(Path(temp),target,fail_finish=True)
            with self.assertRaises(network.RecordingFailure):client.call({},'round')
            self.assertEqual(len(calls),1)

    def test_redirect_is_not_followed_and_no_unknown_cost_is_zero(self):
        self.assertIsNone(network.NoRedirect().redirect_request(None,None,302,None,None,'https://not-used.invalid'))
        totals=reporting.summarize({'scenarios':[],'attempts':[{'attempt_in_turn':1}]})
        self.assertIsNone(totals['usage']['input_tokens']['known_sum'])
        self.assertEqual(totals['usage']['input_tokens']['unknown_attempts'],1)
        self.assertIsNone(totals['actual_total_cost_usd'])


class EntryTests(unittest.TestCase):
    def test_one_hidden_key_and_no_other_input_starts(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);out=io.StringIO()
            with patch.object(app,'ROOT',root),patch.object(app.sys.stdin,'isatty',return_value=True), \
                 contextlib.redirect_stdout(out),patch.object(out,'isatty',return_value=True), \
                 patch.object(app.getpass,'getpass',return_value=FAKE_KEY) as key, \
                 patch('builtins.input',side_effect=AssertionError('No extra prompts allowed')), \
                 patch.object(staged,'execute_staged',return_value=({'status':'synthetic'},root)) as run, \
                 patch.object(app,'LiveMonitor'),patch.object(app.webbrowser,'open'):
                app.interactive()
            self.assertEqual(key.call_count,1);self.assertEqual(run.call_count,1)
            self.assertNotIn(FAKE_KEY,out.getvalue());self.assertFalse((root/'runs/active-live.lock').exists())
            self.assertIn('不超过112次',out.getvalue())

    def test_non_terminal_cannot_request_credentials(self):
        with patch.object(app.sys.stdin,'isatty',return_value=False),patch.object(app.getpass,'getpass') as key:
            with self.assertRaises(RuntimeError):app.interactive()
            key.assert_not_called()

    def test_concurrent_lock_prevents_second_key_prompt(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);(root/'runs').mkdir();lock=root/'runs/active-live.lock';lock.write_text('existing', encoding='utf-8')
            out=io.StringIO()
            with patch.object(app,'ROOT',root),patch.object(app.sys.stdin,'isatty',return_value=True), \
                 contextlib.redirect_stdout(out),patch.object(out,'isatty',return_value=True),patch.object(app.getpass,'getpass') as key:
                app.interactive()
            key.assert_not_called();self.assertEqual(lock.read_text(encoding='utf-8'),'existing')

if __name__=='__main__':unittest.main()
