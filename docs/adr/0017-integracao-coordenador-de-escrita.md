# ADR 0017 — Integração do agrupamento no coordenador de escrita

- Status: aceita para integração no Linux local, com regressão de leitura registrada; validação ampliada pendente.
- Marco: evolução do M1, RF-02; invariantes de RF-04/05 preservadas.
- Base: ADRs 0004, 0010, 0011, 0012, 0014, 0015 e experimento 0016.
- Evolui a coordenação de escrita do ADR 0015; preserva seu gate de query e contratos de prazo.

## Problema e evidência

O servidor atual serializa mutações no gate FIFO de escrita. PUT/PATCH chamam
`Store::update_document`, que faz lookup, transformação e commit Immediate por
operação. Inserções, exclusões e alterações do catálogo também usam o gate.
As leituras usam snapshots independentes. O HTTP retorna o JSON resultante de
cada PUT/PATCH, mesmo quando a transformação é um no-op.

O [ADR 0016](./0016-group-commit-spike.md) demonstrou potencial de amortização
de commits. Na [matriz de chegadas independentes](../benchmarks/collector-open-loop-2026-11-12.md),
a janela zero confirmou aproximadamente 14,5 mil operações/s em alta carga do
experimento, com recusas explícitas. Esperar 1 ms não trouxe ganho consistente
e elevou o p95 em baixa carga de 2,72 para 3,85 ms. Esses resultados são do
spike, não do servidor HTTP.

## Decisão de integração

Um coordenador por instância serializa todas as mutações. Um worker dedicado
executa comandos tipados numa fila FIFO limitada. Não há espera deliberada para
coletar: ao terminar o trabalho anterior, o worker retira o prefixo elegível já
enfileirado. Se houver apenas uma operação, inicia seu processamento imediatamente.
Não há nova janela configurável ou endpoint de transação multi-documento.

### Fronteiras dos componentes

- `kiarni-engine` continua responsável pela transformação pura e semântica JSON.
- `kiarni-storage` recebe uma operação interna de atualização em grupo, sobre
  as tabelas do formato aceito. Resolve coleção e documento, transforma e aplica
  na mesma transação. Não conhece HTTP, canais ou política de fila.
- `kiarni-server` possui comandos, admissão, worker, prazos, encerramento e
  entrega dos recibos. Mapeia resultados para os códigos HTTP existentes.
- Nenhum crate de produto passa a depender de `spikes/group-commit`. Os tipos
  experimentais e seu marcador de store não são reutilizados como formato real.

### Elegibilidade e barreiras

| Comando | Execução no primeiro recorte |
|---|---|
| PUT de documento existente | Elegível para grupo |
| PATCH de documento existente | Elegível para o mesmo grupo de PUT |
| Inserção explícita ou com ID gerado | Barreira, operação isolada |
| Exclusão de documento | Barreira, operação isolada |
| Criação de coleção | Barreira, operação isolada |
| Exclusão de coleção vazia | Barreira, operação isolada |

Um grupo contém até 32 atualizações consecutivas e 8 MiB de corpos. Pode abranger
coleções distintas. O primeiro comando que não cabe ou não é elegível encerra
o grupo; ninguém o ultrapassa. O worker executa a barreira antes de retirar
o próximo grupo. A política não adiciona índices secundários.

Inserção conserva até oito tentativas de geração em caso de colisão, dentro
de sua vez exclusiva, e não repete falhas de commit. Exclusão de coleção
conserva a regra de coleção vazia. O formato persistido e os códigos HTTP
de sucesso/erro existentes permanecem os mesmos.

Todas as rotas mutantes migram para o coordenador quando ele for ativado.
Manter PUT/PATCH numa fila e outras mutações no gate independente permitiria
inversões; essa combinação é vedada. Gate de query e leituras por snapshot
continuam independentes. A fila começa após autenticação, validação do envelope
e leitura/validação do corpo; ordem de TCP ou chegada de cabeçalhos não é ordem
de mutação. A checagem temporária do writer antes de ler o corpo não constitui
admissão e não pode reservar o worker enquanto um cliente envia dados lentamente.

### Ordem e resultado individual

A inserção na fila sob exclusão mútua define a ordem local de admissão. Ela não
é um cursor persistido ou contrato de replay. Quando o grupo inicia, resolve a
coleção novamente no storage; não confia em handle obtido antes de um possível
drop/recreate. Metadados, documento e índice primário permanecem consistentes.

