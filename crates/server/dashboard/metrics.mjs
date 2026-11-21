// Fixed product exposition, not a general Prometheus parser.
export const MAX_METRICS_BYTES = 64 * 1024;
const prefix = 'kiarni_changefeed_';
const histogram = 'commit_to_dispatch_seconds';
export const SERIES = Object.freeze([
  ['subscriptions_active','Assinaturas ativas'],
  ['subscription_handles','Vagas retidas, incluindo assinaturas encerradas'],
  ['queue_events','Eventos nas filas (exclui os em processamento)'],
  ['queue_capacity','Capacidade total configurada das filas'],
  ['subscriptions_total','Inscrições registradas'],
  ['events_published_total','Mudanças confirmadas'],
  ['events_unobserved_total','Mudanças sem observadores'],
  ['events_enqueued_total','Enfileiramentos por assinatura (não confirma entrega)'],
  ...[['overflow','Fila cheia'],['shutdown','Servidor encerrado'],['interrupted','Interrupção'],['cancelled','Cancelamento']]
    .map(([reason,label]) => [`subscriptions_ended_total{reason="${reason}"}`,`Encerramentos: ${label}`]),
  ...['0.0001','0.001','0.01','0.1','1','+Inf']
    .map(bound => [`${histogram}_bucket{le="${bound}"}`,`Amostras com tempo ≤ ${bound === '+Inf' ? 'infinito' : bound} s (cumulativas)`]),
  [`${histogram}_sum`,'Soma dos tempos de distribuição interna (segundos)'],
  [`${histogram}_count`,'Amostras de distribuição interna']
].map(row => Object.freeze(row)));
export function parseMetrics(text) {
  if (new TextEncoder().encode(text).length > MAX_METRICS_BYTES) throw new Error('metrics_too_large');
  const known = new Set(SERIES.map(([name]) => prefix + name));
  const values = new Map();
  for (const line of text.split('\n')) {
    if (!line || line.startsWith('#')) continue;
    const space = line.indexOf(' ');
    const name = line.slice(0,space);
    if (!known.has(name)) continue;
    const value = line.slice(space + 1);
    if (values.has(name)) throw new Error('duplicate_metric');
    if (name.endsWith('_sum')) {
      if (value.length > 128 || !/^(?:\d+(?:\.\d+)?)(?:e[+-]?\d+)?$/i.test(value) || !Number.isFinite(Number(value))) throw new Error('invalid_metric');
    } else if (!/^\d{1,20}$/.test(value) || BigInt(value) > 18446744073709551615n) throw new Error('invalid_metric');
    values.set(name,value);
  }
  const rows = SERIES.map(([name,label]) => {
    if (!values.has(prefix + name)) throw new Error('missing_metric');
    return {name,label,value:values.get(prefix + name)};
  });
  const buckets = rows.slice(12,18).map(row => BigInt(row.value));
  if (buckets.some((value,index) => index && value < buckets[index - 1]) || buckets.at(-1) !== BigInt(rows.at(-1).value)) throw new Error('invalid_histogram');
  return rows;
}
export async function readMetrics(response) {
  if (!response.body) throw new Error('missing_body');
  const reader = response.body.getReader();
  const bytes = new Uint8Array(MAX_METRICS_BYTES);
  let used = 0;
  try {
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) return new TextDecoder('utf-8',{fatal:true}).decode(bytes.subarray(0,used));
      if (used + chunk.value.length > bytes.length) throw new Error('metrics_too_large');
      bytes.set(chunk.value,used); used += chunk.value.length;
    }
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}

