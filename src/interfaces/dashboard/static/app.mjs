import {applyLocale, chooseLocale} from './i18n.mjs';
import {initialState, latestFirst, validRecords, WINDOW_LIMIT} from './state.mjs';
import {requestJson, singleFlight} from './transport.mjs';
import {render} from './render.mjs';
const $ = id => document.getElementById(id), base = new URL('../', import.meta.url);
let saved; try { saved = localStorage.getItem('memory-ledger-locale'); } catch { /* Private mode still works. */ }
const state = initialState(chooseLocale(saved,navigator.languages));
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
  stream = new EventSource(api('api/events/stream'));
  stream.onopen = () => { state.stream = true; draw(); };
  stream.onerror = () => { state.stream = false; draw(); }; // Polling remains the resynchronization path.
  stream.onmessage = () => {
    if (!streamTimer) streamTimer = setTimeout(() => { streamTimer = null; if (!state.paused) void refresh(); },200);
  };
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
    if (state.metadata && state.metadata.instance_id !== metadata.instance_id) { clearSelection(); state.history = null; closeStream(); }
    Object.assign(state,{metadata,summary,live:latestFirst(live),health,healthChecked:Date.now()});
    if (source === 'history' && !healthView && metadata.history_available) {
      const history = await get(`api/operation-log?limit=${WINDOW_LIMIT}`,validRecords);
      if (generation !== epoch) return;
      state.history = latestFirst(history);
    } else if (!metadata.history_available) state.history = null;
    state.lastSuccess = Date.now(); state.error = null; syncStream();
  } catch (error) {
    if (generation !== epoch) return;
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
  if (state.paused) { epoch++; controller?.abort(); state.busy = false; clearTimeout(pollTimer); closeStream(); }
  else { syncStream(); void refresh(); }
  draw();
});
$('refresh').addEventListener('click', () => { manualRefresh = true; void refresh(); });
window.addEventListener('online', () => { if (!state.paused) void refresh(); });
window.addEventListener('pagehide', () => { state.paused = true; epoch++; controller?.abort(); detailController?.abort(); closeStream(); clearTimeout(pollTimer); });
window.addEventListener('pageshow', event => { if (event.persisted) { state.paused = false; void refresh(); } });
applyLocale(state.locale); draw(); void refresh();
