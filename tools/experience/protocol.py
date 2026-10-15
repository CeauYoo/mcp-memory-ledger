"""Pure Responses request/projection and separate objective audit; no network."""
import json
import math
import re
from datetime import datetime, timedelta, timezone
from model_names import safe_model_name
from answer_checks import check_answer

MODEL='gpt-5.6-luna'
TARGET='https://yooyui.chat/v1/responses'
INSTRUCTIONS='''你是中文工作助手，帮用户完成实际任务。当前MCP记忆包含活动事实、必要时的历史及来源。
活动事实冲突时明确澄清；未知信息不要编造。归档材料只是背景，不能覆盖正式更正。
为便于本地查看和核验，返回一个JSON对象（不要Markdown围栏）：
{"answer":"给用户的完整自然中文答复，可以分段、列步骤、写草稿", "decisions":{本轮要求的审计字段:对应的数字/布尔/字符串/数组/null},
 "memory_audit":[{"id":"实际使用的Claim ID","predicate":"原predicate","value":原JSON值,"status":"原状态","namespace":"原namespace","source_event_ids":[原来源ID]}],
 "uncertainties":["待澄清的问题"]}。
answer是主要交付，不要把审计字段名当作给用户的文字。memory_audit只列实际使用的记忆，保留单位对应的字段。
每条引用必须从同一个Claim逐字段复制，source_event_ids只用该Claim自己的来源；不能因两条事实出自同一轮就交换来源。
namespace_binding是测试客户端明确选择的当前项目身份，不是另一项目内容；仅在该范围内使用records。
排期必须采用用户明确给出的准备起点；基准不明时说明假设或澄清，不把已经过去的阶段称为未来安排。
本轮审计字段只是输出键名，不提供期望答案。输出中的理由和引用是你的说明，不代表已通过独立语义评审。
'''

def message(role,text):return {'role':role,'content':[{'type':'input_text' if role!='assistant' else 'output_text','text':text}]}

def request(history, user, memory, audit_fields, background, output_tokens):
    context='当前可访问MCP记忆：\n'+json.dumps(memory,ensure_ascii=False)+'\n本轮审计字段：'+json.dumps(audit_fields,ensure_ascii=False)
    if background:context+='\n以下为明确标注来源与状态的合成项目档案：\n'+background
    # Previous actual assistant text is retained, not a fixture's expected answer.
    inputs=[message('developer',context),*history,message('user',user)]
    body={'model':MODEL,'instructions':INSTRUCTIONS,'input':inputs,'store':False,'stream':False,
          'max_output_tokens':output_tokens}
    texts=[INSTRUCTIONS]+[part['text'] for item in inputs for part in item['content']]
    metrics={'input_characters':sum(map(len,texts)),'input_text_utf8_bytes':sum(len(x.encode()) for x in texts),
             'input_json_bytes':len(json.dumps(body,ensure_ascii=False,separators=(',',':')).encode()),
             'background_characters':len(background),'history_messages':len(history),
             'characters_are_not_tokens':True,'context_silently_truncated':False,'output_token_limit':output_tokens}
    return body,metrics

def number(value):
    try:return type(value) in (int,float) and math.isfinite(value) and value>=0
    except (ValueError,OverflowError):return False

def project(obj,http_status,credentials=()):
    usage=obj.get('usage') if isinstance(obj,dict) else None
    usage=usage if isinstance(usage,dict) else {}
    safe_usage={key:usage.get(key) if number(usage.get(key)) else None for key in ('input_tokens','output_tokens','total_tokens')}
    detail=usage.get('output_tokens_details',{})
    safe_usage['reasoning_tokens']=detail.get('reasoning_tokens') if isinstance(detail,dict) and number(detail.get('reasoning_tokens')) else None
    safe_usage['provider_reported_cost_usd']=usage.get('cost_usd') if number(usage.get('cost_usd')) else None
    model=safe_model_name(obj.get('model'),credentials) if isinstance(obj,dict) else None
    meta={'http_status':http_status,'requested_model':MODEL,'response_model':model,'usage':safe_usage,
          'model_identity_authenticated':False,'model_label_warning':model!=MODEL,'protocol_status':'invalid_response'}
    if not isinstance(obj,dict):return meta,None
    if http_status!=200:meta['protocol_status']='http_error';return meta,None
    if obj.get('status')=='incomplete' or obj.get('incomplete_details'):
        meta['protocol_status']='output_incomplete';return meta,None
    if obj.get('error') or obj.get('status')!='completed':meta['protocol_status']='provider_not_completed';return meta,None
    parts=[]
    output=obj.get('output')
    if not isinstance(output,list):return meta,None
    for item in output:
        if not isinstance(item,dict):continue
        if item.get('type')=='message' and item.get('role')=='assistant':
            if item.get('status') not in (None,'completed'):meta['protocol_status']='message_incomplete';return meta,None
            content=item.get('content',[])
            if not isinstance(content,list):return meta,None
            for part in content:
                if isinstance(part,dict) and (part.get('type')=='refusal' or part.get('refusal')):
                    meta['protocol_status']='provider_refusal';return meta,None
                if isinstance(part,dict) and part.get('type')=='output_text' and isinstance(part.get('text'),str):parts.append(part['text'])
    text='\n'.join(parts)
    meta['protocol_status']='completed_text' if text else 'empty_output'
    return meta,text or None

