import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { subscribe } from "./client.mjs";

const fixture = name => readFileSync(new URL(`fixtures/${name}.sse`, import.meta.url), "utf8");
const endpoint = "http://127.0.0.1:9000/v1/collections/orders/changes";
const terminal = fixture("overflow");
const encode = text => new TextEncoder().encode(text);

function fakeFetch(text, { fragment = false, status = 200, type = "text/event-stream" } = {}) {
  return async () => {
    const bytes = encode(text);
    let cursor = 0;
    return new Response(new ReadableStream({
      pull(controller) {
        if (cursor === bytes.length) { controller.close(); return; }
        const end = fragment ? cursor + 1 : bytes.length;
        controller.enqueue(bytes.slice(cursor, end));
        cursor = end;
      },
    }), { status, headers: { "Content-Type": type } });
  };
}

async function collect(text, options = {}) {
  const output = [];
  for await (const record of subscribe(endpoint, "fixture-only-token", {
    fetchImpl: fakeFetch(text, options), ...options,
  })) output.push(record);
  return output;
}

test("shared Rust fixtures survive fragmentation and keep raw numeric tokens", async () => {
  const records = await collect(fixture("ready") + fixture("change") + terminal, { fragment: true });
  assert.deepEqual(records.map(record => record.event), ["ready", "change", "stream_end"]);
  assert.equal(JSON.parse(records[1].data).sequence, "9007199254740993");
  assert.match(records[1].data, /123456789012345678901234567890/);
  assert.deepEqual(JSON.parse(records[0].data), { snapshot: false, replay: false });
});

test("UTF-8, CRLF, comments and multiline data work across byte boundaries", async () => {
  const wire = ': heartbeat\r\n\r\nevent: change\r\ndata: {"note":"ação",\r\ndata: "x":null}\r\n\r\n' + terminal;
  const records = await collect(wire, { fragment: true });
  assert.deepEqual(JSON.parse(records[0].data), { note: "ação", x: null });
});

test("Bearer stays in the header; redirects, cookies and CORS are disabled", async () => {
  let calls = 0;
  const fetchImpl = async (url, options) => {
    calls++;
    assert.equal(url, endpoint);
    assert.equal(options.headers.Authorization, "Bearer fixture-only-token");
    assert.equal(options.credentials, "omit");
    assert.equal(options.mode, "same-origin");
    assert.equal(options.redirect, "error");
    assert.equal(options.cache, "no-store");
    return fakeFetch(terminal)();
  };
  await collect("", { fetchImpl });
  assert.equal(calls, 1);
});

test("unexpected EOF is an interruption and never reconnects automatically", async () => {
  let calls = 0;
  await assert.rejects(collect("", { fetchImpl: async () => { calls++; return fakeFetch(fixture("ready"))(); } }), /stream_interrupted/);
  assert.equal(calls, 1);
});

test("terminal event ends processing and cancels the stream", async () => {
  let cancelled = false;
  const fetchImpl = async () => new Response(new ReadableStream({
    start(controller) { controller.enqueue(encode(terminal + fixture("change"))); },
    cancel() { cancelled = true; },
  }), { headers: { "Content-Type": "text/event-stream" } });
  const records = await collect("", { fetchImpl });
  assert.equal(records.length, 1);
  assert.equal(records[0].event, "stream_end");
  assert.equal(cancelled, true);
});

test("resume directives and incomplete frames are rejected", async () => {
  for (const field of ["id: 12", "retry: 1000"]) {
    await assert.rejects(collect(`${field}\n${terminal}`), /resume_not_supported/);
  }
  await assert.rejects(collect('event: change\ndata: {"x":1}'), /stream_interrupted/);
});

test("byte limits cover one oversized chunk and many fragmented lines", async () => {
  await assert.rejects(collect("data: " + "x".repeat(100), { maxFrameBytes: 32 }), /frame_too_large/);
  await assert.rejects(collect(": comment\n".repeat(10), { maxFrameBytes: 32, fragment: true }), /frame_too_large/);
  const records = await collect(terminal, { maxFrameBytes: encode(terminal).length });
  assert.equal(records.length, 1);
});

test("authentication errors and wrong content types fail before consuming data", async () => {
  await assert.rejects(collect(terminal, { status: 401 }), /unauthorized/);
  await assert.rejects(collect(terminal, { type: "text/html" }), /invalid_stream_response/);
});

test("invalid UTF-8 is rejected", async () => {
  const fetchImpl = async () => new Response(new Uint8Array([100, 97, 116, 97, 58, 32, 255, 10, 10]), {
    headers: { "Content-Type": "text/event-stream" },
  });
  await assert.rejects(collect("", { fetchImpl }), /invalid_utf8/);
});

test("abort before connection sends no request", async () => {
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(collect("", { signal: controller.signal, fetchImpl: () => assert.fail("unexpected request") }), /aborted/);
});

test("abort while awaiting bytes cancels the reader", async () => {
  const controller = new AbortController();
  let cancelled = false;
  let open;
  const started = new Promise(resolve => { open = resolve; });
  const fetchImpl = async () => new Response(new ReadableStream({
    pull() { open(); }, cancel() { cancelled = true; },
  }), { headers: { "Content-Type": "text/event-stream" } });
  const pending = collect("", { signal: controller.signal, fetchImpl });
  await started;
  controller.abort();
  await assert.rejects(pending, /aborted/);
  assert.equal(cancelled, true);
});

test("URLs carrying credentials or remote plaintext are rejected", async () => {
  for (const url of [endpoint + "?token=fixture", endpoint + "#fixture", "http://example.com/changes", "https://name:password@example.com/changes"]) {
    await assert.rejects(async () => {
      for await (const _ of subscribe(url, "fixture", { fetchImpl: () => assert.fail("unexpected request") })) {}
    }, /invalid_stream_url/);
  }
});
