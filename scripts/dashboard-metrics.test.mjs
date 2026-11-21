import test from 'node:test';
import assert from 'node:assert/strict';
import {parseMetrics, readMetrics, SERIES, MAX_METRICS_BYTES} from '../crates/server/dashboard/metrics.mjs';
const prefix = 'kiarni_changefeed_';
const fixture = (overrides = {}) => SERIES.map(([name]) => `${prefix}${name} ${overrides[name] ?? '0'}`).join('\n') + '\n';
test('fixed series, original large counters and decimal sum', () => {
  const rows = parseMetrics('# HELP irrelevant\n' + fixture({events_published_total:'18446744073709551615',commit_to_dispatch_seconds_sum:'1.234567890123456789e-10'}) + 'future_metric 42\n');
  assert.equal(rows.length,20);
  assert.equal(rows.find(row => row.name === 'events_published_total').value,'18446744073709551615');
  assert.equal(rows.at(-2).value,'1.234567890123456789e-10');
});
test('missing, duplicate, invalid and nonfinite values rejected', () => {
  assert.throws(() => parseMetrics(fixture().replace('kiarni_changefeed_queue_events 0\n','')),/missing_metric/);
  assert.throws(() => parseMetrics(fixture()+'kiarni_changefeed_queue_events 0\n'),/duplicate_metric/);
  for (const value of ['-1','NaN','+Inf','1.2','18446744073709551616','<img>']) assert.throws(() => parseMetrics(fixture({queue_events:value})),/invalid_metric/);
  for (const value of ['NaN','+Inf','1e999','-0.1']) assert.throws(() => parseMetrics(fixture({commit_to_dispatch_seconds_sum:value})),/invalid_metric/);
});
test('histogram requires cumulative buckets and matching count', () => {
  assert.throws(() => parseMetrics(fixture({'commit_to_dispatch_seconds_bucket{le="0.0001"}':'1'})),/invalid_histogram/);
  assert.throws(() => parseMetrics(fixture({commit_to_dispatch_seconds_count:'1'})),/invalid_histogram/);
  const changes = Object.fromEntries(SERIES.filter(([name]) => name.includes('_bucket') || name.endsWith('_count')).map(([name]) => [name,'9007199254740993']));
  assert.equal(parseMetrics(fixture(changes)).at(-1).value,'9007199254740993');
});
test('bounded UTF-8 input and body reader', async () => {
  assert.throws(() => parseMetrics('é'.repeat(MAX_METRICS_BYTES / 2 + 1)),/metrics_too_large/);
  assert.equal(await readMetrics(new Response(fixture())),fixture());
  await assert.rejects(readMetrics(new Response('x'.repeat(MAX_METRICS_BYTES + 1))),/metrics_too_large/);
  await assert.rejects(readMetrics(new Response(new Uint8Array([255]))));
});

