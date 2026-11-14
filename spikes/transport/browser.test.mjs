import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";
import { setTimeout as delay } from "node:timers/promises";
import { chromium } from "playwright";

async function fixture() {
  const binary = fileURLToPath(new URL("../../target/debug/examples/fixture_server", import.meta.url));
  const child = spawn(binary, [], { stdio: ["pipe", "pipe", "pipe"] });
  const queued = [], waiting = [];
  let stderr = "";
  child.stderr.on("data", bytes => { if (stderr.length < 4096) stderr += bytes; });
  const reader = createInterface({ input: child.stdout });
  reader.on("line", line => {
    const value = JSON.parse(line);
    const next = waiting.shift();
    if (next) { clearTimeout(next.timer); next.resolve(value); }
    else { queued.push(value); if (queued.length > 8) child.kill(); }
  });
  const exited = new Promise(resolve => child.once("exit", code => {
    for (const next of waiting.splice(0)) { clearTimeout(next.timer); next.reject(new Error(`fixture_exited_${code}: ${stderr}`)); }
    resolve(code);
  }));
  child.on("error", () => {
    for (const next of waiting.splice(0)) { clearTimeout(next.timer); next.reject(new Error("fixture_spawn_failed")); }
  });
  const next = () => {
    if (queued.length) return Promise.resolve(queued.shift());
    return new Promise((resolve, reject) => {
      const item = { resolve, reject, timer: null };
      item.timer = setTimeout(() => {
        const index = waiting.indexOf(item); if (index >= 0) waiting.splice(index, 1);
        reject(new Error("fixture_response_deadline"));
      }, 5000);
      waiting.push(item);
    });
  };
  const metadata = await next();
  const token = await readFile(metadata.token_file, "utf8");
  const call = async command => { const result = next(); child.stdin.write(JSON.stringify(command) + "\n"); return result; };
  return {
    ...metadata, token, call,
    async stop() {
      if (child.exitCode === null) {
        try { await call({ op: "shutdown" }); } catch { child.kill(); }
        const timeout = setTimeout(() => child.kill(), 3000);
        await exited; clearTimeout(timeout);
      }
      reader.close();
    },
  };
}

async function eventually(predicate) {
  const deadline = Date.now() + 5000;
  while (!(await predicate())) {
    assert.ok(Date.now() < deadline, "condition deadline exceeded");
    await delay(20);
  }
}

async function openStream(page, server) {
  await page.evaluate(async ({ url, token }) => {
    const { subscribe } = await import("/client.mjs");
    window.events = []; window.streamError = null;
    window.controller = new AbortController();
    window.streamDone = (async () => {
      try {
        for await (const record of subscribe(`${url}/v1/collections/orders/changes`, token, { signal: window.controller.signal })) {
          window.events.push(record);
        }
      } catch (error) { window.streamError = error.message; }
    })();
  }, { url: server.url, token: server.token });
  await page.waitForFunction(() => window.events.some(event => event.event === "ready"), { timeout: 5000 });
}

