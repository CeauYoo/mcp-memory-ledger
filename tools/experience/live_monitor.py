"""Loopback-only live acceptance status. Never serves transcripts or filesystem."""
import copy
from collections import Counter
import http.server
import json
import threading

PAGE='''<!doctype html><html lang="zh"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Memory Ledger 实时验收监控</title><style>body{font:16px/1.7 system-ui;max-width:1100px;margin:32px auto;padding:0 24px;color:#182c3c;background:#f4f7fa}h1{font-size:27px}section{background:white;border:1px solid #dbe3eb;border-radius:12px;padding:20px;margin:16px 0}table{width:100%;border-collapse:collapse;font-size:14px}td,th{text-align:left;padding:8px;border-bottom:1px solid #e5eaf0}pre{white-space:pre-wrap;overflow-wrap:anywhere}#connection{color:#356956}</style><h1>Memory Ledger · 实时验收监控</h1><p id="connection">连接中…</p><section><h2 id="status">等待用户启动</h2><p id="budget"></p><pre id="totals"></pre></section><section><h2>当前阶段逐轮状态</h2><table><thead><tr><th>场景</th><th>轮次</th><th>状态</th><th>延迟</th></tr></thead><tbody id="turns"></tbody></table></section><p>仅本机状态、用量和合成场景标题；不显示key、完整请求、回答或数据库。自动通过不代表语义验收通过。正式MCP运行监控仍由服务自身的dashboard提供。</p><script>
async function refresh(){try{const r=await fetch('/api/status',{cache:'no-store'});if(!r.ok)throw Error();const s=await r.json();document.getElementById('connection').textContent='实时连接 · '+new Date().toLocaleTimeString();document.getElementById('status').textContent=s.status;document.getElementById('budget').textContent='阶段：'+(s.phase||'待开始')+' · 当前阶段已预留 '+s.phase_attempts+' 次 · 全程已预留 '+(s.attempts==null?'未知':s.attempts)+' 次 · 两阶段最多112次/共享10次重试';document.getElementById('totals').textContent=JSON.stringify(s.totals||{},null,2);const rows=document.getElementById('turns');rows.replaceChildren();for(const t of s.turns||[]){const tr=document.createElement('tr');for(const v of [t.scenario+' '+t.title,t.number,t.status,t.latency_seconds==null?'—':t.latency_seconds+'s']){const td=document.createElement('td');td.textContent=v;tr.appendChild(td)}rows.appendChild(tr)}}catch(e){document.getElementById('connection').textContent='监控进程已关闭或连接暂时中断；当前画面不是最新状态。'}}refresh();setInterval(refresh,1000);
</script></html>'''


def aggregate_totals(phases):
    """Combine phase summaries without making missing provider usage look known."""
    totals=[phase['totals'] for phase in phases]
    result={'usage':{},'response_model_labels':{},'actual_total_cost_usd':None,
        'software_enforced_usd_cap':False,'human_semantic_review':'not_performed'}
    for field in ('input_tokens','output_tokens','total_tokens','reasoning_tokens','provider_reported_cost_usd'):
        pieces=[total['usage'][field] for total in totals if field in total.get('usage',{})]
        known=[piece['known_sum'] for piece in pieces if piece['known_sum'] is not None]
        result['usage'][field]={'known_sum':sum(known) if known else None,
            'unknown_attempts':sum(piece['unknown_attempts'] for piece in pieces),
            'complete_accounting':bool(pieces) and all(piece['complete_accounting'] for piece in pieces)}
    labels=Counter()
    for total in totals:labels.update(total.get('response_model_labels',{}))
    result['response_model_labels']=dict(labels)
    return result

def projection(kind,report):
    if kind=='plan':
        return {'status':report['status'],'phase':None,'attempts':report.get('total_physical_attempts'),
            'totals':{k:report[k] for k in ('total_physical_attempts','total_explicit_retries','accounting_complete') if k in report},'turns':[]}
    permitted=('turn_status_counts','reserved_physical_attempts','explicit_retries','response_model_labels','usage',
        'actual_total_cost_usd','software_enforced_usd_cap','user_experience_passed','human_semantic_review')
    return {'status':report['status'],'phase':report['phase'],'attempts':len(report['attempts']),
        'totals':{k:copy.deepcopy(v) for k,v in report.get('totals',{}).items() if k in permitted},
        'turns':[{'scenario':s['id'],'title':s['title'],'number':t['number'],'status':t['status'],
            'latency_seconds':t.get('model',{}).get('latency_seconds')} for s in report['scenarios'] for t in s['turns']]}

class LiveMonitor:
    def __init__(self):
        self.lock=threading.Lock();self.phases={}
        self.value={'status':'等待一次隐藏key输入','phase':None,'attempts':0,'phase_attempts':0,'totals':{},'turns':[]}
        monitor=self
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self,*args):pass
            def do_GET(self):
                if self.headers.get('Host')!=f'127.0.0.1:{monitor.server.server_port}':self.send_error(403);return
                if self.path=='/':body=PAGE.encode();kind='text/html; charset=utf-8'
                elif self.path=='/api/status':
                    with monitor.lock:body=json.dumps(monitor.value,ensure_ascii=False).encode()
                    kind='application/json; charset=utf-8'
                else:self.send_error(404);return
                self.send_response(200);self.send_header('Content-Type',kind);self.send_header('Content-Length',str(len(body)))
                self.send_header('Cache-Control','no-store');self.send_header('X-Content-Type-Options','nosniff')
                self.send_header('Content-Security-Policy',"default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'")
                self.end_headers();self.wfile.write(body)
        self.server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
        self.server.daemon_threads=True
        self.thread=threading.Thread(target=self.server.serve_forever,daemon=True)
    @property
    def url(self):return f'http://127.0.0.1:{self.server.server_port}/'
    def publish(self,kind,report):
        value=projection(kind,report)
        with self.lock:
            if kind=='plan':
                totals=aggregate_totals(self.phases.values());totals.update(value['totals'])
                totals['known_physical_attempts_lower_bound']=report.get('known_physical_attempts_lower_bound')
                if not report.get('accounting_complete',False):
                    for usage in totals['usage'].values():
                        usage.update(complete_accounting=False,
                            observed_unknown_attempts=usage['unknown_attempts'],unknown_attempts=None,
                            known_sum_is_lower_bound=True)
                value.update(turns=self.value['turns'],phase=self.value['phase'],phase_attempts=self.value['phase_attempts'],totals=totals)
            else:
                self.phases[value['phase']]=copy.deepcopy(value)
                value['phase_attempts']=value['attempts']
                value['attempts']=sum(phase['attempts'] for phase in self.phases.values())
                value['totals']=aggregate_totals(self.phases.values())
            self.value=value
    def __enter__(self):self.thread.start();return self
    def __exit__(self,*unused):self.server.shutdown();self.server.server_close();self.thread.join()
    def hold(self):
        try:threading.Event().wait()
        except KeyboardInterrupt:pass
