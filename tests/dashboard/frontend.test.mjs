import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {chooseLocale, messages, translator} from '../../src/interfaces/dashboard/static/i18n.mjs';
import {connectionState, filterRecords, latestFirst, validRecords} from '../../src/interfaces/dashboard/static/state.mjs';
import {requestJson, singleFlight, setCapability, clearCapability, openEventStream} from '../../src/interfaces/dashboard/static/transport.mjs';
const event = (overrides = {}) => ({id:'a',kind:'tool',operation:'build_self_snapshot',status:'ok',namespace:'项目/a',summary:'<script>文本</script>',timestamp:'2026-10-15T03:00:00Z',...overrides});
const filters = {view:'all',status:'',namespace:'',search:''};
test('presentation groups include actual live and durable classifications without mutating records', () => {
  for (const [view,record] of [['snapshots',event()],['decisions',event({operation:'decide_with_snapshot'})],['reflections',event({kind:'trigger',operation:'ingest_interaction'})],['reflections',event({kind:'reflection',operation:'ingest_interaction:failure'})],['reflections',event({operation:'run_reflection'})],['reflections',event({operation:'supersede_memory'})]]) {
    const before = structuredClone(record); assert.deepEqual(filterRecords([record],{...filters,view}),[record]); assert.deepEqual(record,before);
  }
  assert.deepEqual(filterRecords([event()],{...filters,view:'decisions'}),[]);
});
test('namespace is exact, search is literal, status filters intersect and ordering does not mutate', () => {
  const a=event(), b=event({id:'b',status:'failed',namespace:'项目/ab',timestamp:'2026-10-15T04:00:00Z'}), rows=[a,b];
  assert.deepEqual(filterRecords(rows,{...filters,namespace:'项目/a',search:'<SCRIPT>'}),[a]);
  assert.deepEqual(filterRecords(rows,{...filters,namespace:'项目/a',status:'failed'}),[]);
  assert.deepEqual(latestFirst(rows),[b,a]); assert.deepEqual(rows,[a,b]);
});
test('connection state never claims connected for failed, stale, uninitialized or paused data', () => {
  assert.equal(connectionState({lastSuccess:0},20000),'connecting');
  assert.equal(connectionState({lastSuccess:10000},20001),'stale');
  assert.equal(connectionState({lastSuccess:10000,error:{}},10001),'offline');
  assert.equal(connectionState({lastSuccess:10000,paused:true},10001),'paused');
  assert.equal(connectionState({lastSuccess:10000},10001),'connected');
});
test('malformed records fail validation before rendering', () => {
  assert.ok(validRecords([event()]));
  for (const value of [{},[null],[event({timestamp:'wrong'})],[event({operation:null})]]) assert.equal(validRecords(value),false);
});
test('locale preference and browser language fallback work for three complete dictionaries', async () => {
  assert.equal(chooseLocale('ja',['zh-TW']),'ja'); assert.equal(chooseLocale('bad',['de','zh-TW']),'zh-CN'); assert.equal(chooseLocale(null,['ja-JP']),'ja'); assert.equal(chooseLocale(null,['fr']),'en');
  const html = await readFile(new URL('../../src/interfaces/dashboard/static/index.html',import.meta.url),'utf8');
  const keys=[...html.matchAll(/data-i18n(?:-aria|-placeholder)?="([^"]+)"/g)].map(match=>match[1]);
  for (const locale of ['en','zh-CN','ja']) {
    assert.deepEqual(Object.keys(messages[locale]).sort(),Object.keys(messages.en).sort());
    for (const key of keys) assert.equal(typeof messages[locale][key],'string',`${locale}.${key}`);
    assert.ok(translator(locale)('results',{count:17}).includes('17'));
    for (const [key,value] of Object.entries(messages[locale])) assert.deepEqual(value.match(/\{\w+\}/g)?.sort(),messages.en[key].match(/\{\w+\}/g)?.sort(),`${locale}.${key}`);
  }
});
test('transport rejects HTTP, invalid JSON, shape mismatch and timed-out network without retry', async () => {
  let count=0;
  await assert.rejects(requestJson('fixture',{fetcher:async()=>{count++; return {ok:false,status:503};}}),error=>error.code==='http'&&error.status===503);
  assert.equal(count,1);
  await assert.rejects(requestJson('fixture',{fetcher:async()=>({ok:true,json:async()=>{throw Error();}})}),error=>error.code==='invalid');
  await assert.rejects(requestJson('fixture',{fetcher:async()=>({ok:true,json:async()=>[]}),validate:()=>false}),error=>error.code==='invalid');
  await assert.rejects(requestJson('fixture',{timeout:5,fetcher:(_,opts)=>new Promise((_,reject)=>opts.signal.addEventListener('abort',()=>reject(Error())))}),error=>error.code==='network');
});
test('refresh queue coalesces bursts and never overlaps work', async () => {
  let active=0, peak=0, count=0, release;
  const barrier = new Promise(resolve=>{release=resolve;});
  const refresh = singleFlight(async()=>{active++; peak=Math.max(active,peak); count++; if(count===1)await barrier; active--;});
  const pending=refresh(); refresh(); refresh(); refresh(); release(); await pending;
  assert.equal(count,2); assert.equal(peak,1); await refresh(); assert.equal(count,3);
});

