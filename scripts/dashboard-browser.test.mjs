// Product dashboard integration. Reuses the pinned, development-only Playwright.
import test from 'node:test';
import assert from 'node:assert/strict';
import { chromium } from '../spikes/transport/node_modules/playwright/index.mjs';
import { spawn } from 'node:child_process';
import { mkdtemp, readdir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';

test('embedded dashboard authenticates, paginates, clears session and handles failure', {timeout: 60000}, async () => {
  const temp = await mkdtemp(join(tmpdir(), 'kiarni-dashboard-'));
  const reservation = createServer();
  await new Promise(resolve => reservation.listen(0, '127.0.0.1', resolve));
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  const origin = `http://127.0.0.1:${port}`;
  const server = spawn(resolve('target/debug/kiarni'), ['serve', '--data-dir', join(temp, 'data'), '--bind', `127.0.0.1:${port}`], {stdio: 'ignore'});
  let browser;
  try {
    let ready = false;
    for (let i = 0; i < 150; i++) {
      try { ready = (await fetch(origin)).ok; } catch {}
      if (ready) break;
      await delay(20);
    }
    assert.ok(ready, 'product server starts');
    const files = await readdir(join(temp, 'data.credentials'));
    const token = await readFile(join(temp, 'data.credentials', files[0]), 'utf8');
    for (let i = 0; i < 101; i++) {
      const response = await fetch(origin + '/v1/collections', {method:'POST', headers:{Authorization:`Bearer ${token}`, 'Content-Type':'application/json'}, body: JSON.stringify({name:`collection_${String(i).padStart(3, '0')}`})});
      assert.equal(response.status, 201);
    }
    for (let i = 0; i < 30; i++) {
      const body = `{"_id":"doc_${String(i).padStart(2, '0')}","amount":${i === 0 ? '9007199254740993' : i},"price":1.234567890123456789,"title":"<img src=x onerror=alert(1)>"}`;
      const response = await fetch(origin + '/v1/collections/collection_000/documents', {
        method:'POST', headers:{Authorization:`Bearer ${token}`, 'Content-Type':'application/json'}, body
      });
      assert.equal(response.status, 201);
    }
    browser = await chromium.launch({headless:true});
    const page = await browser.newPage();
    const errors = [];
    let queriesSent = 0;
    page.on('request', request => { if (request.url().endsWith('/query')) queriesSent++; });
    page.on('pageerror', () => errors.push('page error'));
    await page.goto(origin);
    assert.equal(await page.locator('#session').isVisible(), false);
    assert.equal(await page.locator('#metrics-table').isVisible(),false);
    assert.equal(await page.locator('#http-table').isVisible(),false);
    await page.locator('#token').fill('invalid');
    await page.locator('#connect button').click();
    await page.getByRole('status').filter({hasText:'Credencial inválida'}).waitFor();
    await page.locator('#token').fill(token);
    await page.locator('#connect button').click();
    await page.locator('#session').waitFor();
    assert.equal(await page.locator('#live').textContent(), 'Em execução');
    assert.equal(await page.locator('#ready').textContent(), 'Pronto');
    assert.equal(await page.locator('#collections li').count(), 100);
    assert.ok(await page.locator('#token').inputValue() === '', 'password input cleared');
    const storageEmpty = await page.evaluate(() => localStorage.length === 0 && sessionStorage.length === 0 && document.cookie === '');
    assert.ok(storageEmpty, 'no stored credential');
    assert.ok(!page.url().includes(token), 'no credential in URL');
    const errorPath = '**/v1/errors/recent';
    const collectErrors = async () => {
      await page.locator('#refresh-errors').click();
      await page.locator('#errors-status').filter({hasText:'Observações descartadas'}).waitFor();
    };
    let lastError;
    for (let i = 0; i < 10; i++) {
      const response = await fetch(origin + '/v1/unknown', {headers:{Authorization:`Bearer ${token}`}});
      assert.equal(response.status,404); lastError = await response.json();
    }
    await collectErrors();
    assert.equal(await page.locator('#errors-values tr').count(),8);
    assert.ok((await page.locator('#errors-values tr').first().textContent()).includes(lastError.request_id));
    assert.ok(!(await page.locator('#errors-values').textContent()).includes(token));
    await page.setViewportSize({width:375,height:812});
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),'error table fits mobile viewport');
    await page.setViewportSize({width:1280,height:900});
    await page.route(errorPath,route => route.fulfill({status:200,contentType:'application/json',body:'{"capacity":8,"dropped_total":"0","errors":[]}'}));
    await collectErrors();
    assert.equal(await page.locator('#errors-table').isVisible(),false);
    assert.match(await page.locator('#errors-status').textContent(),/Nenhum erro retido/);
    await page.unroute(errorPath);
    for (const invalid of ['{}', JSON.stringify({capacity:8,dropped_total:'0',errors:Array(9).fill({})})]) {
      await page.route(errorPath,route => route.fulfill({status:200,contentType:'application/json',body:invalid}));
      await page.locator('#refresh-errors').click();
      await page.locator('#errors-status').filter({hasText:'Não foi possível'}).waitFor();
      assert.equal(await page.locator('#errors-values tr').count(),0);
      assert.equal(await page.locator('#session').isVisible(),true);
      await page.unroute(errorPath);
    }
    await page.route(errorPath,route => route.fulfill({status:503,contentType:'application/json',body:'{"error":{"code":"service_unavailable"},"request_id":"req_test"}'}));
    await page.locator('#refresh-errors').click();
    await page.locator('#errors-status').filter({hasText:'service_unavailable'}).waitFor();
    assert.equal(await page.locator('#errors-values tr').count(),0);
    await page.unroute(errorPath);
    let releaseErrors, errorsReached;
    const delayedErrors = new Promise(resolve => {releaseErrors=resolve;});
    const errorsPending = new Promise(resolve => {errorsReached=resolve;});
    await page.route(errorPath,async route => { errorsReached(); await delayedErrors; await route.fulfill({status:200,contentType:'application/json',body:'{"capacity":8,"dropped_total":"0","errors":[]}'}).catch(()=>{}); });
    await page.locator('#refresh-errors').click(); await errorsPending;
    await page.locator('#disconnect').click(); releaseErrors(); await page.unroute(errorPath);
    assert.equal(await page.locator('#errors-values tr').count(),0);
    await page.locator('#token').fill(token); await page.locator('#connect button').click(); await page.locator('#session').waitFor();
    await collectErrors();
    await page.route(errorPath,route => route.fulfill({status:401,contentType:'application/json',body:'{}'}));
    await page.locator('#refresh-errors').click(); await page.locator('#notice').filter({hasText:'Credencial inválida'}).waitFor();
    assert.equal(await page.locator('#session').isVisible(),false);
    assert.equal(await page.locator('#errors-values tr').count(),0);
    await page.unroute(errorPath);
    await page.locator('#token').fill(token); await page.locator('#connect button').click(); await page.locator('#session').waitFor();
    const collectStatus = async () => {
      await page.locator('#refresh-status').click();
      await page.locator('#server-status').filter({hasText:'Coleta concluída'}).waitFor();
    };
    await collectStatus();
    assert.equal(await page.locator('#version').textContent(),'0.1.0');
    assert.match(await page.locator('#uptime').textContent(), /^\d+ s$/);
    assert.match(await page.locator('#connections').textContent(), /^1 \/ \d+$/);
    assert.match(await page.locator('#stream-connections').textContent(), /^0 \/ \d+$/);
    const collectMetrics = async () => {
      await page.locator('#refresh-metrics').click();
      await page.locator('#metrics-status').filter({hasText:'Coleta concluída'}).waitFor();
    };
    const metricValue = name => page.locator('#metrics-values tr').evaluateAll((rows,name) => rows.find(row => row.dataset.metric === name)?.querySelector('td').textContent, name);
    const httpMetricValue = name => page.locator('#http-values tr, #response-values tr, #stream-values tr').evaluateAll((rows,name) => rows.find(row => row.dataset.metric === name)?.querySelector('td').textContent,name);
    const queryMetricValue = name => page.locator('#query-values tr, #query-total-values tr, #query-wait-values tr, #query-execution-values tr').evaluateAll((rows,name) => rows.find(row => row.dataset.metric === name)?.querySelector('td').textContent,name);
    const storageMetricValue = name => page.locator('#storage-values tr, #storage-commit-values tr, #storage-operation-values tr').evaluateAll((rows,name) => rows.find(row => row.dataset.metric === name)?.querySelector('td').textContent,name);
    let metricsRequests = 0;
    page.on('request', request => {if (request.url().endsWith('/v1/metrics')) metricsRequests++;});

    const tableIds = ['metrics','http','response','stream','query','query-total','query-wait','query-execution','storage','storage-commit','storage-operation'];


    await collectMetrics();
    assert.equal(await page.locator('#metrics-values tr').count(),20);
    assert.equal(await page.locator('#http-values tr').count(),8);
    assert.equal(await page.locator('#response-values tr').count(),9);
    assert.equal(await page.locator('#stream-values tr').count(),9);
    assert.equal(await page.locator('#query-values tr').count(),15);
    assert.equal(await queryMetricValue('execution_total{access="secondary"}'),'0');
    assert.equal(await page.locator('#storage-values tr').count(),9);
    assert.equal(await page.locator('#storage-commit-values tr').count(),9);
    assert.equal(await page.locator('#storage-operation-values tr').count(),11);
    assert.equal(await page.locator('#storage-operation option').count(),12);
    assert.equal(await storageMetricValue('file_bytes_available'),'1');
    assert.ok(BigInt(await storageMetricValue('file_logical_bytes'))>0n);
    assert.ok(BigInt(await storageMetricValue('commits_total{outcome="confirmed"}'))>=131n);

    for (const stage of ['total','wait','execution']) assert.equal(await page.locator(`#query-${stage}-values tr`).count(),9);
    assert.equal(await queryMetricValue('completed_total{outcome="success"}'),'0');

    assert.ok(BigInt(await httpMetricValue('requests_total{status_class="4xx"}')) >= 1n);
    assert.equal(await httpMetricValue('request_duration_seconds_count{kind="stream"}'),'0');
    const beforeResponses = BigInt(await httpMetricValue('request_duration_seconds_count{kind="response"}'));
    await collectMetrics();
    assert.ok(BigInt(await httpMetricValue('request_duration_seconds_count{kind="response"}')) > beforeResponses);

    assert.equal(await metricValue('subscriptions_active'),'0');
    const initiallyPublished = BigInt(await metricValue('events_published_total'));
    assert.equal(initiallyPublished,30n);
    const result = page.locator('#query-result');
    const run = async ast => {
      await page.locator('#query').fill(typeof ast === 'string' ? ast : JSON.stringify(ast));
      await page.locator('#run-query').click();
      await page.locator('#query-status').filter({hasText:'Consulta concluída'}).waitFor();
      return result.textContent();
    };
    const collectCount = async () => {
      await page.locator('#refresh-count').click();
      await page.locator('#count-status').filter({hasText:'Contagem exata'}).waitFor();
    };
    await page.getByRole('button', {name:'collection_000', exact:true}).click();
    await result.waitFor();
    assert.equal(JSON.parse(await result.textContent()).count, 25);
    await collectCount();
    assert.equal(await page.locator('#document-count').textContent(),'Total exato: 30');
    await collectMetrics();
    assert.equal(await queryMetricValue('completed_total{outcome="success"}'),'1');
    assert.equal(await queryMetricValue('documents_returned_total'),'25');
    assert.equal(await queryMetricValue('execution_total{access="scan"}'),'1');
    assert.equal(await storageMetricValue('operations_total{operation="query_read",outcome="success"}'),'1');
    const scrapesBeforeSelection = metricsRequests;
    await page.locator('#storage-operation').selectOption('insert');
    assert.equal(await storageMetricValue('operations_total{operation="insert",outcome="success"}'),'30');
    assert.equal(await page.locator('#storage-operation-values tr').count(),11);
    assert.equal(metricsRequests,scrapesBeforeSelection,'operation selection reuses the current observation');
    assert.ok((await page.locator('#storage-operation-caption').textContent()).startsWith('Inserção'));
    await page.locator('#storage-operation').selectOption('query_read');

    for (const stage of ['total','wait','execution']) assert.equal(await queryMetricValue(`duration_seconds_count{stage="${stage}"}`),'1');
    for (const summary of await page.locator('details.query-timing summary').all()) await summary.click();
    for (const stage of ['total','wait','execution']) assert.equal(await page.locator(`#query-${stage}-table`).isVisible(),true);


    assert.ok((await result.textContent()).includes('9007199254740993'));
    assert.ok((await result.textContent()).includes('1.234567890123456789'));
    assert.equal(await result.locator('img').count(), 0, 'document text is inert');
    const feed = page.locator('#feed-result');
    const changesPath = '**/v1/collections/collection_000/changes';
    const metrics = async () => {
      const response = await fetch(origin + '/v1/metrics', {headers:{Authorization:`Bearer ${token}`}});
      return response.text();
    };
    const noActiveStreams = async () => {
      for (let attempt = 0; attempt < 100; attempt++) {
        if (/^kiarni_changefeed_subscriptions_active 0$/m.test(await metrics())) return;
        await delay(20);
      }
      assert.fail('stream capacity was not released');
    };
    const startFeed = async () => {
      await page.locator('#start-feed').click();
      await page.locator('#feed-status').filter({hasText:'Conectado.'}).waitFor();
    };
    const write = async (method, path, body, status) => {
      const response = await fetch(origin + path, {method, headers:{Authorization:`Bearer ${token}`,...(body ? {'Content-Type':'application/json'} : {})},...(body ? {body} : {})});
      assert.equal(response.status,status);
    };
    await startFeed();
    await collectStatus();
    assert.match(await page.locator('#connections').textContent(), /^2 \/ \d+$/);
    assert.match(await page.locator('#stream-connections').textContent(), /^1 \/ \d+$/);
    assert.equal(await page.locator('#stop-feed').isEnabled(),true);
    await collectMetrics();
    assert.equal(await metricValue('subscriptions_active'),'1');
    assert.equal(await httpMetricValue('request_duration_seconds_count{kind="stream"}'),'0', 'active stream has no lifetime sample');

    assert.equal(await feed.isVisible(), false, 'no initial snapshot');
    const originalQuery = await result.textContent();
    const inserted = '{"_id":"live_doc","amount":9007199254740993,"name":"ação <img src=x onerror=alert(1)>"}';
    await write('POST','/v1/collections/collection_000/documents',inserted,201);
    await page.locator('#feed-count').filter({hasText:'1 eventos'}).waitFor();
    assert.equal(await page.locator('#document-count').textContent(),'Total exato: 30', 'count is not automatically reconciled');
    await collectCount();
    assert.equal(await page.locator('#document-count').textContent(),'Total exato: 31');
    assert.equal(await page.locator('#stop-feed').isEnabled(),true);

    assert.ok((await feed.textContent()).includes('9007199254740993'));
    assert.ok((await feed.textContent()).includes('ação'));
    assert.equal(JSON.parse(await feed.textContent()).type,'insert');
    const insertedSequence = JSON.parse(await feed.textContent()).sequence;
    assert.equal(typeof insertedSequence,'string');
    assert.equal(await feed.locator('img').count(),0);
    await write('PUT','/v1/collections/collection_000/documents/live_doc',inserted,200); // no-op
    await write('POST','/v1/collections/collection_001/documents','{"_id":"other"}',201);
    await write('PUT','/v1/collections/collection_000/documents/live_doc','{"_id":"live_doc","amount":9007199254740995}',200);
    await page.locator('#feed-count').filter({hasText:'2 eventos'}).waitFor();
    const update = await feed.textContent();
    assert.ok(update.includes('9007199254740993') && update.includes('9007199254740995'));
    assert.equal(JSON.parse(update).type,'update');
    assert.ok(BigInt(JSON.parse(update).sequence) > BigInt(insertedSequence));
    await write('DELETE','/v1/collections/collection_000/documents/live_doc',null,204);
    await page.locator('#feed-count').filter({hasText:'3 eventos'}).waitFor();
    await collectCount();
    assert.equal(await page.locator('#document-count').textContent(),'Total exato: 30');

    assert.equal(JSON.parse(await feed.textContent()).type,'delete');
    assert.equal(await result.textContent(), originalQuery, 'snapshot not silently merged');
    assert.equal(await page.locator('#feed-result').count(),1, 'single retained event');
    await collectMetrics();
    assert.equal(BigInt(await metricValue('events_published_total')), initiallyPublished + 4n);
    assert.equal(await metricValue('events_enqueued_total'),'3');
    assert.equal(await metricValue('commit_to_dispatch_seconds_count'),'3');
    await page.locator('#stop-feed').click();
    await noActiveStreams();
    await collectMetrics();
    assert.equal(await metricValue('subscriptions_active'),'0');
    assert.equal(await httpMetricValue('request_duration_seconds_count{kind="stream"}'),'1');
    assert.equal(await httpMetricValue('transport_total{outcome="stream_closed"}'),'1');
    assert.equal(await metricValue('subscriptions_ended_total{reason="cancelled"}'),'1');
    assert.equal(await feed.textContent(),'');
    await startFeed();
    assert.equal(await feed.isVisible(),false, 'manual reconnect has no replay');
    await page.getByRole('button',{name:'collection_001',exact:true}).click();
    await noActiveStreams();
    await result.waitFor();
    await write('DELETE','/v1/collections/collection_001/documents/other',null,204);
    await page.getByRole('button',{name:'collection_000',exact:true}).click();
    await result.waitFor();
    // Deterministic browser handling of terminal overflow and interrupted EOF.
    for (const [wire, message] of [
      ['event: ready\ndata: {"snapshot":false,"replay":false}\n\nevent: stream_end\ndata: {"code":"overflow"}\n\n','consumidor atrasado'],
      ['event: ready\ndata: {"snapshot":false,"replay":false}\n\n','Stream interrompido']
    ]) {
      await page.route(changesPath, route => route.fulfill({status:200,contentType:'text/event-stream',body:wire}));
      await page.locator('#start-feed').click();
      await page.locator('#feed-status').filter({hasText:message}).waitFor();
      assert.equal(await page.locator('#start-feed').isEnabled(),true);
      await page.unroute(changesPath);
    }
    await startFeed();
    await page.reload();
    await noActiveStreams();
    assert.equal(await page.locator('#session').isVisible(),false);
    await page.locator('#token').fill(token);
    await page.locator('#connect button').click();
    await page.locator('#session').waitFor();
    await page.getByRole('button',{name:'collection_000',exact:true}).click();
    await result.waitFor();
    await page.route(changesPath, route => route.fulfill({status:401,contentType:'application/json',body:'{}'}));
    await page.locator('#start-feed').click();
    await page.locator('#notice').filter({hasText:'Credencial inválida'}).waitFor();
    assert.equal(await page.locator('#session').isVisible(),false);
    assert.equal(await feed.textContent(),'');
    await noActiveStreams();
    await page.unroute(changesPath);
    await page.locator('#token').fill(token);
    await page.locator('#connect button').click();
    await page.locator('#session').waitFor();
    await page.getByRole('button',{name:'collection_000',exact:true}).click();
    await result.waitFor();
    let raw = await run('{"from":"collection_000","limit":5,"filter":{"op":"eq","field":"amount","value":9007199254740993},"project":["amount","price"]}');
    assert.equal(JSON.parse(raw).count, 1);
    assert.ok(raw.includes('9007199254740993'));
    assert.ok(!raw.includes('title'));
    raw = await run({from:'collection_000',limit:2,sort:[{field:'amount',direction:'desc'}],project:['amount']});
    assert.equal(JSON.parse(raw).documents[0]._id, 'doc_00');
    assert.equal(JSON.parse(raw).documents[1]._id, 'doc_29');
    raw = await run({from:'collection_000',limit:1,id:'doc_03'});
    assert.equal(JSON.parse(raw).documents[0]._id, 'doc_03');
    raw = await run({from:'collection_000',limit:1,id:'absent'});
    assert.equal(JSON.parse(raw).count, 0);
    const beforeInvalid = queriesSent;
    for (const [body, message] of [
      ['{', 'JSON de consulta inválido'],
      ['{"from":"collection_000"}', 'Informe limit'],
      ['{"from":"collection_000","limit":0}', 'Informe limit'],
      ['{"from":"collection_000","limit":1001}', 'Informe limit'],
      ['{"from":"collection_001","limit":1}', 'campo from'],
      ['é'.repeat(9000), 'excede 16 KiB']
    ]) {
      await page.locator('#query').fill(body);
      await page.locator('#run-query').click();
      await page.locator('#query-status').filter({hasText:message}).waitFor();
      assert.equal(await result.isVisible(), false);
    }
    assert.equal(queriesSent, beforeInvalid, 'invalid local input sends no query');
    await page.locator('#query').fill('{"from":"collection_000","limit":1,"unknown":true}');
    await page.locator('#run-query').click();
    await page.locator('#query-status').filter({hasText:'invalid_query'}).waitFor();
    await collectMetrics();
    assert.equal(await queryMetricValue('completed_total{outcome="rejected"}'),'1');
    assert.ok(BigInt(await queryMetricValue('execution_total{access="id"}')) >= 2n);
    assert.equal(BigInt(await queryMetricValue('duration_seconds_count{stage="total"}')),BigInt(await queryMetricValue('duration_seconds_count{stage="execution"}')) + 1n);

    assert.equal(await result.isVisible(), false);
    await page.getByRole('button', {name:'collection_001',exact:true}).click();
    await result.waitFor();
    assert.equal(JSON.parse(await result.textContent()).count, 0);
    await collectCount();
    assert.equal(await page.locator('#document-count').textContent(),'Total exato: 0');

    await page.setViewportSize({width:375,height:812});
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'documents fit mobile');
    await page.setViewportSize({width:1280,height:800});
    let release;
    const delayed = new Promise(resolve => { release = resolve; });
    let reached;
    const pending = new Promise(resolve => { reached = resolve; });
    await page.route('**/v1/collections/collection_000/query', async route => {
      reached(); await delayed;
      await route.fulfill({status:200,contentType:'application/json',body:'{"documents":[{"_id":"stale"}],"count":1}'}).catch(() => {});
    });
    await page.getByRole('button', {name:'collection_000',exact:true}).click();
    await pending;
    await page.locator('#close-documents').click();
    release();
    await page.unroute('**/v1/collections/collection_000/query');
    assert.equal(await page.locator('#documents').isVisible(), false);
    assert.equal(await result.textContent(), '');
    await page.locator('#next').click();
    await page.getByText('1 nesta página', {exact:true}).waitFor();
    assert.equal(await page.locator('#collections li').textContent(), 'collection_100');
    await page.locator('#refresh').click();
    await page.getByText('100 nesta página', {exact:true}).waitFor();
    await page.reload();
    assert.equal(await page.locator('#session').isVisible(), false);
    await page.locator('#token').fill(token);
    await page.locator('#connect button').click();
    await page.locator('#session').waitFor();
    await page.getByRole('button',{name:'collection_000',exact:true}).click();
    await result.waitFor();
    await startFeed();
    await page.locator('#disconnect').click();
    await noActiveStreams();
    assert.equal(await page.locator('#collections li').count(), 0);
    await page.setViewportSize({width:375,height:812});
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'mobile fits viewport');

    await page.locator('#token').fill(token);
    await page.locator('#connect button').click();
    await page.locator('#session').waitFor();
    await page.route('**/v1/collections/collection_000/query', route => route.fulfill({status:401,contentType:'application/json',body:'{}'}));
    await page.getByRole('button', {name:'collection_000',exact:true}).click();
    await page.getByRole('status').filter({hasText:'Credencial inválida'}).waitFor();
    assert.equal(await page.locator('#session').isVisible(), false);
    assert.equal(await result.textContent(), '');
    await page.unroute('**/v1/collections/collection_000/query');
    await page.locator('#token').fill(token);
    await page.locator('#connect button').click();
    await page.locator('#session').waitFor();
    await page.getByRole('button',{name:'collection_000',exact:true}).click();
    await result.waitFor();
    const countPath = '**/v1/collections/collection_000/count';
    await page.route(countPath,route => route.fulfill({status:200,contentType:'application/json',body:'{"count":"10000","exact":false,"limits":{"documents":10000,"document_bytes":8388608}}'}));
    await page.locator('#refresh-count').click();
    await page.locator('#count-status').filter({hasText:'limite inferior'}).waitFor();
    assert.equal(await page.locator('#document-count').textContent(),'Pelo menos 10000 documentos');
    await page.unroute(countPath);
    await page.route(countPath,route => route.fulfill({status:200,contentType:'application/json',body:'{"count":"10000","exact":"false"}'}));
    await page.locator('#refresh-count').click();
    await page.locator('#count-status').filter({hasText:'contagem válida'}).waitFor();
    assert.equal(await page.locator('#document-count').textContent(),'—');
    await page.unroute(countPath);
    await page.route(countPath,route => route.fulfill({status:404,contentType:'application/json',body:'{"error":{"code":"collection_not_found"},"request_id":"req_count"}'}));
    await page.locator('#refresh-count').click();
    await page.locator('#count-status').filter({hasText:'collection_not_found (404) · req_count'}).waitFor();
    await page.unroute(countPath);
    let releaseCount, countReached;
    const delayedCount = new Promise(resolve => {releaseCount = resolve;});
    const countPending = new Promise(resolve => {countReached = resolve;});
    await page.route(countPath,async route => {countReached();await delayedCount;await route.fulfill({status:200,contentType:'application/json',body:'{"count":"999","exact":true,"limits":{"documents":10000,"document_bytes":8388608}}'}).catch(() => {});});
    await page.locator('#refresh-count').click();
    await countPending;
    await page.getByRole('button',{name:'collection_001',exact:true}).click();
    releaseCount();
    await page.unroute(countPath);
    assert.equal(await page.locator('#document-count').textContent(),'—');
    await page.getByRole('button',{name:'collection_000',exact:true}).click();
    await result.waitFor();
    await page.route(countPath,route => route.fulfill({status:401,contentType:'application/json',body:'{}'}));
    await page.locator('#refresh-count').click();
    await page.locator('#notice').filter({hasText:'Credencial inválida'}).waitFor();
    assert.equal(await page.locator('#session').isVisible(),false);
    assert.equal(await page.locator('#document-count').textContent(),'—');
    await page.unroute(countPath);
    await page.locator('#token').fill(token);
    await page.locator('#connect button').click();
    await page.locator('#session').waitFor();
    await page.route('**/v1/status',route => route.fulfill({status:200,contentType:'application/json',body:'{"version":"<img src=x>","uptime_seconds":"9007199254740993","connections":{"admitted":1,"streaming":0,"limit":8,"stream_limit":4}}'}));
    await collectStatus();
    assert.equal(await page.locator('#uptime').textContent(),'9007199254740993 s');
    assert.equal(await page.locator('#version img').count(),0);
    await page.unroute('**/v1/status');
    await page.route('**/v1/status',route => route.fulfill({status:200,contentType:'application/json',body:'{}'}));
    await page.locator('#refresh-status').click();
    await page.locator('#server-status').filter({hasText:'status válido'}).waitFor();
    assert.equal(await page.locator('#version').textContent(),'—');
    assert.equal(await page.locator('#connections').textContent(),'—');
    await page.unroute('**/v1/status');
    await page.route('**/v1/status',route => route.fulfill({status:503,contentType:'application/json',body:'{"error":{"code":"service_unavailable"},"request_id":"req_status"}'}));
    await page.locator('#refresh-status').click();
    await page.locator('#server-status').filter({hasText:'service_unavailable (503) · req_status'}).waitFor();
    await page.unroute('**/v1/status');
    let releaseStatus, statusReached;
    const delayedStatus = new Promise(resolve => {releaseStatus = resolve;});
    const statusPending = new Promise(resolve => {statusReached = resolve;});
    await page.route('**/v1/status',async route => {statusReached();await delayedStatus;await route.fulfill({status:200,contentType:'application/json',body:'{}'}).catch(() => {});});
    await page.locator('#refresh-status').click();
    await statusPending;
    await page.locator('#disconnect').click();
    releaseStatus();
    await page.unroute('**/v1/status');
    assert.equal(await page.locator('#version').textContent(),'—');
    await page.locator('#token').fill(token);
    await page.locator('#connect button').click();
    await page.locator('#session').waitFor();
    await page.route('**/v1/status',route => route.fulfill({status:401,contentType:'application/json',body:'{}'}));
    await page.locator('#refresh-status').click();
    await page.locator('#notice').filter({hasText:'Credencial inválida'}).waitFor();
    assert.equal(await page.locator('#session').isVisible(),false);
    assert.equal(await page.locator('#version').textContent(),'—');
    await page.unroute('**/v1/status');
    await page.locator('#token').fill(token);
    await page.locator('#connect button').click();
    await page.locator('#session').waitFor();
    const realExposition = await (await fetch(origin + '/v1/metrics',{headers:{Authorization:`Bearer ${token}`}})).text();
    await page.route('**/v1/metrics',route => route.fulfill({status:200,contentType:'text/plain',body:realExposition.replace(/kiarni_storage_file_logical_bytes \d+\n/,'kiarni_storage_file_logical_bytes 18446744073709551615\n')}));
    await collectMetrics();
    assert.equal(await storageMetricValue('file_logical_bytes'),'18446744073709551615');
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),'storage tables fit mobile viewport');
    await page.unroute('**/v1/metrics');
    const unavailableFile = realExposition.replace(/kiarni_storage_file_logical_bytes \d+\n/,'kiarni_storage_file_logical_bytes NaN\n').replace('kiarni_storage_file_bytes_available 1','kiarni_storage_file_bytes_available 0');
    await page.route('**/v1/metrics',route => route.fulfill({status:200,contentType:'text/plain',body:unavailableFile}));
    await collectMetrics();
    assert.equal(await storageMetricValue('file_logical_bytes'),'Indisponível');
    assert.equal(await storageMetricValue('file_bytes_available'),'0');
    assert.equal(await page.locator('#http-table').isVisible(),true);
    await page.unroute('**/v1/metrics');
    const invalidStorage = [
      realExposition.replace(/^kiarni_storage_file_bytes_available .*\n/m,''),
      realExposition.replace(/kiarni_storage_file_logical_bytes \d+\n/,'kiarni_storage_file_logical_bytes NaN\n'),
      realExposition.replace(/kiarni_storage_commit_duration_seconds_count \d+/,'kiarni_storage_commit_duration_seconds_count 18446744073709551615'),
      realExposition.replace(/kiarni_storage_operation_duration_seconds_count\{operation="query_read"\} \d+/,'kiarni_storage_operation_duration_seconds_count{operation="query_read"} 18446744073709551615')
    ];
    for (const invalid of invalidStorage) {
      await page.route('**/v1/metrics',route => route.fulfill({status:200,contentType:'text/plain',body:invalid}));
      await page.locator('#refresh-metrics').click();
      await page.locator('#metrics-status').filter({hasText:'métricas válidas'}).waitFor();
      assert.equal(await page.locator('#storage-operation').isDisabled(),true);
      for (const id of tableIds) {
        assert.equal(await page.locator(`#${id}-table`).isVisible(),false);
        assert.equal(await page.locator(`#${id}-values tr`).count(),0);
      }
      await page.unroute('**/v1/metrics');
    }

    await page.route('**/v1/metrics',route => route.fulfill({status:200,contentType:'text/plain',body:realExposition.replace(/kiarni_query_completed_total\{outcome="success"\} \d+\n/,'kiarni_query_completed_total{outcome="success"} 18446744073709551615\n')}));
    await collectMetrics();
    assert.equal(await queryMetricValue('completed_total{outcome="success"}'),'18446744073709551615');
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'query metrics fit mobile viewport');
    await page.unroute('**/v1/metrics');
    const invalidQueries = [realExposition.replace(/^kiarni_query_documents_returned_total .*\n/m,'')];
    for (const stage of ['total','wait','execution']) invalidQueries.push(realExposition.replace(new RegExp(`kiarni_query_duration_seconds_count\\{stage="${stage}"\\} \\d+`),`kiarni_query_duration_seconds_count{stage="${stage}"} 18446744073709551615`));
    for (const invalid of invalidQueries) {
      await page.route('**/v1/metrics',route => route.fulfill({status:200,contentType:'text/plain',body:invalid}));
      await page.locator('#refresh-metrics').click();
      await page.locator('#metrics-status').filter({hasText:'métricas válidas'}).waitFor();
      for (const id of tableIds) {
        assert.equal(await page.locator(`#${id}-table`).isVisible(),false);
        assert.equal(await page.locator(`#${id}-values tr`).count(),0);
      }
      await page.unroute('**/v1/metrics');
    }

    await page.route('**/v1/metrics',route => route.fulfill({status:200,contentType:'text/plain',body:realExposition.replace(/kiarni_http_requests_total\{status_class="2xx"\} \d+\n/,'kiarni_http_requests_total{status_class="2xx"} 18446744073709551615\n')}));
    await collectMetrics();
    assert.equal(await httpMetricValue('requests_total{status_class="2xx"}'),'18446744073709551615');
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'metrics fit mobile viewport');
    await page.unroute('**/v1/metrics');
    for (const invalid of [
      realExposition.replace(/^kiarni_http_requests_total\{status_class="2xx"\} .*\n/m,''),
      realExposition.replace(/kiarni_http_request_duration_seconds_count\{kind="stream"\} \d+/,'kiarni_http_request_duration_seconds_count{kind="stream"} 18446744073709551615')
    ]) {
      await page.route('**/v1/metrics',route => route.fulfill({status:200,contentType:'text/plain',body:invalid}));
      await page.locator('#refresh-metrics').click();
      await page.locator('#metrics-status').filter({hasText:'métricas válidas'}).waitFor();
      for (const id of tableIds) {
        assert.equal(await page.locator(`#${id}-table`).isVisible(),false);
        assert.equal(await page.locator(`#${id}-values tr`).count(),0);
      }
      await page.unroute('**/v1/metrics');
    }

    await page.route('**/v1/metrics',route => route.fulfill({status:200,contentType:'text/plain',body:realExposition.replace(/kiarni_changefeed_events_published_total \d+\n/,'kiarni_changefeed_events_published_total 18446744073709551615\n')}));
    await collectMetrics();
    assert.equal(await metricValue('events_published_total'),'18446744073709551615');
    await page.unroute('**/v1/metrics');
    await page.route('**/v1/metrics',route => route.fulfill({status:200,contentType:'text/plain',body:'broken'}));
    await page.locator('#refresh-metrics').click();
    await page.locator('#metrics-status').filter({hasText:'métricas válidas'}).waitFor();
    assert.equal(await page.locator('#metrics-table').isVisible(),false);
    assert.equal(await page.locator('#metrics-values tr').count(),0);
    assert.equal(await page.locator('#http-values tr, #response-values tr, #stream-values tr, #query-values tr, #query-total-values tr, #query-wait-values tr, #query-execution-values tr').count(),0);
    assert.equal(await page.locator('#storage-values tr, #storage-commit-values tr, #storage-operation-values tr').count(),0);
    assert.equal(await page.locator('#storage-operation').isDisabled(),true);
    await page.unroute('**/v1/metrics');
    await page.route('**/v1/metrics',route => route.fulfill({status:503,contentType:'application/json',body:'{"error":{"code":"service_unavailable"},"request_id":"req_test"}'}));
    await page.locator('#refresh-metrics').click();
    await page.locator('#metrics-status').filter({hasText:'service_unavailable (503) · req_test'}).waitFor();
    assert.equal(await page.locator('#session').isVisible(),true);
    await page.unroute('**/v1/metrics');
    let releaseMetrics, metricsReached;
    const delayedMetrics = new Promise(resolve => {releaseMetrics = resolve;});
    const metricsPending = new Promise(resolve => {metricsReached = resolve;});
    await page.route('**/v1/metrics',async route => {metricsReached();await delayedMetrics;await route.fulfill({status:200,contentType:'text/plain',body:realExposition}).catch(() => {});});
    await page.locator('#refresh-metrics').click();
    await metricsPending;
    await page.locator('#disconnect').click();
    releaseMetrics();
    await page.unroute('**/v1/metrics');
    assert.equal(await page.locator('#metrics-values tr').count(),0);
    assert.equal(await page.locator('#http-values tr, #response-values tr, #stream-values tr, #query-values tr, #query-total-values tr, #query-wait-values tr, #query-execution-values tr').count(),0);
    assert.equal(await page.locator('#storage-values tr, #storage-commit-values tr, #storage-operation-values tr').count(),0);
    assert.equal(await page.locator('#storage-operation').isDisabled(),true);
    await page.locator('#token').fill(token);
    await page.locator('#connect button').click();
    await page.locator('#session').waitFor();
    await page.route('**/v1/metrics',route => route.fulfill({status:401,contentType:'application/json',body:'{}'}));
    await page.locator('#refresh-metrics').click();
    await page.locator('#notice').filter({hasText:'Credencial inválida'}).waitFor();
    assert.equal(await page.locator('#session').isVisible(),false);
    assert.equal(await page.locator('#metrics-values tr').count(),0);
    assert.equal(await page.locator('#http-values tr, #response-values tr, #stream-values tr, #query-values tr, #query-total-values tr, #query-wait-values tr, #query-execution-values tr').count(),0);
    assert.equal(await page.locator('#storage-values tr, #storage-commit-values tr, #storage-operation-values tr').count(),0);
    assert.equal(await page.locator('#storage-operation').isDisabled(),true);
    await page.unroute('**/v1/metrics');
    await page.locator('#token').fill(token);
    await page.locator('#connect button').click();
    await page.locator('#session').waitFor();
    await page.route('**/v1/health/live', route => route.abort());
    await page.locator('#refresh').click();
    await page.getByRole('status').filter({hasText:'Não foi possível'}).waitFor();
    assert.equal(await page.locator('#session').isVisible(), false);
    assert.equal(errors.length, 0);
  } finally {
    await browser?.close();
    const stopped = new Promise(resolve => server.once('exit', resolve));
    if (server.exitCode === null) { server.kill('SIGTERM'); await stopped; }
    await rm(temp, {recursive:true,force:true});
  }
});