test('HTTP counters and separate response/stream histograms preserve raw precision', async () => {
  const {parseHttpMetrics,HTTP_SERIES} = await import('../crates/server/dashboard/metrics.mjs');
  const text = HTTP_SERIES.map(([name]) => `kiarni_http_${name} ${name === 'requests_total{status_class="2xx"}' ? '18446744073709551615' : name.includes('_sum{') ? '1.234567890123456789e-10' : '0'}`).join('\n');
  const rows = parseHttpMetrics(fixture() + text);
  assert.equal(rows.length,26);
  assert.equal(rows.filter(row => row.group === 'totals').length,8);
  assert.equal(rows.filter(row => row.group === 'response').length,9);
  assert.equal(rows.filter(row => row.group === 'stream').length,9);
  assert.equal(rows.find(row => row.name === 'requests_total{status_class="2xx"}').value,'18446744073709551615');
  assert.equal(rows.find(row => row.name === 'request_duration_seconds_sum{kind="response"}').value,'1.234567890123456789e-10');
});
test('HTTP parser rejects partial, duplicate and invalid histograms independently', async () => {
  const {parseHttpMetrics,HTTP_SERIES} = await import('../crates/server/dashboard/metrics.mjs');
  const text = HTTP_SERIES.map(([name]) => `kiarni_http_${name} 0`).join('\n') + '\n';
  assert.throws(() => parseHttpMetrics(fixture()),/missing_metric/);
  assert.throws(() => parseHttpMetrics(text + 'kiarni_http_requests_total{status_class="2xx"} 0\n'),/duplicate_metric/);
  for (const kind of ['response','stream']) {
    assert.throws(() => parseHttpMetrics(text.replace(`_bucket{kind="${kind}",le="0.001"} 0`,`_bucket{kind="${kind}",le="0.001"} 1`)),/invalid_histogram/);
    assert.throws(() => parseHttpMetrics(text.replace(`_count{kind="${kind}"} 0`,`_count{kind="${kind}"} 1`)),/invalid_histogram/);
    for (const raw of ['NaN','+Inf','-1','<img>']) assert.throws(() => parseHttpMetrics(text.replace(`_sum{kind="${kind}"} 0`,`_sum{kind="${kind}"} ${raw}`)),/invalid_metric/);
  }
  for (const raw of ['-1','1.2','18446744073709551616']) assert.throws(() => parseHttpMetrics(text.replace('status_class="2xx"} 0',`status_class="2xx"} ${raw}`)),/invalid_metric/);
  assert.throws(() => parseHttpMetrics('x'.repeat(MAX_METRICS_BYTES + 1)),/metrics_too_large/);
});

test('query metrics preserve counters and distinguish total/wait/execution histograms', async () => {
  const {parseQueryMetrics,QUERY_SERIES} = await import('../crates/server/dashboard/metrics.mjs');
  const text = QUERY_SERIES.map(([name]) => `kiarni_query_${name} ${name === 'completed_total{outcome="success"}' ? '18446744073709551615' : name.includes('_sum{') ? '1.234567890123456789e-10' : '0'}`).join('\n');
  const rows = parseQueryMetrics(fixture() + text);
  assert.equal(rows.length,42);
  assert.equal(rows.filter(row => row.group === 'totals').length,15);
  for (const group of ['total','wait','execution']) assert.equal(rows.filter(row => row.group === group).length,9);
  assert.equal(rows.find(row => row.name === 'completed_total{outcome="success"}').value,'18446744073709551615');
  assert.equal(rows.find(row => row.name === 'duration_seconds_sum{stage="wait"}').value,'1.234567890123456789e-10');
});
test('query parser rejects missing/duplicate samples and each inconsistent stage', async () => {
  const {parseQueryMetrics,QUERY_SERIES} = await import('../crates/server/dashboard/metrics.mjs');
  const text = QUERY_SERIES.map(([name]) => `kiarni_query_${name} 0`).join('\n')+'\n';
  assert.throws(() => parseQueryMetrics(fixture()),/missing_metric/);
  assert.throws(() => parseQueryMetrics(text+'kiarni_query_documents_returned_total 0\n'),/duplicate_metric/);
  for (const stage of ['total','wait','execution']) {
    assert.throws(() => parseQueryMetrics(text.replace(`_bucket{stage="${stage}",le="0.001"} 0`,`_bucket{stage="${stage}",le="0.001"} 1`)),/invalid_histogram/);
    assert.throws(() => parseQueryMetrics(text.replace(`_count{stage="${stage}"} 0`,`_count{stage="${stage}"} 1`)),/invalid_histogram/);
    for (const raw of ['NaN','+Inf','-1','<img>']) assert.throws(() => parseQueryMetrics(text.replace(`_sum{stage="${stage}"} 0`,`_sum{stage="${stage}"} ${raw}`)),/invalid_metric/);
  }
  for (const raw of ['-1','1.2','18446744073709551616']) assert.throws(() => parseQueryMetrics(text.replace('documents_returned_total 0',`documents_returned_total ${raw}`)),/invalid_metric/);
  assert.throws(() => parseQueryMetrics('é'.repeat(MAX_METRICS_BYTES)),/metrics_too_large/);
});

