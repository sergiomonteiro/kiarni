# ADR 0027 — Erros recentes em memória

- Estado: aceito para o recorte RF-08/09 do M3
- Data: 2026-11-14

## Contexto

O dashboard precisa mostrar erros recentes com correlação por request ID, sem
reter documentos, depender de stderr ou criar um serviço de logs obrigatório.

## Decisão

Expor GET/HEAD `/v1/errors/recent`, autenticado e com as mesmas restrições de
Host/Origin da API. Sem corpo ou query string. Resposta JSON `capacity: 8`,
`dropped_total` decimal em string e `errors`, do registro inserido mais recente
para o mais antigo. HEAD mantém os headers da representação, sem corpo.

Reter somente conclusões HTTP 4xx/5xx, falhas locais de escrita e encerramentos
SSE por overflow/interrupção. Status é pretendido, não confirmação de entrega;
a duração de SSE cobre toda a conexão encerrada. Encerramento normal/shutdown
não entra. Falhas anteriores à admissão do handler não entram.

Cada registro contém timestamp de parede em milissegundos (string ou null),
request ID gerado pelo servidor, método normalizado, operação reconhecida ou
null, status, código estável ou null, duração em microssegundos como string,
resultado de transporte e motivo de encerramento de stream ou null. Não reter
URL, coleção, ID de documento, AST, headers, endereço de cliente ou payload.

Um anel fixo de oito registros substitui o mais antigo quando cheio. Produtores
usam try_lock: contenção descarta a observação e incrementa `dropped_total`
saturante. O snapshot copia os registros sob lock curto e serializa depois de
liberá-lo. A ordenação é de inserção aceita, não ordem de commits nem timestamps.
Histórico e snapshot cabem na reserva existente de telemetria, verificada por
teste. Nenhuma alocação dinâmica no registro retido.

A captura precede o filtro/fila do logger e funciona com logs desligados. Reinício
apaga o histórico; não é trilha de auditoria, replay ou cursor durável. A API não
consulta storage. Dashboard coleta manualmente, exibe texto, cancela resultados
obsoletos e apaga dados ao desconectar. Erros de autenticação também entram no
anel; apenas chamadas autenticadas podem lê-lo.

## Alternativas e consequências

Ler stderr tornaria a interface dependente de arquivos e níveis de log. Um
histórico persistente ampliaria retenção e formato de storage. Um canal por
observação adicionaria worker e fila desnecessários. O anel limitado perde
registros antigos e sob contenção deliberadamente; não bloqueia writers ou
espera por consumidores. O endpoint pode aguardar apenas a cópia/inserção curta
sob mutex, nunca I/O. Coletas bem-sucedidas não alimentam o próprio histórico.
