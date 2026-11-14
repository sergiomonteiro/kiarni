# ADR 0026 — Métricas de primitivas de storage e commit

Status: aceito para alpha local. Data: 2026-11-14. M3/RF-09 parcial.

## Decisão

Instrumentar fronteiras lógicas de Store, sem scans adicionais, mudança de
formato, dependências ou política de commit. Expor no `/v1/metrics` autenticado:

- operations_total{operation,outcome=success|error} e operation_duration_seconds
  para doze primitivas fixas: health, catalog_list, catalog_lookup, count,
  query_read, document_read, insert, update, delete, collection_create,
  collection_drop e update_group. São chamadas instrumentadas, não I/O físico,
  requisições HTTP ou quantidade de documentos. Erros incluem conflitos de domínio.
- commits_total{outcome=confirmed|uncertain} e commit_duration_seconds: somente
  tentativas de commit no handle aberto, sem bootstrap. Ok significa confirmação
  retornada pelo engine; erro é uncertain, nunca rollback presumido. Aborts e
  no-ops não são tentativas de commit.
- group_receipts_total{commit=confirmed|not_required} e group_items_total com
  changed/noop/rejected: somente receipts retornados; erro pós-commit pode ter
  commit confirmado sem receipt. Grupo misto pode retornar receipt com itens
  recusados. Itens não são páginas escritas ou HTTPs confirmados.
- file_logical_bytes e file_bytes_available: tamanho lógico de data.redb via
  metadata de um descritor de leitura mantido durante a vida do Store. Não mede
  blocos físicos, bytes de documentos ou espaço livre. Falha de metadata expõe
  NaN e disponibilidade zero, sem inventar tamanho zero.

Histogramas usam buckets/saturação/release-acquire equivalentes aos HTTP, em
implementação local ao crate storage para preservar fronteiras. Estado fixo e
sem nomes/IDs/payloads, filas, logs ou mapas por coleção. Cabe na reserva de
telemetria já existente de 4 KiB junto de HTTP/query; corpo de exposição pertence
à reserva de resposta. Não é teto de RSS nem benchmark de custo.

Adicionar um descritor de leitura por Store, aberto após validação e fechado
após o Database (ordem dos campos), sem adquirir locks novos. Fstat não varre
banco nem abre transação de escrita, mas continua sujeito ao filesystem; não
prometer deadline físico. Scrape não entra no writer/gate nem espera logger.
Contadores são observações concorrentes, reiniciam em Store::open e não são
persistidos. Falhas de startup/panic/término abrupto não têm observação garantida.

## Escopo

Preservar formato 1, resultados, locks, durabilidade Immediate e publicação
pós-commit. Helpers de fixture e chamadas aninhadas não devem contar a mesma
primitiva duas vezes; documentar a fronteira de cada nome. Sem medição de páginas,
cache hit, fsync isolado ou documentos examinados neste incremento. No incremento inicial, o dashboard
manteve suas 87 séries; a integração foi entregue conforme descrito abaixo. RF-09 segue parcial.

## Evidências

[Contrato e validações locais](../STORAGE_METRICS.md): suíte padrão com 258
testes e suíte com profiling passaram; fmt, clippy, release e Chromium também.
Testes mostram no-op sem commit, grupo misto, confirmação do engine sem receipt,
privacidade, exposição limitada, coleta com transação aberta e reset. A fixture
de concorrência foi isolada da expiração durante sua montagem; o limite de
produção não foi alterado. Sem benchmark novo ou falha física de disco injetada.

## Apresentação no dashboard

O [painel operacional](../DASHBOARD_METRICS.md) agora valida as 150 séries de
storage junto das 87 existentes. Resumo de arquivo/commit/grupos, histograma de
commit expansível e uma das doze primitivas por seletor, reutilizando a mesma
coleta manual de até 64 KiB/dez segundos. Mostra Indisponível para tamanho NaN
com disponibilidade zero; inconsistências reais limpam todas as tabelas.
Contadores u64 continuam como texto original. Sem alterar endpoint, séries,
backend, persistência, autenticação ou dependências de runtime.
