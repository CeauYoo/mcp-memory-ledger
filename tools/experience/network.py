"""Only network layer; default proxies/TLS, explicit finite retry policy, private diagnostics."""
import json
import http.client
import time
from datetime import datetime,timezone
import urllib.request
import urllib.error
from local_records import CallRecords,body_record
from protocol import TARGET,project

MAX_ATTEMPTS=100
MAX_RETRIES=10
TIMEOUT_SECONDS=180
MAX_RESPONSE_BYTES=4*1024*1024
MAX_INPUT_BYTES=256*1024

class RecordingFailure(RuntimeError):pass
class BudgetExhausted(RuntimeError):pass
class InputLimitExceeded(RuntimeError):pass

def now():return datetime.now(timezone.utc).isoformat()

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,*args,**kwargs):return None

def error_kind(status,obj):
    if status in (401,403):return 'authentication_or_authorization_stop'
    if status==402:return 'quota_stop'
    error=obj.get('error',{}) if isinstance(obj,dict) else {}
    codes={error[field].lower() for field in ('code','type') if isinstance(error.get(field),str)} if isinstance(error,dict) else set()
    if codes & {'invalid_api_key','authentication_error','invalid_authentication','unauthorized',
                'permission_denied','access_denied','invalid_token'}:
        return 'authentication_or_authorization_stop'
    code=' '.join(codes)
    if any(word in code.lower() for word in ('insufficient_quota','quota_exceeded','insufficient_balance','balance_not_enough','credit_exhausted')):
        return 'quota_stop'
    message=error.get('message','') if isinstance(error,dict) else ''
    message=message.lower() if isinstance(message,str) else ''
    if any(word in message for word in ('invalid api key','incorrect api key','无效的令牌','api key无效')):
        return 'authentication_or_authorization_stop'
    if any(word in message for word in ('余额不足','额度耗尽','insufficient balance','quota exhausted','quota has been exhausted')):
        return 'quota_stop'
    return None

class Provider:
    def __init__(self,folder,hashes,key,reserve,finish,target=TARGET,sleep=time.sleep,retry_limit=MAX_RETRIES):
        self.folder,self.hashes,self.key=folder,hashes,key
        self.reserve,self.finish,self.target,self.sleep=reserve,finish,target,sleep
        self.retry_count=0
        self.retry_limit=retry_limit
        self.opener=urllib.request.build_opener(NoRedirect())

    def call(self,body,logical_id):
        raw=json.dumps(body,ensure_ascii=False,separators=(',',':')).encode()
        if len(raw)>MAX_INPUT_BYTES:raise InputLimitExceeded('serialized_input_exceeds_256_kib')
        for attempt in (1,2):
            metadata={'logical_request_id':logical_id,'attempt_in_turn':attempt,
                      'retry':attempt>1,'network_route':'system_default_unchanged','redirects_followed':0}
            try:
                ordinal=self.reserve(metadata)
                records=CallRecords(self.folder,ordinal,self.hashes,metadata)
                started=now();records.request(self.target,raw,[self.key],started)
            except BudgetExhausted:raise
            except Exception:raise RecordingFailure('request_reservation_or_record_failed') from None
            status=None;headers=[];response=b'';complete=False;transport='dispatched';obj=None
            tick=time.monotonic();dispatched=True
            try:
                request=urllib.request.Request(self.target,data=raw,headers={'Content-Type':'application/json','Authorization':'Bearer '+self.key},method='POST')
                try:stream=self.opener.open(request,timeout=TIMEOUT_SECONDS)
                except urllib.error.HTTPError as error:stream=error
                with stream:
                    status=stream.status;headers=list(stream.headers.items());declared=stream.headers.get('Content-Length')
                    deadline=time.monotonic()+TIMEOUT_SECONDS
                    while True:
                        remaining=deadline-time.monotonic()
                        if remaining<=0:raise TimeoutError()
                        try:stream.fp.raw._sock.settimeout(remaining)
                        except AttributeError:pass
                        chunk=stream.read1(min(65536,MAX_RESPONSE_BYTES+1-len(response)))
                        if not chunk:
                            complete=declared is None or (declared.isdecimal() and int(declared)==len(response))
                            transport='http_response' if complete else 'short_response'
                            break
                        response+=chunk
                        if len(response)>MAX_RESPONSE_BYTES:
                            response=response[:MAX_RESPONSE_BYTES];transport='response_size_limit';break
                sanitized=body_record(response,[self.key],complete)
                if complete:
                    try:obj=json.loads(sanitized.get('body_text_utf8',''))
                    except (ValueError,TypeError,RecursionError):pass
            except (TimeoutError,urllib.error.URLError,OSError,http.client.HTTPException):transport='network_failure'
            finally:
                try:records.response(status,headers,response,[self.key],complete,started,transport)
                except Exception:raise RecordingFailure('response_record_failed') from None
            meta,text=project(obj,status,[self.key])
            meta.update(metadata,call_number=ordinal,started_utc=started,finished_utc=now(),
                        latency_seconds=round(time.monotonic()-tick,4),transport_status=transport,
                        body_complete=complete,dispatched=dispatched)
            if not complete:meta['protocol_status']='transport_incomplete';text=None
            fatal=error_kind(status,obj);meta['fatal_stop']=fatal
            retryable=(transport in ('network_failure','short_response') or status in (429,502,503,504)) and not fatal
            retry=retryable and attempt==1 and self.retry_count<self.retry_limit
            meta['will_retry']=retry
            try:self.finish(meta)
            except Exception:raise RecordingFailure('attempt_summary_failed') from None
            if retry:
                self.retry_count+=1;self.sleep(1);continue
            return {'metadata':meta,'text':text,'fatal':fatal}
        raise RuntimeError('unreachable_attempt')
