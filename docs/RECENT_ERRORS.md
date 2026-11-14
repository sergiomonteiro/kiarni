# Erros recentes — API e dashboard

RF-08/09, M3. GET/HEAD `/v1/errors/recent` requer Bearer e Host/Origin válidos,
como `/v1/status`. Sem query string ou corpo. Cache-Control: no-store.

```json
{"capacity":8,"dropped_total":"0","errors":[{"request_id":"req_00000000000000000000000000000000","timestamp_ms":"1794528000000","method":"POST","operation":"Query","status":422,"error_code":"invalid_query","duration_us":"420","transport":"written","stream_end_reason":null}]}
```

Oito registros no máximo, do último inserido para o primeiro. Retém conclusões
4xx/5xx (inclusive autenticação), falhas locais de escrita e encerramento de SSE
por overflow/interrupção. Shutdown normal não entra. Cada registro só aparece
após a tentativa de resposta/encerramento; operações em andamento não aparecem.
Falhas anteriores à admissão do handler não são registradas.

Request ID é o mesmo da resposta; método é normalizado, operação pode ser null
se ainda não reconhecida. Código é estável ou null, nunca mensagem interna livre.
Timestamp usa relógio de parede, pode ser null e não define ordem de commit.
Timestamp e duração são strings decimais; duração de stream é sua vida inteira,
não latência de eventos. Status é pretendido; escrita local não comprova entrega.

Não contém URL, coleção, ID de documento, AST, payload, headers, IP ou credencial.
O anel é global ao processo e independe do nível/fila de logs. Sob contenção, o
produtor descarta sem esperar e incrementa `dropped_total` saturante (u64 em string).
Esse contador mede contenção, não substituições normais de entradas antigas.
Reiniciar apaga tudo. Não é auditoria, histórico completo, replay ou cursor.

No dashboard, **Atualizar erros** faz uma coleta manual com prazo de dez segundos,
sem interromper consulta ou feed. Sem erros retidos mostra estado vazio. Falhas
ou resposta inválida apagam a coleta anterior; 401 encerra a sessão local.
Desconectar, atualizar visão geral ou sair da página cancela a coleta e apaga os
registros; respostas atrasadas não restauram dados. Conteúdo exibido como texto,
com limite de oito linhas e leitura de resposta limitada a 64 KiB.

## Evidência

- Unitários: retenção/ordem, campos limitados, filtro de conclusões, contenção sem
  espera, perdas saturantes e reserva agregada de telemetria abaixo de 4 KiB.
- Servidor real: autenticação, métodos, Host/Origin, query/body recusados, HEAD,
  correlação por request ID, privacidade, oito entradas, logs off e reinício.
- Navegador real: API real, oito linhas, vazio, resposta inválida, 503, 401,
  desconexão durante coleta e viewport de 375 px.

Comandos/resultados consolidados na [revisão do dashboard](./DASHBOARD_REVIEW.md).
[ADR 0027](./adr/0027-erros-recentes.md).
