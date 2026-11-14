# Servidor e coordenador do produto

O módulo `writer` implementa o coordenador do ADR 0017, usado por todas as
rotas HTTP mutantes. O gate do ADR 0015 permanece para a query exclusiva.
Não existe dependência de spike.

## Uso e ordem

`Coordinator::start(Arc<Store>, Policy, Arc<AtomicBool>)` cria um worker com
stack explícita de 768 KiB. `client()` fornece produtores clonáveis.
`Client::submit(Command)` transfere strings e corpo do comando; retorna um
Ticket após admissão. `Ticket::wait()` recebe Reply, cujo `outcome()` pode ser
consultado por referência. O resultado da espera indica falha de admissão/
worker; o resultado dentro de Reply indica sucesso ou erro daquela operação.

| Comando | Execução |
|---|---|
| Update Replace/MergePatch | Até 32 updates consecutivos e 8 MiB de corpos, sem espera deliberada |
| Insert | Barreira isolada, com geração de IDs e até oito tentativas de colisão preservadas |
| Delete | Barreira isolada |
| CreateCollection / DropCollection | Barreiras isoladas; drop continua exigindo coleção vazia |
| Subscribe | Barreira interna, valida coleção e registra somente eventos futuros; stream SSE autenticado |

Um comando não ultrapassa outro que espera orçamento. O worker só reúne o
prefixo elegível da fila; comandos expirados/cancelados não executam. Cada
grupo usa `Store::update_group`, que resolve os nomes no instante da execução
e guarda o JSON próprio de cada resultado. Não há releitura posterior que
substitua um resultado intermediário pelo estado final do documento.
Sequências são locais, com possíveis lacunas por cancelamento, sem replay.

Ao ativar o módulo, todas as mutações devem passar pela mesma instância. Não
misturar este coordenador com escritores diretos ou outro gate independente.
Arc<Store> permite leituras por snapshot enquanto o worker escreve; exclusividade
dos caminhos mutantes deve ser garantida pela composição do servidor.

## Limites e ciclo de vida

Policy aceita 1–128 requisições e limite de documento de 1 byte até 1 MiB.
Também recebe os limites de streams (1–32) e filas (1–16) da configuração.
O total de pedidos inclui espera por orçamento, comandos em execução e respostas
ainda retidas. O orçamento de comandos admitidos é `min(max_requests, 64)`, com
até 8 MiB das capacidades alocadas de strings/corpos. Retirar da fila não libera
esses créditos. A medição usa capacity, não apenas comprimento dos dados.

Antes de reservar um pedido, o coordenador valida nomes/IDs, comprimento do corpo
e capacidades: até 128 bytes por string e 1 MiB por Vec de corpo. Capacidades
excessivas são recusadas com write_busy. Essa defesa interna também limita as
alocações que aguardam orçamento; não altera o contrato de tamanho HTTP, ainda
não conectado. O adaptador precisará respeitar esse formato de propriedade.

A reserva RAII acompanha o comando até seu Reply ser descartado, incluindo o
JSON resultante. A API só empresta o resultado; não oferece extração de um
candidato próprio que escaparia dessa reserva. Cópias feitas pelo chamador são
responsabilidade de seu orçamento. Por contagem, há até 64 resultados admitidos
de até 1 MiB cada, além dos dados de entrada e trabalho transitório do engine.
Isso não é limite superior de RSS nem revisão do plano de buffers HTTP.

O prazo de 100 ms começa antes da admissão e não é renovado ao entrar na fila.
Inclui espera por espaço e espera até início da execução. Expiração antes do
início retorna write_busy e remove o comando sob o mesmo mutex que o worker usa
para iniciá-lo. Após o início, a espera não cancela commit: seu resultado pode
demorar mais de 100 ms. Descartar Ticket cancela somente se ainda estiver na fila.

Cada comando usa um canal de resposta de capacidade 1, com envio sem espera.
Não ler a resposta não bloqueia o worker, mas continua ocupando sua reserva.
Descartar ticket de comando já iniciado não libera a reserva até o worker
terminar e descartar o resultado. Lease é liberada fora do mutex da fila.

`close()`, `shutdown()` e Drop fecham admissão e devolvem shutting_down para
comandos que não começaram. O grupo iniciado termina. O sinal AtomicBool externo
também é observado; workers o conferem no máximo a cada 25 ms de espera ociosa.
Join de commit em andamento não tem prazo físico garantido. shutdown reporta
falha de join; resultados de execução são comunicados por comando.

Erros individuais não interrompem as mutações seguintes. Falha de grupo no
storage fecha admissão e não gera sucesso; falhas de commit são conservadoramente
incertas. Pânico fecha a fila e libera canais/reservas; perda de resposta não
deve ser interpretada como rollback garantido. Não há retry automático.
O módulo não publica eventos, registra payloads ou introduz configuração pública.

## Verificação e próxima etapa

Onze testes novos verificam barreiras de todo CRUD, drop/recreate, geração de
IDs, erros individuais, 32 produtores concorrentes, resultados próprios, FIFO
de espera por vagas, capacidade alocada, respostas retidas, timeout, cancelamento,
encerramento/sinal externo, desconexão em execução e pânico do worker.

```bash
cargo test -p kiarni-server writer::tests --offline
cargo test --workspace --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
python3 scripts/check_product_dependencies.py
```

