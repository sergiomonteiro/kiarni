# ADR 0019 — Métricas de changefeeds

- Status: aceita no recorte local M2/RF-05 e RF-09 parcial.
- Base: PRD RF-09 e ADRs 0004/0005/0012/0018.

## Decisão

Expor GET/HEAD `/v1/metrics` no mesmo servidor, com Bearer, Host/Origin,
limites e request ID existentes. Não aceitar corpo ou query. Erros continuam
JSON; sucesso usa `text/plain; version=0.0.4; charset=utf-8`, sem cache.
O [formato texto Prometheus](https://prometheus.io/docs/instrumenting/exposition_formats/)
segue a preferência do PRD, sem serviço adicional obrigatório.

O recorte contém somente feeds. Requests, queries, conexões genéricas, storage
e logs estruturados continuam no restante do RF-09. Nomes/rótulos são fixos,
sem coleção, documento, credencial, request ID ou entrada de cliente.

## Semântica sob o prefixo kiarni_changefeed_

| Métrica | Interpretação |
|---|---|
| subscriptions_active | Assinaturas elegíveis para eventos |
| subscription_handles | Handles retidos, inclusive terminais até Drop |
| queue_events | Eventos retidos nas filas, excluindo processamento em voo |
| queue_capacity | Capacidade configurada: slots × itens por fila |
| subscriptions_total | Inscrições registradas, não confirmações de ready recebido |
| events_published_total | Mudanças confirmadas, inclusive sem observadores |
| events_unobserved_total | Mudanças sem observadores ativos no fanout |
| events_enqueued_total | Enqueues por assinatura, não entrega/ack do cliente |
| subscriptions_ended_total{reason} | Primeiro motivo: overflow, shutdown, interrupted ou cancelled |
| commit_to_dispatch_seconds | Histograma do retorno da confirmação de storage até completar fanout observado |

No-op/rejeição não incrementa publicação. Cancelled é Drop ativo sem terminal
anterior; timeout/erro, expiração e frame excessivo marcam interrupted. Primeiro
motivo vence, sem duplicação ao desconectar. Shutdown é contado, sem garantia
de scrape posterior ao fim do processo.

O histograma inclui preparar evento e tentativas que resultem em overflow;
não mede commit, espera do writer ou entrega pelo socket. Sem observadores,
não há amostra. Buckets cumulativos em segundos: 0,0001; 0,001; 0,01; 0,1; 1;
+Inf. Grupos compartilham a referência de confirmação por mudança observada.

Contadores reiniciam por processo e saturam sem wrap. Gauges são observações
operacionais concorrentes, não snapshot transacional global. Filas usam contagem
atômica por slot e liberam backlog antes de reutilizá-lo. Histograma é fixo,
sem retenção de amostras ou séries por usuário.

Um handle somente-leitura compartilha o dispatcher: o scrape não espera na fila
de escrita ou pelo commit, mas pode disputar o lock curto de inscrição/fanout,
limitado a 32 assinaturas. O estado fixo e a resposta cabem nas reservas atuais.
Não se alteram configuração, teto de buffers, persistência ou garantias do SSE.
Esse recorte não declara ganho de CPU/memória nem todo RF-09 concluído.

## Evidências

Passaram 234 testes Rust, com e sem profiling, fmt, clippy, build release e a
fronteira de nove pacotes. Dois workers de fixture são ignorados diretamente e
executados pelos testes pais. Cinco testes novos cobrem contabilidade/buckets,
consulta sem aguardar writer e HTTP autenticado com stream real. Um teste
existente confirma que a amostra aparece somente após publicação pós-commit.
Não houve promtool ou servidor Prometheus externo; o formato foi validado
localmente, com famílias estáticas e checks de buckets/count no HTTP.
A [revisão do gate M2](../M2_REVIEW.md) aprovou esse recorte localmente;
todo o RF-09 permanece pendente.
