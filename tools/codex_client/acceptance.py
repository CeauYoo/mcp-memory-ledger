"""Actual installed Codex app-server MCP client, no model turns or credentials."""
import json
from pathlib import Path
import queue
import subprocess
import threading
import time


class CodexClient:
    def __init__(self,root,codex,binary,config):
        self.root=root
        self.folder=root/('client-'+str(time.time_ns()));self.folder.mkdir()
        home=self.folder/'codex-home';home.mkdir()
        env={'PATH':'/usr/bin:/bin','HOME':str(self.folder),'CODEX_HOME':str(home),'TMPDIR':str(self.folder)}
        server={'command':str(binary),'args':['serve'],'env':{'AGENT_LLM_MM_CONFIG':str(config or root/'local-storage.toml')},
                'enabled_tools':['ingest_interaction','recall_memory','get_memory','supersede_memory','get_reflection_history']}
        # TOML inline tables are explicit, local and contain no credentials.
        toml='{command='+json.dumps(server['command'])+',args=["serve"],env={AGENT_LLM_MM_CONFIG='+json.dumps(server['env']['AGENT_LLM_MM_CONFIG'])+'},enabled_tools='+json.dumps(server['enabled_tools'])+'}'
        args=[str(codex),'-c','model_provider="acceptance-offline"','-c','model="offline-no-model"',
            '-c','model_providers.acceptance-offline={name="Offline acceptance",base_url="http://127.0.0.1:1/v1",wire_api="responses",requires_openai_auth=false}',
            '-c','mcp_servers.memory-ledger-acceptance='+toml,'app-server','--stdio']
        self.proc=subprocess.Popen(args,env=env,cwd=root,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,encoding='utf-8')
        self.lines=queue.Queue();self.counter=0;self.methods=[]
        def read():
            for line in self.proc.stdout:self.lines.put(line)
            self.lines.put(None)
        threading.Thread(target=read,daemon=True).start()
    def request(self,method,params):
        self.counter+=1;current=self.counter;self.methods.append(method)
        self.proc.stdin.write(json.dumps({'id':current,'method':method,'params':params})+'\n');self.proc.stdin.flush()
        until=time.monotonic()+45
        while True:
            line=self.lines.get(timeout=max(0.1,until-time.monotonic()))
            if line is None:raise RuntimeError('client_closed')
            message=json.loads(line)
            if 'method' in message and 'id' in message:raise RuntimeError('client_requires_confirmation_stop')
            if message.get('id')==current:
                if 'error' in message:raise RuntimeError('codex_rpc_error_'+str(message['error'].get('code','unknown')))
                return message['result']
            if time.monotonic()>until:raise RuntimeError('client_timeout')
    def __enter__(self):
        try:
            self.request('initialize',{'clientInfo':{'name':'memory-ledger-isolated-acceptance','version':'1'},'capabilities':{'experimentalApi':True}})
            self.proc.stdin.write(json.dumps({'method':'initialized'})+'\n');self.proc.stdin.flush()
            result=self.request('thread/start',{'cwd':str(self.root),'ephemeral':True,'approvalPolicy':'on-request','sandbox':'workspace-write'})
            self.thread=result['thread']['id']
            self.inventory=self.request('mcpServerStatus/list',{'threadId':self.thread,'serverName':'memory-ledger-acceptance','detail':'full'})
            return self
        except BaseException:self.__exit__();raise
    def __exit__(self,*unused):
        self.proc.stdin.close()
        try:self.proc.wait(timeout=8)
        except subprocess.TimeoutExpired:self.proc.kill();self.proc.wait()
        self.proc.stdout.close()
    def tool(self,name,args):
        result=self.request('mcpServer/tool/call',{'threadId':self.thread,'server':'memory-ledger-acceptance','tool':name,'arguments':args})
        if result.get('isError'):raise RuntimeError('mcp_tool_failed')
        return result['structuredContent']

