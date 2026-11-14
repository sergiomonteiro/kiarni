# Coletor concorrente: latência e janelas — 2026-11-12

Investigação M1/RF-02, seguindo o ADR 0016 em experimento. O agrupamento de
operações já enfileiradas trouxe ganho expressivo sem espera deliberada. Uma
janela de 1 ms ampliou throughput em 32 clientes, mas prejudicou clientes
isolados. Estes dados não sustentam uma janela fixa universal ou adoção no
servidor. A alternativa de janela zero merece ser a referência do próximo teste.

## Ambiente e método

[Dados das 75 rodadas](./collector-2026-11-12.json). Linux 7.2.6, Ryzen 7 6800H,
16 CPUs lógicas, 15,9 GB de RAM, ext4, Rust/Cargo 1.99.1. O JSON inclui versões,
afinidade, hashes de fontes/binário e estado do workspace. Foi medido o workspace
local com alterações, sem CI remoto. Não houve builds ou testes concorrentes.

São três repetições de 2 segundos por política/carga. Cinco políticas: grupo de
uma operação com janela zero; grupos de até 32 com janelas de 0, 50, 200 e
1000 µs. Cinco cargas: 1/8/32/128 threads em closed-loop e um cliente que espera
5 ms após a resposta. A ordem das políticas gira entre repetições. Cada rodada
tem store novo, 256 documentos de 1 KiB, cache de 8 MiB e commit Immediate.
Admissão limitada a 64 operações e 8 MiB, incluindo o grupo ativo. Cada thread
altera seu próprio documento, com revisão nova por tentativa.

Seed, inicialização e verificação pós-reabertura ficam fora do tempo. Não há
warmup. A latência completa inclui construir o corpo, admissão, espera e
recepção da confirmação. O throughput inclui a conclusão das últimas operações
e pausas finais dos produtores. Rejeições `Full` têm percentis separados e um
backoff de 100 µs antes da próxima revisão; não há retry da operação recusada.

Trata-se de um benchmark dentro do mesmo processo, **sem HTTP, autenticação ou
eventos**. As taxas abaixo não são capacidade do servidor. O controle de uma
operação por commit usa o mesmo coletor instrumentado das outras políticas.

## Throughput e latência

Cada célula é a mediana entre as três rodadas. p95/p99 são medianas dos
percentis por rodada, e não percentis de amostras reunidas.

| Clientes | Grupo máximo / janela | Confirmadas/s | p95 completo | p99 completo | Média de operações/grupo |
|---|---|---:|---:|---:|---:|
| 1 | 1 / 0 µs | 843 | 2,428 ms | 2,915 ms | 1,00 |
| 1 | 32 / 0 µs | 871 | 2,379 ms | 2,900 ms | 1,00 |
| 1 | 32 / 200 µs | 702 | 2,636 ms | 2,960 ms | 1,00 |
| 1 | 32 / 1000 µs | 433 | 3,600 ms | 3,953 ms | 1,00 |
| 8 | 1 / 0 µs | 942 | 10,718 ms | 12,383 ms | 1,00 |
| 8 | 32 / 0 µs | 3312 | 3,802 ms | 4,712 ms | 4,01 |
| 8 | 32 / 50 µs | 3445 | 3,673 ms | 4,556 ms | 4,01 |
| 8 | 32 / 200 µs | 3278 | 3,691 ms | 4,591 ms | 4,02 |
| 8 | 32 / 1000 µs | 3281 | 3,719 ms | 4,345 ms | 8,00 |
| 32 | 1 / 0 µs | 901 | 39,869 ms | 41,458 ms | 1,00 |
| 32 | 32 / 0 µs | 10404 | 4,642 ms | 5,364 ms | 19,22 |
| 32 | 32 / 50 µs | 10379 | 4,569 ms | 5,338 ms | 18,85 |
| 32 | 32 / 200 µs | 10447 | 4,485 ms | 5,278 ms | 19,26 |
| 32 | 32 / 1000 µs | 12713 | 4,232 ms | 5,090 ms | 27,58 |

