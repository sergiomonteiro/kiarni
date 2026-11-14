# 0004 — Ordenação pós-commit e overflow de assinaturas

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md))
- Data: 2026-11-09
- Requisitos e marco: M0; RF-05 e preparação para RF-06/RF-09.

## Contexto

Commit seguido de publicação em tarefas independentes pode inverter a ordem dos
eventos. Precisamos demonstrar ordenação e memória limitada sem decidir o protocolo
de rede. O spike usa o storage experimental do ADR 0003; não integra o servidor.

## Opções consideradas

- Publicar após cada commit em tarefas independentes: simples, mas não preserva
  ordem quando uma tarefa é atrasada entre commit e publicação.
- Fila de writers com worker dedicado: permite admissão limitada, mas introduz
  fila, worker e shutdown adicionais antes de avaliar a solução mínima.
- Seção crítica única cobrindo mutação, commit, sequência e publicação: simples
  de demonstrar, com custo de serialização e fanout no caminho da escrita.
- Esperar espaço de assinante: permite que consumidor lento bloqueie writers.
- Descartar eventos silenciosamente: esconde a perda do estado reativo.
- Desconectar assinatura em overflow: torna a lacuna explícita e limita retenção.

## Decisão proposta para o experimento

O servidor evolui a admissão para espera limitada no ADR 0015. A descrição
abaixo preserva o protocolo exercitado pelo spike original de eventos.

- Um coordenador encapsula o storage; todas as mutações passam pela mesma seção
  crítica. A admissão de writer usa `try_lock`: concorrência retorna `busy`, sem
  acumular uma fila interna ilimitada de writers.
- A seção crítica cobre leitura anterior, commit confirmado, atribuição de
  sequência e publicação por `try_send`. Não esperar pelo consumidor.
- Sequência `u64` é local à instância em execução, começa em 1, cresce somente
  após commits bem-sucedidos e não é persistida. Esgotamento impede novo commit.
  Não é timestamp, cursor durável, replay ou promessa de exactly-once. Como a
  sequência é da instância, assinaturas de coleção podem observar lacunas por
  commits em outras coleções.
- Assinaturas somente de coleção. Eventos insert/update/delete carregam ID,
  coleção, sequência e estados anterior/posterior pertinentes. Eventos e payloads
  são compartilhados via `Arc`, com Debug sem conteúdo de documentos ou IDs.
- Até 8 assinantes e fila entre 1 e 8 eventos por assinante, com capacidade explícita
  ao abrir o coordenador. Os testes usam capacidades 1, 4 e 8; não há default público.
  Cada estado de documento já está limitado a 1 MiB. O limite conservador de
  payload retido nas filas é 128 MiB (8 × 8 × 2 MiB), além de metadados e objetos
  em processamento. Handles do consumidor fora das filas não são controlados.
- Ao encher a fila, sinalizar `overflow` fora da fila de dados e remover o sender.
  O consumidor observa erro terminal e descarta backlog ao recebê-lo, sem receber
  uma falsa indicação de continuidade. Outros assinantes continuam. Handles
  terminais continuam contando nas 8 vagas até Drop, para impedir retenção
  ilimitada de backlogs de assinaturas substituídas. O transporte futuro deverá
  descartar o handle ao encerrar a conexão. O primeiro motivo terminal é mantido.
- A assinatura começa após sua inscrição sob o mesmo lock. Não há snapshot
  inicial nem histórico. Reconexão recebe somente eventos futuros.
- Drop da assinatura libera sua vaga; shutdown encerra assinaturas, rejeita novas
  escritas/assinaturas e não drena backlog. O consumidor observa `shutdown`.
- Falha antes de commit não emite evento. A injeção após commit e antes de publicar
  simula uma lacuna: o dado permanece e as assinaturas ativas são interrompidas.
  Isso não torna commit e publicação atômicos; um crash real pode perder o evento.

## Consequências e limites

O escritor não espera espaço nas filas, mas paga fanout limitado e commit síncrono.
Um processo suspenso ou I/O travado pode reter o lock; não há timeout/cancelamento
de I/O neste spike. Subscrição/shutdown usam lock bloqueante e não são operações
para executar diretamente no futuro runtime async. Não há escolha SSE/WebSocket,
contrato de rede, autenticação ou garantias para escritas fora do coordenador.

O experimento usa erro `busy`; a política de retry do servidor permanece aberta.
Limites são parâmetros experimentais, não defaults públicos congelados.

## Evidências e validação

Executar `cargo test -p kiarni-feed-spike --locked`. Os testes exercitam storage
real, pausa sincronizada após commit, falha antes/depois de commit, consumidores
independentes, overflow, concorrência, reconexão, shutdown e limites de admissão.
Os testes passaram localmente em Linux; detalhes e limitações estão no
[relatório do spike](../spikes/feeds.md). Não houve execução de CI remoto/macOS.