Dentro da transação, cada atualização lê o estado produzido pelas anteriores.
Operações no mesmo ID seguem essa ordem. Cada recibo guarda seu próprio JSON
resultante, e não consulta o documento novamente após o commit.
Exemplo: dois PATCHs para `v=1` e depois `v=2` no mesmo documento retornam,
respectivamente, seus estados `v=1` e `v=2`, mesmo que compartilhem o commit.
O snapshot visível a outras conexões só mostra o grupo confirmado inteiro.
Não se promete visibilidade externa dos estados intermediários do grupo.

Erros de domínio, como coleção/documento ausente, ID imutável ou documento
inválido, rejeitam somente aquela operação antes de alterar sua entrada. Outras
operações válidas do grupo podem confirmar. No-op conserva seu JSON de resposta.
Se não houver mudanças válidas, a transação aborta e os resultados saem após
o aborto. Se houver mudanças, nenhum resultado do grupo é entregue antes da
conclusão do commit, inclusive rejeições individuais e no-ops.

Falha de engine, layout ou integridade é falha do grupo inteiro, não rejeição
individual. Antes do commit, aborta sem confirmações. Falha durante commit é
conservadoramente resultado incerto: nunca afirmar rollback só pelo erro.
O coordenador fecha a admissão de escritas e devolve erros existentes, sem retry
automático ou sucesso. Comandos ainda não executados são encerrados como não
executados, sem invocar storage. O mesmo fechamento se aplica a pânico/perda
do worker. Uma resposta 503 de storage não é promessa de que nada foi persistido.
Não se introduz código público novo para esse caso neste recorte.

### Admissão, prazos e memória

Limites propostos, sujeitos à verificação do plano de buffers antes da ativação:

| Recurso | Política de integração |
|---|---|
| Comandos admitidos não liberados, ativos ou aguardando | `min(max_requests, 64)` |
| IDs, nomes e corpos próprios desses comandos | Até 8 MiB |
| Atualizações por grupo | Até 32 |
| Corpos por grupo | Até 8 MiB |
| Corpo/documento por operação | Limite configurado, até 1 MiB do formato |
| Espera deliberada para formar grupo | Zero |
| Espera para obter orçamento ou iniciar execução | Até 100 ms, sem renovar prazo |
| Recibo por comando | Canal de capacidade 1, envio sem esperar leitor |

O orçamento inclui o grupo ativo. Retirar comandos da fila não libera bytes.
Um request validado que espera orçamento continua limitado pelas vagas de
requisição e seu corpo já reservado. Espera por orçamento mantém FIFO; uma
operação menor não ultrapassa uma maior. Esgotamento de prazo antes de iniciar
execução retorna `503/write_busy`, sem executar depois.

O prazo de 100 ms é ancorado na entrada do pedido de admissão, como evolução
do gate do ADR 0015; não se reinicia ao obter bytes ou entrar na fila. A escolha
entre expirar e começar ocorre sob o mesmo estado sincronizado. Depois de
iniciada a execução, não há timeout que afirme cancelamento do commit. A espera
de uma conexão pode terminar com resultado desconhecido, sem retry automático.

Corpos são transferidos para o comando, evitando cópia integral adicional.
A reserva de memória de cada requisição acompanha comando e recibo por RAII:
desconectar o socket não libera orçamento de uma mutação ainda em execução nem
permite criar novas requisições ilimitadas enquanto o worker retém seus buffers.
Reservas só são liberadas quando os dados correspondentes deixam de existir.
Cliente que não lê sua resposta não bloqueia o worker, e seus buffers continuam
contabilizados até descarte ou envio pelo transporte limitado existente.

**Porta de implementação:** a fórmula de buffers em `apps/kiarni/src/cli.rs`
deve ser conferida antes da ativação. Ela já considera custos por request,
cache, query e feeds, mas não demonstra sozinha o custo deste coordenador.
A revisão deve explicitar propriedade de corpos, até 32 JSONs de resposta de
1 MiB, estado anterior/candidato transitório, metadados/canais e stack do worker.
Não somar buffers transferidos duas vezes ou assumir que o recibo mínimo do
spike representa o recibo HTTP. O plano continua sujeito a BUFFER_PLAN_CAP;
não elevar o teto nem tornar configurações antes válidas inválidas sem uma
decisão específica. Se o desenho não couber, reduzir a política interna e
documentar o ajuste antes de ativar, preservando os limites de entrada existentes.
Esse plano limita buffers explícitos, não constitui limite superior de RSS.

### Encerramento e desconexão

