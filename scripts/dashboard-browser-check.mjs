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
for (const key of ['--chrome','--binary','--output']) assert.ok(options[key],`${key} is required`);
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
try {
  child=spawn(options['--python']??'python3',['-B','scripts/dashboard-browser-fixture.py',path.resolve(options['--binary']),path.join(output,'fixture')],{stdio:['pipe','pipe','pipe']});
  child.stderr.pipe(createWriteStream(path.join(output,'fixture.stderr.log')));
  createInterface({input:child.stdout}).on('line',line=>{const value=JSON.parse(line);const waiter=fixtureWaiters.shift();if(waiter)waiter(value);else fixtureQueue.push(value);});
  const fixture=await nextFixture(), upstream=new URL(fixture.url);
  proxy=createServer((req,res)=>{
    requests.push({method:req.method,path:req.url});
    const api=req.url.includes('/api/');
    if(api&&fault==='partial'&&req.url.endsWith('/api/status')) {res.writeHead(503,{'Content-Type':'application/json'});res.end('{}');return;}
    if(api&&fault==='partial') {slowPending++;res.once('close',()=>slowPending--);}
    if (api && fault==='503') {res.writeHead(503,{'Content-Type':'application/json'});res.end('{}');return;}
    if (fault==='empty' && req.url.includes('/api/events?')) {res.writeHead(200,{'Content-Type':'application/json'});res.end('[]');return;}
    if (api && fault==='malformed') {res.writeHead(200,{'Content-Type':'application/json'});res.end('{');return;}
    const forward=()=>{
      if(res.destroyed)return;
      const remote=httpRequest({hostname:'127.0.0.1',port:upstream.port,path:req.url,method:req.method},response=>{
        if(inject&&req.url.includes('/api/events?')) {
          let body='';response.on('data',chunk=>body+=chunk);response.on('end',()=>{
            const data=JSON.parse(body);data.push({id:'synthetic-xss',operation:'<img src=x onerror="window.injected=true">'+'長'.repeat(180),kind:'tool',status:'ok',namespace:'very-long-namespace-'+ 'a'.repeat(220),summary:'<script>window.injected=true</script>',timestamp:'2029-01-01T00:00:00Z',payload:{}});
            res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify(data));
          });
        } else {res.writeHead(response.statusCode,response.headers);response.pipe(res);}
      });
      remote.on('error',()=>{if(!res.headersSent)res.writeHead(502);res.end();});
      res.on('close',()=>remote.destroy());req.pipe(remote);
    };
    if(api&&fault==='partial')setTimeout(forward,3000);else if(api&&fault==='delay')setTimeout(forward,700);else forward();
  });
  await new Promise(resolve=>proxy.listen(0,'127.0.0.1',resolve));
  const url=`http://127.0.0.1:${proxy.address().port}/qa/desk`;
  const profile=path.join(output,'chrome-profile');
  chrome=spawn(options['--chrome'],['--headless=new','--remote-debugging-port=0',`--user-data-dir=${profile}`,'--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--disable-sync','about:blank'],{stdio:['ignore','ignore','pipe']});
  chrome.stderr.pipe(createWriteStream(path.join(output,'chrome.stderr.log')));
  ownership={fixture:child.pid,mcp:fixture.pid,chrome:chrome.pid};
  await writeFile(path.join(output,'owned-processes.json'),JSON.stringify(ownership,null,2));
  let port; for(let i=0;i<150;i++){try{port=(await readFile(path.join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0];break;}catch{await pause(100);}}
  assert.ok(port,'Chrome must start');
  const version=await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  socket=new WebSocket(version.webSocketDebuggerUrl);await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=reject;});
  socket.onmessage=event=>{const message=JSON.parse(event.data);if(message.id){const pending=inflight.get(message.id);if(pending){clearTimeout(pending.timer);inflight.delete(message.id);message.error?pending.reject(Error(JSON.stringify(message.error))):pending.resolve(message.result);}}else if(message.method==='Runtime.exceptionThrown')errors.push(message.params.exceptionDetails);};
  const target=await call('Target.createTarget',{url:'about:blank'});session=(await call('Target.attachToTarget',{targetId:target.targetId,flatten:true})).sessionId;
  await call('Page.enable');await call('Runtime.enable');await call('Network.enable');
  await call('Emulation.setDeviceMetricsOverride',{width:1440,height:1050,deviceScaleFactor:1,mobile:false});
  await call('Page.navigate',{url});
  await until("document.querySelectorAll('#records button').length >= 30 && !document.querySelector('#refresh').disabled");
  await check('non-root redirect, module load, real MCP records and process metadata',async()=>{
    assert.equal(await evaluate('location.pathname'),'/qa/desk/');
    assert.ok((await evaluate("document.getElementById('scope').textContent")).includes(String(fixture.pid)));
    assert.equal(await evaluate("document.querySelectorAll('#status-filter option').length"),7);
  });
  await choose('#locale','en');
  await check('actual navigation filters snapshots and decisions',async()=>{
    await click('[data-view="snapshots"]');await until("!document.querySelector('#refresh').disabled && document.querySelectorAll('#records button').length===1");
    assert.ok((await evaluate("document.querySelector('#records').textContent")).includes('build_self_snapshot'));
    await click('[data-view="decisions"]');await until("!document.querySelector('#refresh').disabled && document.querySelectorAll('#records button').length===1");
    assert.ok((await evaluate("document.querySelector('#records').textContent")).includes('decide_with_snapshot'));
    await click('[data-view="all"]');await until("document.querySelectorAll('#records button').length>=30");
  });
  await check('selection and keyboard focus survive a real MCP append and polling',async()=>{
    await click('#records li:nth-child(5) button');await until("!document.querySelector('#detail-content').hidden");
    const selected=await evaluate("document.querySelector('#records [aria-pressed=true]').dataset.record");
    await evaluate("document.querySelector('#search-filter').focus()");
    child.stdin.write(JSON.stringify({action:'append',number:31})+'\n');await nextFixture();
    await until("Number(document.querySelector('#metric-total').textContent)>=33");await pause(2800);
    assert.equal(await evaluate("document.querySelector('#records [aria-pressed=true]').dataset.record"),selected);
    assert.equal(await evaluate('document.activeElement.id'),'search-filter');
  });
  await check('language switch persists selection and preference, all three layouts',async()=>{
    for(const locale of ['zh-CN','ja','en']) {await choose('#locale',locale);assert.equal(await evaluate('document.documentElement.lang'),locale);assert.ok(await evaluate("!!document.querySelector('#records [aria-pressed=true]')"));await evaluate('scrollTo(0,0)');await screenshot('desktop-'+locale);}
  });
  await check('exact namespace, status, empty results and clear filters',async()=>{
    await evaluate("const input=document.querySelector('#namespace-filter');input.value='project/dashboard-qa';input.dispatchEvent(new Event('input',{bubbles:true}));");
    assert.ok(await evaluate("[...document.querySelectorAll('.record-bottom span')].every(e=>e.textContent==='project/dashboard-qa')"));
    await choose('#status-filter','failed');assert.equal(await evaluate("document.querySelectorAll('#records button').length"),1);
    await evaluate("const search=document.querySelector('#search-filter');search.value='no-such-record';search.dispatchEvent(new Event('input',{bubbles:true}));");
    assert.equal(await evaluate("document.querySelectorAll('#records button').length"),0);
    assert.equal(await evaluate("document.querySelector('#selection-note').hidden"),false);
    await click('#clear');assert.ok(await evaluate("document.querySelectorAll('#records button').length>=30"));
  });
  await check('history detail retains stored timestamp and nullable summaries',async()=>{
    await click('[data-view="history"]');await until("!document.querySelector('#refresh').disabled && document.querySelectorAll('#records button').length>0");await click('#records button');await until("document.querySelector('#payload').textContent.includes('request_summary_json')");
    assert.ok((await evaluate("document.querySelector('#detail-fields').textContent")).includes('Time'));
  });
  await check('rapid source changes ignore delayed old responses',async()=>{
    fault='delay';await click('[data-view="history"]');await pause(80);await click('[data-view="snapshots"]');await pause(80);await click('[data-view="all"]');fault='';
    await until("!document.querySelector('#refresh').disabled && document.querySelectorAll('#records button').length>=30");assert.equal(await evaluate("document.querySelector('#source').value"),'live');assert.equal(await evaluate("document.querySelector('#close-detail').hidden"),true);
  });
  await check('HTTP failure, malformed payload, offline feedback and recovery',async()=>{
    for(const failure of ['503','malformed']) {fault=failure;await click('#refresh');await until("!document.querySelector('#error').hidden");assert.equal(await evaluate("document.querySelector('#connection').dataset.tone"),'error');fault='';await until("!document.querySelector('#refresh').disabled");await click('#refresh');await until("document.querySelector('#error').hidden");}
    await call('Network.emulateNetworkConditions',{offline:true,latency:0,downloadThroughput:0,uploadThroughput:0});await click('#refresh');await until("!document.querySelector('#error').hidden");await screenshot('offline');
    await call('Network.emulateNetworkConditions',{offline:false,latency:0,downloadThroughput:-1,uploadThroughput:-1});await until("!document.querySelector('#refresh').disabled");await click('#refresh');await until("document.querySelector('#error').hidden");
  });
  await check('one failed API cancels slow sibling requests before the next refresh',async()=>{
    fault='partial';await click('#refresh');await until("!document.querySelector('#error').hidden");await pause(150);assert.equal(slowPending,0);
    fault='';await click('#refresh');await until("document.querySelector('#error').hidden");
  });
  await check('pause/resume and explicit refresh while paused',async()=>{
    await click('#pause');const count=requests.length;await pause(3000);assert.equal(requests.length,count);assert.ok((await evaluate("document.querySelector('#connection').textContent")).includes('paused'));
    await click('#refresh');await until("!document.querySelector('#refresh').disabled");assert.ok(requests.length>count);await click('#pause');await until("document.querySelector('#connection').textContent.includes('Connected')");
  });
  await check('SSE refresh resumes after cancelling a pending event timer',async()=>{
    child.stdin.write(JSON.stringify({action:'append',number:32})+'\n');await nextFixture();await click('#pause');await pause(300);await click('#pause');await until("!document.querySelector('#refresh').disabled");
    const count=Number(await evaluate("document.querySelector('#metric-total').textContent"));
    child.stdin.write(JSON.stringify({action:'append',number:33})+'\n');await nextFixture();
    await until(`Number(document.querySelector('#metric-total').textContent)>${count}`,1500);
  });
  await check('empty source has an explicit state',async()=>{
    fault='empty';await click('#refresh');await until("!document.querySelector('#refresh').disabled && document.querySelectorAll('#records button').length===0");
    assert.ok((await evaluate("document.querySelector('#empty').textContent")).includes('No records.'));
    fault='';await click('#refresh');await until("document.querySelectorAll('#records button').length>=30");
  });
  await check('long text and untrusted record markup remain text across 15 responsive layouts',async()=>{
    inject=true;await click('#refresh');await until("document.querySelector('#records').textContent.includes('<img')");
    for(const locale of ['en','zh-CN','ja']) {
      await choose('#locale',locale);
      for(const width of [320,375,768,1024,1440]) {
        await call('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:false});await evaluate('scrollTo(0,0)');
        assert.equal(await evaluate('document.documentElement.scrollWidth<=innerWidth'),true,`${locale} ${width} overflow`);
        assert.equal(await evaluate("document.querySelectorAll('#records img,#records script').length"),0);assert.equal(await evaluate('window.injected??false'),false);
        assert.ok(await evaluate("[...document.querySelectorAll('#navigation button')].every(b=>b.getBoundingClientRect().height>=44)"));
        await screenshot(`layout-${locale}-${width}`);
      }
    }
    await click('#records button');await until("document.querySelector('#detail-message').textContent.includes('利用できません')");checks.push('missing detail reports expired record');
    inject=false;
  });
  await check('mobile selection reaches details and close returns to the record',async()=>{
    await choose('#locale','en');await call('Emulation.setDeviceMetricsOverride',{width:375,height:812,deviceScaleFactor:1,mobile:false});await click('#refresh');await until("!document.querySelector('#refresh').disabled");
    await click('#records li:nth-child(3) button');await until("!document.querySelector('#detail-content').hidden");
    assert.equal(await evaluate('document.activeElement.id'),'detail');assert.ok(await evaluate("document.querySelector('#detail').getBoundingClientRect().top>=0 && document.querySelector('#detail').getBoundingClientRect().top<innerHeight/3"));await screenshot('mobile-detail');
    await click('#close-detail');assert.ok(await evaluate("Boolean(document.activeElement.dataset.record)"));assert.ok(await evaluate("document.activeElement.getBoundingClientRect().top<innerHeight"));
  });
  await check('200 percent zoom, keyboard navigation and reduced motion',async()=>{
    await call('Emulation.setDeviceMetricsOverride',{width:1024,height:900,deviceScaleFactor:1,mobile:false});
    // CSS zoom exercises actual reflow; viewport is additionally checked at 320 CSS px above.
    await evaluate("document.documentElement.style.zoom='2'");assert.equal(await evaluate('document.documentElement.scrollWidth<=innerWidth'),true);assert.ok(await evaluate("[...document.querySelectorAll('.metric')].every(e=>e.getBoundingClientRect().width>=180)"));await evaluate('scrollTo(0,0)');await screenshot('zoom-200');await evaluate("document.querySelector('.operations').scrollIntoView({block:'start'})");await screenshot('zoom-200-controls');await evaluate("document.documentElement.style.zoom='1'");
    await call('Emulation.setEmulatedMedia',{features:[{name:'prefers-reduced-motion',value:'reduce'}]});assert.equal(await evaluate("getComputedStyle(document.querySelector('#refresh')).transitionDuration"),'0s');
    await evaluate("document.querySelector('#pause').focus()");await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Tab',code:'Tab',windowsVirtualKeyCode:9});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Tab',code:'Tab',windowsVirtualKeyCode:9});assert.equal(await evaluate('document.activeElement.id'),'refresh');
    await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await until("!document.querySelector('#refresh').disabled");
  });
  await check('health view checks actual endpoint with read-only semantics',async()=>{
    await click('[data-view="health"]');await until("!document.querySelector('#refresh').disabled && !document.querySelector('#health-view').hidden");assert.ok((await evaluate("document.querySelector('#health-fields').textContent")).includes('mock'));
    assert.ok(requests.every(request=>request.method==='GET'));assert.deepEqual(errors,[]);
  });
  await choose('#locale','ja');await call('Page.reload');await until("document.querySelectorAll('#records button').length>=30");assert.equal(await evaluate('document.documentElement.lang'),'ja');checks.push('language preference survives reload');
  await writeFile(path.join(output,'report.json'),JSON.stringify({passed:true,browser:version.Browser,fixture,checks,requests,errors,limits:'Synthetic mock data; isolated runtime only. No user database, credentials, paid calls, deployment restart or fresh-machine claim.'},null,2));
  console.log(`Report: ${output}/report.json`);
} catch(error) {
  await writeFile(path.join(output,'failure.json'),JSON.stringify({message:error.message,checks,errors,requests},null,2));
  try {await screenshot('failure');}catch{}
  throw error;
} finally {
  await cleanup();
}
