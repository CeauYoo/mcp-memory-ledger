import {renderTasks} from './tasks.mjs';
import {translator} from './i18n.mjs';
import {connectionState, filterRecords, WINDOW_LIMIT} from './state.mjs';
import {errorText} from './transport.mjs';
const $ = id => document.getElementById(id);
const text = (id, value) => { const node = $(id); if (node.textContent !== String(value)) node.textContent = value; };
const formatTime = (value, locale) => value ? new Date(value).toLocaleString(locale, {hour12:false}) : '—';
function fieldList(target, fields) {
  target.replaceChildren(...fields.map(([label, value]) => {
    const item = document.createElement('div'), dt = document.createElement('dt'), dd = document.createElement('dd');
    dt.textContent = label; dd.textContent = value; item.append(dt, dd); return item;
  }));
}
function recordButton(record, state, t) {
  const button = document.createElement('button');
  button.type = 'button'; button.dataset.record = record.id;
  button.setAttribute('aria-pressed', String(record.id === state.selected));
  const top = document.createElement('span'); top.className = 'record-top';
  const name = document.createElement('strong'); name.className = 'record-name'; name.textContent = record.operation;
  const status = document.createElement('span'); status.className = 'badge record-status'; status.textContent = t(`status_${record.status}`);
  if (record.status === 'failed' || record.status === 'rejected') status.dataset.tone = 'error';
  top.append(name, status);
  const summary = document.createElement('p'); summary.className = 'record-summary'; summary.textContent = record.summary || t('none');
  const bottom = document.createElement('span'); bottom.className = 'record-bottom';
  const time = document.createElement('time'); time.dateTime = record.timestamp; time.textContent = formatTime(record.timestamp, state.locale);
  const ns = document.createElement('span'); ns.textContent = record.namespace ?? t('none');
  const kind = document.createElement('span'); kind.className = 'record-kind'; kind.textContent = t(`kind_${record.kind}`);
  bottom.append(time, ns); button.append(top, kind, summary, bottom);
  return button;
}
function renderRecords(records, state, t) {
  const list = $('records'), existing = new Map([...list.children].map(li => [li.dataset.id, li]));
  const focused = document.activeElement?.dataset.record;
  records.forEach((record, index) => {
    let li = existing.get(record.id);
    if (!li) { li = document.createElement('li'); li.className = 'record'; li.dataset.id = record.id; }
    existing.delete(record.id);
    const signature = JSON.stringify([record, state.locale, state.selected === record.id]);
    if (li.signature !== signature) { li.replaceChildren(recordButton(record, state, t)); li.signature = signature; }
    if (list.children[index] !== li) list.insertBefore(li, list.children[index] ?? null);
  });
  existing.forEach(li => li.remove());
  // Preserve focus only when it was in this list. Refresh never steals input focus.
  if (focused) {
    const same = [...list.querySelectorAll('button')].find(button => button.dataset.record === focused);
    if (same && document.activeElement !== same) same.focus({preventScroll:true});
    else if (!same) $('refresh').focus({preventScroll:true});
  }
}
let detailSignature = '';
export function render(state) {
  renderTasks(state);
  const t = translator(state.locale), meta = state.metadata, summary = state.summary;
  const connection = connectionState(state);
  text('connection', connection === 'connected' ? t('connected', {mode:t(state.stream ? 'streaming' : 'polling')}) : t(connection));
  $('connection').dataset.tone = connection === 'offline' ? 'error' : connection === 'connected' ? 'ok' : 'warning';
  text('freshness', state.lastSuccess ? t('updated', {time:new Date(state.lastSuccess).toLocaleTimeString(state.locale,{hour12:false})}) : t('neverUpdated'));
  text('pause', t(state.paused ? 'resume' : 'pause')); $('pause').setAttribute('aria-pressed', String(state.paused));
  text('refresh', t(state.busy ? 'refreshing' : 'refresh')); $('refresh').disabled = state.busy;
  $('locale').value = state.locale; $('source').value = state.source;
  $('navigation').querySelectorAll('button').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.view === state.view)));
  text('view-title', t(state.view));
  for (const [key, field] of [['total','total_events'],['tools','tool_events'],['reflections','reflection_events'],['failed','failed_events']]) text(`metric-${key}`, summary?.[field] ?? '—');
  for (const [key,field] of [['total','total_events'],['reflections','reflection_events'],['failed','failed_events']]) text(`side-${key}`,summary?.[field] ?? '—');
  text('scope', meta ? t('scope', {pid:meta.process_id,count:summary?.total_events ?? '—',capacity:meta.event_capacity}) : t('connecting'));
  const latest = state.live?.[0]; text('latest-operation', latest?.operation ?? t('empty'));
  $('detail').setAttribute('aria-busy', String(state.detailState === 'loading'));
  text('latest-meta', latest ? [t(`kind_${latest.kind}`),t(`status_${latest.status}`)].join(' · ') : '—');
  const healthView = state.view === 'health';
  $('detail').hidden = healthView;
  $('result-count').hidden = healthView;
  text('runtime-transport', summary?.runtime.transport ? `${summary.runtime.transport} MCP` : 'MCP');
  $('filters').hidden = healthView; $('records-layout').hidden = healthView; $('health-view').hidden = !healthView;
  text('window-note', healthView ? t('httpHealth') : t(state.source === 'live' ? 'liveWindow' : 'historyWindow', {limit:WINDOW_LIMIT}));
  $('error').hidden = !state.error;
  if (state.error) text('error', t('requestError', {resource:t(state.error.resource),reason:errorText(state.error,t)}));
  if (healthView) fieldList($('health-fields'), [
    [t('httpHealth'),state.health?.status === 'ok' ? t('status_ok') : t('pendingHealth')],
    [t('checked'),formatTime(state.healthChecked,state.locale)],
    [t('readOnlyField'),state.health ? t(state.health.read_only ? 'yes' : 'no') : '—'],
    [t('provider'),summary?.runtime.provider ?? '—'],[t('transport'),summary?.runtime.transport ?? '—'],
    [t('process'),meta?.process_id ?? '—'],[t('instance'),meta?.instance_id ?? '—'],
    [t('started'),formatTime(meta?.started_at,state.locale)],
    [t('historyAvailable'),meta ? t(meta.history_available ? 'yes' : 'no') : '—'],[t('capacity'),meta?.event_capacity ?? '—']
  ]);
  const records = state[state.source], filtered = filterRecords(records ?? [], state);
  text('result-count', t('results', {count:filtered.length}));
  $('empty').hidden = filtered.length > 0;
  if (!filtered.length) text('empty', state.source === 'history' && meta && !meta.history_available ? t('unavailable') : records === null ? t('loading') : t(records.length ? 'noResults' : 'empty'));
  renderRecords(filtered, state, t);
  $('close-detail').hidden = !state.selected;
  $('selection-note').hidden = !state.selected || filtered.some(record => record.id === state.selected);
  text('selection-note', t('outsideWindow'));
  $('detail-content').hidden = !state.detail;
  $('detail-message').hidden = !!state.detail;
  text('detail-message', t(state.detailState === 'loading' ? 'loading' : state.detailState === 'expired' ? 'expired' : state.detailState === 'error' ? 'detailError' : 'selectRecord'));
  const signature = JSON.stringify([state.locale,state.detail]);
  if (state.detail && signature !== detailSignature) {
    const detail = state.detail;
    fieldList($('detail-fields'), [['id',detail.id],['operation',detail.operation],['kind',t(`kind_${detail.kind}`)],['status',t(`status_${detail.status}`)],['time',formatTime(detail.timestamp,state.locale)],['namespaceField',detail.namespace ?? t('none')],['correlation',detail.correlation_id ?? t('none')],['summary',detail.summary || t('none')]].map(([key,value]) => [t(key),value]));
    text('payload', JSON.stringify(detail.payload,null,2));
  }
  detailSignature = signature;
}
