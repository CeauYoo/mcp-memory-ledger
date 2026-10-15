"""Sequential targeted/full orchestration; the user enters one key, never stored."""
import html
import json
from pathlib import Path
import uuid
import run
from catalog import SCENARIOS
from local_records import write_new
from network import now
from protocol import TARGET
import support

MAX_TOTAL_ATTEMPTS=112
MAX_TOTAL_RETRIES=10

def targeted_scenarios():return [s for s in SCENARIOS if s['id'] in run.TARGETED_IDS]

def can_expand(report):
    rows=[t for s in report['scenarios'] for t in s['turns']]
    return (report['status']=='completed_review_required' and len(rows)==12
            and all(t['status']=='objective_checks_passed_review_required' for t in rows))

def next_limits(reports,planned):
    attempts=[a for r in reports for a in r['attempts']]
    retries=sum(bool(a.get('retry')) for a in attempts)
    retry_limit=MAX_TOTAL_RETRIES-retries
    cap=min(planned+retry_limit,MAX_TOTAL_ATTEMPTS-len(attempts),100)
    if retry_limit<0 or cap<planned:raise RuntimeError('staged_budget_exhausted')
    return cap,retry_limit

def execute_staged(key,*,offline=False,target=TARGET,progress=print,observer=None):
    folder=run.ROOT/'staged'/('plan-'+uuid.uuid4().hex[:12]);folder.mkdir(parents=True,mode=0o700)
    report={'mode':'offline_staged_fixture' if offline else 'real_provider_user_started',
        'status':'running','started_utc':now(),'finished_utc':None,'phases':[],
        'targeted_rounds':12,'full_rounds':90,'maximum_total_attempts':MAX_TOTAL_ATTEMPTS,
        'maximum_total_retries':MAX_TOTAL_RETRIES,'software_enforced_usd_cap':False,
        'automatic_gate_is_not_semantic_acceptance':True,'human_semantic_review':'not_performed'}
    reports=[]
    def save():
        support.save(folder/'summary.json',report)
        if observer and report['status']!='running':observer('plan',report)
    write_new(folder/'plan.json',{'phase_order':['targeted','full'],'targeted_scenarios':run.TARGETED_IDS,
        'normal_requests':102,'maximum_physical_attempts':112,'maximum_shared_retries':10,
        'no_automatic_resume':True,'key_stored':False})
    save()
    try:
        for phase,selected in [('targeted',targeted_scenarios()),('full',SCENARIOS)]:
            cap,retries=next_limits(reports,sum(len(s['turns']) for s in selected))
            entry={'phase':phase,'run_id':None,'status':'starting',
                'physical_attempts':None,'explicit_retries':None,'known_physical_attempts':0}
            report['phases'].append(entry);save()
            def started(path):
                entry.update(run_id=path.name,status='running',report_relative_path=str(Path('../../runs')/path.name/'report.html'))
                save()  # Link the child before its first possible dispatch.
            result,path=run.execute(key,offline=offline,target=target,scenarios=selected,
                phase=phase,attempt_cap=cap,retry_limit=retries,progress=progress,on_start=started,observer=observer)
            reports.append(result)
            entry.update({'phase':phase,'run_id':path.name,'status':result['status'],
                'report_relative_path':str(Path('../../runs')/path.name/'report.html'),
                'physical_attempts':len(result['attempts']),
                'known_physical_attempts':len(result['attempts']),
                'explicit_retries':sum(bool(a.get('retry')) for a in result['attempts'])})
            save()
            if phase=='targeted' and not can_expand(result):
                report['status']='targeted_gate_failed_full_not_started';break
            if phase=='full':report['status']=result['status']
    except (Exception,KeyboardInterrupt):
        report['status']='interrupted_or_local_failure_no_resume'
        # A failure after dispatch must not turn unknown accounting into zero.
        for entry in report['phases']:
            if entry['physical_attempts'] is not None:continue
            entry['status']='unknown_after_local_failure'
            if entry['run_id']:
                try:
                    saved=json.loads((run.ROOT/'runs'/entry['run_id']/'summary.json').read_text(encoding='utf-8'))
                    entry['known_physical_attempts']=len(saved['attempts'])
                except (OSError,ValueError,KeyError,TypeError):pass
    finally:
        report['finished_utc']=now()
        complete=all(p['physical_attempts'] is not None for p in report['phases'])
        report['accounting_complete']=complete
        report['total_physical_attempts']=sum(p['physical_attempts'] for p in report['phases']) if complete else None
        report['total_explicit_retries']=sum(p['explicit_retries'] for p in report['phases']) if complete else None
        report['known_physical_attempts_lower_bound']=sum(p['known_physical_attempts'] for p in report['phases'])
        if report['status']=='running':report['status']='interrupted_or_local_failure_no_resume'
        save()
        rows=''.join('<li>'+html.escape(p['phase'])+'：'+html.escape(p['status'])+
            (' <a href="'+html.escape(p['report_relative_path'],quote=True)+'">本机详细报告</a>' if p.get('report_relative_path') else '')+'</li>' for p in report['phases'])
        (folder/'report.html').write_text('<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src \'none\'"><title>定向后全量复测</title><h1>定向后全量复测</h1><p>自动检查通过不代表正文语义验收通过。费用未知，不声称美元封顶。</p><ul>'+rows+'</ul><pre>'+html.escape(json.dumps(report,ensure_ascii=False,indent=2))+'</pre>', encoding='utf-8')
        (folder/'report.html').chmod(0o600)
    return report,folder