No fechamento da admissão, pedidos que ainda não começaram recebem
`shutting_down` e não são executados, preservando o ADR 0015. Um grupo marcado
como iniciado é concluído com sua confirmação durável ou falha. Essa política
difere intencionalmente do spike, que drena todos os comandos admitidos.
O worker não espera janela de coleta, socket ou consumo dos recibos para sair.
O join de um commit em andamento continua sem prazo físico garantido.

Desconexão anterior ao início pode cancelar o comando sob a mesma transição
sincronizada usada para timeout, se já detectada. Após o início, não cancela
alterações do grupo. Uma detecção tardia não é promessa de cancelamento.
Não abandonar silenciosamente comandos com memória ou canais ainda retidos.

### Eventos e índices futuros

Este recorte não ativa changefeeds. Reserva a fronteira de ordenação: após
commit confirmado, os futuros eventos são construídos e enfileirados na ordem
das operações válidas que mudaram estado, antes de iniciar o próximo grupo.
No-ops e rejeições não geram evento. Para o mesmo ID, preservar transições
intermediárias caso eventos sejam implementados; ler somente o estado final
não seria suficiente. Os buffers anterior/novo e filas de eventos precisam de
orçamento próprio e testes antes de RF-05 entrar no coordenador.
Não antecipar agora essa retenção nem prometer replay/exactly-once.

## Incrementos e critérios de aceitação

1. **Storage:** introduzir a operação interna de grupo no crate real, ainda sem
   uso pelo HTTP. Verificar JSON individual, PUT/PATCH misturados, mesmo ID,
   coleções distintas, no-op, rejeição individual, rollback, erro de commit e
   kill/restart antes/depois da confirmação. Sem formato experimental em dados reais.
2. **Coordenador:** comandos tipados para todas as mutações, FIFO único, limites,
   reservas RAII e cancelamento antes do início. Testar barreiras, bytes contando
   grupo ativo/recibos, timeout versus início, pânico, encerramento e cliente
   desconectado. Conferir o plano de buffers antes de ativar.
3. **HTTP:** ativar o mesmo coordenador em todas as rotas mutantes num incremento
   revisável, preservando códigos, JSONs e headers. Testes black-box com clientes
   independentes: PUT/PATCH no mesmo ID, insert/delete entre updates, drop/recreate,
   erros individuais, leitura/query durante escrita e shutdown.
4. **Desempenho:** comparar binários HTTP anterior e novo na mesma matriz e
   durabilidade, com hashes, rejeições, latências e resultados desconhecidos.
   Investigar regressões de baixa carga e carga mista antes de aceitar o ADR.

Cada incremento executa fmt, clippy e testes aplicáveis. Os testes existentes
continuam sendo a referência do contrato HTTP; resultados dos spikes não os
substituem. Datasets fora do cache e testes de estabilidade por minutos seguem
pendentes e devem acompanhar a avaliação de capacidade, sem declarar liderança.

## Alternativas e limites

- **Manter commit por operação:** referência de correção e comparação; sem ganho
  de amortização, mas é o comportamento vigente até a integração passar.
- **Janela fixa de 1 ms:** rejeitada como padrão proposto por não apresentar
  ganho consistente no open-loop e aumentar a latência de grupos unitários.
- **Agrupar todo CRUD já no primeiro recorte:** amplia erros, geração de IDs e
  catálogo na mesma transação; barreiras reduzem a superfície inicial.
- **Fila independente só para updates:** rejeitada por quebrar a ordem global
  com inserções, exclusões e operações de coleção.

O agrupamento aumenta o conjunto de operações afetadas por uma falha de commit,
retém respostas por grupo e elimina visibilidade de estados intermediários para
leitores externos. São efeitos internos propostos, que exigem testes e medição.
Não há alteração de licença, protocolo, formato persistido ou requisito do MVP.

## Estado do incremento de storage

O primeiro incremento foi implementado em `crates/storage/src/updates.rs`, com
`Store::update_group`. Usa manifesto e tabelas reais, sem dependência do spike.
O método de documento único permanece disponível; a integração HTTP usa o grupo
por meio do coordenador. Seus resultados de desempenho estão na validação abaixo.

A primitiva limita grupos, resolve coleções na transação, guarda o JSON de cada
resultado, separa rejeição individual de falha fatal e mantém commit Immediate.
Confere schema/metadados sem recriar tabelas ausentes. Dez testes novos verificam
as invariantes, incluindo concorrência, rollback e kill/restart antes e depois
da confirmação em duas coleções. O worker de fixture é ignorado na execução
direta e invocado pelo teste pai.

