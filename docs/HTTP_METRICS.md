# Métricas agregadas HTTP

Recorte M3/RF-09: GET/HEAD `/v1/metrics` passa a incluir métricas HTTP além das
[séries existentes de feeds](./CHANGEFEED_METRICS.md). Mesmos Bearer, Host/Origin,
request ID e no-store; sem query ou corpo. Não requer Prometheus em runtime.

Com o cliente `kc` do [guia local](./ALPHA_LOCAL.md):

```bash
kc http://127.0.0.1:8711/v1/metrics
```

## Séries e interpretação

| Família | Rótulos fixos | Significado |
| --- | --- | --- |
| `kiarni_http_requests_total` | `status_class`: 1xx, 2xx, 3xx, 4xx, 5xx | Conclusões por classe do status pretendido; 4xx/5xx são erros HTTP. |
| `kiarni_http_transport_total` | `outcome`: written, write_failed, stream_closed | Resultado local de transporte. Status 2xx também pode terminar com write_failed. |
| `kiarni_http_request_duration_seconds` | `kind`: response, stream; `le` nos buckets | Histograma separado para respostas comuns e vida inteira de streams encerrados. |

Buckets cumulativos: 0.001, 0.01, 0.1, 1, 5, 30 e +Inf segundos. Cada kind tem
`_bucket`, `_sum` e `_count`; +Inf e count usam a mesma observação. A soma acumula
microssegundos inteiros e é exposta como segundos em ponto flutuante. Buckets não
são percentis e não há amostras individuais, taxas ou média calculada no servidor.

Para response, o tempo inclui recepção, validação, espera/execução e escrita até
a observação no handler; exclui descarte final de corpo recusado e backlog do
kernel. Não equivale a tempo do engine/storage. Para stream, mede toda a vida
da conexão aceita e registra somente ao terminar. Uma recusa de assinatura,
como 401/429, é resposta comum. Um stream aberto não aparece no total de
conclusões nem no histograma de streams; use status da admissão e métricas de
assinaturas para observar atividade atual.

Os totais incluem dashboard/assets, health, status, contagem, consultas, erros de
autenticação/parsing no handler e o próprio endpoint de métricas. O scrape só é
contado após escrever a resposta, portanto não inclui sua própria conclusão no
corpo que acabou de coletar. HEAD também conta e descreve seu próprio snapshot;
Content-Length não precisa ser igual ao de um GET anterior. Não há identificação por URL,
coleção, documento, token ou request ID.

`written` não prova leitura pelo cliente. `stream_closed` não significa entrega
bem-sucedida de eventos. Status é pretendido, não confirmação de commit; use os
contratos de escrita e feeds para essas garantias.

## Concorrência, perdas e limites

Métricas são atualizadas antes do logger e independem de log_level, fila de logs
cheia, stderr bloqueado e descartes de registros. Estado fixo AtomicU64, com
incremento saturante e sem mapas, filas ou novo lock compartilhado no caminho
HTTP. Buckets atualizam do maior para o menor com release; coleta lê do menor
para o maior com acquire. Não há espera pelo writer, gate de queries ou logger.

A coleta é observação concorrente, não snapshot transacional global: totais,
transporte e sum podem divergir transitoriamente durante conclusões em paralelo.
Contadores reiniciam no processo e saturam sem wrap; sum também satura. Há
reserva nominal adicional de 4 KiB para estado/snapshot, preservando as
configurações antes válidas, inclusive com uma vaga HTTP e profiling. Não é
limite de RSS nem declaração de ausência de impacto em desempenho.

Conclusões sem request ID/resposta construída, panic ou encerramento abrupto
não são garantidas nesses contadores. Conexões ainda no backlog ou recusadas
antes do handler não entram. Não há métrica in-flight neste recorte: ocupação
atual é consultada em [status HTTP](./SERVER_STATUS.md).

O [dashboard](./DASHBOARD_METRICS.md) apresenta estas métricas em tabelas de
conclusões/transporte, duração de respostas e duração de streams encerrados,
junto da tabela de feeds. A coleta permanece manual, sem taxas ou percentis
derivados. [Métricas de queries](./QUERY_METRICS.md) também estão na API, em famílias
próprias de processamento. [Métricas de storage](./STORAGE_METRICS.md) também estão na API; sua
apresentação no dashboard e erros recentes continuam pendentes.

## Verificação

Testes determinísticos cobrem classes de erro, falha de transporte, separação
de SSE, buckets, soma, saturação e atualização concorrente. Servidor real
verifica log_level=off, contagem entre scrapes, stream ativo/encerrado, privacidade
e reset no restart. O teste existente de 800 requisições com stderr sem leitura
agora também verifica que a contagem HTTP permanece disponível. O parser de
métricas do dashboard continua aceitando as séries de feeds e ignorando novas.

[ADR 0024](./adr/0024-metricas-http.md). Não houve validação com Prometheus externo,
promtool ou benchmark novo nesta entrega. O RF-09 continua parcial.

Validação local em 2026-11-13: 250 testes Rust passaram no perfil padrão; a suíte
com `kiarni-server/profiling` também passou. Passaram fmt, clippy com e sem
profiling, build release, fronteira de nove pacotes, quatro testes do parser de
métricas do dashboard, integração Chromium real, links locais e
`git diff --check`. Dois workers de fixture continuam ignorados diretamente e
executados pelos testes pais. Foram adicionados dois testes unitários e uma
integração HTTP, além de ampliar o caso de sink bloqueado. O teste antigo de
HEAD foi ajustado para não comparar tamanhos de snapshots diferentes. CI remoto
e outras plataformas não foram executados nesta entrega.

## Validação operacional posterior

A [revisão M3](./RELEASE_REVIEW.md) e o [ensaio com coletor externo](./OPERATIONAL_VALIDATION.md)
registram o estado atual e os resultados por commit/duração. Os limites e
resultados datados acima descrevem aquele incremento; dashboard/storage/erros
recentes já estão na [revisão do dashboard](./DASHBOARD_REVIEW.md).
Não inferir validação prolongada de produção a partir de uma janela controlada.
