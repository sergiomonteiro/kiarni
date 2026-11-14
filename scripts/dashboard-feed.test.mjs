import test from 'node:test';
import assert from 'node:assert/strict';
import {subscribe, MAX_FRAME_BYTES} from '../crates/server/dashboard/feed.mjs';
const ready = 'event: ready\ndata: {"snapshot":false,"replay":false}\n\n';
const end = 'event: stream_end\ndata: {"code":"shutdown"}\n\n';
const encoder = new TextEncoder();
function source(parts, options = {}) {
  let cancelled = false, request;
  const stream = new ReadableStream({start(controller) {for (const part of parts) controller.enqueue(typeof part === 'string' ? encoder.encode(part) : part); controller.close();}, cancel() {cancelled = true;}});
  return {get cancelled() {return cancelled;}, get request() {return request;},
    fetchImpl: async (path, init) => { request = {path, ...init}; return new Response(stream, {status:200,headers:{'Content-Type':'text/event-stream; charset=utf-8'}, ...options}); }};
}
async function collect(fixture, options = {}) {
  const result = [];
  for await (const record of subscribe('/v1/collections/orders/changes','private', {fetchImpl:fixture.fetchImpl,...options})) result.push(record);
  return result;
}
test('fragmented UTF-8, multiline data, heartbeat and exact numbers', async () => {
  const raw = '{\n"amount":9007199254740993,"name":"ação","price":1.234567890123456789\n}';
  const wire = encoder.encode(ready + ': heartbeat\n\n' + 'event: change\ndata: ' + raw.replaceAll('\n','\ndata: ') + '\n\n' + end);
  const fixture = source([...wire].map(byte => new Uint8Array([byte])));
  const records = await collect(fixture);
  assert.equal(records.length,3); assert.equal(records[1].data,raw);
  assert.equal(fixture.request.mode,'same-origin'); assert.equal(fixture.request.redirect,'error');
  assert.equal(fixture.request.credentials,'omit'); assert.equal(fixture.request.headers.Accept,'text/event-stream');
  assert.equal(fixture.request.headers.Authorization,'Bearer private');
});
test('CRLF dialect accepted', async () => {
  assert.equal((await collect(source([(ready + end).replaceAll('\n','\r\n')]))).length,2);
});
test('EOF and truncated frames are interruption, never implicit success', async () => {
  for (const suffix of ['', 'event: change\ndata: {']) await assert.rejects(collect(source([ready + suffix])), /stream_interrupted/);
});
test('bounded frame bytes and line count', async () => {
  await assert.rejects(collect(source([new Uint8Array(MAX_FRAME_BYTES + 1).fill(120)])),/frame_too_large/);
  await assert.rejects(collect(source([': x\n'.repeat(8193) + '\n'])),/too_many_lines/);
});
test('invalid UTF-8, resume fields and malformed lifecycle refused', async () => {
  await assert.rejects(collect(source([new Uint8Array([255,10,10])])),/encoded data/);
  for (const wire of ['id: 1\n\n', 'retry: 10\n\n', 'event: change\ndata: {}\n\n', ready + ready,
    'event: ready\ndata: {"snapshot":true,"replay":false}\n\n', 'event: unknown\ndata: {}\n\n']) {
    await assert.rejects(collect(source([wire])));
  }
});
test('unauthorized and bad content type refused before parsing', async () => {
  await assert.rejects(collect(source([], {status:401})),/unauthorized/);
  await assert.rejects(collect(source([], {headers:{'Content-Type':'application/json'}})),/stream_http_200/);
});
test('abort releases active reader', async () => {
  let cancelled = false;
  const active = new AbortController();
  const stream = new ReadableStream({start(controller) {controller.enqueue(encoder.encode(ready));},cancel() {cancelled = true;}});
  const iterator = subscribe('/v1/collections/orders/changes','private',{signal:active.signal, fetchImpl:async () => new Response(stream,{headers:{'Content-Type':'text/event-stream'}})});
  assert.equal((await iterator.next()).value.event,'ready');
  active.abort(); await assert.rejects(iterator.next(),/aborted/); assert.ok(cancelled);
});
test('only fixed relative collection path accepted', async () => {
  for (const path of ['https://other.invalid/changes','/v1/collections/orders/changes?token=x']) {
    const iterator = subscribe(path,'private',{fetchImpl:() => {throw new Error('unexpected_fetch');}});
    await assert.rejects(iterator.next(),/invalid_path/);
  }
});