Passaram fmt, clippy e 186 testes do workspace, com dois workers de fixture
ignorados. Clippy também passou com `kiarni-server/profiling`. Falhas de processo
e hooks privados foram exercitados; EIO, disco cheio e falha física de commit
não foram simulados. [Contrato e verificações](../../crates/storage/README.md).

## Estado do incremento de coordenador

Implementado em `crates/server/src/writer.rs` e integrado ao HTTP. Todos
os comandos mutantes passam pela mesma fila nos testes do módulo: updates
adjacentes são agrupados e os demais comandos funcionam como barreiras.
O worker tem stack explícita de 768 KiB; não usa crates experimentais.

Reservas acompanham comando/Reply, com orçamento de requisições, capacidades
de entrada e slots admitidos. O prazo de 100 ms não é renovado. Expiração,
cancelamento e shutdown não executam comandos ainda na fila, enquanto grupos
iniciados concluem sua transação. Respostas usam canais de capacidade 1 e envio
sem espera. Falha fatal de grupo/pânico fecha a admissão.

Onze testes novos cobrem todo CRUD, barreiras, drop/recreate, geração de IDs,
32 produtores, resultados individuais, FIFO por orçamento, reservas de respostas,
timeout, desconexão, sinal externo e pânico. Passaram fmt, clippy, fronteiras de
dependências e 197 testes do workspace, com dois workers de fixture ignorados.
[Contrato do coordenador](../../crates/server/README.md).

## Ativação HTTP e validação local — 2026-11-13

Todas as rotas mutantes agora usam a mesma instância do coordenador. O HTTP
transfere os corpos para os comandos, preserva JSON por operação, códigos,
headers e request IDs. O gate antigo permanece para queries, não para escrita.
Shutdown fecha a fila antes de aguardar handlers e worker, mantendo a raiz
ocupada até o término dos commits iniciados.

`submit_with_permit` compartilha a permissão de conexão com a Lease. O recibo
é retido até o envio, protegendo também suas cópias HTTP. Se o handler for
descartado após início, a permissão permanece com o comando até liberação dos
buffers. Não ler o socket não bloqueia o worker e não cria novas reservas.

O envio de corpos grandes usa write_vectored, sem cópia integral de wire;
respostas de até 4 KiB mantêm um buffer pequeno com write comum. A stack do
worker foi reduzida de 2 MiB para 768 KiB e testada com profundidade JSON máxima
e corpo próximo de 1 MiB em PUT/PATCH. O plano de buffers inclui stack, 64 KiB
fixos e 1 KiB por request, financiados pela cópia removida. O novo plano é menor
que o anterior mesmo com uma conexão; o teto de 512 MiB e a validade das
configurações anteriores permanecem. [Fórmula](../../crates/server/README.md).

Passaram 201 testes com e sem profiling, fmt, clippy e fronteiras de dependências.
O teste black-box usa 32 clientes sobre o mesmo ID, verifica o JSON individual,
snapshots concorrentes, HEAD, restart, exclusão e recriação da coleção. O teste
de permissão confirma retenção do slot após descarte do handler em execução.

Na matriz HTTP local, 32 clientes passaram de 805 para 6873 PUT/s, com p95 de
44,3 para 6,5 ms; mixed passou de 4302 para 15164 operações/s. Não houve falhas,
rejeições ou escritas desconhecidas nas duas matrizes. Porém read caiu 5,96%
com 8 clientes e 4,32% com 32. Um ajuste no envio pequeno não eliminou a queda.
A aceitação local considera o ganho de escrita e os testes de contrato, sem
declarar ganho universal; a regressão fica como pendência explícita para a
investigação posterior de CPU/memória. [Resultados](../benchmarks/http-writer-2026-11-13.md).

Não se declarou capacidade fora do cache, estabilidade prolongada, correção em
macOS por estes testes, ou proteção contra falha física/energia. O profiling
de fases individuais ainda não contabiliza commits de grupos. Changefeeds
continuam exigindo ordenação e orçamento próprios antes de ativação.

## Revisita dos executáveis exatos — 2026-11-26

[48 rodadas intercaladas e controles A/A](../benchmarks/writer-read-revisit-2026-11-26.md)
nos dois executáveis com os hashes originais. Diferenças medianas de leitura
menores que a queda inicial e dispersão importante: não se confirmou uma queda
consistente de 4%–6%, nem se demonstrou ausência de regressão. A recuperação do
artefato integrado supera a lacuna anterior de localização. Preservado o
registro histórico, sem atribuição causal ou mudança de runtime/contrato.