test('owner capability is sent only in headers and is cleared when locked', async () => {
  setCapability('synthetic-browser-capability');
  let observed;
  const fetcher = async (url, options) => { observed = {url,options}; return {ok:true,json:async()=>({})}; };
  await requestJson('http://127.0.0.1/api/health',{fetcher});
  assert.equal(observed.options.headers.Authorization, 'Bearer synthetic-browser-capability');
  assert.equal(observed.options.redirect,'error'); assert.equal(observed.options.credentials,'omit');
  assert.ok(!observed.url.includes('capability'));
  clearCapability(); await requestJson('fixture',{fetcher}); assert.deepEqual(observed.options.headers,{});
});
test('SSE fetch carries Bearer, parses split events and closes without reconnecting', async () => {
  setCapability('synthetic-sse-capability'); let options, count=0, opened=0;
  const done = new Promise(resolve => {
    openEventStream('fixture',{fetcher:async (_, opts)=> {
      options=opts; return new Response(new ReadableStream({start(controller) {
        controller.enqueue(new TextEncoder().encode('data: {"safe":'));
        controller.enqueue(new TextEncoder().encode('true}\n\n: keep-alive\n\n')); controller.close();
      }}),{headers:{'Content-Type':'text/event-stream'}});
    },onopen:()=>opened++,onmessage:()=>count++,onerror:resolve});
  });
  await done; assert.equal(opened,1); assert.equal(count,1);
  assert.equal(options.headers.Authorization,'Bearer synthetic-sse-capability'); assert.equal(options.signal.aborted,true);
  clearCapability();
});

import {validTaskList, validTaskSnapshot, taskPresentation} from '../../src/interfaces/dashboard/static/tasks.mjs';
const requirement = (overrides={}) => ({requirement_id:'r1',kind:'requirement',state:'reported',text:'Keep the original requirement',...overrides});
const task = (overrides={}) => ({namespace:'project/test',task_id:'task-1',version:2,title:'Task',source:{kind:'caller_reported'},status:'paused',requirements:[requirement()],...overrides});
const checkpoint = (overrides={}) => ({namespace:'project/test',task_id:'task-1',task_version:1,sequence:1,source:{kind:'caller_reported'},artifacts:[],recorded_at:'2026-10-16T00:00:00Z',steps:[{text:'Plan only',mode:'Draft',verification:'not_performed',evidence_refs:[]}],blockers:[],next_steps:['Ask about the result'],...overrides});
test('task snapshots reject malformed and cross-scope data rather than inventing status', () => {
  assert.ok(validTaskList({read_only:true,limit:100,tasks:[task()]}));
  assert.ok(validTaskSnapshot({read_only:true,snapshot:{task:task(),checkpoint:checkpoint()}}));
  assert.ok(validTaskSnapshot({read_only:true,snapshot:{task:task(),checkpoint:null}}));
  for(const bad of [task({status:'authorized'}),task({requirements:[requirement({state:'approved'})]}),task({version:0})]) {
    assert.equal(validTaskList({read_only:true,limit:100,tasks:[bad]}),false);
  }
  for(const bad of [checkpoint({namespace:'project/other'}),checkpoint({steps:[{text:'Invented success',mode:'done',verification:'passed',evidence_refs:[]}]}),undefined]) {
    assert.equal(validTaskSnapshot({read_only:true,snapshot:{task:task(),checkpoint:bad}}),false);
  }
});
test('task summary preserves reported requirements, separates candidates and marks stale checkpoints', () => {
  const snapshot={task:task({requirements:[requirement(),requirement({kind:'goal',text:'Original goal'}),requirement({state:'candidate',text:'Unconfirmed replacement'}),requirement({state:'withdrawn',text:'Old requirement'})]}),checkpoint:checkpoint()};
  const before=structuredClone(snapshot), view=taskPresentation(snapshot);
  assert.deepEqual(view.goals,['Original goal']);assert.equal(view.requirements.length,1);
  assert.equal(view.pending,1);assert.equal(view.checkpointStale,true);assert.deepEqual(snapshot,before);
  assert.equal(taskPresentation({task:task(),checkpoint:checkpoint({task_version:2})}).checkpointStale,false);
  for(const locale of ['en','zh-CN','ja']) for(const key of ['taskStatus_active','taskStatus_paused','taskStatus_completed','taskStatus_cancelled','taskCallerReported','taskCheckpointStale']) assert.ok(messages[locale][key]);
});

