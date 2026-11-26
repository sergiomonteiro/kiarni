# Logs estruturados de requisições

Recorte M3/RF-09. `kiarni serve` emite linhas JSON em stderr para requisições
concluídas, inclusive erros e encerramento de SSE. O request ID é o mesmo de
`X-Request-ID` e dos erros HTTP, gerado pelo servidor; headers recebidos não o
substituem. Não há serviço externo obrigatório ou arquivo de log interno.

```bash
cargo run -p kiarni -- serve --data-dir ./kiarni-data --log-level info
```

Configuração `log_level`: `off`, `error`, `warn`, `info` (default). A precedência
é CLI `--log-level` > `KIARNI_LOG_LEVEL` > JSON `{"log_level":"warn"}` > default.
Valores são estritos mesmo em fontes sobrescritas. `info` inclui sucessos;
`warn` inclui 4xx, falha de escrita ou terminais SSE overflow/interrupted;
`error` inclui 5xx. `off` desliga somente request logs e não cria esse worker.
Mensagens de bootstrap/CLI e profiling opcional são diagnósticos separados.

Exemplo ilustrativo de registro:

```json
{"timestamp_ms":"1794528000000","event":"request_complete","level":"info","request_id":"req_00000000000000000000000000000000","method":"POST","operation":"Query","status":200,"error_code":null,"duration_us":"420","transport":"written","stream_end_reason":null,"dropped_before":0}
```

## Semântica e privacidade

- `timestamp_ms`: tempo Unix em milissegundos, em string; null se indisponível.
  `duration_us`: tempo monotônico em microssegundos, em string. Para requisição
  comum inclui recepção, execução e tentativa de escrita até antes da emissão
  do log; exclui descarte final de corpo recusado e espera no backlog do kernel.
- `operation` é nome fixo da operação validada depois da autenticação, ou null
  se não resolvida (inclui assets do dashboard). `method` vem de lista fixa;
  métodos desconhecidos são OTHER e falhas antes do parsing são UNKNOWN.
- `status` é o status pretendido da resposta, não comprovação de entrega.
  `transport=written` indica escrita local concluída; write_failed indica
  falha/timeout. Nenhum dos dois certifica leitura pelo cliente.
- SSE emite uma linha somente ao encerrar: transport=stream_closed, status
  pretendido 200 e duração da conexão inteira. `stream_end_reason` pode ser
  overflow/shutdown/interrupted ou null quando o terminal não é conhecido,
  inclusive fechamento pelo peer. Não há log por documento/evento transmitido.
- `error_code` é o código fixo do catálogo HTTP ou null. Não contém mensagens
  dinâmicas de erros internos. A ordem de emissão não é ordem de commit.
- Não se registram path/query, coleção, ID de documento, payload, AST, headers,
  IP ou credenciais. Isso também impede diagnóstico pelo conteúdo de consultas:
  use request ID/operation/duração para correlacionar com o cliente.

Falha de geração de request ID, backlog e conexões recusadas antes do handler
não têm registro neste recorte. Uma interrupção abrupta pode perder registros.
Estes logs não constituem auditoria nem prova de durabilidade/exactly-once.

## Limites e saída lenta

Handlers enviam registros tipados por try_send para uma fila de 32 posições;
um worker dedicado serializa e escreve stderr. Fila cheia/sink indisponível
causa descarte, sem esperar por capacidade. `dropped_before` reporta perdas
observadas desde o registro anterior quando há próxima linha emitida; não
substitui um contador durável nem garante relatório final de perda.

Um sink bloqueado pode reter apenas esse worker, o registro em emissão e a fila
limitada. Shutdown fecha o produtor e aguarda até 50 ms; após isso não espera
pelo sink. Uma escrita parcialmente concluída pode deixar linha incompleta.
Coletores devem filtrar linhas JSON válidas com event=request_complete; stderr
não é exclusivamente JSON, pois os diagnósticos existentes permanecem.

A stack do worker é 128 KiB, com reserva de 16 KiB para fila/metadados. O plano
de buffers inclui ambas, preservando configurações antes válidas. São reservas
nominais, não limite de RSS ou garantia de latência. Não houve benchmark novo
para quantificar custo dos logs; `off` permite comparar sem esse trabalho.

O profiling opcional mantém o formato KIARNI_PROFILE, mas também faz emissão
best-effort com espera de até 50 ms no shutdown, stack de 32 KiB e buffer até
8 KiB. Sink bloqueado pode perder o relatório em vez de impedir encerramento.
O plano inclui esses 40 KiB quando a feature está ativa. Para medições, drene
stderr continuamente ou use um arquivo e confira se o relatório está completo.
Não há nova thread de diagnóstico por requisição, rotação ou retries.

## Verificação e pendências

Testes cobrem níveis/precedência e entradas inválidas, filtros de severidade,
schema limitado, fila cheia, registro de perdas e shutdown com sink bloqueado.
Servidor real confirma request IDs, sucesso/erro, privacidade e encerramento SSE;
800 requisições com stderr sem leitura preservam atendimento e encerramento,
com e sem profiling.

[ADR 0023](./adr/0023-logs-de-requisicoes.md). O dashboard agora apresenta métricas de
storage e [erros recentes](./RECENT_ERRORS.md), estes independentes do logger.
A [revisão RF-08/09](./DASHBOARD_REVIEW.md) consolida cobertura e limitações.
Não houve validação de coletor externo ou benchmark desta entrega.

Validação local em 2026-11-13: `cargo fmt --all -- --check`, clippy do workspace
com e sem `kiarni-server/profiling`, ambas as suítes
`cargo test --workspace --locked --offline` e
`cargo test --workspace --features kiarni-server/profiling --locked --offline`
passaram: 247 testes em cada perfil, dois workers de fixture ignorados
diretamente e executados pelos testes pais. Passaram também build release,
fronteira de nove pacotes, links locais e `git diff --check`. Seis casos novos
cobrem configuração, logger e integração HTTP. CI remoto e outras plataformas
não foram executados nesta entrega.

As [métricas HTTP](./HTTP_METRICS.md) agora medem conclusões e latência
independentemente dos níveis e descartes dos logs.

As [métricas de consultas](./QUERY_METRICS.md) agora distinguem processamento,
admissão e limites sem depender desses logs.

As [métricas de storage](./STORAGE_METRICS.md) agora distinguem primitivas,
retorno do commit e receipts sem depender do logger.

## Medição local

A [comparação HTTP de 2026-11-26](./benchmarks/telemetry-http-2026-11-26.md)
registra logs off/info, destino em arquivo temporário, leitores de telemetria
e perdas best-effort. Custos variam entre cenários; não é promessa de overhead
fixo nem de desempenho com sink lento. O relatório preserva tentativas abortadas
e limitações do gerador/host.
