import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {chooseLocale, messages, translator} from '../../src/interfaces/dashboard/static/i18n.mjs';
import {connectionState, filterRecords, latestFirst, validRecords} from '../../src/interfaces/dashboard/static/state.mjs';
import {requestJson, singleFlight} from '../../src/interfaces/dashboard/static/transport.mjs';
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
