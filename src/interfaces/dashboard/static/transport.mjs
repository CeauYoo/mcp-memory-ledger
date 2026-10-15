export class RequestError extends Error {
  constructor(code, status) { super(code); this.code = code; this.status = status; }
}
export async function requestJson(url, {signal, validate = () => true, fetcher = fetch, timeout = 8000} = {}) {
  const controller = new AbortController();
  const abort = () => controller.abort();
  if (signal?.aborted) abort(); else signal?.addEventListener('abort', abort, {once:true});
  const timer = setTimeout(abort, timeout);
  try {
    const response = await fetcher(url, {signal:controller.signal, cache:'no-store', credentials:'same-origin'});
    if (!response.ok) throw new RequestError('http', response.status);
    let value;
    try { value = await response.json(); } catch { throw new RequestError(controller.signal.aborted ? 'network' : 'invalid'); }
    if (!validate(value)) throw new RequestError('invalid');
    return value;
  } catch (error) {
    if (error instanceof RequestError) throw error;
    throw new RequestError('network');
  } finally { clearTimeout(timer); signal?.removeEventListener('abort', abort); }
}
// At most one refresh plus one coalesced follow-up; callers never overlap snapshots.
export function singleFlight(run) {
  let running = null, queued = false;
  return function refresh() {
    if (running) { queued = true; return running; }
    running = (async () => {
      do { queued = false; await run(); } while (queued);
    })().finally(() => { running = null; });
    return running;
  };
}
export function errorText(error, t) {
  if (error?.code === 'http') return t('httpError', {status:error.status});
  return t(error?.code === 'invalid' ? 'invalidData' : 'networkError');
}
