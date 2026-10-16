export const WINDOW_LIMIT = 100;
// These are presentation groups; preserve the server's recorded kind in details.
export function matchesGroup(record, group) {
  if (group === 'reflections') return ['reflection', 'trigger'].includes(record.kind) || ['run_reflection','supersede_memory'].includes(record.operation);
  if (group === 'decisions') return record.kind === 'decision' || record.operation === 'decide_with_snapshot';
  if (group === 'snapshots') return record.kind === 'snapshot' || record.operation === 'build_self_snapshot';
  return true;
}
export function filterRecords(records, {view, status, namespace, search}) {
  const needle = search.trim().toLocaleLowerCase();
  return records.filter(record => matchesGroup(record, view)
    && (!status || record.status === status)
    && (!namespace || record.namespace === namespace)
    && (!needle || [record.operation, record.id, record.summary, record.correlation_id, record.namespace]
      .some(value => String(value ?? '').toLocaleLowerCase().includes(needle))));
}
export function latestFirst(records) {
  return [...records].sort((a, b) => Date.parse(b.timestamp) - Date.parse(a.timestamp) || (b.sequence ?? 0) - (a.sequence ?? 0));
}
export function connectionState({paused, lastSuccess, error}, now = Date.now()) {
  if (paused) return 'paused';
  if (error) return 'offline';
  if (!lastSuccess) return 'connecting';
  return now - lastSuccess > 10000 ? 'stale' : 'connected';
}
export function validRecords(value) {
  return Array.isArray(value) && value.every(record => record && typeof record.id === 'string'
    && typeof record.operation === 'string' && typeof record.kind === 'string'
    && typeof record.status === 'string' && typeof record.summary === 'string'
    && typeof record.timestamp === 'string' && Number.isFinite(Date.parse(record.timestamp)));
}
export function initialState(locale) {
  return {locale, view:'all', source:'live', status:'', namespace:'', search:'', paused:false,
    taskWorkspaces:null, taskWorkspaceError:null, tasks:null, taskNamespace:'', taskId:'', taskSnapshot:null, taskUses:null, taskUseError:null, taskError:null, taskBusy:false,
    busy:false, lastSuccess:0, error:null, stream:false, metadata:null, summary:null,
    live:null, history:null, health:null, healthChecked:0, selected:null, detail:null, detailState:'idle'};
}