test("Chromium real: Bearer, two streams, post-commit read, cancel, reconnect and revoke", { timeout: 30000 }, async t => {
  const server = await fixture();
  let browser;
  t.after(async () => { await browser?.close(); await server.stop(); });
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext();
  // Only our loopback fixture is test content. No external web resources.
  await context.route("**/*", route => {
    if (new URL(route.request().url()).origin === server.url) return route.continue();
    return route.abort();
  });
  await context.addCookies([{ name: "fixture_cookie", value: "not-a-credential", url: server.url }]);
  const observed = [];
  const observations = [];
  context.on("request", request => {
    if (!request.url().endsWith("/changes")) return;
    observations.push((async () => {
      // headers() omits some security-related headers; allHeaders() is needed
      // to actually verify Cookie is absent on the wire.
      const headers = await request.allHeaders();
      observed.push({
        bearer: headers.authorization === `Bearer ${server.token}`,
        cookie: Boolean(headers.cookie), tokenInUrl: request.url().includes(server.token),
      });
    })());
  });
  const first = await context.newPage(); const second = await context.newPage();
  await first.goto(server.url); await second.goto(server.url);
  const denied = await first.evaluate(async () => {
    return (await fetch("/v1/collections/orders/changes", { headers: { Accept: "text/event-stream" } })).status;
  });
  assert.equal(denied, 401);
  assert.equal((await server.call({ op: "stats" })).subscriptions, 0);
  const resume = await first.evaluate(async token => {
    const response = await fetch("/v1/collections/orders/changes", {
      headers: { Authorization: `Bearer ${token}`, Accept: "text/event-stream", "Last-Event-ID": "1" }, credentials: "omit",
    });
    return { status: response.status, code: (await response.json()).error.code };
  }, server.token);
  assert.deepEqual(resume, { status: 400, code: "resume_not_supported" });
  assert.equal((await server.call({ op: "stats" })).subscriptions, 0);
  await openStream(first, server); await openStream(second, server);
  const committed = await server.call({ op: "insert", id: "browser-1" });
  assert.equal(committed.ok, true);
  for (const page of [first, second]) {
    await page.waitForFunction(() => window.events.some(event => event.event === "change"), { timeout: 5000 });
    const event = await page.evaluate(() => window.events.find(event => event.event === "change"));
    assert.equal(JSON.parse(event.data).sequence, String(committed.sequence));
    assert.equal(JSON.parse(event.data).after.note, "ação");
    assert.ok(event.data.includes("123456789012345678901234567890"));
  }
  const visible = await first.evaluate(async token => {
    const response = await fetch("/v1/collections/orders/documents/browser-1", { headers: { Authorization: `Bearer ${token}` }, credentials: "omit" });
    return { status: response.status, body: await response.text() };
  }, server.token);
  assert.equal(visible.status, 200);
  assert.ok(visible.body.includes("123456789012345678901234567890"));
  await Promise.all(observations);
  assert.ok(observed.some(request => !request.bearer && request.cookie), "negative control must actually send the fixture cookie");
  const authenticated = observed.filter(request => request.bearer);
  assert.ok(authenticated.length >= 3);
  assert.ok(authenticated.every(request => !request.cookie && !request.tokenInUrl));
  await first.evaluate(async () => { window.controller.abort(); await window.streamDone; });
  await eventually(async () => (await server.call({ op: "stats" })).streams === 1);
  await server.call({ op: "insert", id: "browser-2" });
  await second.waitForFunction(() => window.events.filter(event => event.event === "change").length === 2, { timeout: 5000 });
  assert.equal(await first.evaluate(() => window.events.filter(event => event.event === "change").length), 1);
  assert.equal((await server.call({ op: "stats" })).subscriptions, 2);
  await openStream(first, server);
  await delay(150);
  assert.equal(await first.evaluate(() => window.events.filter(event => event.event === "change").length), 0);
  await server.call({ op: "insert", id: "browser-3" });
  await first.waitForFunction(() => window.events.some(event => event.event === "change"), { timeout: 5000 });
  assert.equal((await server.call({ op: "revoke" })).ok, true);
  for (const page of [first, second]) {
    await page.waitForFunction(() => window.events.some(event => event.event === "stream_end"), { timeout: 5000 });
    assert.equal(await page.evaluate(() => JSON.parse(window.events.find(event => event.event === "stream_end").data).code), "interrupted");
    await page.evaluate(() => window.streamDone);
  }
  await eventually(async () => (await server.call({ op: "stats" })).streams === 0);
  const afterRevoke = await first.evaluate(async token => {
    const { subscribe } = await import("/client.mjs");
    try { for await (const _ of subscribe(new URL("/v1/collections/orders/changes", location.href), token)) {} }
    catch (error) { return error.message; }
  }, server.token);
  assert.equal(afterRevoke, "unauthorized");
  assert.equal((await server.call({ op: "stats" })).subscriptions, 3);
  t.diagnostic(`Chromium ${browser.version()}; two authenticated browser streams; Bearer only in header, authenticated SSE omits cookies`);
});
