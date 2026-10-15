#!/usr/bin/env python3
"""One user-entered key starts the bounded 30-scenario conversation replay."""
import argparse
import getpass
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid
import warnings
import webbrowser

from catalog import SCENARIOS,background,validate_catalog
from ledger import Ledger
from local_records import capture_code,write_new
from network import Provider,MAX_ATTEMPTS,MAX_RETRIES,MAX_INPUT_BYTES,RecordingFailure,BudgetExhausted,InputLimitExceeded,now
from protocol import MODEL,TARGET,request,message,evaluate
from reporting import summarize,render
from live_monitor import LiveMonitor
import support

ROOT=Path(__file__).resolve().parents[2]/'target/experience-acceptance'
TARGETED_IDS=('S16','S19','S23','S25')

def execute(key,*,offline=False,target=TARGET,scenarios=None,progress=print,
            phase='full',attempt_cap=MAX_ATTEMPTS,retry_limit=MAX_RETRIES,on_start=None,observer=None):
    selected=SCENARIOS if scenarios is None else scenarios
    canonical=SCENARIOS if phase=='full' else [s for s in SCENARIOS if s['id'] in TARGETED_IDS]
    if phase not in ('full','targeted') or not 1<=attempt_cap<=MAX_ATTEMPTS or not 0<=retry_limit<=MAX_RETRIES:
        raise RuntimeError('invalid_execution_limits')
    if not offline and (target!=TARGET or selected!=canonical):raise RuntimeError('live_contract_mismatch')
    catalog=validate_catalog();support.verify_binary()
    output=ROOT/'runs';output.mkdir(parents=True,exist_ok=True,mode=0o700)
    folder=output/(('offline-' if offline else 'live-')+uuid.uuid4().hex[:12]);folder.mkdir(mode=0o700)
    if on_start:on_start(folder)
    hashes=capture_code(folder)
    write_new(folder/'scenario-manifest.json',{'scenarios':selected,'catalog':catalog,'code_sha256':hashes})
    report={'run_id':folder.name,'mode':'offline_provider_fixture' if offline else 'real_provider_user_started',
        'status':'running','started_utc':now(),'finished_utc':None,'requested_model':MODEL,
        'target':'loopback_fixture' if offline else TARGET,'code_sha256':hashes,'binary':support.verify_binary(),
        'planned_scenarios':len(selected),'planned_logical_requests':sum(len(s['turns']) for s in selected),
        'phase':phase,'maximum_physical_attempts':attempt_cap,'maximum_retries':retry_limit,
        'maximum_serialized_input_bytes':MAX_INPUT_BYTES,
        'service_provider_total_budget_usd':5,'software_enforced_usd_cap':False,
        'model_identity_authenticated':False,'real_codex_app_client_verified':False,'new_machine_verified':False,
        'memory_write_mode':'explicit_user_facts_predeclared_by_fixture','model_memory_extraction_verified':False,
        'attempts':[],'scenarios':[{'id':s['id'],'title':s['title'],'category':s['category'],
          'turns':[{'number':i+1,'status':'not_run'} for i in range(len(s['turns']))]} for s in selected]}
    def save():
        report['totals']=summarize(report);support.save(folder/'summary.json',report)
        support.save(output/('latest-offline-summary.json' if offline else 'latest-live-summary.json'),report)
        if observer:observer('run',report)
    def reserve(metadata):
        if len(report['attempts'])>=attempt_cap:raise BudgetExhausted('physical_budget_exhausted')
        report['attempts'].append({**metadata,'call_number':len(report['attempts'])+1,
                                  'dispatch_state':'reserved_possible_charge','reserved_utc':now()})
        save()  # Durable reservation before any external dispatch, including each retry.
        return len(report['attempts'])
    def finish(metadata):
        previous=report['attempts'][metadata['call_number']-1]
        previous.update(metadata,dispatch_state='attempt_finished');save()
    save();provider=Provider(folder,hashes,key,reserve,finish,target,sleep=(lambda _:None) if offline else __import__('time').sleep,
        retry_limit=retry_limit)
    active=None
    try:
        ledger=Ledger(folder)
        for scenario,case in zip(selected,report['scenarios']):
            history=[];previous_ns=None;storage_failed=False
            for index,(item,row) in enumerate(zip(scenario['turns'],case['turns']),1):
                active=row
                if storage_failed:row['status']='blocked_storage_dependency';save();continue
                row['status']='preparing_storage';save()
                try:memory,storage=ledger.turn(scenario,item)
                except Exception:
                    row.update(status='storage_failed',storage={'status':'failed','error_detail':'local_storage_check_failed'})
                    if phase=='targeted':report['status']='targeted_storage_failed_stop';return report,folder
                    storage_failed=True;save();continue
                ns=memory['namespace'];reset=item.get('restart') or (previous_ns is not None and ns!=previous_ns)
                if reset:history=[]
                previous_ns=ns
                document='' if reset else background(scenario)
                body,metrics=request(history,item['user'],memory,list(item['expected']),document,scenario['output_tokens'])
                row.update(status='calling_model',storage=storage,context=metrics,history_reset=bool(reset),
                    history_reset_reason='explicit_new_session_or_project_change' if reset else None)
                save();result=provider.call(body,f'{scenario["id"]}-{index:02d}')
                meta=result['metadata'];text=result['text']
                assessment=evaluate(text,item,memory,scenario.get('forbidden_text',())) if text else None
                row.update(status=assessment['status'] if assessment else meta['protocol_status'],
                    model=meta,assessment=assessment,local_record=f'turns/{scenario["id"]}-{index:02d}.json')
                write_new(folder/row['local_record'],{'scenario_id':scenario['id'],'turn':index,'user':item['user'],
                    'request':body,'assistant_text':text,'memory':memory,'expected':item['expected'],
                    'assessment':assessment,'metadata':meta,'context':metrics,'code_sha256':hashes,
                    'local_only':True,'natural_language_review':'needs_human_review'})
                # Carry actual returned model text, even an objectively wrong answer. Never inject expected answers.
                history.append(message('user',item['user']))
                if text is not None:history.append(message('assistant',text))
                else:history.append(message('developer','上一轮没有收到完整可用回答；不要假设助手已经给出了任何结论。'))
                save();progress(f'{scenario["id"]} {index}/3 {scenario["title"]}：{row["status"]} · {meta["latency_seconds"]:.1f}s')
                if result['fatal']:
                    report['status']=result['fatal'];return report,folder
                if phase=='targeted' and row['status']!='objective_checks_passed_review_required':
                    report['status']='targeted_check_failed_stop';return report,folder
        report['status']='completed_review_required'
        if any(t['status']!='objective_checks_passed_review_required' for c in report['scenarios'] for t in c['turns']):
            report['status']='completed_with_failures_review_required'
    except (KeyboardInterrupt,EOFError):
        report['status']='interrupted'
        if active and active['status'] in ('calling_model','preparing_storage'):active['status']='interrupted'
    except (RecordingFailure,OSError):
        report['status']='recording_failed_stop'
        if active:active['status']='recording_failed_stop'
    except BudgetExhausted:
        report['status']='physical_budget_exhausted'
        if active:active['status']='not_dispatched_budget_exhausted'
    except InputLimitExceeded:
        report['status']='input_limit_exceeded_stop'
        if active:active['status']='not_dispatched_input_limit_exceeded'
    except Exception:
        report['status']='local_execution_failed_stop'
        if active:active['status']='local_execution_failed_stop'
    finally:
        provider.key=None;report['finished_utc']=now();save();render(folder,report)
    return report,folder