Sem espera deliberada, operações acumuladas durante um commit já permitem
agrupar o trabalho seguinte. Em 32 clientes, a mediana de throughput cresceu
11,5 vezes frente ao controle, enquanto o p95 caiu cerca de 88%. A janela de
1 ms acrescentou aproximadamente 22% de throughput em relação à janela zero
nessa concorrência. Com um cliente, não formou grupos maiores que um e reduziu
o throughput quase pela metade frente à janela zero.

## Baixa carga

Um cliente, com pausa de 5 ms entre operações, também não tem oportunidade de
agrupar. O throughput inclui essa pausa; a latência da operação não a inclui.

| Grupo máximo / janela | Confirmadas/s | p50 completo | p95 completo | p95 de fila/coleta |
|---|---:|---:|---:|---:|
| 1 / 0 µs | 146,2 | 1,458 ms | 3,051 ms | 0,092 ms |
| 32 / 0 µs | 146,0 | 1,458 ms | 3,020 ms | 0,102 ms |
| 32 / 50 µs | 144,9 | 1,496 ms | 3,014 ms | 0,127 ms |
| 32 / 200 µs | 141,3 | 1,700 ms | 3,283 ms | 0,311 ms |
| 32 / 1000 µs | 126,4 | 2,466 ms | 4,141 ms | 1,128 ms |

A fila/coleta mede admissão até início do processamento, sem serviço de storage.
O aumento observado é consistente com a espera configurada. Não há evidência
de benefício para uma janela fixa de 200 µs nessa carga.

## Saturação e correção

Foram confirmadas **675077 operações**, com **zero falhas de execução** e todas
as 75 verificações dos documentos dos clientes após reabertura aprovadas.
Não houve rejeições com até 32 clientes ou no cenário de pausa de 5 ms.

Em 128 clientes, acima do orçamento de 64 operações, houve **12174778 rejeições
Full**, equivalentes a **97,71% das tentativas** desse conjunto de rodadas.
São recusas explícitas de admissão, não commits confirmados nem operações de
resultado incerto. Os percentis de sucesso abaixo não incluem as recusas.

| Grupo máximo / janela | Confirmadas/s | p95 de sucesso | Rejeições nas 3 rodadas |
|---|---:|---:|---:|
| 1 / 0 µs | 952 | 71,544 ms | 2449348 |
| 32 / 0 µs | 11649 | 7,122 ms | 2428384 |
| 32 / 50 µs | 12002 | 6,613 ms | 2431351 |
| 32 / 200 µs | 12245 | 6,427 ms | 2431910 |
| 32 / 1000 µs | 12211 | 6,482 ms | 2433785 |

O gerador produz muitas novas tentativas após cada recusa, com pausa curta.
Esse padrão altera a contenção de CPU e a carga oferecida; não demonstra que
o sistema atende integralmente 128 clientes ou garante justiça na admissão.
O relatório JSON contém contagens, latências de recusa e amostras por rodada.

## Limites e próximo incremento

As rodadas são curtas, o dataset cabe no cache e há variação entre repetições.
O experimento mede substituições de documentos distintos por cliente, sem
contenção de vários clientes sobre o mesmo documento. Não foram medidos RSS,
alocações, energia, carga open-loop, CRUD completo ou retenção de eventos.
A instrumentação adiciona timestamps e metadados por grupo nas cinco políticas.

O ADR 0016 permanece em experimento; a janela padrão de 200 µs não foi promovida
a decisão de produção. O próximo incremento deve comparar janela zero com a
melhor alternativa em rodadas prolongadas e chegadas independentes das respostas,
incluindo distribuição de admissão por cliente e limites de memória observados.
A integração no servidor e a publicação de eventos continuam pendentes.

## Reprodução e verificações

```bash
cargo build -p kiarni-group-commit-spike --bin collector_bench --release --locked --offline
python3 benches/group_commit.py --collector --output /tmp/collector.json
```

Use um arquivo novo. Para rodadas mais longas, passe `--seconds 10`; para alterar
repetições, use `--repetitions 3`. Metodologia: [benchmarks](../../benches/README.md).

Passaram `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --offline -- -D warnings`,
`cargo test --workspace --offline` (174 testes e um worker de fixture ignorado),
`python3 -m unittest discover -s benches -p test_http_load.py` (3 testes) e
`python3 scripts/check_product_dependencies.py` (8 pacotes de produto).