const httpPrefix = 'kiarni_http_';
const httpHistogram = 'request_duration_seconds';
const httpBounds = ['0.001','0.01','0.1','1','5','30','+Inf'];
export const HTTP_SERIES = Object.freeze([
  ...['1xx','2xx','3xx','4xx','5xx'].map(status => [`requests_total{status_class="${status}"}`,`Conclusões HTTP ${status}${status === '4xx' ? ' · erros do cliente' : status === '5xx' ? ' · erros do servidor' : ''}`,'totals']),
  ...[['written','Escritas locais concluídas'],['write_failed','Falhas de escrita local'],['stream_closed','Streams encerrados']]
    .map(([outcome,label]) => [`transport_total{outcome="${outcome}"}`,label,'totals']),
  ...['response','stream'].flatMap(kind => [
    ...httpBounds.map(bound => [`${httpHistogram}_bucket{kind="${kind}",le="${bound}"}`,`Amostras ≤ ${bound === '+Inf' ? 'infinito' : bound} s (cumulativas)`,kind]),
    [`${httpHistogram}_sum{kind="${kind}"}`,'Soma das durações (segundos)',kind],
    [`${httpHistogram}_count{kind="${kind}"}`,'Quantidade de amostras',kind]
  ])
].map(row => Object.freeze(row)));
export function parseHttpMetrics(text) {
  if (new TextEncoder().encode(text).length > MAX_METRICS_BYTES) throw new Error('metrics_too_large');
  const known = new Map(HTTP_SERIES.map(([name,label,group]) => [httpPrefix + name,{name,label,group}]));
  const values = new Map();
  for (const line of text.split('\n')) {
    if (!line || line.startsWith('#')) continue;
    const space = line.indexOf(' '), name = line.slice(0,space);
    if (!known.has(name)) continue;
    if (values.has(name)) throw new Error('duplicate_metric');
    const raw = line.slice(space + 1);
    if (name.startsWith(httpPrefix + httpHistogram + '_sum{')) {
      if (raw.length > 128 || !/^(?:\d+(?:\.\d+)?)(?:e[+-]?\d+)?$/i.test(raw) || !Number.isFinite(Number(raw))) throw new Error('invalid_metric');
    } else if (!/^\d{1,20}$/.test(raw) || BigInt(raw) > 18446744073709551615n) throw new Error('invalid_metric');
    values.set(name,raw);
  }
  const rows = [...known].map(([key,row]) => {
    if (!values.has(key)) throw new Error('missing_metric');
    return {...row,value:values.get(key)};
  });
  for (const kind of ['response','stream']) {
    const histogram = rows.filter(row => row.group === kind);
    const buckets = histogram.slice(0,7).map(row => BigInt(row.value));
    if (buckets.some((value,index) => index && value < buckets[index - 1]) || buckets.at(-1) !== BigInt(histogram.at(-1).value)) throw new Error('invalid_histogram');
  }
  return rows;
}

const queryPrefix = 'kiarni_query_';
const queryHistogram = 'duration_seconds';
export const QUERY_SERIES = Object.freeze([
  ...[['success','Respostas montadas com sucesso'],['rejected','Rejeições de entrada'],['busy','Admissão indisponível'],['scan_limit','Limites de scan'],['result_limit','Limites de resultado'],['not_found','Coleção não encontrada'],['storage_error','Storage indisponível'],['evaluation_error','Falhas de avaliação'],['interrupted','Processamento interrompido'],['internal_error','Falhas internas']]
    .map(([outcome,label]) => [`completed_total{outcome="${outcome}"}`,label,'totals']),
  ['execution_total{access="id"}','Execuções com caminho por ID escolhido','totals'],
  ['execution_total{access="scan"}','Execuções com caminho de scan escolhido','totals'],
  ['execution_total{access="secondary"}','Execuções com índice secundário escolhido','totals'],
  ['documents_returned_total','Documentos em respostas montadas com sucesso','totals'],
  ['result_bytes_total','Bytes de envelopes montados com sucesso','totals'],
  ...['total','wait','execution'].flatMap(stage => [
    ...httpBounds.map(bound => [`${queryHistogram}_bucket{stage="${stage}",le="${bound}"}`,`Amostras ≤ ${bound === '+Inf' ? 'infinito' : bound} s (cumulativas)`,stage]),
    [`${queryHistogram}_sum{stage="${stage}"}`,'Soma das durações (segundos)',stage],
    [`${queryHistogram}_count{stage="${stage}"}`,'Quantidade de amostras',stage]
  ])
].map(row => Object.freeze(row)));
export function parseQueryMetrics(text) {
  if (new TextEncoder().encode(text).length > MAX_METRICS_BYTES) throw new Error('metrics_too_large');
  const known = new Map(QUERY_SERIES.map(([name,label,group]) => [queryPrefix + name,{name,label,group}]));
  const values = new Map();
  for (const line of text.split('\n')) {
    if (!line || line.startsWith('#')) continue;
    const space = line.indexOf(' '), name = line.slice(0,space);
    if (!known.has(name)) continue;
    if (values.has(name)) throw new Error('duplicate_metric');
    const raw = line.slice(space + 1);
    if (name.startsWith(queryPrefix + queryHistogram + '_sum{')) {
      if (raw.length > 128 || !/^(?:\d+(?:\.\d+)?)(?:e[+-]?\d+)?$/i.test(raw) || !Number.isFinite(Number(raw))) throw new Error('invalid_metric');
    } else if (!/^\d{1,20}$/.test(raw) || BigInt(raw) > 18446744073709551615n) throw new Error('invalid_metric');
    values.set(name,raw);
  }
  const rows = [...known].map(([key,row]) => {
    if (!values.has(key)) throw new Error('missing_metric');
    return {...row,value:values.get(key)};
  });
  for (const stage of ['total','wait','execution']) {
    const histogram = rows.filter(row => row.group === stage);
    const buckets = histogram.slice(0,7).map(row => BigInt(row.value));
    if (buckets.some((value,index) => index && value < buckets[index - 1]) || buckets.at(-1) !== BigInt(histogram.at(-1).value)) throw new Error('invalid_histogram');
  }
  return rows;
}

