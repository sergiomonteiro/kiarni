# Coleta concorrente — experimento do M1

Este crate investiga a amortização de commits Immediate para RF-02 e prepara
a avaliação das invariantes de RF-05. Não é dependência do servidor.
O [ADR 0016](../../docs/adr/0016-group-commit-spike.md) continua em experimento.

`collector::Collector::start` recebe o store exclusivo do spike e uma `Policy`.
Ele cria **um worker**. `client()` fornece handles clonáveis para produtores.
`Client::try_submit` recebe ID, corpo emprestado e operação Replace ou MergePatch;
copia os dados somente depois de verificar os limites de admissão. Não espera
por espaço: devolve `Full` quando o orçamento estiver ocupado. O mutex serializa
a admissão e a cópia; não é uma interface lock-free.

| Limite | Padrão do experimento | Máximo aceito |
|---|---:|---:|
| Operações admitidas ainda em execução ou espera | 64 | 256 |
| Bytes próprios de IDs e corpos ainda em execução ou espera | 8 MiB | 16 MiB |
| Operações por grupo | 32 | 32 |
| Bytes de corpos por grupo | 8 MiB | 8 MiB |
| Janela de coleta | 200 µs | 10 ms |
| Corpo por operação | 1 MiB | 1 MiB |

Esses padrões são hipóteses do spike, sem alteração na configuração do produto.
Zero é aceito somente para a janela, permitindo avaliar coleta sem espera.
Um corpo que nunca caberia nos limites é recusado como `TooLarge`; ID inválido
é recusado antes da cópia. JSON e regras de mutação são validados pelo engine
durante a execução, com rejeição individual documentada no experimento original.

## Ordem, tempo e memória

As admissões recebem uma sequência local crescente. Ela determina a ordem FIFO,
inclusive entre alterações no mesmo ID. Não é cursor de replay nem número
persistido. Um grupo consome somente um prefixo da fila que cabe nos limites;
não ultrapassa uma operação grande para buscar outras menores.

A janela termina no instante da primeira admissão do grupo mais a duração
configurada. Novas chegadas não renovam esse prazo. Tempo já gasto na fila
consome a janela. Ao alcançar o limite de operações ou o próximo corpo não
caber, a coleta termina. Essa regra limita a espera voluntária de coleta;
escalonamento do SO, fila e commit podem aumentar a latência total.

O orçamento de operações e bytes inclui o grupo retirado da fila e em execução.
Corpos e IDs são liberados antes de devolver espaço à admissão. Metadados de
fila, canais e respostas têm quantidade limitada pelas operações admitidas;
respostas já entregues pertencem ao chamador. Essa contabilidade não limita
RSS total: buffers do engine, transformação, cache redb e entradas externas do
chamador são custos adicionais. Estados de changefeed ainda não são retidos.

## Confirmação, falhas e encerramento

Cada admissão devolve um `Ticket`; `wait()` recebe seu resultado individual.
Sucesso chega somente após commit Immediate confirmado, ou aborto de grupo
sem alterações. O campo `committed` indica se o grupo teve commit, enquanto
`outcome` distingue Changed, NoOp e Rejected. Uma resposta por operação usa canal
de capacidade 1 e envio sem espera, para que clientes que não leem a resposta
não detenham o worker. Descartar um ticket não cancela sua mutação.

Uma falha transacional fecha a admissão: o grupo recebe `Transaction(error)` e
operações restantes recebem `NotExecuted`. `CommitUncertain` não vira sucesso.
Pânico do worker fecha a admissão e solta os canais; tickets sem resposta recebem
`WorkerLost`, que também não deve ser interpretado como rollback garantido.
Não há retry automático.

`shutdown()` e Drop fecham a admissão, acordam o worker, drenam as operações
aceitas sem aguardar janela adicional e fazem join. Clones de Client sobreviventes
recusam novas operações. O join não tem timeout: um commit em andamento depende
do armazenamento. Erros de execução são comunicados pelos tickets; o retorno
de shutdown acusa falha de join, não agrega os resultados das mutações.

## Verificação e próximo incremento

```bash
cargo test -p kiarni-group-commit-spike --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
```

Oito testes novos verificam orçamento de operações e bytes incluindo grupo
ativo, confirmação posterior ao commit, divisão de grupos por bytes e quantidade,
16 produtores concorrentes com ordem por admissão, conclusão de operação isolada,
encerramento drenando pendências, rejeições/no-op, ticket descartado, rollback,
resultado incerto e pânico do worker. Somam-se aos quatro testes anteriores,
incluindo kill/restart. O worker de crash ignorado é invocado pelo teste pai.

O executável `kiarni-group-commit-spike` continua medindo **grupos pré-formados**
com `Store::apply`; seus relatórios anteriores não medem este coletor.
O novo executável `collector_bench` mede chamadas concorrentes ao coletor,
com percentis de latência completa, fila/coleta e serviço, rejeições de admissão
e tamanho médio dos grupos. Os tempos acrescentados às confirmações são locais
ao experimento. Não constituem métricas da API nem cursor durável.

```bash
cargo build -p kiarni-group-commit-spike --bin collector_bench --release --locked --offline
python3 benches/group_commit.py --collector --output /tmp/collector.json
```

A metodologia e os limites estão no [README dos benchmarks](../../benches/README.md).
CRUD completo, eventos e integração no servidor permanecem fora desta entrega.

O executável `collector_open_loop` acrescenta chegadas uniformes independentes
das confirmações, 32 identidades virtuais, contabilização explícita de atrasos do
gerador e observação de RSS em processo novo por rodada. `Ticket::try_receive`
permite polling sem criar um waiter por operação. O gerador limita tickets a 256,
e o coletor mantém sua política própria de 64 operações. Leia a metodologia antes
de interpretar essas taxas como carga efetivamente oferecida.