## Composição HTTP e plano de buffers

O handler usa `submit_with_permit`, compartilhando a vaga original de conexão
por Arc com a Lease do comando/Reply. Essa vaga acompanha o trabalho mesmo que
o handler seja descartado. O recibo permanece vivo enquanto o handler copia e
envia a resposta, junto da reserva de transporte. No shutdown, a fila é fechada
antes do join dos handlers; o writer termina antes de liberar a raiz.

Corpos PUT/PATCH/POST são transferidos do buffer de leitura para o comando.
Respostas de até 4 KiB usam um buffer pequeno para cabeçalhos/corpo e write;
corpos maiores usam write_vectored sem uma cópia adicional do payload no wire.
Envios parciais avançam os slices sob um prazo único. Status, Content-Length,
Location e request ID são preservados.

`kiarni_server::buffer_plan_bytes` confere o perfil real pelo teto do formato
de 1 MiB, mesmo após reduzir o limite de entrada. Sejam N=max_requests,
R=max_result_bytes, Q=max_query_bytes, S=max_streams e F=feed_queue:

```text
antigo = N*(4 MiB + R + 2Q) + S*(2F MiB + 3 MiB) + 8 MiB + 32 MiB
novo   = N*(3 MiB + R + 2Q + 1 KiB) + S*(2F MiB + 3 MiB)
         + 8 MiB + 32 MiB + 768 KiB + 64 KiB
```

O slot removido de cópia integral financia a stack de 768 KiB, metadados fixos
de 64 KiB e 1 KiB por request. Mesmo com N=1, 832 KiB + N KiB <= N MiB.
O limite antigo de 512 MiB continua verificado, sem ampliar configurações ou
invalidar combinações antes aceitas. Os 3 MiB por request cobrem corpos, estado
transitório e resultados em fases distintas; a resposta retida tem até 1 MiB.
As cópias pequenas não alteram o pico dos corpos grandes. Cache e scratch de
query permanecem. É planejamento de buffers explícitos, não teto de RSS.

Testes adicionais verificam a desigualdade em N=1..128, PUT/PATCH com JSON na
profundidade máxima e próximo de 1 MiB na stack configurada, permissão de
transporte após descarte do handler e 32 clientes TCP com resultados próprios,
consultas concorrentes, restart e drop/recreate. Investigação de CPU/RSS fica
para sua etapa. O feature de profiling ainda mede fases de storage do caminho
individual; seus contadores de commit não descrevem o caminho de grupos.
No caminho HTTP atual, put_admission mede envio/admissão na fila, não o commit.

## Publicação pós-commit

O writer possui o dispatcher. Reserva sequência antes da mutação, captura estados
na transação e publica somente após confirmação, antes da próxima operação e dos
recibos. No-op/rejeição não publica. Updates observados compartilham o JSON after
com o recibo; sem assinantes, não há captura de estados. Insert e delete também
publicam, enquanto catálogo funciona como barreira. Drop de coleção interrompe
suas assinaturas e recriar o nome exige reinscrição.

Falha fatal, pânico ou falha entre commit/publicação interrompe observadores;
dado confirmado não é desfeito. O teste com pausa após commit prova a fronteira
e o teste de falha nesse ponto prova o resultado incerto, sem falso sucesso.
Shutdown termina o grupo iniciado e fecha feeds. A rota SSE está ativa com testes de streams reais.

## Transporte SSE

GET /v1/collections/{collection}/changes exige Bearer e Accept text/event-stream.
A inscrição usa o mesmo writer como barreira antes de enviar ready. Last-Event-ID
é recusado e não existe snapshot/replay. O stream usa o worker da conexão, sem
spawn extra; a permissão exclusiva promove a conexão antes de compartilhá-la
com o comando. Limites geral/stream e handle do dispatcher duram até descarte.

Sessão é revalidada antes de eventos e a cada espera de até 25 ms. recv_timeout
acorda imediatamente com eventos; o intervalo de 25 ms serve para observar auth,
shutdown e cancelamento. Heartbeat e deadline usam a configuração existente.
Frames são enviados em pedaços de 8 KiB sob um prazo absoluto e sem frame inteiro
materializado. Um evento em processamento retém até 2 MiB mais buffer/metadados,
abaixo dos 3 MiB de reserva por stream; filas mantêm sua reserva separada.

Overflow e terminais têm prioridade. Erro durante frame parcial fecha a conexão
sem anexar um falso frame terminal; EOF é interrupção para o cliente. Expiração,
drop de coleção e shutdown encerram streams. Terminal é best-effort, com tentativa
limitada a 25 ms. Consumidor lento não bloqueia o writer. Configuração pública,
limites de documento, formato persistido e bind loopback não foram alterados.
[Roteiro local](../../docs/ALPHA_LOCAL.md).

## Métricas de feeds

GET/HEAD `/v1/metrics` usa a mesma autenticação e retorna Prometheus texto 0.0.4.
Um handle somente-leitura evita aguardar comandos ou commits do writer. Gauges
de assinaturas/fila, contadores de publicação/enqueue/encerramento e histograma
de confirmação→fanout usam memória e rótulos fixos, sem payloads. A referência
de tempo começa imediatamente após o retorno do storage. Não mede entrega ao
cliente. [Semântica completa](../../docs/CHANGEFEED_METRICS.md).
