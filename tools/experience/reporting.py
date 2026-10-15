"""Local-only readable transcript and shareable numeric/status summary."""
from collections import Counter
import html
import json
from pathlib import Path
import support

def summarize(report):
    turns=[turn for case in report['scenarios'] for turn in case['turns']]
    attempts=report['attempts']
    usage={}
    for field in ('input_tokens','output_tokens','total_tokens','reasoning_tokens','provider_reported_cost_usd'):
        values=[a.get('usage',{}).get(field) for a in attempts]
        known=[v for v in values if v is not None]
        usage[field]={'known_sum':sum(known) if known else None,'unknown_attempts':len(values)-len(known),
                      'complete_accounting':len(known)==len(values) and bool(values)}
    return {'turn_status_counts':dict(Counter(t['status'] for t in turns)),
        'reserved_physical_attempts':len(attempts),'explicit_retries':sum(a['attempt_in_turn']>1 for a in attempts),
        'response_model_labels':dict(Counter(a.get('response_model') or 'unknown_or_filtered' for a in attempts)),
        'usage':usage,'actual_total_cost_usd':None,'price_table_available':False,
        'provider_reported_cost_is_unverified':True,'software_enforced_usd_cap':False,
        'user_experience_passed':False,'human_semantic_review':'not_performed',
        'usage_missing_does_not_mean_free':True}

def render(folder,report):
    def esc(text):return html.escape(str(text),quote=True)
    parts=['<!doctype html><html lang="zh"><meta charset="utf-8"><meta name="viewport" content="width=device-width">',
      '<meta http-equiv="Content-Security-Policy" content="default-src \'none\'; style-src \'unsafe-inline\'">',
      '<title>30场景体验报告</title><style>body{font:16px/1.65 system-ui;max-width:1100px;margin:40px auto;padding:0 24px;color:#17212b;background:#fafafa}details{background:white;border:1px solid #ddd;border-radius:10px;padding:14px;margin:12px 0}summary{cursor:pointer;font-weight:650}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#f1f4f6;padding:14px;border-radius:8px}h3{margin-bottom:4px}.note{color:#5c6470}table{border-collapse:collapse;width:100%}td,th{padding:8px;border-bottom:1px solid #ddd;text-align:left}</style>',
      '<h1>30场景 · 连续对话体验报告</h1>',
      '<p>这是本机合成场景回放；存储、结构化核验和自然答复分开评估。正文默认待人工复核，不能据此宣称用户体验全部通过。</p>',
      '<p>模式：'+esc(report['mode'])+'｜状态：'+esc(report['status'])+'｜请求模型：'+esc(report['requested_model'])+'</p>',
      '<pre>'+esc(json.dumps(report['totals'],ensure_ascii=False,indent=2))+'</pre>']
    for case in report['scenarios']:
        parts.append('<details><summary>'+esc(case['id']+' · '+case['title']+' · '+case['category'])+'</summary>')
        for item in case['turns']:
            parts.append('<h3>第'+str(item['number'])+'轮 · '+esc(item['status'])+'</h3>')
            parts.append('<details><summary>核验、用量与上下文长度</summary><pre>'+esc(json.dumps({k:v for k,v in item.items() if k not in ('local_record',)},ensure_ascii=False,indent=2))+'</pre></details>')
            path=folder/'turns'/f'{case["id"]}-{item["number"]:02d}.json'
            if path.exists():
                record=json.loads(path.read_text(encoding='utf-8'))
                answer=record.get('assistant_text')
                if isinstance(answer,str):
                    try:
                        parsed=json.loads(answer)
                        if isinstance(parsed,dict) and isinstance(parsed.get('answer'),str):answer=parsed['answer']
                    except (ValueError,TypeError,RecursionError):pass
                else:answer='本轮没有完整可用的助手答复；请查看失败记录。'
                parts.append('<h4>用户输入</h4><pre>'+esc(record['user'])+'</pre><h4>助手答复 · 语义待复核</h4><pre>'+esc(answer)+'</pre>')
                for title,key in [('完整模型返回与审计字段','assistant_text'),('检索记忆及来源','memory'),
                                  ('本地评分预期（未发送给模型）','expected'),('本轮实际完整请求','request')]:
                    value=record.get(key)
                    if not isinstance(value,str):value=json.dumps(value,ensure_ascii=False,indent=2)
                    parts.append('<details><summary>'+title+'</summary><pre>'+esc(value)+'</pre></details>')
        parts.append('</details>')
    parts.append('</html>');path=folder/'report.html';path.write_text('\n'.join(parts), encoding='utf-8');path.chmod(0o600)
    return path
