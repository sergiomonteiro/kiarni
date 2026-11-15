# Métricas dos changefeeds

GET `/v1/metrics` retorna Prometheus texto 0.0.4. HEAD mantém headers, sem corpo.
O endpoint agora também inclui [métricas HTTP](./HTTP_METRICS.md); este guia
descreve somente as famílias kiarni_changefeed_.
Acesso exige o mesmo Bearer e Host/Origin da API. Não aceita query ou corpo.
Request ID, no-store, limites de resultado e prazos HTTP continuam aplicados.

Com a função kc do [roteiro da alpha](./ALPHA_LOCAL.md):

```bash
kc http://127.0.0.1:8711/v1/metrics
```

Todas as séries têm prefixo `kiarni_changefeed_`:

- subscriptions_active mostra quem ainda recebe eventos; subscription_handles
  inclui terminais que continuam ocupando vagas.
- queue_events mede backlog retido, sem eventos em processamento; queue_capacity
  mostra o teto configurado. Descarte terminal e cancelamento também reduzem fila.
- events_published_total conta mudanças confirmadas; events_unobserved_total
  separa as que não tinham observadores. No-op e erros não contam.
- events_enqueued_total conta por assinatura: um evento para dois assinantes
  incrementa dois. Isso não comprova leitura pelo cliente.
- subscriptions_ended_total tem reason fixo: overflow, shutdown, interrupted
  ou cancelled. O primeiro motivo vence, sem contagem dupla ao descartar o handle.
- commit_to_dispatch_seconds expõe buckets cumulativos, sum e count: mede da
  confirmação retornada pelo storage até concluir o fanout observado, incluindo
  preparação do evento e overflow. Não inclui socket/ack ou commit. Não há
  amostra sem observadores; buckets não são percentis calculados pelo servidor.

São métricas agregadas, sem rótulos de coleção, ID, payload ou token. Gauges
podem variar durante scrape; séries reiniciam com o processo. Não se garante
uma última coleta depois do shutdown. Não há amostras individuais retidas.
O scrape não entra na fila do writer; usa uma vaga HTTP normal e o lock curto
do dispatcher, sem aguardar storage. Configurações anteriores permanecem válidas.

## Verificação e escopo

Testes cobrem backlog, consumo/overflow, handles terminais, primeiro motivo,
cancelamento, publicação sem observadores e buckets cumulativos. A integração
HTTP usa stream real, insert, no-op/erro e desconexão, verifica privacidade,
GET/HEAD, autenticação, Origin e recusa de corpo/query. Outro teste consulta
métricas enquanto o writer está bloqueado.

Este recorte descreve somente feeds. [HTTP](./HTTP_METRICS.md),
[consultas](./QUERY_METRICS.md) e [logs de requisições](./REQUEST_LOGS.md) têm
guias próprios. [Storage](./STORAGE_METRICS.md) já tem primitivas/commits e
tamanho lógico na API; apresentação no dashboard e diagnóstico interno
necessário seguem pendentes.
O [gate M2 foi aprovado localmente](./M2_REVIEW.md), com as evidências de
reatividade e métricas e seus limites registrados. O [dashboard](./DASHBOARD_METRICS.md) já apresenta esses indicadores por coleta
manual autenticada, sem taxas ou percentis derivados.
[Contrato e decisões](./adr/0019-metricas-de-changefeeds.md).

## Validação operacional posterior

A [revisão M3](./RELEASE_REVIEW.md) e o [ensaio com coletor externo](./OPERATIONAL_VALIDATION.md)
registram o estado atual e os resultados por commit/duração. Os limites e
resultados datados acima descrevem aquele incremento; dashboard/storage/erros
recentes já estão na [revisão do dashboard](./DASHBOARD_REVIEW.md).
Não inferir validação prolongada de produção a partir de uma janela controlada.
