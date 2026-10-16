#!/usr/bin/env node
// Dependency-free real Chrome QA. Only an owned mock MCP fixture, browser profile and loopback proxy.
// Node >=22; use an already installed Chrome via --chrome. No browser downloads.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createServer, request as httpRequest} from 'node:http';
import {createInterface} from 'node:readline';
import {mkdir, readFile, writeFile} from 'node:fs/promises';
import {createWriteStream} from 'node:fs';
import path from 'node:path';
const options = Object.fromEntries(process.argv.slice(2).reduce((pairs,value,index,args)=>index%2 ? pairs : [...pairs,[value,args[index+1]]],[]));
for (const key of ['--chrome','--output']) assert.ok(options[key],`${key} is required`);
const output=path.resolve(options['--output']); await mkdir(output,{recursive:false});
const pause = ms => new Promise(resolve=>setTimeout(resolve,ms));
const checks=[], errors=[], requests=[];
let child, chrome, proxy, socket, session, fault='', inject=false, fixtureQueue=[], fixtureWaiters=[], rpcCounter=0, slowPending=0, cleanupPromise, ownership={};
const inflight=new Map();
function stopOwnedProcess(proc, gracefulInput) {
  if(!proc || proc.exitCode!==null || proc.signalCode) return Promise.resolve();
  return new Promise(resolve=>{
    const signalTimer=gracefulInput ? setTimeout(()=>proc.kill('SIGTERM'),1000) : null;
    const timer=setTimeout(()=>proc.kill('SIGKILL'),gracefulInput ? 8000 : 5000);
    proc.once('exit',()=>{clearTimeout(timer);clearTimeout(signalTimer);resolve();});
    if(gracefulInput) { if(!proc.stdin.destroyed) proc.stdin.end(gracefulInput); }
    else proc.kill('SIGTERM');
  });
}
function cleanup() {
  return cleanupPromise ??= (async()=>{
    socket?.close();for(const pending of inflight.values())clearTimeout(pending.timer);
    proxy?.closeAllConnections();proxy?.close();
    await Promise.all([stopOwnedProcess(chrome),stopOwnedProcess(child,JSON.stringify({action:'stop'})+'\n')]);
    await writeFile(path.join(output,'cleanup.json'),JSON.stringify({chromeExited:!chrome||chrome.exitCode!==null||Boolean(chrome.signalCode),fixtureExited:!child||child.exitCode!==null||Boolean(child.signalCode)},null,2));
  })();
}
for(const [signal,code] of [['SIGINT',130],['SIGTERM',143]]) process.once(signal,()=>{void cleanup().finally(()=>process.exit(code));});
function nextFixture() { return fixtureQueue.length ? Promise.resolve(fixtureQueue.shift()) : new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('fixture timeout')),30000);fixtureWaiters.push(value=>{clearTimeout(timer);resolve(value);});}); }
function call(method,params={}) {
  const id=++rpcCounter;
  return new Promise((resolve,reject)=>{const timer=setTimeout(()=>{inflight.delete(id);reject(Error(`CDP timeout: ${method}`));},15000);inflight.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params,...(session ? {sessionId:session} : {})}));});
}
async function evaluate(expression) {
  const value=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});
  if (value.exceptionDetails) throw Error(value.exceptionDetails.text+': '+value.exceptionDetails.exception?.description);
  return value.result.value;
}
async function until(expression,timeout=12000) {
  const start=Date.now(); while(Date.now()-start<timeout) { if(await evaluate(expression))return; await pause(75); }
  throw Error(`condition timeout: ${expression}`);
}
async function click(selector) {
  await evaluate(`document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block:'center'})`);
  const point=await evaluate(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};})()`);
  await call('Input.dispatchMouseEvent',{type:'mousePressed',...point,button:'left',clickCount:1});
  await call('Input.dispatchMouseEvent',{type:'mouseReleased',...point,button:'left',clickCount:1});
}
async function choose(selector,value) {
  await evaluate(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.value=${JSON.stringify(value)};e.dispatchEvent(new Event('change',{bubbles:true}));})()`);
}
async function screenshot(name) {
  const result=await call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
  await writeFile(path.join(output,name+'.png'),Buffer.from(result.data,'base64'));
}
async function check(name,run) { await run(); checks.push(name); console.log(`PASS ${name}`); }

