"""Real MCP storage. Explicit user facts are deterministic fixtures, never model extraction."""
import json
import subprocess
import support

SUBJECT='scenario_memory'

def require(value, code):
    if not value:raise RuntimeError(code)

def validate_written(record,draft,event_id):
    require(record is not None,'written_claim_missing')
    for field in ('owner','namespace','subject','predicate','mode'):
        require(record.get(field)==draft[field],'written_claim_'+field)
    require(record.get('status')=='Active','written_claim_status')
    require(json.dumps(json.loads(record['object']),sort_keys=True)==json.dumps(json.loads(draft['object']),sort_keys=True),
            'written_claim_value')
    canonical_event=event_id if event_id.startswith('event:') else 'event:'+event_id
    require(canonical_event in record['provenance']['evidence_event_references'],'written_claim_source')

class Ledger:
    def __init__(self, folder):
        self.config=support.write_config(folder,folder/'memory.sqlite')
        subprocess.run([str(support.BINARY),'init'],cwd=folder,env=support.child_environment(self.config),
                       stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=True,timeout=30)
        self.current={};self.history={};self.sequence=0

    def draft(self,ns,key,value):
        return {'owner':'World','namespace':ns,'subject':SUBJECT,'predicate':key,
                'object':json.dumps(value,ensure_ascii=False,separators=(',',':')),'mode':'Observed'}

    def apply(self,client,ns,facts,utterance,conflict=False):
        if not facts:return
        self.sequence+=1;serial=self.sequence
        event=client.tool('ingest_interaction',{'request_id':f'user-{serial}',
            'event':{'owner':'World','namespace':ns,'kind':'Observation','summary':utterance},
            'claim_drafts':[], 'episode_reference':f'episode:user-turn-{serial}'})
        state=self.current.setdefault(ns,{});history=self.history.setdefault(ns,[])
        for index,(key,value) in enumerate(facts.items()):
            old=state.get(key,[])
            if old and not conflict:
                replacements=[]
                for old_index,record in enumerate(old):
                    args={'request_id':f'correct-{serial}-{index}-{old_index}','namespace':ns,
                        'claim_reference':record['id'],'replacement_claim':self.draft(ns,key,value),
                        'replacement_evidence_event_ids':[event['event_id']],
                        'summary':'Explicit synthetic user correction: '+utterance}
                    response=client.tool('supersede_memory',args)
                    require(client.tool('supersede_memory',args)==response,'correction_idempotency')
                    retired=client.tool('get_memory',{'namespace':ns,'record_type':'Claim','id':record['id']})['record']
                    require(retired['status']=='Superseded','correction_old_status')
                    history.append(retired)
                    reflection=client.tool('get_reflection_history',{'namespace':ns,'claim_reference':record['id']})
                    require(bool(reflection['reflections']),'correction_history')
                    fresh=client.tool('get_memory',{'namespace':ns,'record_type':'Claim','id':response['replacement_claim_id']})['record']
                    validate_written(fresh,self.draft(ns,key,value),event['event_id']);replacements.append(fresh)
                state[key]=replacements
            else:
                result=client.tool('ingest_interaction',{'request_id':f'fact-{serial}-{index}',
                    'event':{'owner':'World','namespace':ns,'kind':'Observation','summary':utterance},
                    'claim_drafts':[self.draft(ns,key,value)],'episode_reference':f'episode:user-turn-{serial}'})
                fresh=client.tool('get_memory',{'namespace':ns,'record_type':'Claim','id':result['event_id']+':claim:0'})['record']
                validate_written(fresh,self.draft(ns,key,value),result['event_id'])
                state[key]=old+[fresh] if conflict else [fresh]

    def turn(self,scenario,item):
        base='project/experience-30-'+scenario['id'].lower()
        ns=base+('-foreign' if item.get('namespace')=='foreign' else '')
        # A new native process for every turn, including explicit history-reset turns.
        with support.Client(self.config) as client:
            if scenario.get('foreign') and base+'-foreign' not in self.current:
                name=scenario.get('project_names',{}).get('foreign','未命名独立项目')
                self.apply(client,base+'-foreign',scenario['foreign'],'Synthetic separate-project source: '+name)
            self.apply(client,ns,item['put'],item['user'])
            self.apply(client,ns,item.get('conflict',{}),item['user'],conflict=True)
            result=client.tool('recall_memory',{'namespace':ns,'query':SUBJECT,'limit':100})
            require(not result.get('has_more'),'retrieval_incomplete')
            records=[x['record'] for x in result['records'] if x['record'].get('subject')==SUBJECT
                     and x['record'].get('status')=='Active']
            expected={record['id'] for values in self.current.get(ns,{}).values() for record in values}
            require({x['id'] for x in records}==expected,'active_storage_state_mismatch')
            verified={record['id']:record for values in self.current.get(ns,{}).values() for record in values}
            for record in records:
                for field in ('owner','namespace','subject','predicate','object','mode','status','provenance'):
                    require(record[field]==verified[record['id']][field],'recalled_claim_content_mismatch')
            if item.get('history'):
                for historical in self.history.get(ns,[]):
                    record=client.tool('get_memory',{'namespace':ns,'record_type':'Claim','id':historical['id']})['record']
                    require(record is not None and record['status']=='Superseded','history_not_preserved')
                    records.append(record)
            projected=[]
            for record in records:
                require(record['namespace']==ns,'namespace_mismatch')
                ids=record['provenance']['evidence_event_references'];require(bool(ids),'missing_evidence')
                evidence=[]
                for event_id in ids:
                    event=client.tool('get_memory',{'namespace':ns,'record_type':'Event','id':event_id})['record']
                    require(event is not None and event['namespace']==ns,'evidence_scope')
                    evidence.append({'id':event_id,'summary':event['summary']})
                projected.append({'id':record['id'],'predicate':record['predicate'],'value':json.loads(record['object']),
                    'status':record['status'],'namespace':ns,'source_event_ids':ids,'evidence':evidence})
            other=base if ns.endswith('-foreign') else base+'-foreign'
            denied=0
            for values in self.current.get(other,{}).values():
                for record in values:
                    require(client.tool('get_memory',{'namespace':ns,'record_type':'Claim','id':record['id']})['record'] is None,
                            'foreign_direct_read_visible');denied+=1
        memory={'namespace':ns,'records':projected}
        project_name=scenario.get('project_names',{}).get('foreign' if item.get('namespace')=='foreign' else 'base')
        if project_name:
            # A fixture-selected display binding, not a new claim extracted by the model.
            memory['namespace_binding']={'namespace':ns,'project_name':project_name,'source':'synthetic_fixture_scope_selection'}
        return memory, {'status':'passed','new_native_process':True,
            'active_records_verified':len(expected),'source_events_verified':sum(len(x['evidence']) for x in projected),
            'foreign_direct_reads_denied':denied,'history_records_verified':sum(x['status']=='Superseded' for x in projected),
            'memory_writer':'deterministic_explicit_user_fact_fixture','model_memory_extraction_verified':False}
