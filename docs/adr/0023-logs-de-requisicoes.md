# ADR 0023 — Logs estruturados e limitados de requisições

Status: aceito para alpha local. Data: 2026-11-13. M3/RF-09 parcial.

## Decisão

Emitir uma linha JSON em stderr ao concluir cada requisição admitida com request
ID disponível. Incluir timestamp Unix em ms (string, ou null se indisponível),
level, event=request_complete, request_id gerado pelo servidor, método de lista
fixa, operação validada (ou null), status HTTP pretendido, código estável de erro
(ou null), duração monotônica em microssegundos (string), resultado de transporte
e motivo terminal SSE quando conhecido. Não registrar path/query, nomes/IDs de
coleção/documento, headers, corpos, AST, credenciais, IP ou mensagens de erro
dinâmicas. Entradas arbitrárias não são rótulos de log.

Status pretendido não prova entrega pelo socket nem commit, e encerramento do
stream não prova entrega de eventos. SSE gera registro somente ao terminar;
duração inclui sua vida inteira. Chamadas curtas incluem recepção, execução e
tentativa de escrita, excluindo o descarte final de corpo recusado. Backlog do
kernel, falha de geração de request ID e conexões recusadas antes do handler não
produzem registro neste recorte. Não são logs de auditoria ou trilha durável.

Adicionar `log_level`: off/error/warn/info, default info, CLI > KIARNI_LOG_LEVEL
> JSON > default, validação estrita. Info inclui sucessos, warn inclui 4xx e
falhas de transporte/terminais de interrupção, error inclui 5xx. Não adicionar
modo de payload/debug. Com off, não criar worker de logs.

Fila sync_channel com 32 registros tipados, envio por try_send sem espera e um
worker exclusivo com stack de 128 KiB. Reserva de 16 KiB para fila/metadados,
além da stack, incluída no plano de buffers. Não muda configurações antes
válidas: os 144 KiB adicionais cabem na reserva economizada de cópia HTTP,
inclusive com uma requisição admitida. Não é teto de RSS.

Quando fila/sink indisponível, descartar registro, sem bloquear writers ou HTTP.
`dropped_before` informa descartes observados desde o registro anterior quando
uma linha seguinte consegue ser emitida; não garante relatório final de perda.
Sink lento pode bloquear somente esse worker, não os handlers. No shutdown,
fechar o produtor e esperar até 50 ms pelo flush; se não terminar, desanexar o
worker e permitir fim do processo. Registros finais podem se perder. Sem retry,
spawns ilimitados, arquivo/rotação internos ou dependência externa obrigatória.

## Limites

Não reformatar mensagens existentes de bootstrap/CLI ou profiling como se fossem
request logs; consumidores devem selecionar linhas JSON event=request_complete.
Não prometer ordem de commit pela ordem dos logs. Sem buffer de erros recentes
no dashboard, logs internos completos ou métricas de queries/storage neste
incremento. Não declarar ganho ou ausência de regressão de desempenho sem medir.

O teste com profiling revelou bloqueio da emissão antiga em stderr cheio. O
relatório opcional KIARNI_PROFILE passa a usar uma única emissão best-effort no
shutdown, com worker de 32 KiB, buffer até 8 KiB e espera de até 50 ms. Esses
40 KiB adicionais somente com a feature também cabem no orçamento anterior,
mesmo com uma vaga HTTP. O formato do relatório permanece o mesmo; um sink
bloqueado pode perder o relatório em vez de travar o encerramento.

## Evidências

[Verificações e contrato operacional](../REQUEST_LOGS.md): 247 testes passaram
com e sem profiling, incluindo o teste que reproduziu o bloqueio antigo do
relatório no shutdown, privacidade, request IDs, fila cheia e perdas reportadas.
Fmt, clippy, release e fronteira de pacotes passaram. Não houve benchmark novo,
validação remota de CI ou coletor externo; os logs permanecem best-effort.
