# Primeiro ciclo de otimização HTTP — 2026-11-12

Foram comparadas quatro versões com a mesma matriz de 36 rodadas: 1/8/32
processos clientes, três repetições, dois segundos por rodada e 256 documentos
de 1 KiB. As configurações de servidor e parâmetros de carga dos artefatos
são idênticos. O ambiente Linux, filesystem e versões estão registrados em
cada JSON. Servidor e clientes compartilham o mesmo computador.

## Experimentos

| Versão | Mudança | Confirmações | Respostas 503 | Erros de transporte | Escritas de resultado desconhecido |
|---|---|---:|---:|---:|---:|
| Baseline | sleep de accept e try_lock global | 147904 | 328976 | 0 | 0 |
| Poll | Prontidão do listener, mudança isolada | 349164 | 674161 | 0 | 0 |
| Coordenação intermediária | Snapshots e espera limitada | 399950 | 171 | 170 | 41 |
| FIFO final | Ordem de admissão, liberação de conexões e backpressure | 378550 | 0 | 0 | 0 |

As contagens acima combinam cenários diferentes e servem para conferir os
resultados das execuções. As taxas devem ser comparadas por cenário e número
de clientes. Os dados de cada versão permanecem disponíveis.
[Baseline](./http-baseline-2026-11-12.json) ·
[Poll](./http-poll-2026-11-12.json) ·
[Intermediária](./http-coordination-2026-11-12.json) ·
[Final](./http-fifo-2026-11-12.json)

Poll retirou o piso de aproximadamente 25 ms em baixa concorrência. A versão
intermediária revelou timeouts de admissão e retenção de vagas no fechamento
de conexões. A versão final usa fila FIFO com espera máxima de 100 ms e libera
o corpo já consumido antes de encerrar o worker. Quando todas as vagas estão
ocupadas, o acceptor aguarda uma conclusão de worker antes de aceitar outra
conexão. A espera usa notificação, com checagem de shutdown a cada 25 ms.
[ADR 0015](../adr/0015-coordenacao-concorrente.md)

## Comparação com o baseline

Taxas e p95 são medianas das três repetições. A proporção de 503 agrega as
tentativas das três repetições. Todos os percentis e latências de falha estão
nos arquivos JSON. A latência final inclui a espera de admissão.

| Clientes | Cenário | Baseline/s | Final/s | 503 baseline (%) | 503 final (%) | p95 final (ms) |
|---|---|---|---|---|---|---|
| 1 | read | 39.51 | 3258.20 | 0.00 | 0.00 | 0.388 |
| 1 | write | 39.51 | 578.85 | 0.00 | 0.00 | 3.063 |
| 1 | query | 39.53 | 616.94 | 0.00 | 0.00 | 2.141 |
| 1 | mixed | 39.48 | 1552.57 | 0.00 | 0.00 | 1.518 |
| 8 | read | 212.00 | 24191.58 | 32.23 | 0.00 | 0.432 |
| 8 | write | 39.12 | 786.65 | 87.50 | 0.00 | 12.085 |
| 8 | query | 39.09 | 878.79 | 87.50 | 0.00 | 10.488 |
| 8 | mixed | 179.28 | 4376.44 | 43.93 | 0.00 | 8.571 |
| 32 | read | 19623.41 | 20492.56 | 7.99 | 0.00 | 2.027 |
| 32 | write | 1030.79 | 803.34 | 94.51 | 0.00 | 43.938 |
| 32 | query | 671.98 | 910.22 | 96.40 | 0.00 | 38.396 |
| 32 | mixed | 4431.55 | 4267.15 | 81.59 | 0.00 | 37.454 |

A leitura com um cliente avançou aproximadamente 82 vezes, e p95 caiu de
25,768 ms para 0,388 ms. Com oito clientes, a mediana de leitura passou de
212 para 24.192 confirmações/s. O ensaio final confirmou todas as tentativas.

A escrita com 32 clientes caiu cerca de 22%, de 1.031 para 803 confirmações/s.
A fila atende as requisições admitidas, elevando a latência observada desses
clientes. p95 de escrita ficou em 43,938 ms e de query em 38,396 ms. Esses
resultados mantêm o caminho de escrita e a latência sob saturação como
prioridades. A variação e o custo de fairness/commit precisam de profiling.

As rodadas são curtas e o dataset cabe no cache. Hardware, frequência e carga
do host podem variar entre versões. Para leitura com 32 clientes, os intervalos
das versões se sobrepõem parcialmente; a diferença de mediana exige novas
execuções prolongadas. O cenário mixed também variou bastante no baseline.

## Correção e reprodução

As últimas escritas confirmadas ficaram visíveis após cada rodada. A validação
das queries coincidiu com o resultado esperado. A versão final passou por 161
testes Rust, incluindo FIFO, timeout, backpressure no limite de workers,
snapshots durante escrita, shutdown, concorrência e kill/restart.

```bash
cargo build -p kiarni --release --locked --offline
python3 benches/http_load.py --output docs/benchmarks/http-repro-optimized.json
```

Use uma saída nova em cada execução. Os hashes permitem identificar os
binários medidos. O workspace continua com alterações locais e as versões
medidas antecedem o versionamento em um commit de revisão.

## Próximo incremento

Medir separadamente CPU, parsing, wakeups e tempo de commit nas escritas.
Investigar agrupamento de commits em spike próprio caso o custo de sincronização
domine, preservando a confirmação durável. Ampliar o benchmark para datasets
maiores e rodadas longas, além de comparar alternativas sob garantias equivalentes.
[Plano de desempenho](../PERFORMANCE.md)