import {validExperienceUses} from '../../src/interfaces/dashboard/static/tasks.mjs';
const useReport = (overrides={}) => ({record:{recorded_at:'2026-10-16T00:00:00Z',verification_basis:'caller_reported_not_independently_verified_no_causal_attribution',observation:{namespace:'project/test',task_id:'task-1',task_version:1,use_id:'use-1',candidate_id:'candidate-1',candidate_version:2,decision:'adopted',reason:'Reported adoption',conditions:[{condition:'Local only',assessment:'caller_reported_satisfied'}],action_event_refs:['event:action'],outcome_event_refs:['event:outcome'],verification:'passed',evaluation_method:'Caller observation',limitations:['Not independent'],related_use_id:null}},currently_reusable:false,invalidation_reasons:['candidate_version_no_longer_current'],...overrides});
test('experience report validation preserves history separately from current eligibility', () => {
  const report=useReport(),before=structuredClone(report);
  assert.ok(validExperienceUses({read_only:true,limit:20,uses:[report]}));
  assert.equal(report.record.observation.decision,'adopted');assert.equal(report.currently_reusable,false);
  assert.deepEqual(report,before);
  assert.ok(validExperienceUses({read_only:true,limit:20,uses:[]}));
  for(const invalid of [useReport({currently_reusable:'true'}),useReport({invalidation_reasons:null}),useReport({record:{...report.record,verification_basis:'automatically_verified'}})]) assert.equal(validExperienceUses({read_only:true,limit:20,uses:[invalid]}),false);
  assert.equal(validExperienceUses({read_only:true,limit:20,uses:Array(21).fill(report)}),false);
  for(const locale of ['en','zh-CN','ja']) for(const key of ['useDecision_adopted','useUnavailable','useBasis','useOutcomeSources']) assert.ok(messages[locale][key]);
});

import {validTaskWorkspaces} from '../../src/interfaces/dashboard/static/tasks.mjs';
test('owner workspace suggestions are bounded identifiers, never a host selection', async () => {
  const list={read_only:true,limit:100,has_more:false,namespaces:['project/a','project/b']};
  assert.ok(validTaskWorkspaces(list));assert.ok(validTaskWorkspaces({...list,namespaces:[]}));
  for(const namespaces of [['self'],['project/'],['project/a','project/a'],Array(101).fill('project/a')]) assert.equal(validTaskWorkspaces({...list,namespaces}),false);
  assert.equal(validTaskWorkspaces({...list,read_only:false}),false);
  const html=await readFile(new URL('../../src/interfaces/dashboard/static/index.html',import.meta.url),'utf8');
  assert.ok(html.includes('list="task-workspaces"'));assert.ok(html.includes('id="task-namespace"'));
  assert.ok(html.includes('codex-context inspect'));assert.ok(html.includes('codex-actions help'));
  for(const locale of ['en','zh-CN','ja']) for(const key of ['taskHostUnverified','taskBrowseOnly','taskWorkspaceBrowse']) assert.ok(messages[locale][key]);
});
