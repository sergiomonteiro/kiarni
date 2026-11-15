# Métricas de processamento de consultas

Recorte M3/RF-09 sobre a API/AST existente. GET/HEAD `/v1/metrics` acrescenta
famílias `kiarni_query_`, com a mesma autenticação, Host/Origin e limites da API.
Não altera consultas, dados persistidos, gate FIFO ou resultados. As métricas
independem de log_level e stderr; não exigem serviço externo em runtime.

## O que é contado

Uma tentativa começa quando o servidor reconhece POST de query após autenticar
e validar Host/Origin, antes de validar corpo/AST. Falha anterior de autenticação,
parsing de headers ou resolução de método/rota pertence só às métricas HTTP.
Contagem de documentos usa o mesmo gate, mas não é query para estas famílias.

`kiarni_query_completed_total{outcome}` tem dez valores fixos:

| Outcome | Interpretação |
| --- | --- |
| success | Resposta de consulta montada com sucesso, inclusive resultado vazio. |
| rejected | Corpo, AST ou outro requisito de entrada recusado; inclui recepção de corpo que falha antes de executar. |
| busy | Gate de execução indisponível, inclusive timeout de admissão. |
| scan_limit | Orçamento de documentos/bytes do dataset atingido no storage ou avaliação. |
| result_limit | Resultado excede o teto do engine ou do envelope HTTP configurado. |
| not_found | Coleção não encontrada durante processamento. |
| storage_error | Storage indisponível, conforme erro estável da API. |
| evaluation_error | Comparação ou ordenação encontrou número/tipo não suportado. |
| interrupted | Processamento interrompido por shutdown observado. |
| internal_error | Falha interna/inconsistência mapeada pelo contrato existente. |

Complexidade inválida da AST é rejected, mesmo quando compartilha o código
público query_too_complex com limites de scan. Scan_limit exige ter atingido o
orçamento de dataset; não é inferido somente pelo status 422 ou nome do erro.

## Tempos e caminhos

`kiarni_query_duration_seconds{stage}` separa:

- total: reconhecimento da operação autenticada até montar a resposta. Inclui
  leitura restante do corpo, validação, espera e processamento. Não inclui
  recepção de headers, autenticação ou escrita no socket.
- wait: entrada no gate FIFO, incluindo recusas. Compartilhar a vaga com uma
  contagem de documentos também pode gerar espera de query.
- execution: somente depois de admitir; inclui reparse da AST, lookup de
  coleção, snapshot/leitura, avaliação, serialização e checagem do envelope.

Rejeição antes do gate tem amostra total, sem wait/execution. Falha no gate tem
total/wait, sem execution. A soma usa microssegundos saturantes, expostos em
segundos. Buckets cumulativos: 0.001, 0.01, 0.1, 1, 5, 30 e +Inf; cada stage
expõe sum/count. Buckets não são percentis. A duração não é tempo isolado de
storage nem tempo completo percebido pelo cliente.

`kiarni_query_execution_total{access="id"|"scan"}` registra o caminho escolhido
após parse na execução admitida. Id significa AST com `id`; sem esse campo,
scan. Também conta uma escolha cuja execução falha depois, por exemplo coleção
inexistente. Não mede documentos examinados, páginas lidas ou acesso a disco.

`kiarni_query_documents_returned_total` e `kiarni_query_result_bytes_total`
acumulam somente respostas montadas com sucesso; bytes incluem o envelope JSON.
Erro e resultados parciais abortados não incrementam esses valores. Success não
prova entrega ao cliente: uma escrita no socket pode falhar depois. Verifique
[métricas HTTP](./HTTP_METRICS.md) e [logs](./REQUEST_LOGS.md) para transporte.

## Limites, concorrência e privacidade

Estado fixo, contadores atômicos saturantes e histogramas compartilhando a mesma
implementação do HTTP. Sem mapas por coleção, filas ou lock adicional. Scrape
não aguarda gate, writer ou logger. Os estados HTTP/query e seus snapshots cabem
nos 4 KiB de telemetria já reservados. Isso não constitui teto de RSS nem prova
de desempenho. Não há mudança nos orçamentos de scan ou no formato persistido.

A observação ocorre ao finalizar o processamento. Uma consulta ainda em curso
não tem amostra final; panic/encerramento abrupto não garantem registro. Coleta
concorrente não é snapshot transacional global: famílias e estágios podem estar
transitoriamente em momentos distintos. Contadores reiniciam com o processo.

Não há rótulos com coleção, campos, AST, IDs, payload, token ou request ID. Não
há amostras individuais, histórico, média ou percentis calculados. O [dashboard](./DASHBOARD_METRICS.md) apresenta estas séries junto de HTTP e
feeds: uma tabela de resultados/caminhos e três histogramas expansíveis, sem
histórico, médias ou percentis.

## Verificação e próximos incrementos

Testes de unidade cobrem outcomes, estágios, erros sem contabilizar payload,
reserva de memória e timeout/shutdown no gate real. A integração HTTP usa logs
desligados e demonstra rejeição antes de executar, sucesso vazio, ID/scan,
coleção inexistente, avaliação não suportada, precisão de bytes, privacidade e
reset no restart. Outra integração usa dados reais para exceder limites de
scan e resultado sem contabilizar respostas parciais.

[ADR 0025](./adr/0025-metricas-de-consultas.md). As [métricas de storage](./STORAGE_METRICS.md) agora cobrem primitivas, commits
e tamanho lógico. Documentos/bytes examinados e erros recentes continuam
pendentes. RF-09 continua parcial. Não houve benchmark novo nem validação com
Prometheus externo/promtool nesta entrega.

Validação local em 2026-11-14: 255 testes Rust passaram com e sem
`kiarni-server/profiling` (dois workers de fixture ignorados diretamente e
executados pelos testes pais). Passaram fmt, clippy em ambos os perfis, build
release, fronteira de nove pacotes, seis testes do parser de métricas, Chromium
real, links locais e `git diff --check`. Foram adicionados três testes de unidade
e duas integrações HTTP. O histograma comum mantém o formato HTTP anterior,
validado também pelo dashboard. CI remoto, outras plataformas e coletor externo
não foram executados nesta entrega.

## Validação operacional posterior

A [revisão M3](./RELEASE_REVIEW.md) e o [ensaio com coletor externo](./OPERATIONAL_VALIDATION.md)
registram o estado atual e os resultados por commit/duração. Os limites e
resultados datados acima descrevem aquele incremento; dashboard/storage/erros
recentes já estão na [revisão do dashboard](./DASHBOARD_REVIEW.md).
Não inferir validação prolongada de produção a partir de uma janela controlada.
