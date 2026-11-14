import {subscribe} from '/dashboard/feed.js';
import {parseMetrics, parseHttpMetrics, parseQueryMetrics, parseStorageMetrics, STORAGE_OPERATIONS, readMetrics} from '/dashboard/metrics.js';
'use strict';
(() => {
  const el = id => document.getElementById(id);
  let token = '', cursor = null, controller = null, generation = 0;
  let selected = null, queryController = null, queryGeneration = 0;
  let feedController = null, feedGeneration = 0;
  function stopFeed() {
    feedGeneration++;
    feedController?.abort();
    feedController = null;
    el('feed-result').textContent = '';
    el('feed-result').hidden = true;
    el('feed-count').textContent = '0 eventos nesta conexão';
    el('feed-status').textContent = 'Acompanhamento parado.';
    el('start-feed').disabled = false;
    el('stop-feed').disabled = true;
  }
  async function startFeed() {
    stopFeed();
    if (!selected || !token) return;
    const current = feedGeneration;
    const active = new AbortController();
    feedController = active;
    const timeout = setTimeout(() => active.abort(), 10000);
    let count = 0;
    el('start-feed').disabled = true;
    el('stop-feed').disabled = false;
    el('feed-status').textContent = 'Conectando ao stream…';
    try {
      for await (const record of subscribe(`/v1/collections/${selected}/changes`, token, {signal:active.signal})) {
        if (current !== feedGeneration) return;
        if (record.event === 'ready') {
          clearTimeout(timeout);
          el('feed-status').textContent = 'Conectado. Aguardando mudanças futuras da coleção.';
        } else if (record.event === 'change') {
          count = Math.min(Number.MAX_SAFE_INTEGER, count + 1);
          el('feed-count').textContent = `${count} eventos nesta conexão`;
          el('feed-result').textContent = record.data;
          el('feed-result').hidden = false;
        } else {
          const code = JSON.parse(record.data).code;
          const reason = {overflow:'consumidor atrasado', shutdown:'servidor encerrado', interrupted:'interrupção'}[code] || 'interrupção';
          el('feed-status').textContent = `Stream encerrado: ${reason}. Consulte novamente antes de reconectar; pode haver mudanças não recebidas.`;
        }
      }
    } catch (error) {
      if (current !== feedGeneration) return;
      if (error.message === 'unauthorized') {
        clear(); el('notice').textContent = 'Credencial inválida ou expirada. Conecte novamente.'; return;
      }
      el('feed-status').textContent = 'Stream interrompido ou recusado. Consulte novamente antes de reconectar; pode haver mudanças não recebidas.';
    } finally {
      clearTimeout(timeout);
      if (current === feedGeneration) {
        active.abort(); feedController = null;
        el('start-feed').disabled = false;
        el('stop-feed').disabled = true;
      }
    }
  }
  let countController = null, countGeneration = 0;
  function clearCount() {
    countGeneration++;
    countController?.abort(); countController = null;
    el('document-count').textContent = '—';
    el('count-status').textContent = 'Contagem ainda não consultada.';
    el('refresh-count').disabled = false;
  }
  async function loadCount() {
    clearCount();
    if (!selected || !token) return;
    const current = countGeneration;
    const active = new AbortController();
    countController = active;
    const timeout = setTimeout(() => active.abort(),10000);
    el('refresh-count').disabled = true;
    el('count-status').textContent = 'Contando documentos…';
    try {
      const response = await fetch(`/v1/collections/${selected}/count`, {
        headers:{Authorization:`Bearer ${token}`}, signal:active.signal,
        mode:'same-origin', credentials:'omit', redirect:'error', cache:'no-store', referrerPolicy:'no-referrer'
      });
      if (current !== countGeneration) return;
      if (response.status === 401) {
        clear(); el('notice').textContent = 'Credencial inválida ou expirada. Conecte novamente.'; return;
      }
      const data = await response.json();
      if (current !== countGeneration) return;
      if (!response.ok) {
        el('count-status').textContent = `${data.error?.code || 'Erro HTTP'} (${response.status}) · ${data.request_id || ''}`;
        return;
      }
      if (typeof data.count !== 'string' || !/^\d{1,5}$/.test(data.count) || BigInt(data.count) > 10000n ||
          typeof data.exact !== 'boolean' || data.limits?.documents !== 10000 || data.limits?.document_bytes !== 8388608)
        throw new Error('invalid_count');
      el('document-count').textContent = data.exact ? `Total exato: ${data.count}` : `Pelo menos ${data.count} documentos`;
      el('count-status').textContent = data.exact
        ? 'Contagem exata no snapshot desta leitura; não acompanha novas mudanças automaticamente.'
        : 'Limite de leitura atingido. Este valor é um limite inferior, não uma estimativa do total.';
    } catch (error) {
      if (current !== countGeneration) return;
      el('count-status').textContent = error.name === 'AbortError'
        ? 'A contagem excedeu o tempo de espera.' : 'Não foi possível obter uma contagem válida. Tente novamente.';
    } finally {
      clearTimeout(timeout);
      if (current === countGeneration) {countController = null;el('refresh-count').disabled = false;}
    }
  }
  function closeDocuments() {
    clearCount();
    stopFeed();
    queryGeneration++;
    queryController?.abort();
    queryController = null;
    selected = null;
    el('documents').hidden = true;
    el('query').value = '';
    el('selected-collection').textContent = '';
    el('query-result').textContent = '';
    el('query-result').hidden = true;
    el('query-status').textContent = '';
    el('run-query').disabled = false;
    el('query').disabled = false;
  }
  let statusController = null, statusGeneration = 0;
  function clearStatus() {
    statusGeneration++;
    statusController?.abort(); statusController = null;
    for (const id of ['version','uptime','connections','stream-connections']) el(id).textContent = '—';
    el('server-status').textContent = 'Use Atualizar status para consultar o serviço.';
    el('refresh-status').disabled = false;
  }
  async function loadStatus() {
    clearStatus();
    if (!token) return;
    const current = statusGeneration;
    const active = new AbortController();
    statusController = active;
    const timeout = setTimeout(() => active.abort(),10000);
    el('refresh-status').disabled = true;
    el('server-status').textContent = 'Consultando status…';
    try {
      const response = await fetch('/v1/status', {
        headers:{Authorization:`Bearer ${token}`}, signal:active.signal,
        mode:'same-origin', credentials:'omit', redirect:'error', cache:'no-store', referrerPolicy:'no-referrer'
      });
      if (current !== statusGeneration) return;
      if (response.status === 401) {
        clear(); el('notice').textContent = 'Credencial inválida ou expirada. Conecte novamente.'; return;
      }
      const data = await response.json();
      if (current !== statusGeneration) return;
      if (!response.ok) {
        el('server-status').textContent = `${data.error?.code || 'Erro HTTP'} (${response.status}) · ${data.request_id || ''}`;
        return;
      }
      const c = data.connections;
      if (typeof data.version !== 'string' || data.version.length > 128 ||
          typeof data.uptime_seconds !== 'string' || !/^\d{1,20}$/.test(data.uptime_seconds) ||
          !c || ![c.admitted,c.streaming,c.limit,c.stream_limit].every(Number.isSafeInteger) ||
          c.limit < 1 || c.limit > 128 || c.stream_limit < 1 || c.stream_limit > 32 || c.stream_limit > c.limit ||
          c.admitted < 1 || c.admitted > c.limit || c.streaming < 0 || c.streaming > c.stream_limit || c.streaming > c.admitted)
        throw new Error('invalid_status');
      el('version').textContent = data.version;
      el('uptime').textContent = `${data.uptime_seconds} s`;
      el('connections').textContent = `${c.admitted} / ${c.limit}`;
      el('stream-connections').textContent = `${c.streaming} / ${c.stream_limit}`;
      el('server-status').textContent = `Coleta concluída às ${new Date().toLocaleTimeString()}. Valores pontuais do serviço HTTP.`;
    } catch (error) {
      if (current !== statusGeneration) return;
      el('server-status').textContent = error.name === 'AbortError'
        ? 'A consulta de status excedeu o tempo de espera.' : 'Não foi possível obter um status válido. Tente novamente.';
    } finally {
      clearTimeout(timeout);
      if (current === statusGeneration) {statusController = null;el('refresh-status').disabled = false;}
    }
  }
  let metricsController = null, metricsGeneration = 0;
  let storageRows = [];
  for (const [value,label] of STORAGE_OPERATIONS) el('storage-operation').add(new Option(label,value));
  el('storage-operation').value = 'query_read';
  const buildMetricRows = data => data.map(({name,label,value}) => {
    const row = document.createElement('tr'); row.dataset.metric = name;
    const title = document.createElement('th'); title.scope = 'row'; title.textContent = label;
    const cell = document.createElement('td'); cell.textContent = name === 'file_logical_bytes' && value === 'NaN' ? 'Indisponível' : value;
    row.append(title,cell); return row;
  });
  function renderStorageOperation() {
    const rows = storageRows.filter(row => row.group === `operation:${el('storage-operation').value}`);
    el('storage-operation-caption').textContent = `${STORAGE_OPERATIONS.find(([value]) => value === el('storage-operation').value)?.[1] || 'Primitiva selecionada'} · chamadas e duração em segundos`;
    el('storage-operation-values').replaceChildren(...buildMetricRows(rows));
    el('storage-operation-table').hidden = rows.length === 0;
  }
  el('storage-operation').addEventListener('change',renderStorageOperation);
  function clearMetrics() {
    metricsGeneration++;
    storageRows = [];
    el('storage-operation').disabled = true;
    metricsController?.abort(); metricsController = null;
    for (const [values,table] of [['metrics-values','metrics-table'],['http-values','http-table'],['response-values','response-table'],['stream-values','stream-table'],['query-values','query-table'],['query-total-values','query-total-table'],['query-wait-values','query-wait-table'],['query-execution-values','query-execution-table'],['storage-values','storage-table'],['storage-commit-values','storage-commit-table'],['storage-operation-values','storage-operation-table']]) {
      el(values).replaceChildren(); el(table).hidden = true;
    }
    el('metrics-status').textContent = 'Nenhuma coleta nesta página. Use Atualizar métricas.';
    el('refresh-metrics').disabled = false;
  }
  async function loadMetrics() {
    clearMetrics();
    if (!token) return;
    const current = metricsGeneration;
    const active = new AbortController();
    metricsController = active;
    const timeout = setTimeout(() => active.abort(), 10000);
    el('refresh-metrics').disabled = true;
    el('metrics-status').textContent = 'Coletando métricas…';
    try {
      const response = await fetch('/v1/metrics', {
        headers:{Authorization:`Bearer ${token}`}, signal:active.signal,
        mode:'same-origin', credentials:'omit', redirect:'error', cache:'no-store', referrerPolicy:'no-referrer'
      });
      if (current !== metricsGeneration) return;
      if (response.status === 401) {
        clear(); el('notice').textContent = 'Credencial inválida ou expirada. Conecte novamente.'; return;
      }
      const text = await readMetrics(response);
      if (current !== metricsGeneration) return;
      if (!response.ok) {
        const data = JSON.parse(text);
        el('metrics-status').textContent = `${data.error?.code || 'Erro HTTP'} (${response.status}) · ${data.request_id || ''}`;
        return;
      }
      if (response.headers.get('content-type')?.split(';')[0].trim() !== 'text/plain') throw new Error('invalid_metrics_response');
      const rows = parseMetrics(text);
      const httpRows = parseHttpMetrics(text);
      const queryRows = parseQueryMetrics(text);
      const storage = parseStorageMetrics(text);
      el('metrics-values').replaceChildren(...buildMetricRows(rows));
      el('metrics-table').hidden = false;
      for (const [group,values,table] of [['totals','http-values','http-table'],['response','response-values','response-table'],['stream','stream-values','stream-table']]) {
        el(values).replaceChildren(...buildMetricRows(httpRows.filter(row => row.group === group)));
        el(table).hidden = false;
      }
      for (const [group,values,table] of [['totals','query-values','query-table'],['total','query-total-values','query-total-table'],['wait','query-wait-values','query-wait-table'],['execution','query-execution-values','query-execution-table']]) {
        el(values).replaceChildren(...buildMetricRows(queryRows.filter(row => row.group === group)));
        el(table).hidden = false;
      }
      for (const [group,values,table] of [['summary','storage-values','storage-table'],['commit','storage-commit-values','storage-commit-table']]) {
        el(values).replaceChildren(...buildMetricRows(storage.filter(row => row.group === group)));
        el(table).hidden = false;
      }
      storageRows = storage;
      el('storage-operation').disabled = false;
      renderStorageOperation();
      el('metrics-status').textContent = `Coleta concluída às ${new Date().toLocaleTimeString()}. Valores pontuais; use Atualizar métricas para uma nova coleta.`;
    } catch (error) {
      if (current !== metricsGeneration) return;
      el('metrics-status').textContent = error.name === 'AbortError'
        ? 'A coleta excedeu o tempo de espera. Nenhum valor exibido.'
        : 'Não foi possível obter métricas válidas. Nenhum valor exibido; tente novamente.';
    } finally {
      clearTimeout(timeout);
      if (current === metricsGeneration) { metricsController = null; el('refresh-metrics').disabled = false; }
    }
  }
  let errorsController = null, errorsGeneration = 0;
  function clearErrors() {
    errorsGeneration++;
    errorsController?.abort(); errorsController = null;
    el('errors-values').replaceChildren(); el('errors-table').hidden = true;
    el('errors-status').textContent = 'Use Atualizar erros para consultar.';
    el('refresh-errors').disabled = false;
  }
  async function loadErrors() {
    clearErrors();
    if (!token) return;
    const current = errorsGeneration;
    const active = new AbortController();
    errorsController = active;
    const timeout = setTimeout(() => active.abort(), 10000);
    el('refresh-errors').disabled = true;
    el('errors-status').textContent = 'Consultando erros…';
    try {
      const response = await fetch('/v1/errors/recent', {
        headers:{Authorization:`Bearer ${token}`}, signal:active.signal,
        mode:'same-origin', credentials:'omit', redirect:'error', cache:'no-store', referrerPolicy:'no-referrer'
      });
      if (current !== errorsGeneration) return;
      if (response.status === 401) {
        clear(); el('notice').textContent = 'Credencial inválida ou expirada. Conecte novamente.'; return;
      }
      // Same bounded UTF-8 reader as metrics: never buffer an unlimited response.
      const raw = await readMetrics(response);
      if (current !== errorsGeneration) return;
      const data = JSON.parse(raw);
      if (!response.ok) {
        el('errors-status').textContent = `${data.error?.code || 'Erro HTTP'} (${response.status}) · ${data.request_id || ''}`;
        return;
      }
      const decimal = value => typeof value === 'string' && /^\d{1,20}$/.test(value) && BigInt(value) <= 18446744073709551615n;
      const safeWord = value => typeof value === 'string' && /^[A-Za-z_]{1,64}$/.test(value);
      if (response.headers.get('content-type')?.split(';')[0].trim() !== 'application/json' ||
          data.capacity !== 8 || !decimal(data.dropped_total) || !Array.isArray(data.errors) || data.errors.length > 8)
        throw new Error('invalid_errors');
      const rows = data.errors.map(entry => {
        if (!entry || !/^req_[a-f0-9]{32}$/.test(entry.request_id) ||
            !['GET','HEAD','POST','PUT','PATCH','DELETE','OPTIONS','OTHER'].includes(entry.method) ||
            !(entry.operation === null || safeWord(entry.operation)) ||
            !Number.isInteger(entry.status) || entry.status < 100 || entry.status > 599 ||
            !(entry.error_code === null || safeWord(entry.error_code)) || !decimal(entry.duration_us) ||
            !(entry.timestamp_ms === null || (decimal(entry.timestamp_ms) && BigInt(entry.timestamp_ms) <= 8640000000000000n)) ||
            !['written','write_failed','stream_closed'].includes(entry.transport) ||
            ![null,'overflow','interrupted','shutdown'].includes(entry.stream_end_reason)) throw new Error('invalid_error_entry');
        const row = document.createElement('tr');
        const values = [entry.timestamp_ms === null ? 'Indisponível' : new Date(Number(entry.timestamp_ms)).toISOString(),
          entry.request_id, `${entry.method} ${entry.operation || 'Não reconhecida'}`, `${entry.status} / ${entry.error_code || '—'}`,
          entry.duration_us, `${entry.transport} / ${entry.stream_end_reason || '—'}`];
        for (const value of values) { const cell = document.createElement('td'); cell.textContent = value; row.append(cell); }
        return row;
      });
      el('errors-values').replaceChildren(...rows); el('errors-table').hidden = rows.length === 0;
      el('errors-status').textContent = `${rows.length ? `${rows.length} erros nesta coleta.` : 'Nenhum erro retido.'} Observações descartadas por contenção desde o início: ${data.dropped_total}.`;
    } catch (error) {
      if (current !== errorsGeneration) return;
      el('errors-status').textContent = error.name === 'AbortError'
        ? 'A consulta de erros excedeu o tempo de espera. Nenhum erro exibido.'
        : 'Não foi possível obter erros válidos. Nenhum erro exibido; tente novamente.';
    } finally {
      clearTimeout(timeout);
      if (current === errorsGeneration) {errorsController = null; el('refresh-errors').disabled = false;}
    }
  }
  function clear() {
    clearStatus();
    clearMetrics();
    clearErrors();
    generation++;
    closeDocuments();
    controller?.abort();
    controller = null;
    token = '';
    cursor = null;
    el('token').value = '';
    el('collections').replaceChildren();
    el('live').textContent = el('ready').textContent = '—';
    el('count').textContent = '';
    el('session').hidden = true;
    el('login').hidden = false;
  }
  async function load(after = null) {
    clearStatus();
    clearMetrics();
    clearErrors();
    closeDocuments();
    const current = ++generation;
    controller?.abort();
    controller = new AbortController();
    const signal = controller.signal;
    const timeout = setTimeout(() => controller?.abort(), 10000);
    el('refresh').disabled = el('next').disabled = true;
    el('connect').querySelector('button').disabled = true;
    el('notice').textContent = 'Consultando servidor…';
    const get = async path => {
      const response = await fetch(path, {headers: {Authorization: `Bearer ${token}`}, signal,
        cache: 'no-store', mode: 'same-origin', credentials: 'omit', redirect: 'error', referrerPolicy: 'no-referrer'});
      if (response.status === 401) throw new Error('Credencial inválida ou expirada. Conecte novamente.');
      if (!response.ok) {
        const data = await response.json();
        throw new Error(`${data.error?.code || 'Erro HTTP'} (${response.status}) · ${data.request_id || ''}`);
      }
      return response.json();
    };
    try {
      // Sequential requests also work with the minimum admission limit of one.
      const live = await get('/v1/health/live');
      const ready = await get('/v1/health/ready');
      const page = await get('/v1/collections?limit=100' + (after ? `&after=${encodeURIComponent(after)}` : ''));
      if (generation !== current) return;
      el('live').textContent = live.status === 'live' ? 'Em execução' : 'Indisponível';
      el('ready').textContent = ready.status === 'ready' ? 'Pronto' : 'Indisponível';
      el('collections').replaceChildren(...page.collections.map(name => {
        const item = document.createElement('li');
        const button = document.createElement('button');
        button.type = 'button'; button.className = 'collection-button'; button.textContent = name;
        button.addEventListener('click', () => {
          closeDocuments(); selected = name;
          el('selected-collection').textContent = name;
          el('query').value = JSON.stringify({from: name, limit: 25}, null, 2);
          el('documents').hidden = false;
          el('query').focus();
          runQuery();
        });
        item.append(button); return item;
      }));
      cursor = page.next_after;
      el('next').hidden = !cursor;
      el('empty').hidden = page.collections.length !== 0;
      el('count').textContent = `${page.collections.length} nesta página`;
      el('login').hidden = true;
      el('session').hidden = false;
      el('notice').textContent = 'Atualizado agora. Use Atualizar para consultar novamente.';
    } catch (error) {
      if (generation !== current) return;
      clear();
      el('notice').textContent = error.name === 'AbortError'
        ? 'O servidor não respondeu a tempo. Conecte novamente.'
        : error instanceof TypeError ? 'Não foi possível acessar o servidor. Conecte novamente.' : error.message;
    } finally {
      clearTimeout(timeout);
      if (generation === current || !token) {
        el('refresh').disabled = el('next').disabled = false;
        el('connect').querySelector('button').disabled = false;
      }
    }
  }
  async function runQuery() {
    const current = ++queryGeneration;
    queryController?.abort();
    el('query-result').textContent = '';
    el('query-result').hidden = true;
    const body = el('query').value;
    try {
      if (new TextEncoder().encode(body).length > 16384) throw new Error('Consulta excede 16 KiB.');
      let ast;
      try { ast = JSON.parse(body); } catch { throw new Error('JSON de consulta inválido.'); }
      if (!ast || typeof ast !== 'object' || Array.isArray(ast) || ast.from !== selected)
        throw new Error('O campo from deve corresponder à coleção selecionada.');
      if (!Number.isInteger(ast.limit) || ast.limit < 1 || ast.limit > 1000)
        throw new Error('Informe limit inteiro de 1 a 1000.');
    } catch (error) {
      el('query-status').textContent = error.message;
      el('run-query').disabled = false;
      return;
    }
    const active = new AbortController();
    queryController = active;
    const timeout = setTimeout(() => active.abort(), 10000);
    el('run-query').disabled = true;
    el('query').disabled = true;
    el('query-status').textContent = 'Consultando documentos…';
    try {
      // Validation above does not reserialize literals: send original numeric bytes.
      const response = await fetch(`/v1/collections/${encodeURIComponent(selected)}/query`, {
        method: 'POST', body, headers: {Authorization: `Bearer ${token}`, 'Content-Type': 'application/json'},
        signal: active.signal, cache: 'no-store', mode: 'same-origin', credentials: 'omit',
        redirect: 'error', referrerPolicy: 'no-referrer'
      });
      if (current !== queryGeneration) return;
      if (response.status === 401) {
        clear(); el('notice').textContent = 'Credencial inválida ou expirada. Conecte novamente.'; return;
      }
      const raw = await response.text();
      if (current !== queryGeneration) return;
      if (!response.ok) {
        const data = JSON.parse(raw);
        el('query-status').textContent = `${data.error?.code || 'Erro HTTP'} (${response.status}) · ${data.request_id || ''}`;
        return;
      }
      // Never parse/stringify document results or interpret document content as HTML.
      el('query-result').textContent = raw;
      el('query-result').hidden = false;
      el('query-status').textContent = 'Consulta concluída. Resultado limitado pela AST e pelo servidor.';
    } catch (error) {
      if (current !== queryGeneration) return;
      el('query-status').textContent = error.name === 'AbortError'
        ? 'A consulta excedeu o tempo de espera. Nenhum resultado exibido.'
        : 'Não foi possível consultar documentos. Tente novamente.';
    } finally {
      clearTimeout(timeout);
      if (current === queryGeneration) {
        el('run-query').disabled = false;
        el('query').disabled = false;
        queryController = null;
      }
    }
  }
  el('refresh-errors').addEventListener('click',loadErrors);
  el('refresh-count').addEventListener('click',loadCount);
  el('refresh-status').addEventListener('click',loadStatus);
  el('refresh-metrics').addEventListener('click', loadMetrics);
  el('start-feed').addEventListener('click', startFeed);
  el('stop-feed').addEventListener('click', stopFeed);
  el('query-form').addEventListener('submit', event => { event.preventDefault(); runQuery(); });
  el('close-documents').addEventListener('click', closeDocuments);
  el('connect').addEventListener('submit', event => {
    event.preventDefault(); token = el('token').value.trim(); el('token').value = '';
    if (token) load();
  });
  el('refresh').addEventListener('click', () => load());
  el('next').addEventListener('click', () => load(cursor));
  el('disconnect').addEventListener('click', () => {
    clear(); el('notice').textContent = 'Desconectado desta página.'; el('token').focus();
  });
  window.addEventListener('pagehide', clear);
})();