export const STORAGE_OPERATIONS = Object.freeze([
  ['health','Readiness do storage'],['catalog_list','Listagem de coleções'],['catalog_lookup','Lookup de coleção'],['count','Contagem de documentos'],['query_read','Leitura para consulta'],['document_read','Leitura de documento'],['insert','Inserção'],['update','Atualização individual'],['delete','Exclusão de documento'],['collection_create','Criação de coleção'],['collection_drop','Exclusão de coleção'],['update_group','Atualização em grupo']
].map(row => Object.freeze(row)));
const storagePrefix = 'kiarni_storage_';
export const STORAGE_SERIES = Object.freeze([
  ['file_logical_bytes','Tamanho lógico de data.redb (bytes)','summary'],
  ['file_bytes_available','Metadados do arquivo disponíveis (1 = sim, 0 = não)','summary'],
  ...[['confirmed','Commits confirmados pelo engine'],['uncertain','Tentativas de commit sem confirmação']].map(([outcome,label]) => [`commits_total{outcome="${outcome}"}`,label,'summary']),
  ...[['confirmed','Resultados de grupo com commit'],['not_required','Resultados de grupo sem commit necessário']].map(([commit,label]) => [`group_receipts_total{commit="${commit}"}`,label,'summary']),
  ...[['changed','Itens alterados em resultados de grupo'],['noop','Itens sem alteração em resultados de grupo'],['rejected','Itens recusados em resultados de grupo']].map(([outcome,label]) => [`group_items_total{outcome="${outcome}"}`,label,'summary']),
  ...httpBounds.map(bound => [`commit_duration_seconds_bucket{le="${bound}"}`,`Amostras ≤ ${bound === '+Inf' ? 'infinito' : bound} s (cumulativas)`,'commit']),
  ['commit_duration_seconds_sum','Soma das durações (segundos)','commit'],
  ['commit_duration_seconds_count','Quantidade de tentativas de commit','commit'],
  ...STORAGE_OPERATIONS.flatMap(([operation]) => [
    ...[['success','Chamadas retornadas com sucesso'],['error','Chamadas retornadas com erro']].map(([outcome,label]) => [`operations_total{operation="${operation}",outcome="${outcome}"}`,label,`operation:${operation}`]),
    ...httpBounds.map(bound => [`operation_duration_seconds_bucket{operation="${operation}",le="${bound}"}`,`Amostras ≤ ${bound === '+Inf' ? 'infinito' : bound} s (cumulativas)`,`operation:${operation}`]),
    [`operation_duration_seconds_sum{operation="${operation}"}`,'Soma das durações (segundos)',`operation:${operation}`],
    [`operation_duration_seconds_count{operation="${operation}"}`,'Quantidade de chamadas medidas',`operation:${operation}`]
  ])
].map(row => Object.freeze(row)));
export function parseStorageMetrics(text) {
  if (new TextEncoder().encode(text).length > MAX_METRICS_BYTES) throw new Error('metrics_too_large');
  const known = new Map(STORAGE_SERIES.map(([name,label,group]) => [storagePrefix+name,{name,label,group}]));
  const values = new Map();
  for (const line of text.split('\n')) {
    if (!line || line.startsWith('#')) continue;
    const space = line.indexOf(' '), name = line.slice(0,space);
    if (!known.has(name)) continue;
    if (values.has(name)) throw new Error('duplicate_metric');
    const raw = line.slice(space+1);
    if (name === storagePrefix+'file_logical_bytes' && raw === 'NaN') {
      values.set(name,raw); continue;
    }
    if (name === storagePrefix+'commit_duration_seconds_sum' || name.startsWith(storagePrefix+'operation_duration_seconds_sum{')) {
      if (raw.length > 128 || !/^(?:\d+(?:\.\d+)?)(?:e[+-]?\d+)?$/i.test(raw) || !Number.isFinite(Number(raw))) throw new Error('invalid_metric');
    } else if (!/^\d{1,20}$/.test(raw) || BigInt(raw) > 18446744073709551615n) throw new Error('invalid_metric');
    values.set(name,raw);
  }
  const rows = [...known].map(([key,row]) => {
    if (!values.has(key)) throw new Error('missing_metric');
    return {...row,value:values.get(key)};
  });
  const available = values.get(storagePrefix+'file_bytes_available');
  const bytes = values.get(storagePrefix+'file_logical_bytes');
  if (!['0','1'].includes(available) || (available === '0') !== (bytes === 'NaN')) throw new Error('invalid_file_metric');
  for (const group of ['commit',...STORAGE_OPERATIONS.map(([operation]) => `operation:${operation}`)]) {
    const histogram = rows.filter(row => row.group === group && !row.name.startsWith('operations_total'));
    const buckets = histogram.slice(0,7).map(row => BigInt(row.value));
    if (buckets.some((value,index) => index && value < buckets[index-1]) || buckets.at(-1) !== BigInt(histogram.at(-1).value)) throw new Error('invalid_histogram');
  }
  return rows;
}
