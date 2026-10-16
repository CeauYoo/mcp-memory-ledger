import {applyLocale, chooseLocale} from './i18n.mjs';
import {initialState, latestFirst, validRecords, WINDOW_LIMIT} from './state.mjs';
import {requestJson, singleFlight, setCapability, clearCapability, openEventStream} from './transport.mjs';
import {validTaskList, validTaskSnapshot, validExperienceUses, validTaskWorkspaces} from './tasks.mjs';
import {render} from './render.mjs';
const $ = id => document.getElementById(id), base = new URL('../', import.meta.url);
let saved; try { saved = localStorage.getItem('memory-ledger-locale'); } catch { /* Private mode still works. */ }
const state = initialState(chooseLocale(saved,navigator.languages));
let authEpoch = 0, authController;
let taskEpoch = 0, taskController;
let epoch = 0, detailEpoch = 0, controller, detailController, stream, streamTimer, pollTimer, manualRefresh = false;
const draw = () => render(state);
const api = path => new URL(path,base);
function clearSelection() {
  detailEpoch++; detailController?.abort(); state.selected = null; state.detail = null; state.detailState = 'idle';
}
function closeStream() {
  stream?.close(); stream = null; clearTimeout(streamTimer); streamTimer = null; state.stream = false;
}
function syncStream() {
  if (state.paused || !state.metadata?.sse_enabled) { closeStream(); return; }
  if (stream) return;
  stream = openEventStream(api('api/events/stream'), {
    onopen: () => { state.stream = true; draw(); },
    onerror: () => { closeStream(); draw(); },
    onmessage: () => {
      if (!streamTimer) streamTimer = setTimeout(() => { streamTimer = null; if (!state.paused) void refresh(); },200);
    }
  });
}
const validSummary = value => value?.runtime && typeof value.runtime.provider === 'string'
  && ['total_events','tool_events','reflection_events','failed_events'].every(key => Number.isInteger(value[key]) && value[key] >= 0);
const validMetadata = value => typeof value?.instance_id === 'string' && Number.isInteger(value.process_id)
  && Number.isInteger(value.event_capacity) && typeof value.history_available === 'boolean' && typeof value.sse_enabled === 'boolean';
