# Dispatcher de coleção e framing SSE

`kiarni-feeds` é independente de HTTP e storage concreto. Não depende de spikes,
não inicia threads e é usado pelo writer e pelo transporte SSE do produto.
Usa apenas kiarni-core e std. Métricas completas do M2 permanecem pendentes.

Dispatcher aceita 1–32 assinaturas e filas de 1–16 eventos, conforme o ADR 0012. subscribe recebe
somente eventos futuros, sem snapshot/histórico. Não é clonável; inscrição e
publicação exigem propriedade mutável exclusiva. Consumidores usam try_next
em threads independentes.

Change valida nomes, ID e JSON before/after, com até 1 MiB por estado e `_id`
igual à chave. Insert tem apenas after; delete, apenas before; update tem ambos.
Os bytes são preservados sem conversão numérica. Kind não prova mutação real:
rejeições e no-ops precisam ser filtrados pelo futuro coordenador.

## Publicação e fronteira de commit

1. Preparar estados validados e seu orçamento no futuro caminho de escrita.
2. reserve(1..32) verifica esgotamento de sequência antes do commit e empresta
   o dispatcher mutavelmente. Reservar não consome números.
3. Somente após commit confirmado, publish_confirmed publica as mudanças na
   ordem do grupo. Essa é uma precondição do chamador, não uma prova de storage.
4. finish encerra a publicação. Números excedentes não são consumidos. Aborto/
   no-op pode descartar a reserva não usada sem emitir evento.

A sequência local começa em 1 e cresce por mudança publicada, inclusive sem
assinantes. Não é persistida nem cursor de replay. Coleções podem ver lacunas
por eventos de outras coleções. A futura inscrição passará pelo writer como
barreira, sem correr em paralelo ao commit.

Publicação parcial abandonada sem finish encerra o dispatcher como Interrupted.
interrupt permite fechamento explícito. Antes do primeiro evento, Drop não sabe
se houve commit: o chamador deve interromper se o dado confirmou e a publicação
falhou. Não há atomicidade commit/publicação, replay ou exactly-once.

## Backpressure e limites

Eventos usam Arc compartilhado e fanout por try_send, sem esperar espaço.
Overflow sinaliza terminal fora da fila, remove sender e preserva os outros
consumidores. Ao observar terminal, o consumidor descarta backlog; um evento
já entregue pode estar em processamento. O primeiro motivo terminal vence.
Shutdown/interrupção fecham novas inscrições e reservas.

Handle terminal ocupa vaga até Drop, evitando backlogs substituídos ilimitados.
O teto teórico isolado é 32×16×2 MiB = 1 GiB; compartilhamento reduz cópias.
O servidor restringe as combinações pelo plano global de 512 MiB; defaults
continuam em oito streams e fila quatro, sem redução silenciosa de configuração.
Eventos retidos pelo chamador, frames SSE e estados temporários de grupos ficam
fora desse orçamento e exigem limites na integração. Debug não inclui coleção,
ID ou payload. Isso não representa limite superior de RSS.

## Verificação e próximo incremento

Nove testes verificam filtragem/lacunas, Arc, dois consumidores em threads
distintas, ordem, overflow, backlog, vagas terminais, reconexão sem histórico,
esgotamento, reserva sem publicação, interrupção parcial e estados inválidos.

```bash
cargo test -p kiarni-feeds --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
python3 scripts/check_product_dependencies.py
```

O writer já captura before/after dentro das transações e publica após commit,
incluindo estados intermediários de um grupo. Inscrição é uma barreira com
validação de coleção existente. Drop interrompe os observadores daquela geração;
recriar o nome não reinscreve automaticamente. Sem observadores ativos, o writer
avança a sequência confirmada sem reter payloads; essa operação é recusada se
houver assinante da coleção. Payloads capturados usam Arc compartilhado com o
resultado de updates, evitando duplicação integral para evento e resposta.

Os testes de integração do coordenador pausam antes de executar e depois de
commit/before-publication, verificam ausência de eventos antecipados e os estados
já confirmados. Exercitam grupos, no-ops, rejeições, registro como barreira,
insert/delete, overflow, drop/recreate, falha antes/depois de commit e shutdown.
A rota de changes agora usa esse fluxo pelo servidor HTTP, com Bearer e
revalidação da sessão. O módulo sse pré-calcula o tamanho codificado e percorre
os estados em slices com prefixos data:, sem construir uma String de 3 MiB.
O transporte mantém um evento de até 2 MiB e buffer de 8 KiB: cabe na reserva
existente de 3 MiB por stream, mesmo com a fila preenchida novamente. Frames
acima de 3 MiB são recusados antes de enviar change. Não há replay ou snapshot.

## Telemetria

metrics() retorna um handle somente-leitura para snapshots, independente da fila
do writer. Contadores e histograma são fixos; há um contador atômico por slot
para backlog, incluindo terminais retidos. Drop remove o sender, drena e só então
libera o slot, evitando misturar gerações da contagem. Motivos são contados uma
vez, incluindo cancelamento e encerramentos do transporte. O renderer usa apenas
famílias e rótulos estáticos. [Contrato](../../docs/CHANGEFEED_METRICS.md).