def verify(root,codex,binary):
    ns='project/codex-acceptance';other='project/codex-foreign';calls=[]
    def require(ok,label):
        if not ok:raise RuntimeError(label)
    def draft(scope,value):return {'owner':'World','namespace':scope,'subject':'codex_acceptance_fact','predicate':'venue','object':json.dumps(value,ensure_ascii=False),'mode':'Observed'}
    def ingest(client,scope,value,request):
        return client.tool('ingest_interaction',{'request_id':request,
            'event':{'owner':'World','namespace':scope,'kind':'Observation','summary':'Synthetic Codex client acceptance: '+value},
            'claim_drafts':[draft(scope,value)],'episode_reference':'episode:'+request})
    def get(client,scope,kind,ref):return client.tool('get_memory',{'namespace':scope,'record_type':kind,'id':ref})['record']
    version_home=root/'version-home';version_home.mkdir()
    version=subprocess.run([str(codex),'--version'],env={'PATH':'/usr/bin:/bin','HOME':str(version_home),'CODEX_HOME':str(version_home)},capture_output=True,text=True,encoding='utf-8',check=True,timeout=20).stdout.strip()
    nonce=str(time.time_ns())
    workspace=root/('workflow-'+nonce);workspace.mkdir()
    config=workspace/'local-storage.toml'
    config.write_text('transport="stdio"\ndatabase_url='+json.dumps('sqlite://'+str(workspace/'memory.sqlite'))+'\n[model]\nprovider="mock"\n[dashboard]\nenabled=false\n[daemon]\nenabled=false\n', encoding='utf-8')
    subprocess.run([str(binary),'init'],env={'PATH':'/usr/bin:/bin','HOME':str(workspace),'AGENT_LLM_MM_CONFIG':str(config)},cwd=workspace,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    with CodexClient(root,codex,binary,config) as client:
        inventory=client.inventory['data'];require(len(inventory)==1,'server_inventory')
        require(set(inventory[0]['tools'])=={'ingest_interaction','recall_memory','get_memory','supersede_memory','get_reflection_history'},'allowlisted_tools')
        initial=ingest(client,ns,'西厅','initial-'+nonce);foreign=ingest(client,other,'PRIVATE_SYNTHETIC_CEDAR','foreign-'+nonce)
        old_ref=initial['event_id']+':claim:0';foreign_ref=foreign['event_id']+':claim:0';calls+=client.methods
    with CodexClient(root,codex,binary,config) as client:
        recalled=client.tool('recall_memory',{'namespace':ns,'query':'codex_acceptance_fact','limit':100})
        require(old_ref in json.dumps(recalled),'restart_recall_missing')
        old=get(client,ns,'Claim',old_ref)
        require(json.loads(old['object'])=='西厅','restart_value')
        source_ref=initial['event_id'] if initial['event_id'].startswith('event:') else 'event:'+initial['event_id']
        require(source_ref in old['provenance']['evidence_event_references'],'source_reference')
        require(get(client,ns,'Event',initial['event_id']) is not None,'source_event')
        source=client.tool('ingest_interaction',{'request_id':'correction-source-'+nonce,
            'event':{'owner':'World','namespace':ns,'kind':'Observation','summary':'Synthetic explicit correction to 东厅'},
            'claim_drafts':[],'episode_reference':'episode:correction-'+nonce})
        args={'request_id':'correction-'+nonce,'namespace':ns,'claim_reference':old_ref,
              'replacement_claim':draft(ns,'东厅'),'replacement_evidence_event_ids':[source['event_id']],
              'summary':'Synthetic user-confirmed venue correction'}
        correction=client.tool('supersede_memory',args)
        require(client.tool('supersede_memory',args)==correction,'correction_idempotency')
        require(get(client,ns,'Claim',old_ref)['status']=='Superseded','old_status')
        new_ref=correction['replacement_claim_id'];fresh=get(client,ns,'Claim',new_ref)
        require(json.loads(fresh['object'])=='东厅' and fresh['status']=='Active','corrected_value')
        require(bool(client.tool('get_reflection_history',{'namespace':ns,'claim_reference':old_ref})['reflections']),'correction_history')
        calls+=client.methods
    with CodexClient(root,codex,binary,config) as client:
        require(get(client,ns,'Claim',new_ref)['status']=='Active','second_restart')
        require(get(client,ns,'Claim',foreign_ref) is None,'foreign_claim_visible')
        require(get(client,ns,'Event',foreign['event_id']) is None,'foreign_event_visible')
        require(not client.tool('get_reflection_history',{'namespace':other,'claim_reference':old_ref})['reflections'],'foreign_history_visible')
        recalled=client.tool('recall_memory',{'namespace':ns,'query':'codex_acceptance_fact','limit':100})
        require('PRIVATE_SYNTHETIC_CEDAR' not in json.dumps(recalled),'foreign_canary_leak')
        missing=client.tool('recall_memory',{'namespace':ns,'query':'UNRECORDED_BOOKING_REFERENCE_793','limit':10})
        require(not missing['records'],'unknown_information_invented')
        calls+=client.methods
    evidence={'status':'passed_actual_codex_app_server_mcp_client','codex_version':version,'mcp_binary_sha256':__import__('hashlib').sha256(binary.read_bytes()).hexdigest(),
        'client_processes':3,'client_restarts':2,'mcp_tools_allowlisted':5,
        'mcp_tool_calls':calls.count('mcpServer/tool/call'),'model_turns':calls.count('turn/start'),
        'model_provider_endpoint':'loopback_port_1_no_model_service',
        'write_restart_recall_provenance_correction_replay_history_isolation_unknown':'passed',
        'global_config_or_project_trust_modified':False,'historical_credentials_loaded':False,
        'desktop_ui_registration_verified':False,'new_machine_verified':False,'experimental_app_server_api':True}
    (root/'client-evidence.json').write_text(json.dumps(evidence,ensure_ascii=False,indent=2)+'\n', encoding='utf-8')
    print(json.dumps(evidence,ensure_ascii=False))

def main():
    import argparse
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--codex',required=True,type=Path)
    parser.add_argument('--binary',required=True,type=Path)
    parser.add_argument('--output',required=True,type=Path)
    args=parser.parse_args()
    codex=args.codex.resolve(strict=True);binary=args.binary.resolve(strict=True);root=args.output.resolve()
    root.mkdir(parents=True,exist_ok=False)
    verify(root,codex,binary)

if __name__=='__main__':main()