const validHealth = value => value?.status === 'ok' && value.read_only === true;
const refresh = singleFlight(async () => {
  if (state.paused && !manualRefresh) return;
  manualRefresh = false;
  clearTimeout(pollTimer);
  const generation = epoch, source = state.source, healthView = state.view === 'health';
  const requestController = new AbortController();
  controller = requestController; state.busy = true; draw();
  const get = (path, validate) => requestJson(api(path), {signal:requestController.signal,validate});
  try {
    const [metadata,summary,live,health] = await Promise.all([
      get('api/status',validMetadata), get('api/summary',validSummary),
      get(`api/events?limit=${WINDOW_LIMIT}`,validRecords), get('api/health',validHealth)
    ]);
    if (generation !== epoch) return;
    if (state.metadata && state.metadata.instance_id !== metadata.instance_id) { clearSelection(); state.history = null; clearTasks(); closeStream(); }
    Object.assign(state,{metadata,summary,live:latestFirst(live),health,healthChecked:Date.now()});
    if (source === 'history' && !healthView && metadata.history_available) {
      const history = await get(`api/operation-log?limit=${WINDOW_LIMIT}`,validRecords);
      if (generation !== epoch) return;
      state.history = latestFirst(history);
    } else if (!metadata.history_available) state.history = null;
    state.lastSuccess = Date.now(); state.error = null; syncStream();
    await refreshTasks();
  } catch (error) {
    if (generation !== epoch) return;
    if (error.status === 401) lock();
    state.error = {code:error.code,status:error.status,resource:healthView ? 'healthResource' : source === 'history' ? 'historyResource' : 'liveResource'};
  } finally {
    requestController.abort(); // A failed sibling must not outlive this refresh group.
    if (generation === epoch) { state.busy = false; draw(); }
    if (!state.paused) pollTimer = setTimeout(() => void refresh(),2500);
  }
});
function changeSource(source, view = state.view) {
  epoch++; controller?.abort(); clearSelection();
  state.source = source; state.view = view; state.error = null; state.busy = false;
  draw(); manualRefresh = true; void refresh();
}
async function selectRecord(id) {
  clearSelection(); const generation = detailEpoch, source = state.source;
  state.selected = id; state.detailState = 'loading'; draw();
  if (matchMedia('(max-width:1200px)').matches) { $('detail').focus({preventScroll:true}); $('detail').scrollIntoView({block:'start'}); }
  detailController = new AbortController();
  try {
    const detail = await requestJson(api(`api/${source === 'live' ? 'events' : 'operation-log'}/${encodeURIComponent(id)}`), {
      signal:detailController.signal,validate:value => validRecords([value]) && value.id === id && value.read_only === true
    });
    if (generation !== detailEpoch || source !== state.source) return;
    state.detail = detail; state.detailState = 'ready';
  } catch (error) {
    if (generation !== detailEpoch) return;
    state.detailState = error.status === 404 ? 'expired' : 'error';
  }
  draw();
  if (document.activeElement === $('detail') && matchMedia('(max-width:1200px)').matches) $('detail').scrollIntoView({block:'start'});
}
function clearTasks() {
  taskEpoch++; taskController?.abort();
  Object.assign(state,{taskWorkspaces:null,taskWorkspaceError:null,tasks:null,taskId:'',taskSnapshot:null,taskUses:null,taskUseError:null,taskError:null,taskBusy:false});
}
async function refreshTasks() {
  const generation=++taskEpoch; taskController?.abort();
  if (!state.metadata?.tasks_available) { clearTasks(); return; }
  taskController=new AbortController(); const signal=taskController.signal;
  const namespace=state.taskNamespace, selected=state.taskId;
  state.taskBusy=true; state.taskError=null; draw();
  try {
    try {
      const workspaces=await requestJson(api('api/task-workspaces'),{signal,validate:validTaskWorkspaces});
      if(generation!==taskEpoch)return;
      state.taskWorkspaces=workspaces;state.taskWorkspaceError=null;
    } catch(error) {
      if(generation!==taskEpoch)return;
      if(error.status===401){lock();return;}
      state.taskWorkspaces=null;state.taskWorkspaceError={code:error.code,status:error.status};
    }
    if(!namespace)return;
    const list=await requestJson(api(`api/tasks?${new URLSearchParams({namespace})}`),{signal,validate:validTaskList});
    if(generation!==taskEpoch)return;
    if(list.tasks.some(task=>task.namespace!==namespace))throw Object.assign(new Error(),{code:'invalid'});
    state.tasks=list.tasks;
    if(selected) {
      const value=await requestJson(api(`api/tasks/${encodeURIComponent(selected)}?${new URLSearchParams({namespace})}`),{signal,validate:validTaskSnapshot});
      if(generation!==taskEpoch)return;
      if(value.snapshot.task.namespace!==namespace || value.snapshot.task.task_id!==selected)throw Object.assign(new Error(),{code:'invalid'});
      state.taskSnapshot=value.snapshot;
      try {
        const uses=await requestJson(api(`api/tasks/${encodeURIComponent(selected)}/experience-uses?${new URLSearchParams({namespace})}`),{signal,validate:validExperienceUses});
        if(generation!==taskEpoch)return;
        if(uses.uses.some(use=>use.record.observation.namespace!==namespace || use.record.observation.task_id!==selected))throw Object.assign(new Error(),{code:'invalid'});
        state.taskUses=uses.uses;state.taskUseError=null;
      } catch(error) {
        if(generation!==taskEpoch)return;
        if(error.status===401){lock();return;}
        state.taskUses=null;state.taskUseError={code:error.code,status:error.status};
      }
    }
  } catch(error) {
    if(generation!==taskEpoch)return;
    if(error.status===401){lock();return;}
    state.taskSnapshot=null; state.taskUses=null; state.taskUseError=null; state.taskError={code:error.code,status:error.status};
  } finally { if(generation===taskEpoch){state.taskBusy=false;draw();} }
}
$('task-scope').addEventListener('submit',event=>{
  event.preventDefault();clearTasks();state.taskNamespace=$('task-namespace').value.trim();draw();void refreshTasks();
});
$('task-select').addEventListener('change',event=>{
  state.taskId=event.target.value;state.taskSnapshot=null;state.taskUses=null;state.taskUseError=null;draw();void refreshTasks();
});
$('navigation').addEventListener('click', event => {
  const view = event.target.closest('button[data-view]')?.dataset.view;
  if (!view) return;
  changeSource(view === 'history' ? 'history' : view === 'health' ? state.source : 'live',view);
});
$('source').addEventListener('change', event => changeSource(event.target.value, ['history','all'].includes(state.view) ? (event.target.value === 'history' ? 'history' : 'all') : state.view));
for (const [id,field] of [['status-filter','status'],['namespace-filter','namespace'],['search-filter','search']]) {
  $(id).addEventListener(id === 'status-filter' ? 'change' : 'input', event => { state[field] = event.target.value; draw(); });
}
$('clear').addEventListener('click', () => {
  state.status = state.namespace = state.search = '';
  for (const id of ['status-filter','namespace-filter','search-filter']) $(id).value = '';
  draw();
});
$('records').addEventListener('click', event => { const button = event.target.closest('button[data-record]'); if (button) void selectRecord(button.dataset.record); });
$('close-detail').addEventListener('click', () => {
  const id = state.selected; clearSelection(); draw();
  const destination = [...$('records').querySelectorAll('button')].find(button => button.dataset.record === id) ?? $('refresh');
  destination.focus({preventScroll:true}); destination.scrollIntoView({block:'center'});
});
$('locale').addEventListener('change', event => {
  state.locale = chooseLocale(event.target.value); applyLocale(state.locale); draw();
  try { localStorage.setItem('memory-ledger-locale',state.locale); } catch { /* Optional preference persistence. */ }
});
$('pause').addEventListener('click', () => {
  state.paused = !state.paused;
  if (state.paused) { epoch++; controller?.abort(); taskEpoch++; taskController?.abort(); state.taskBusy = false; state.busy = false; clearTimeout(pollTimer); closeStream(); }
  else { syncStream(); void refresh(); }
  draw();
});
$('refresh').addEventListener('click', () => { manualRefresh = true; void refresh(); });
window.addEventListener('online', () => { if (!state.paused) void refresh(); });
window.addEventListener('pagehide', () => { if (!$('owner-logout').hidden) lock(); authEpoch++; authController?.abort(); clearTasks(); clearCapability(); state.paused = true; epoch++; controller?.abort(); detailController?.abort(); closeStream(); clearTimeout(pollTimer); });
window.addEventListener('pageshow', event => { if (event.persisted) void start(); });
function lock() {
  authEpoch++; authController?.abort();
  epoch++; detailEpoch++; controller?.abort(); detailController?.abort(); closeStream(); clearTimeout(pollTimer);
  clearTasks(); state.taskNamespace = ''; $('task-namespace').value = ''; clearCapability(); state.paused = true;
  Object.assign(state, {live:[],history:null,summary:null,metadata:null,selected:null,detail:null,detailState:"idle",lastSuccess:0,health:null,healthChecked:0,busy:false,error:null});
  $('detail-fields').replaceChildren(); $('payload').textContent = ''; $('health-fields').replaceChildren();
  draw(); if (!$('owner-auth').open) $('owner-auth').showModal(); $('owner-token').focus();
}
$('owner-logout').addEventListener('click', lock);
$('owner-auth').addEventListener('cancel', event => event.preventDefault());
$('owner-login').addEventListener('submit', async event => {
  event.preventDefault(); const button = $('owner-submit'); button.disabled = true;
  const generation = ++authEpoch; authController?.abort(); authController = new AbortController();
  setCapability($('owner-token').value); $('owner-token').value = '';
  try {
    await requestJson(api('api/health'), {validate:validHealth,signal:authController.signal});
    if (generation !== authEpoch) return;
    $('owner-auth').close(); $('owner-error').hidden = true; $('owner-logout').hidden = false;
    state.paused = false; void refresh();
  } catch { if (generation !== authEpoch) return; clearCapability(); $('owner-error').hidden = false; $('owner-token').focus(); }
  finally { button.disabled = false; }
});
async function start() {
  try { await requestJson(api('api/health'), {validate:validHealth}); state.paused = false; void refresh(); }
  catch (error) { if (error.status === 401) lock(); else { state.paused = false; void refresh(); } }
}
applyLocale(state.locale); draw(); void start();