const capability = 'synthetic-admin-browser-fixture-only';
const assets = path.resolve('src/interfaces/dashboard/static');
const taskSource={kind:'caller_reported'};
const syntheticTask={namespace:'project/synthetic',task_id:'browser-task',version:2,title:'Synthetic continuity task',status:'paused',source:taskSource,requirements:[{requirement_id:'goal',kind:'goal',state:'reported',text:'Original browser goal',source:taskSource},{requirement_id:'keep',kind:'requirement',state:'reported',text:'<script>window.taskInjected=true</script>'+'長い要件 '.repeat(20),source:taskSource},{requirement_id:'candidate',kind:'requirement',state:'candidate',text:'Unconfirmed replacement',source:{kind:'model_asserted'}}]};
const syntheticCheckpoint={namespace:'project/synthetic',task_id:'browser-task',task_version:1,sequence:1,recorded_at:'2026-10-16T00:00:00Z',source:taskSource,steps:[{step_id:'plan',text:'Plan only',mode:'Draft',verification:'not_performed',evidence_refs:[]}],blockers:['Awaiting result'],next_steps:['Check output'],artifacts:[]};
const syntheticUse={record:{recorded_at:'2026-10-16T00:00:00Z',verification_basis:'caller_reported_not_independently_verified_no_causal_attribution',observation:{namespace:'project/synthetic',task_id:'browser-task',task_version:1,use_id:'browser-use',candidate_id:'browser-experience',candidate_version:2,decision:'adopted',reason:'Synthetic report',conditions:[{condition:'Fixture only',assessment:'caller_reported_satisfied'}],action_event_refs:['event:synthetic-outcome'],outcome_event_refs:['event:synthetic-outcome'],verification:'passed',evaluation_method:'Synthetic observation',limitations:['Not independently verified'],related_use_id:null}},currently_reusable:false,invalidation_reasons:['task_revision_no_longer_current_active']};
const row = {id:'fixture',operation:'recall_memory',namespace:'project/synthetic',kind:'tool',status:'ok',summary:'Synthetic owner record',timestamp:'2026-10-15T00:00:00Z',payload:{},read_only:true};
try {
  let url, taskNamespace='project/synthetic';
  if (options['--binary']) {
    child=spawn(options['--python']??'python3',['-B','scripts/dashboard-admin-browser-fixture.py',path.resolve(options['--binary']),path.join(output,'fixture')],{stdio:['pipe','pipe','pipe']});
    child.stderr.pipe(createWriteStream(path.join(output,'fixture.stderr.log')));
    createInterface({input:child.stdout}).on('line',line=>{const value=JSON.parse(line);const waiter=fixtureWaiters.shift();if(waiter)waiter(value);else fixtureQueue.push(value);});
    const fixture=await nextFixture(); url=fixture.url; taskNamespace=fixture.task_namespace;
  } else {
  proxy=createServer(async(req,res)=>{
    requests.push({method:req.method,path:req.url,authorized:req.headers.authorization===`Bearer ${capability}`});
    res.setHeader('Cache-Control','no-store');
    res.setHeader('Content-Security-Policy',"default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'");
    if(req.url.startsWith('/qa/desk/api/')) {
      if(req.headers.authorization!==`Bearer ${capability}`) {res.writeHead(401);res.end('{}');return;}
      if(req.url.endsWith('/stream')) {res.writeHead(200,{'Content-Type':'text/event-stream'});res.write('data: {}\n\n');return;}
      const name=req.url.split('/api/')[1].split('?')[0];
      const value=name==='health'?{status:'ok',read_only:true}:name==='status'?{instance_id:'synthetic',process_id:1,event_capacity:32,history_available:false,tasks_available:true,sse_enabled:true}:name==='summary'?{runtime:{provider:'mock',transport:'local-core'},total_events:1,tool_events:1,reflection_events:0,failed_events:0}:name==='events'?[row]:row;
      if(name==='task-workspaces') {
        res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({namespaces:[taskNamespace],limit:100,has_more:false,read_only:true}));return;
      }
      if(name.endsWith('/experience-uses')) {
        const namespace=new URL(req.url,'http://localhost').searchParams.get('namespace');
        res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({uses:namespace===taskNamespace?[syntheticUse]:[],limit:20,read_only:true}));return;
      }
      if(name==='tasks' || name.startsWith('tasks/')) {
        const namespace=new URL(req.url,'http://localhost').searchParams.get('namespace');
        const present=namespace===taskNamespace;
        res.writeHead(name==='tasks' || present ? 200 : 404,{'Content-Type':'application/json'});
        res.end(JSON.stringify(name==='tasks'?{tasks:present?[syntheticTask]:[],limit:100,read_only:true}:present?{snapshot:{task:syntheticTask,checkpoint:syntheticCheckpoint},read_only:true}:{error:'task not found'}));return;
      }
      res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify(value));return;
    }
    const pathname=new URL(req.url,'http://localhost').pathname;
    const filename=pathname==='/qa/desk/'?'index.html':pathname.startsWith('/qa/desk/assets/')?pathname.slice('/qa/desk/assets/'.length):'';
    if(!['index.html','app.mjs','transport.mjs','state.mjs','render.mjs','i18n.mjs','tasks.mjs','dashboard.css','memory_chan_hero.png','memory_chan_sidebar.png'].includes(filename)){res.writeHead(404);res.end();return;}
    const types={html:'text/html',mjs:'text/javascript',css:'text/css',png:'image/png'};
    res.writeHead(200,{'Content-Type':types[filename.split('.').at(-1)]});res.end(await readFile(path.join(assets,filename)));
  });
  await new Promise(resolve=>proxy.listen(0,'127.0.0.1',resolve));
  url=`http://127.0.0.1:${proxy.address().port}/qa/desk/`;
  }
  const profile=path.join(output,'chrome-profile');
  chrome=spawn(options['--chrome'],['--headless=new','--remote-debugging-port=0',`--user-data-dir=${profile}`,'--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--disable-sync','about:blank'],{stdio:['ignore','ignore','pipe']});
  chrome.stderr.pipe(createWriteStream(path.join(output,'chrome.stderr.log')));
  ownership={chrome:chrome.pid};
  await writeFile(path.join(output,'owned-processes.json'),JSON.stringify(ownership,null,2));
  let port; for(let i=0;i<150;i++){try{port=(await readFile(path.join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0];break;}catch{await pause(100);}}
  assert.ok(port,'Chrome must start');
  const version=await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  socket=new WebSocket(version.webSocketDebuggerUrl);await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=reject;});
  socket.onmessage=event=>{const message=JSON.parse(event.data);if(message.id){const pending=inflight.get(message.id);if(pending){clearTimeout(pending.timer);inflight.delete(message.id);message.error?pending.reject(Error(JSON.stringify(message.error))):pending.resolve(message.result);}}else if(message.method==='Runtime.exceptionThrown')errors.push({text:message.params.exceptionDetails.text}); else if(message.method==='Network.requestWillBeSent') { const r=message.params.request; requests.push({method:r.method,path:new URL(r.url).pathname,authorized:Object.entries(r.headers).some(([name,value])=>name.toLowerCase()==='authorization'&&value===`Bearer ${capability}`)}); }};
  const target=await call('Target.createTarget',{url:'about:blank'});session=(await call('Target.attachToTarget',{targetId:target.targetId,flatten:true})).sessionId;
  await call('Page.enable');await call('Runtime.enable');await call('Network.enable');
  await call('Emulation.setDeviceMetricsOverride',{width:1440,height:1050,deviceScaleFactor:1,mobile:false});
  await call('Page.navigate',{url});

  await until("document.querySelector('#owner-auth')?.open");
  await choose('#locale','en');
  await check('unauthenticated shell opens owner login without records',async()=>{
    assert.equal(await evaluate("document.querySelectorAll('#records button').length"),0);
    await screenshot('owner-login');
  });
  const login=async token=>{await evaluate(`document.querySelector('#owner-token').value=${JSON.stringify(token)}; document.querySelector('#owner-login').requestSubmit()`);};
  await check('wrong credential stays locked and clears password field',async()=>{
    await login('synthetic-wrong');await until("!document.querySelector('#owner-error').hidden");
    assert.equal(await evaluate("document.querySelector('#owner-token').value"),'');
    assert.equal(await evaluate("document.querySelector('#owner-auth').open"),true);
  });
  await check('owner login reads records and authenticated SSE without storage',async()=>{
    await login(capability);await until("!document.querySelector('#owner-auth').open && document.querySelectorAll('#records button').length>=1");
    await until("document.querySelector('#connection').textContent.includes('stream')");
    assert.equal(await evaluate('JSON.stringify([localStorage,sessionStorage,document.cookie,location.href]).includes('+JSON.stringify(capability)+')'),false);
    assert.ok(requests.some(r=>r.path.endsWith('/stream')&&r.authorized));
    await screenshot('owner-unlocked');
  });
  await check('owner task summary preserves requirements, pending candidates and stale checkpoint',async()=>{
    await until("!document.querySelector('#task-panel').hidden");
    await until("document.querySelectorAll('#task-workspaces option').length===1");
    assert.equal(await evaluate("document.querySelector('#task-workspaces option').value"),taskNamespace);
    assert.equal(await evaluate("document.querySelector('#task-namespace').value"),'');
    assert.ok((await evaluate("document.querySelector('[data-i18n=taskHostUnverified]').textContent")).includes('unverified'));
    assert.equal(await evaluate("document.querySelector('#task-select').value"),'');
    await evaluate(`document.querySelector('#task-namespace').value=${JSON.stringify(taskNamespace)};document.querySelector('#task-scope').requestSubmit()`);
    await until("document.querySelector('#task-select').options.length===2");
    await choose('#task-select','browser-task');await until("!document.querySelector('#task-content').hidden");
    const content=await evaluate("document.querySelector('#task-content').textContent");
    assert.ok(content.includes('Original browser goal'));assert.ok(content.includes('Paused'));
    assert.ok(content.includes('Earlier task revision'));assert.ok(content.includes('Not performed'));
    assert.equal(await evaluate("document.querySelectorAll('#task-requirements li').length"),1);
    assert.equal(await evaluate("Boolean(window.taskInjected)"),false);
    await until("document.querySelectorAll('#task-uses > li').length===1");
    const useText=await evaluate("document.querySelector('#task-uses').textContent");
    assert.ok(useText.includes('Reported adopted'));assert.ok(useText.includes('Not currently reusable'));
    assert.ok(useText.includes('Outcome sources'));assert.ok(useText.includes('event:'));
    assert.ok(useText.includes('not independently verified'));
    await evaluate("document.querySelector('#task-uses').parentElement.open=true;document.querySelector('#task-uses details').open=true");
    await screenshot('owner-experience-reports');
    await evaluate("document.querySelector('#task-content details').open=true;document.querySelector('#task-checkpoint').open=true");
    for(const [locale,label] of [['zh-CN','已暂停'],['ja','一時停止'],['en','Paused']]) {
      await choose('#locale',locale);
      assert.ok((await evaluate("document.querySelector('#task-fields').textContent")).includes(label));
    }
    await screenshot('owner-task-summary');
    await call('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
    await evaluate("document.querySelector('#task-panel').scrollIntoView({block:'start'})");
    assert.ok(await evaluate("document.documentElement.scrollWidth<=innerWidth+1"));
    await screenshot('owner-task-mobile');
    await call('Emulation.setDeviceMetricsOverride',{width:1440,height:1050,deviceScaleFactor:1,mobile:false});
    await evaluate("document.querySelector('#task-namespace').value='project/empty-browser-fixture';document.querySelector('#task-scope').requestSubmit()");
    await until("document.querySelector('#task-message').textContent.includes('No tasks')");
    assert.equal(await evaluate("document.querySelector('#task-fields').textContent"),'');
    assert.equal(await evaluate("document.querySelector('#task-uses').textContent"),'');
    await evaluate(`document.querySelector('#task-namespace').value=${JSON.stringify(taskNamespace)};document.querySelector('#task-scope').requestSubmit()`);
    await until("document.querySelector('#task-select').options.length===2");
    await choose('#task-select','browser-task');await until("!document.querySelector('#task-content').hidden");
  });
  await check('lock clears records and detail DOM then allows repeated login',async()=>{
    await click('#records button');await until("!document.querySelector('#detail-content').hidden");
    await click('#owner-logout');await until("document.querySelector('#owner-auth').open");
    assert.equal(await evaluate("document.querySelector('#payload').textContent"),'');
    assert.equal(await evaluate("document.querySelector('#task-fields').textContent"),'');
    assert.equal(await evaluate("document.querySelector('#task-uses').textContent"),'');
    assert.equal(await evaluate("document.querySelector('#task-namespace').value"),'');
    assert.equal(await evaluate("document.querySelectorAll('#task-workspaces option').length"),0);
    assert.equal(await evaluate("document.querySelectorAll('#records button').length"),0);
    await login(capability);await until("!document.querySelector('#owner-auth').open && document.querySelectorAll('#records button').length>=1");
  });
  await check('Back navigation requires fresh owner authentication',async()=>{
    const history=await call('Page.getNavigationHistory');
    const entry=history.entries[history.currentIndex].id;
    await call('Page.navigate',{url:'about:blank'});await until("location.href==='about:blank'");
    await call('Page.navigateToHistoryEntry',{entryId:entry});
    await until("document.querySelector('#owner-auth')?.open");
    assert.equal(await evaluate("document.querySelectorAll('#records button').length"),0);
    await login(capability);await until("!document.querySelector('#owner-auth').open");
  });
  await check('reload never retains owner authority; three locales and narrow layout',async()=>{
    await call('Page.reload');await until("document.querySelector('#owner-auth')?.open");
    await login(capability);await until("!document.querySelector('#owner-auth').open");
    for(const locale of ['zh-CN','ja','en']) {await choose('#locale',locale);assert.equal(await evaluate('document.documentElement.lang'),locale);}
    await call('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
    await click('#owner-logout');await until("document.querySelector('#owner-auth').open");
    assert.ok(await evaluate("document.querySelector('#owner-auth').getBoundingClientRect().right <= innerWidth"));
    await screenshot('owner-login-mobile');
  });
  assert.equal(errors.length,0,JSON.stringify(errors));
  await writeFile(path.join(output,'report.json'),JSON.stringify({passed:true,checks,requests,errors,backend:options['--binary']?'real-rust-core':'synthetic-node',limits:'Isolated synthetic data and test-only capability. Real Rust core only when --binary is supplied. No model, user database, production credential, or deployment.'},null,2));
} finally { await cleanup(); }
