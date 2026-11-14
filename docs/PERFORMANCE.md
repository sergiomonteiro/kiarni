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
e fila limitada. Ainda não há medição comparativa de seu impacto; novas matrizes
devem registrar log_level e destino de stderr. O relatório opcional KIARNI_PROFILE
agora tem flush best-effort limitado no shutdown: confira a presença/completude
antes de usar uma medição e drene stderr ou direcione para arquivo.

As [métricas de storage](./STORAGE_METRICS.md) acrescentam instrumentação fixa
de primitivas/commit também nos grupos PUT/PATCH. Elas permanecem ativas com
log_level=off e não são a feature opcional de profiling. Seu impacto ainda não
foi medido em uma nova matriz; os benchmarks anteriores mantêm a proveniência
das versões que efetivamente mediram.