test('storage parser preserves u64 and validates every fixed operation', async () => {
  const {parseStorageMetrics,STORAGE_SERIES,STORAGE_OPERATIONS} = await import('../crates/server/dashboard/metrics.mjs');
  const text = STORAGE_SERIES.map(([name]) => `kiarni_storage_${name} ${name === 'file_bytes_available' ? '1' : name === 'file_logical_bytes' || name === 'commits_total{outcome="confirmed"}' ? '18446744073709551615' : '0'}`).join('\n');
  const rows = parseStorageMetrics(text);
  assert.equal(rows.length,150); assert.equal(rows.filter(row => row.group === 'summary').length,9);
  assert.equal(rows.filter(row => row.group === 'commit').length,9);
  for (const [operation] of STORAGE_OPERATIONS) assert.equal(rows.filter(row => row.group === `operation:${operation}`).length,11);
  assert.equal(rows.find(row => row.name === 'file_logical_bytes').value,'18446744073709551615');
  assert.equal(rows.find(row => row.name === 'commits_total{outcome="confirmed"}').value,'18446744073709551615');
});
test('storage unavailable metadata is explicit and invalid pairs are rejected', async () => {
  const {parseStorageMetrics,STORAGE_SERIES} = await import('../crates/server/dashboard/metrics.mjs');
  const fixture = (available,bytes) => STORAGE_SERIES.map(([name]) => `kiarni_storage_${name} ${name === 'file_bytes_available' ? available : name === 'file_logical_bytes' ? bytes : '0'}`).join('\n');
  assert.equal(parseStorageMetrics(fixture('0','NaN')).find(row => row.name === 'file_logical_bytes').value,'NaN');
  for (const [available,bytes] of [['1','NaN'],['0','0'],['2','100'],['1','-1']]) assert.throws(() => parseStorageMetrics(fixture(available,bytes)));
});
test('storage missing/duplicate fields and all histograms are checked', async () => {
  const {parseStorageMetrics,STORAGE_SERIES,STORAGE_OPERATIONS} = await import('../crates/server/dashboard/metrics.mjs');
  const text = STORAGE_SERIES.map(([name]) => `kiarni_storage_${name} ${name === 'file_bytes_available' ? '1' : '0'}`).join('\n')+'\n';
  assert.throws(() => parseStorageMetrics(''),/missing_metric/);
  assert.throws(() => parseStorageMetrics(text+'kiarni_storage_file_bytes_available 1\n'),/duplicate_metric/);
  for (const [operation] of STORAGE_OPERATIONS) {
    assert.throws(() => parseStorageMetrics(text.replace(`_bucket{operation="${operation}",le="0.001"} 0`,`_bucket{operation="${operation}",le="0.001"} 1`)),/invalid_histogram/);
    assert.throws(() => parseStorageMetrics(text.replace(`_count{operation="${operation}"} 0`,`_count{operation="${operation}"} 1`)),/invalid_histogram/);
  }
  assert.throws(() => parseStorageMetrics(text.replace('commit_duration_seconds_count 0','commit_duration_seconds_count 1')),/invalid_histogram/);
  for (const raw of ['NaN','+Inf','-1','<img>']) assert.throws(() => parseStorageMetrics(text.replace('commit_duration_seconds_sum 0',`commit_duration_seconds_sum ${raw}`)),/invalid_metric/);
  assert.throws(() => parseStorageMetrics(text.replace('file_logical_bytes 0','file_logical_bytes 18446744073709551616')),/invalid_metric/);
  assert.throws(() => parseStorageMetrics('x'.repeat(MAX_METRICS_BYTES+1)),/metrics_too_large/);
});
