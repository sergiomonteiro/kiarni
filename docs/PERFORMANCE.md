# Desempenho como critério de engenharia

O objetivo é tornar o Kiarni competitivo em throughput útil, latência e custo
de operação no recorte single-node. Cada otimização deve vir acompanhada de
medição reproduzível e testes de correção, recuperação e limites de recursos.

## Critérios de comparação

- Contar operações confirmadas, incluindo a durabilidade contratada, e registrar
  rejeições, timeouts e escritas com resultado desconhecido.
- Publicar latências de sucesso p50/p95/p99 junto da contagem de amostras e das
  latências de falha. A espera por admissão faz parte da latência do cliente.
- Registrar dataset, padrão de acesso, conexões, versões, hashes, hardware,
  filesystem, duração, repetições e modelo de geração de carga.
- Comparar configurações com as mesmas garantias de commit e recuperação.
  Expor diferenças de conexão, cache, índices, payload e autenticação.

## Incrementos de investigação

1. Remover a espera fixa de accept e medir a alteração isoladamente.
2. Permitir leituras por snapshot e usar admissão limitada e FIFO para operações
   exclusivas, verificando contenção e comportamento no limite de conexões.
3. Ampliar a matriz de carga com datasets fora do cache, tamanhos de documento
   variados, rodadas prolongadas e carga com chegadas independentes das respostas.
4. Medir CPU, alocações, criação de threads, parsing repetido, tempo de fila e
   tempo de commit. Usar o perfil para selecionar a próxima intervenção.
5. Avaliar reutilização de workers/conexões e agrupamento de commits quando os
   dados justificarem, preservando confirmação durável e publicação pós-commit.
6. Comparar alternativas com configuração equivalente e testar regressões em
   ambiente controlado antes de estabelecer metas quantitativas de release.

### Escalabilidade vertical — etapa posterior à investigação #9

Objetivo registrado: aproveitar os cores disponíveis e a memória de uma única
máquina de forma consistente, com concorrência limitada. Não confundir número
de clientes com uso eficiente de cores nem prometer crescimento linear.

Quando essa etapa for priorizada, medir o mesmo binário/dataset com conjuntos
controlados de 1, 2, 4 e mais cores disponíveis no host; registrar throughput
confirmado, p50/p95/p99, recusas, CPU por core e RSS. Separar leituras por ID,
scans, escritas e cargas mistas; incluir documentos/datasets de tamanhos
variados e estabilidade sustentada. Investigar serialização, contenção, filas,
alocações e I/O antes de escolher mudanças. Preservar budgets, durabilidade e
ordem pós-commit. Definir metas somente com baseline e ambiente reproduzível.

A #9 investiga leitura e telemetria; não conclui essa avaliação de CPU/memória.
Esta etapa permanece planejada, sem otimização ampla antecipada ou expansão
para cluster/sharding. A escalabilidade horizontal é uma avaliação distinta.

O benchmark inicial usa 256 documentos de 1 KiB em cache e clientes locais.
Ele orienta o primeiro ciclo de otimização. A avaliação de capacidade precisa
acompanhar a ampliação dessa matriz.
[Metodologia](../benches/README.md) ·
[Baseline](./benchmarks/http-baseline-2026-11-12.md)

O primeiro perfil de PUT identificou o commit Immediate como custo dominante.
Foi criado um experimento isolado de grupos pré-formados, documentado no
[ADR 0016](./adr/0016-group-commit-spike.md), ainda sem adoção no servidor.
A coleta limitada foi medida em uma
[matriz de 75 rodadas](./benchmarks/collector-2026-11-12.md), incluindo clientes
concorrentes, baixa carga, espera e rejeições. Agrupar operações já enfileiradas
sem espera deliberada apresentou ganho; uma janela fixa de 1 ms favoreceu
32 clientes e penalizou clientes isolados. A janela experimental não foi
promovida a configuração do produto.

A investigação foi ampliada com
[18 rodadas open-loop de 10 segundos](./benchmarks/collector-open-loop-2026-11-12.md),
com distribuição de admissão por 32 identidades virtuais e RSS amostrado em
processos novos. Não apareceu vantagem consistente da janela de 1 ms em alta
carga, enquanto ela elevou a latência em baixa carga. Janela zero é a candidata
preferida para desenhar a integração do agrupamento no coordenador do produto.
A integração com eventos precisa preservar a ordem e a publicação posterior
ao commit.
RSS observado não substitui orçamento de memória completo ou contagem de
alocações. Datasets fora do cache, clientes de rede reais, cargas mistas e
estabilidade por minutos continuam necessários antes de uma meta de release.

O [ADR 0017](./adr/0017-integracao-coordenador-de-escrita.md) foi integrado ao
HTTP: fila única para mutações, grupos PUT/PATCH sem espera e barreiras de CRUD.
O plano de buffers e a reserva de transporte acompanham comandos e respostas.
Na [comparação local](./benchmarks/http-writer-2026-11-13.md), PUT com 32 clientes
passou de 805 para 6873 confirmações/s, mas leitura caiu entre 4% e 6% com 8/32
clientes. A investigação dessa regressão e de aproveitamento de CPU/memória
fica registrada para a etapa posterior, sem afirmar melhora universal.

