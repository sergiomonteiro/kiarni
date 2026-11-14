// Product SSE dialect. Bounded UTF-8 frame, no document JSON coercion or replay.
export const MAX_FRAME_BYTES = 3 * 1024 * 1024;
const MAX_LINES = 8192;
export async function* subscribe(path, token, {signal, fetchImpl = globalThis.fetch} = {}) {
  if (!/^\/v1\/collections\/[A-Za-z0-9_-]{1,128}\/changes$/.test(path)) throw new Error('invalid_path');
  const response = await fetchImpl(path, {
    headers: {Accept: 'text/event-stream', Authorization: `Bearer ${token}`},
    signal, mode: 'same-origin', credentials: 'omit', redirect: 'error',
    cache: 'no-store', referrerPolicy: 'no-referrer'
  });
  if (response.status !== 200 || !response.body ||
      response.headers.get('content-type')?.split(';')[0].trim() !== 'text/event-stream') {
    await response.body?.cancel();
    const error = new Error(response.status === 401 ? 'unauthorized' : `stream_http_${response.status}`);
    throw error;
  }
  const reader = response.body.getReader();
  const abort = () => { reader.cancel().catch(() => {}); };
  signal?.addEventListener('abort', abort, {once:true});
  const bytes = new Uint8Array(MAX_FRAME_BYTES);
  const decoder = new TextDecoder('utf-8', {fatal:true});
  let used = 0, lineStart = 0, lines = 0, ready = false;
  try {
    while (true) {
      if (signal?.aborted) throw new Error('aborted');
      const chunk = await reader.read();
      if (signal?.aborted) throw new Error('aborted');
      if (chunk.done) throw new Error('stream_interrupted');
      for (const byte of chunk.value) {
        if (used === MAX_FRAME_BYTES) throw new Error('frame_too_large');
        bytes[used++] = byte;
        if (byte !== 10) continue;
        if (++lines > MAX_LINES) throw new Error('too_many_lines');
        const length = used - lineStart;
        const blank = length === 1 || (length === 2 && bytes[used - 2] === 13);
        lineStart = used;
        if (!blank) continue;
        const text = decoder.decode(bytes.subarray(0, used));
        let event = '', data = [];
        for (const raw of text.split('\n')) {
          const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw;
          if (line.includes('\r')) throw new Error('invalid_framing');
          if (!line || line.startsWith(':')) continue;
          const colon = line.indexOf(':');
          const field = colon < 0 ? line : line.slice(0, colon);
          let value = colon < 0 ? '' : line.slice(colon + 1);
          if (value.startsWith(' ')) value = value.slice(1);
          if (field === 'event') {
            if (event) throw new Error('invalid_event');
            event = value;
          } else if (field === 'data') data.push(value);
          else throw new Error('unsupported_field');
        }
        used = 0; lineStart = 0; lines = 0;
        if (!event && !data.length) continue; // heartbeat
        if (!data.length || !['ready','change','stream_end'].includes(event)) throw new Error('invalid_event');
        if (!ready) {
          if (event !== 'ready') throw new Error('missing_ready');
          const value = JSON.parse(data.join('\n'));
          if (value.snapshot !== false || value.replay !== false) throw new Error('invalid_ready');
          ready = true;
        } else if (event === 'ready') throw new Error('duplicate_ready');
        // Document data remains original text, including exact numeric literals.
        yield {event, data: data.join('\n')};
        if (event === 'stream_end') return;
      }
    }
  } finally {
    signal?.removeEventListener('abort', abort);
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}