def interactive():
    os.umask(0o077)
    if not sys.stdin.isatty() or not sys.stdout.isatty():raise RuntimeError('interactive_terminal_required')
    validate_catalog();support.verify_binary()
    print('Memory Ledger · 定向后全量复测')
    print('站点：'+TARGET+'\n请求模型：'+MODEL)
    print('先S16/S19/S23/S25各3轮（12次）；自动检查无失败才继续30场景×3轮（90次）。')
    print('定向阶段首次失败即停；不会自动重跑整个阶段。')
    print('两阶段正常102次；共享最多10次显式重试，总发送不超过112次。')
    print('短/中/长上下文；长档1.2万—3万字符；输出按任务2304—4608 tokens。')
    print('每次完整JSON输入最多256KiB，超限停止，不静默裁剪。')
    print('使用你的服务商5美元总额度（含先前测试消耗）。单价未知，脚本不能保证美元封顶。')
    print('只需输入一次key即自动开始。模型名差异只记警告；结果、usage和完整对话仅存本机。')
    print('可随时Ctrl+C停止；已发送请求可能计费。开始后自动打开实时监控，完整报告保存在本机。')
    output=ROOT/'runs';output.mkdir(parents=True,exist_ok=True,mode=0o700);lock=output/'active-live.lock'
    try:write_new(lock,{'created_utc':now(),'purpose':'prevent_concurrent_user_runs'})
    except FileExistsError:
        print('已有运行或未正常结束的运行锁；本次不会索取key或发送请求。请核对上次本机报告。');return
    try:
        with warnings.catch_warnings():
            warnings.simplefilter('error',getpass.GetPassWarning)
            key=getpass.getpass('API key（隐藏；输入后开始，留空取消）：').strip()
        if not key:return
        if any(c in key for c in '\r\n'):raise RuntimeError('invalid_key_input')
        from staged import execute_staged
        with LiveMonitor() as monitor:
            print('实时监控：'+monitor.url+'（自动分配端口，仅本机）')
            webbrowser.open(monitor.url)
            try:report,folder=execute_staged(key,observer=monitor.publish)
            finally:key=None
            print('结束：'+report['status']+'\n本机报告：'+str(folder/'report.html'))
            print('安全摘要：'+str(folder/'summary.json'))
            lock.unlink(missing_ok=True)
            print('实时监控保留最终状态；按Ctrl+C关闭监控。此时不会再发送模型请求。')
            monitor.hold()
    finally:lock.unlink(missing_ok=True)

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--rehearse',action='store_true');args=parser.parse_args()
    try:
        if args.rehearse:
            from test_experience import rehearse
            rehearse()
        else:interactive()
    except (KeyboardInterrupt,EOFError):print('已停止；已产生记录保留，不自动恢复或重跑。')
    except Exception:
        print('运行已停止。请查看本机安全摘要；原始错误及凭据不输出到终端。');sys.exit(1)
