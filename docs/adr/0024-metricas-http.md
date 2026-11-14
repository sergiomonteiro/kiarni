# ADR 0024 — Métricas agregadas de conclusão HTTP

Status: aceito para alpha local. Data: 2026-11-13. M3/RF-09 parcial.

## Decisão

Acrescentar séries de cardinalidade fixa ao GET/HEAD autenticado `/v1/metrics`,
sem modificar as séries de feeds. Medir no handler, antes do envio ao logger:
log_level, fila cheia e falhas de stderr não alteram as métricas.

- `kiarni_http_requests_total{status_class="1xx".."5xx"}`: requisições concluídas
  por classe do status pretendido. 4xx/5xx identificam erros HTTP; não comprovam
  entrega ou commit. Falhas de autenticação e parsing no handler entram.
- `kiarni_http_transport_total{outcome="written"|"write_failed"|"stream_closed"}`:
  resultado local de transporte. Erro de escrita pode ocorrer com status 2xx;
  stream_closed não implica sucesso ou entrega de eventos.
- `kiarni_http_request_duration_seconds{kind="response"|"stream"}`: histogramas
  separados, com buckets cumulativos 0.001, 0.01, 0.1, 1, 5, 30 e +Inf segundos,
  sum e count. Response mede recepção, execução e escrita até a observação no
  handler, excluindo descarte final de corpo recusado. Stream mede a vida inteira
  de uma assinatura HTTP aceita, somente ao encerrar. Recusa de SSE é response.

Usar contadores AtomicU64 fixos, com soma saturante. Buckets são incrementados
com release em ordem decrescente e lidos com acquire em ordem crescente para manter monotonicidade sob
concorrência. Reutilizar o bucket +Inf como count no snapshot. Soma usa micros
inteiros saturantes, expostos em segundos; precisão temporal de um microssegundo
para sum. Não reter amostras, strings de entrada, mapas por cliente ou histórico.

O scrape não entra no writer/gate de queries nem espera o logger. É observação
concorrente, não snapshot transacional de todas as séries: total, transporte e
sum podem ter pequenos desvios transitórios entre si. Contadores reiniciam com
o processo e saturam sem wrap. Reservar 4 KiB para estado/snapshot no plano de
buffers, dentro da economia anterior inclusive com uma vaga e profiling.

## Limites

Contar somente conclusões observadas com request ID e resposta construídos.
Não incluir backlog, recusas antes do handler, falha de geração de request ID,
panic ou término abrupto. Inclui assets, health, status, count e o endpoint de
métricas; cada scrape só aparece na coleta seguinte após sua conclusão. SSE
ativo não entra nesses totais/histogramas até encerrar; admissão e métricas de
feeds continuam fornecendo estado ativo.

Sem rótulos de URL, método arbitrário, operação, coleção, ID, token ou request
ID. Não calcular taxas/percentis no servidor. No incremento inicial da API, o dashboard
manteve suas vinte séries de feeds; a apresentação HTTP foi entregue no
incremento seguinte descrito abaixo. Métricas de execução de queries e storage
permanecem pendentes. Não declarar impacto de desempenho sem benchmark.

## Evidências

[Verificações e interpretação](../HTTP_METRICS.md): suíte padrão com 250 testes
e suíte com profiling passaram, assim como fmt, clippy, release e Chromium real.
Testes cobrem saturação/concorrência, logger desligado/bloqueado, SSE, reset e
privacidade. HEAD agora é testado por seu próprio snapshot, pois o scrape muda
os contadores HTTP. Não houve benchmark novo ou validação externa de Prometheus.

## Apresentação no dashboard

O [painel operacional](../DASHBOARD_METRICS.md) agora apresenta as 26 séries HTTP
em três tabelas (contagens/transporte, resposta e stream), junto das vinte de
feeds. Coleta manual única, 64 KiB/dez segundos, valores originais sem arredondar
contadores u64, sem taxas/percentis. O parser valida todas as famílias antes de
renderizar; ausência/inconsistência limpa todas as tabelas. Mantêm-se autenticação,
cancelamento, limpeza de sessão no 401 e rejeição de respostas antigas. Sem nova
dependência, endpoint ou alteração das séries do backend.
