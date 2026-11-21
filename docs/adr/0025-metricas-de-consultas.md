# ADR 0025 — Métricas de processamento de consultas

Status: aceito para alpha local. Data: 2026-11-14. M3/RF-09 parcial.

## Decisão

Adicionar séries fixas ao `/v1/metrics`, sem alterar AST, resultados ou limites.
Contar POST de query após autenticação, Host/Origin e resolução da operação,
antes de validar corpo/AST. Falhas anteriores pertencem somente ao HTTP.
Contagem de documentos compartilha o gate mas não entra nas métricas de query.

- `kiarni_query_completed_total{outcome}`: success, rejected, busy, scan_limit,
  result_limit, not_found, storage_error, evaluation_error, interrupted,
  internal_error. Scan_limit exige limite de dataset no storage/avaliação;
  complexidade inválida da AST é rejected. Busy indica admissão indisponível.
- `kiarni_query_duration_seconds{stage="total"|"wait"|"execution"}`: histogramas
  com os mesmos buckets e soma em micros/segundos do HTTP. Total vai do início
  da operação reconhecida à montagem da resposta (inclui leitura restante do
  corpo e validação).
  Wait cobre entrada no gate, inclusive recusa; execution só existe se admitida,
  incluindo reparse, snapshot, avaliação, serialização e limite do envelope.
  Não incluir escrita no socket, recepção de headers ou autenticação.
- `kiarni_query_execution_total{access="id"|"scan"|"secondary"}`: caminho escolhido após
  parse da AST na execução admitida, inclusive quando depois falha. Não é
  contador de leituras de páginas ou documentos examinados.
- `kiarni_query_documents_returned_total` e `kiarni_query_result_bytes_total`:
  somente resultados montados com sucesso, bytes do envelope JSON completo.
  Não comprovam entrega; resultado vazio também é sucesso.

Reutilizar o histograma atômico de cardinalidade fixa do HTTP, sem framework
novo: saturação, release/acquire e +Inf igual ao count. Manter estado de query
junto dos 4 KiB de telemetria já reservados; buffers de exposição pertencem à
reserva de resposta. Sem filas, logger, writer ou locks adicionais. Scrape não
espera pelo gate. Estado ativo de uma consulta só aparece ao finalizar.

Sem nomes de coleção, AST, campos, IDs, tokens ou request IDs nos rótulos. Sem
histórico/amostras individuais. Coleta concorrente, não snapshot transacional;
reinício zera contadores. Panic/término abrupto não garantem observação. Erros de
transporte após processamento não mudam o outcome da query; ver HTTP/logs.

Atualização 2026-11-21, ADR 0034: secondary é uma terceira categoria fixa,
preenchida após seleção efetiva do índice e preservada em falha posterior.
Antes da seleção, manter a classificação inicial id/scan, inclusive para coleção
inexistente. query_read cobre seleção/carregamento, excluindo avaliação/serialização.
Dashboard atual valida 42 séries de query, com quinze totais/caminhos e 27 valores
de histogramas. Sem novo campo na resposta HTTP nem rótulos com dados de usuário.

## Escopo

Métricas de storage e documentos/bytes examinados ficam para outro incremento.
Não expandir o scan para medir, não mudar ordenação/gate/limites e não registrar
payload. No incremento inicial, o dashboard validava suas 46 séries HTTP/feeds. A
apresentação das novas séries foi entregue no incremento descrito abaixo. RF-09 não é declarado concluído.

## Evidências

[Contrato e verificações](../QUERY_METRICS.md): 255 testes Rust passaram com e
sem profiling. Gate real demonstrou recusa/shutdown sem executar; integrações
cobriram resultados, ID/scan, limites reais, privacidade e reinício. Fmt, clippy,
release e Chromium passaram, preservando a apresentação HTTP/feeds existente.
Sem benchmark novo ou mudança de persistência/contrato das consultas.

## Apresentação no dashboard

O [painel de métricas](../DASHBOARD_METRICS.md) apresenta também as 41 séries de
query: quatorze resultados/caminhos/totais e 27 valores dos três histogramas.
São 87 séries em oito tabelas; tempos de query ficam em seções expansíveis.
Coleta manual de até 64 KiB/dez segundos, validação conjunta antes de renderizar
e limpeza de todas as tabelas em falha/401/cancelamento. Contadores permanecem
texto original u64; sem AST, rótulos de coleção, taxas, médias ou percentis. Sem
alterar backend, persistência, autenticação ou dependências de runtime.