Os [logs estruturados de requisições](./REQUEST_LOGS.md) têm nível configurável
e fila limitada. A [comparação de 2026-11-26](./benchmarks/telemetry-http-2026-11-26.md)
registra off/info e leitores de métricas/erros no produto atual, com custos
observados e limites de interpretação; novas matrizes devem registrar log_level
e destino de stderr. O relatório opcional KIARNI_PROFILE
agora tem flush best-effort limitado no shutdown: confira a presença/completude
antes de usar uma medição e drene stderr ou direcione para arquivo.

As [métricas de storage](./STORAGE_METRICS.md) acrescentam instrumentação fixa
de primitivas/commit também nos grupos PUT/PATCH. Elas permanecem ativas com
log_level=off e não são a feature opcional de profiling. A matriz de 2026-11-26 mantém essa instrumentação ativa nos dois modos:
não isola seu custo contra um servidor sem métricas. Os benchmarks anteriores
mantêm a proveniência das versões que efetivamente mediram.

## Índice secundário no HTTP — 2026-11-22

[Relatório e três matrizes brutas](./benchmarks/secondary-index-http-2026-11-22.md):
270 rodadas válidas no mesmo binário, sem/com índice opcional, com custo de
escrita, rejeições e estado após reinício. Tags espalhadas, 4.096 × 1 KiB,
um cliente: throughput pareado 53,32× para um candidato e 4,42× em N/8;
alterar o campo indexado reduziu throughput em 14,4%. São medidas locais de
closed-loop, sem perfil CPU/RSS ou capacidade universal. Mantido o limiar
conservador; sua optimalidade fora dos casos medidos não foi demonstrada.

## Leitura e telemetria — primeiro incremento da #9

[Relatório, tentativas abortadas e dados brutos](./benchmarks/telemetry-http-2026-11-26.md):
64 rodadas válidas, quatro repetições, 256 × 1 KiB e clientes 1/8. Em oito
clientes sem leitores, info/off apresentou razões medianas de throughput de
0,9847 para GET 200 e 0,9722 para GET 404 esperado. Com um cliente houve inversão
do sinal; não se atribuiu ganho intrínseco aos logs. Leitor de métricas/erros
a 2 Hz por endpoint não mostrou perda consistente em todos os pares.

A coleta expôs limites do próprio experimento: portas diferentes por instalação
acumulavam TIME_WAIT até o teto do host. Reutilizar uma porta entre instalações
independentes permitiu completar a matriz sem novo overflow observado, mantendo
nova conexão por request e sem alterar o kernel. Isso não prova a causa dos
timeouts anteriores nem resolve a regressão histórica do writer. Esta continua
revisitada posteriormente com os dois executáveis exatos, conforme seção
seguinte; nenhuma otimização Rust foi escolhida nesse primeiro incremento.

## Revisita histórica da leitura — segundo incremento da #9

O [executável integrado foi recuperado por hash](./benchmarks/writer-read-revisit-2026-11-26.md)
no cache do Cargo. 48 rodadas intercaladas e válidas, com controles A/A,
1/8/32 clientes e mesmo dataset inicial: integrado/anterior teve medianas
pareadas de 0,9920 e 0,9943 em 8/32 clientes, com dispersão importante inclusive
entre rodadas do mesmo binário. A queda antiga de 4%–6% não foi confirmada
consistentemente; não se provou ausência de regressão ou causa no writer.

Não alterar GET/coordenação para corrigir esse percentual sem isolá-lo. O
próximo recorte é reduzir interferência do experimento e aumentar a janela de
observação, mantendo rastreabilidade e o histórico. Binários exatos estão
preservados localmente fora de target; não são release nem artefatos remotos.
A #9 continua aberta. CPU/RAM ampla permanece na etapa de escalabilidade vertical.

## Coleta longa e separação do gerador — terceiro incremento da #9

[32 rodadas de dez segundos](./benchmarks/writer-read-pinned-2026-11-27.md),
8,03 milhões de leituras corretas, conjuntos de quatro cores físicos separados
para clientes e servidor. As máscaras são temporárias nos processos do teste;
não há alteração da afinidade padrão ou implementação de escala por cores.

Medianas integrado/anterior próximas de 1 (0,99995 / 1,00485 para 8/32 clientes).
Não se confirmou queda consistente de 4%–6%. Alguns controles ficaram menos
dispersos; outros ainda oscilaram. Mudaram afinidade, duração e warmup juntos,
portanto não se isolou a contribuição de cada um.

O gerador com 32 clientes usou cerca de 7,94 CPUs lógicas-equivalentes das oito
atribuídas a ele. Esse sinal pode limitar a carga oferecida e mascarar diferenças
no servidor. Próximo recorte: gerador de benchmark mais leve e limitado, com
validação equivalente e controle A/A, antes de comparar servidores. Não repetir
o mesmo teste Python indefinidamente nem inferir capacidade do Kiarni a partir
desse teto. A #9 e o planejamento amplo de CPU/RAM continuam separados.
