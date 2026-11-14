# ADR 0016 — Avaliação de agrupamento de commits

- Status: proposta em experimento.
- Marco: investigação de desempenho do M1, RF-02 e preparação para RF-05.

## Hipótese

O perfil do PUT registrou aproximadamente 1,09 ms no commit, 45 µs na
transformação e 18 µs na validação HTTP. O rastreamento de chamadas de escrita
concentrou 87% do tempo selecionado em fdatasync. Agrupar mutações pode
amortizar a sincronização preservando Durability::Immediate.

## Experimento

Criar um store exclusivo do spike, com layout de tabelas semelhante ao produto,
cache de 8 MiB e marcador próprio antes de abrir o engine. Comparar grupos
pré-formados de 1, 8 e 32 substituições, com commit Immediate por grupo.
Limitar cada grupo a 32 operações e 8 MiB de corpos, mantendo o limite de
1 MiB por documento. Usar a transformação do engine do produto.

Validar cada operação antes de alterar sua entrada. Rejeição de documento
inválido ou ausente preserva as outras operações válidas do grupo. Operações
no mesmo ID seguem a ordem de entrada. Um grupo sem alteração aborta a
transação. O recibo só retorna após commit confirmado ou aborto do no-op.
Falha durante commit retorna resultado incerto e impede confirmação ao cliente.

Testar rollback anterior ao commit, leitura de snapshot durante staging,
rejeição individual, ID repetido no grupo, no-op, limites e kill/restart antes
e após commit. O retorno de recibos e a publicação futura dependem da
confirmação do grupo inteiro. Eventos devem preservar a ordem das mutações.

## Limites e decisão pendente

O benchmark inicial mede o potencial de amortização com grupos pré-formados.
O coletor concorrente foi implementado separadamente no mesmo spike, com
admissão limitada, FIFO, orçamento incluindo operações em execução e janela
ancorada na primeira admissão. Sua política e testes estão no
[README do spike](../../spikes/group-commit/README.md).
Medir a latência de espera, validar CRUD completo e retenção dos estados de
changefeed continuam necessários antes de integrar o servidor.
O orçamento de memória dessa integração deve incluir corpos, estados e recibos.
Permanecem os contratos de entrega best-effort e recuperação testada de processo.

A adoção no produto depende dos resultados, de uma política limitada de coleta
e da integração correta com o coordenador de eventos. Os dados do experimento
ficam separados do formato persistido aceito no ADR 0011.

## Evidência local

Os quatro testes de contrato verificam rejeição individual, ordem no mesmo ID,
snapshot anterior ao commit, no-op, limites, rollback injetado, resultado incerto
após confirmação, recusa de raiz estrangeira e kill/restart nos dois lados do
commit. O quinto teste é somente o worker ignorado, executado pelos subprocessos
do teste de crash. Isso demonstra recuperação de processo, sem simular queda de
energia ou falha física de disco. Não há integração com eventos neste spike.

Resultados e reprodução: [perfil e agrupamento](../benchmarks/group-commit-2026-11-12.md).

A etapa de coleta acrescenta oito testes, mantendo o total deste crate em doze
testes ativos e um worker de fixture ignorado. Eles cobrem múltiplos produtores,
limites, drenagem no encerramento e falhas do worker. Os valores padrão de
coleta são hipóteses locais e não constituem decisão de produção. A medição
anterior continua referente aos grupos pré-formados.

Uma matriz posterior de 75 rodadas mediu o coletor com 1/8/32/128 produtores,
um cenário de baixa carga e cinco políticas de agrupamento. Em 32 clientes,
janela zero já apresentou ganho relevante; 1 ms favoreceu essa concorrência,
mas penalizou clientes isolados. O cenário de 128 produtores teve alta taxa de
recusa pelo limite de 64 admissões. As medições não justificam adotar uma janela
fixa universal. O executável de medição adiciona um teste de percentis e tamanho
do dataset: o crate passa a ter treze testes ativos, além do worker ignorado.
[Resultados do coletor](../benchmarks/collector-2026-11-12.md).
Status continua em experimento, sem mudança no coordenador do produto.

Uma matriz open-loop de 18 rodadas de 10 segundos comparou janelas de zero e
1 ms sob taxas de 200/5000/15000 chegadas/s. Foram 1183734 confirmações,
28046 recusas Full e 220 chegadas perdidas pelo gerador, sem falhas de execução.
As verificações após reabertura passaram. O ganho de throughput da janela de
1 ms não foi consistente; em baixa carga ela acrescentou espera com grupos
unitários. Janela zero passa a ser a candidata preferida neste recorte.
Distribuição de admissão e RSS foram observados, com os limites metodológicos
descritos no [relatório open-loop](../benchmarks/collector-open-loop-2026-11-12.md).
Dois testes no novo executável verificam a medição, elevando o total do crate
a quinze testes ativos, além do worker de fixture ignorado.

A aceitação em produção continua pendente do desenho e dos testes de integração
com o coordenador de CRUD, limites e confirmação. A primeira execução open-loop
foi substituída após correção da cadência de polling do gerador, ficando
preservada apenas como diagnóstico.

O desenho de integração está na proposta do
[ADR 0017](./0017-integracao-coordenador-de-escrita.md). Ele distingue as garantias
do spike das exigências do produto: JSON por operação, barreiras de CRUD,
espera de até 100 ms, cancelamento antes do início no shutdown e plano de buffers.
Este ADR permanece como registro experimental; a existência da proposta não
significa que o servidor já agrupa commits.