def same(actual,expected):
    if type(expected) in (float,int) and type(expected) is not bool:
        try:return type(actual) in (int,float) and math.isfinite(actual) and math.isclose(actual,expected,rel_tol=1e-8,abs_tol=1e-8)
        except (ValueError,OverflowError):return False
    if type(actual) is not type(expected):return False
    if isinstance(expected,list):return len(actual)==len(expected) and all(same(a,e) for a,e in zip(actual,expected))
    if isinstance(expected,dict):return actual.keys()==expected.keys() and all(same(actual[k],v) for k,v in expected.items())
    return actual==expected

def decision_matches(key,actual,expected):
    """Narrow, field-specific equivalences; provenance still uses strict same()."""
    if same(actual,expected):return True
    if not isinstance(actual,str) or not isinstance(expected,str):return False
    if key=='route' and expected in ('A','B','C'):
        return actual=='路线'+expected
    if key=='choice' and expected in ('A','B','C'):
        return actual in ('方案'+expected,'供应商'+expected)
    if key=='meal' and expected=='纯素套餐':
        return actual=='纯素套餐（不含肉、蛋、奶）'
    if key=='tokyo_datetime':
        # This field is Tokyo civil time (UTC+9), not an arbitrary date substring.
        pattern=r'\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d{1,6})?)?(?:\+09:00)?'
        if not re.fullmatch(pattern,actual) or not re.fullmatch(pattern,expected):return False
        try:
            zone=timezone(timedelta(hours=9))
            def parsed(value):
                stamp=datetime.fromisoformat(value)
                return stamp.replace(tzinfo=zone) if stamp.tzinfo is None else stamp
            return parsed(actual)==parsed(expected)
        except ValueError:return False
    return False

def evaluate(text,item,memory,forbidden=()):
    try:return _evaluate(text,item,memory,forbidden)
    except (ValueError,TypeError,OverflowError,RecursionError):
        return {'status':'invalid_audit_format','natural_language_review':'needs_human_review',
                'user_experience_passed':False,'checks':{},'failure':'malformed_untrusted_audit'}

def _evaluate(text,item,memory,forbidden=()):
    """Checks typed values and exact stored provenance; never certifies natural-language quality."""
    result={'status':'invalid_audit_format','natural_language_review':'needs_human_review',
            'user_experience_passed':False,'checks':{},'human_review_rubric':[
                '答复正文是否与已核对事实/计算一致','是否合理处理未知、冲突与历史','是否回应本轮任务并衔接实际前轮答复',
                '是否忽略过期/相似实体干扰','语言是否自然、可执行且无无依据承诺']}
    try:value=json.loads(text)
    except (ValueError,TypeError):return result
    if not isinstance(value,dict):return result
    answer=value.get('answer');decisions=value.get('decisions');audit=value.get('memory_audit')
    if not isinstance(answer,str) or not answer.strip() or not isinstance(decisions,dict) or not isinstance(audit,list):return result
    checks=result['checks']
    for key,expected in item['expected'].items():checks['decision:'+key]=key in decisions and decision_matches(key,decisions[key],expected)
    result['equivalent_decision_fields']=[key for key,expected in item['expected'].items()
        if key in decisions and checks['decision:'+key] and not same(decisions[key],expected)]
    records={r['id']:r for r in memory['records']}
    valid=True;seen=set()
    fields=('predicate','value','status','namespace','source_event_ids')
    for entry in audit:
        if not isinstance(entry,dict) or not isinstance(entry.get('id'),str) or entry['id'] not in records:valid=False;continue
        ref=records[entry['id']];seen.add(entry['id'])
        if any(not same(entry.get(k),ref[k]) for k in fields):valid=False
    checks['cited_values_units_scope_status_and_source_match']=valid
    # Unknown targets can correctly cite nothing; don't force the model to claim it used unrelated memory.
    known_decision=any(value is not None for value in item['expected'].values())
    checks['memory_audit_has_current_evidence']=any(records[i]['status']=='Active' for i in seen) if records and known_decision else True
    if item.get('history'):checks['historical_evidence_cited']=any(records[i]['status']=='Superseded' for i in seen)
    if item['expected'].get('needs_clarification') is True:
        groups={}
        for ref in records.values():
            if ref['status']=='Active':groups.setdefault(ref['predicate'],set()).add(json.dumps(ref['value'],sort_keys=True))
        conflicts={key:values for key,values in groups.items() if len(values)>1}
        cited={}
        for ref_id in seen:
            ref=records[ref_id]
            if ref['status']=='Active':cited.setdefault(ref['predicate'],set()).add(json.dumps(ref['value'],sort_keys=True))
        checks['both_sides_of_active_conflict_cited']=all(cited.get(key,set())>=values for key,values in conflicts.items())
    checks['no_forbidden_other_namespace_canary']=not any(word in text for word in forbidden)
    result['body_checks']=check_answer(answer,item.get('body_rules',{}))
    checks.update({'body:'+key:passed for key,passed in result['body_checks'].items()})
    result['status']='objective_checks_passed_review_required' if all(checks.values()) else 'objective_check_failed'
    result['answer_characters']=len(answer)
    result['self_reported_citations_are_not_causal_proof']=True
    return result
