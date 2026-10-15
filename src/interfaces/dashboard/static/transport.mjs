// Capabilities are deliberately tab-memory only: never storage, cookies, or URLs.
let capability = '';
export function setCapability(value) { capability = value; }
export function clearCapability() { capability = ''; }
function authHeaders(url) {
  if (!capability) return {};
  if (typeof location !== 'undefined' && new URL(url, location.href).origin !== location.origin) throw new Error('cross-origin');
  return {Authorization:`Bearer ${capability}`};
}
export function openEventStream(url, {onopen, onmessage, onerror, fetcher = fetch} = {}) {
  const controller = new AbortController();
  void (async () => {
    try {
      const response = await fetcher(url, {headers:authHeaders(url), signal:controller.signal, cache:'no-store', credentials:'omit', redirect:'error'});
      if (!response.ok || !response.headers.get('content-type')?.startsWith('text/event-stream')) throw new Error('stream');
      onopen?.();
      const reader = response.body.getReader(), decoder = new TextDecoder();
      let pending = '';
      while (!controller.signal.aborted) {
        const {value, done} = await reader.read();
        if (done) break;
        pending += decoder.decode(value, {stream:true}).replace(/\r/g, '');
        let end;
        while ((end = pending.indexOf('\n\n')) >= 0) {
          const frame = pending.slice(0,end); pending = pending.slice(end+2);
          if (frame.split('\n').some(line => line.startsWith('data:'))) onmessage?.();
        }
        if (pending.length > 4 * 1024 * 1024) throw new Error('oversized stream frame');
      }
      if (!controller.signal.aborted) onerror?.();
    } catch { if (!controller.signal.aborted) onerror?.(); }
    finally { controller.abort(); }
  })();
  return {close() { controller.abort(); }};
}
export class RequestError extends Error {
  constructor(code, status) { super(code); this.code = code; this.status = status; }
}
export async function requestJson(url, {signal, validate = () => true, fetcher = fetch, timeout = 8000} = {}) {
  const controller = new AbortController();
  const abort = () => controller.abort();
  if (signal?.aborted) abort(); else signal?.addEventListener('abort', abort, {once:true});
  const timer = setTimeout(abort, timeout);
  try {
    const response = await fetcher(url, {signal:controller.signal, cache:'no-store', credentials:'omit', redirect:'error', headers:authHeaders(url)});
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
