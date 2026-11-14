# Coleta com chegadas independentes — 2026-11-12

Investigação M1/RF-02, ADR 0016 ainda em experimento. A janela zero é a candidata
preferida neste recorte: esperar 1 ms não trouxe vantagem consistente de
throughput em alta carga e acrescentou latência em baixa carga. O coordenador
do servidor permanece sem alteração; estes números não representam capacidade
da API HTTP.

## Método e ambiente

[Dados finais de 18 rodadas](./collector-open-loop-2026-11-12.json): três
repetições de 10 segundos, taxas uniformes de 200/5000/15000 chegadas/s e
janelas de 0/1000 µs. São rodadas cinco vezes mais longas que as anteriores,
mas ainda não são um teste prolongado de estabilidade.

Cada rodada usa um subprocesso novo, 32 identidades virtuais em round-robin,
256 documentos de 1 KiB, cache de 8 MiB, limite de 64 operações/8 MiB admitidas
e grupos de até 32 com commit Immediate. Cada identidade modifica seu próprio
documento. O gerador mantém no máximo 256 tickets, sem esperar confirmação
para gerar a próxima chegada. A ordem das janelas alterna entre repetições.
Não há warmup. Inicialização e verificação pós-reabertura ficam fora da medição.

Linux 7.2.6, Ryzen 7 6800H, 16 CPUs lógicas, aproximadamente 15,9 GB de RAM,
ext4, Rust/Cargo 1.99.1. Versões, afinidade, hashes de fontes/binário/runner e
estado do workspace constam do JSON. Não houve compilação ou testes simultâneos.
Foi medido o workspace local com alterações; não houve validação remota de CI.

Um único gerador atende as 32 identidades, intercalando admissões e polling.
Sono solicitado de no máximo 50 µs e spin/polling no trecho final reduzem o
atraso de observação, com custo de CPU. O SO pode alongar o sono. São clientes
virtuais no mesmo processo, sem rede, autenticação ou changefeeds.

Chegadas perdidas por atraso do gerador são contadas, sem rajada de compensação
ou retry. Não se confundem com `Full` do coletor. O throughput divide confirmações
pelo tempo efetivo incluindo drenagem. A latência abaixo vai do horário previsto
de chegada até a observação da confirmação, incluindo atrasos do gerador.
O JSON também separa latência desde o envio real e fila/coleta.

## Throughput e latência de sucesso

Valores são medianas de três rodadas. Percentis são medianas dos percentis
nearest-rank de cada rodada, não percentis de todas as amostras reunidas.

| Chegadas/s | Janela | Confirmadas/s | p50 previsto→confirmação | p95 | p99 | Média de operações/grupo |
|---|---|---:|---:|---:|---:|---:|
| 200 | 0 µs | 200,07 | 1,282 ms | 2,715 ms | 4,629 ms | 1,00 |
| 200 | 1000 µs | 200,05 | 2,318 ms | 3,853 ms | 4,979 ms | 1,00 |
| 5000 | 0 µs | 4997,11 | 1,708 ms | 3,306 ms | 4,107 ms | 5,83 |
| 5000 | 1000 µs | 4998,33 | 1,758 ms | 3,306 ms | 3,803 ms | 7,05 |
| 15000 | 0 µs | 14528,74 | 2,723 ms | 4,191 ms | 5,648 ms | 25,20 |
| 15000 | 1000 µs | 14555,37 | 2,808 ms | 4,171 ms | 5,543 ms | 26,01 |

Em 200 chegadas/s, os grupos continuaram unitários. A janela de 1 ms acrescentou
aproximadamente 1,04 ms ao p50 e 1,14 ms ao p95. Em 15000 chegadas/s, a diferença
entre medianas de throughput foi de apenas 0,18%; a ordem entre políticas mudou
entre repetições. Isso não demonstra benefício consistente da espera fixa.
Não se deve comparar essas taxas diretamente com as da matriz closed-loop,
que usa outro gerador e outro padrão de concorrência.

## Contabilidade de carga e recusas

Das **1212000 chegadas previstas**, houve **1183734 confirmações**, **28046
recusas Full**, **220 chegadas perdidas pelo gerador** e **zero falhas de
execução**. A identidade contábil passou nas 18 rodadas. Todas as verificações
dos documentos dos clientes após reabertura passaram.

| Chegadas/s | Janela | Full, soma das 3 rodadas | Perdidas pelo gerador, soma |
|---|---|---:|---:|
| 200 | 0 µs | 0 | 0 |
| 200 | 1000 µs | 0 | 0 |
| 5000 | 0 µs | 1 | 57 |
| 5000 | 1000 µs | 0 | 51 |
| 15000 | 0 µs | 14002 | 58 |
| 15000 | 1000 µs | 14043 | 54 |

O gerador perdeu 0,018% do cronograma total. Os tickets observados atingiram no
máximo 65, abaixo de seu teto de 256; as perdas desta matriz vieram do cronograma,
não do teto de tickets. O fato de haver 65 tickets não viola o limite de 64 do
coletor: uma operação pode terminar e liberar orçamento antes de seu recibo ser
observado pelo gerador.

Em 15000 chegadas/s, aproximadamente 3,12% das chegadas previstas foram recusadas.
Essas recusas não entram nos percentis de sucesso ou throughput confirmado.
Não há promessa de atender integralmente a taxa oferecida. Latências de recusa
e atrasos do gerador estão no JSON. A mediana do p99 de atraso do gerador ficou
entre 0,001 e 0,052 ms nas células; isso não limita seus piores atrasos.

## Distribuição por identidade e memória

Nenhuma identidade ficou sem confirmação. O índice de Jain mínimo das contagens
confirmadas foi 0,999936. Em alta carga, as frações de admissão por identidade
variaram de 96,46% a 97,27% com janela zero e de 96,10% a 97,52% com 1 ms,
considerando tentativas realmente enviadas nas três rodadas de cada política.
As contagens completas de confirmações, recusas e perdas do gerador por identidade
estão no relatório. O produtor serial em round-robin favorece essa distribuição;
o resultado não prova justiça entre conexões ou threads reais concorrentes.

VmRSS foi amostrado a cada 100 ms e nos extremos da medição. RSS inicial variou
de 4,12 a 4,24 MiB. O maior pico **observado** nas 18 rodadas foi de 9,62 MiB.

| Chegadas/s | Janela | Mediana do pico de RSS observado |
|---|---|---:|
| 200 | 0 µs | 4,59 MiB |
| 200 | 1000 µs | 4,58 MiB |
| 5000 | 0 µs | 6,56 MiB |
| 5000 | 1000 µs | 6,47 MiB |
| 15000 | 0 µs | 9,57 MiB |
| 15000 | 1000 µs | 9,57 MiB |

RSS inclui redb, threads e buffers crescentes de amostras do benchmark. O aumento
entre taxas também reflete mais amostras retidas; não é memória exclusiva da
fila nem evidência de vazamento. A amostragem pode perder picos curtos. Não foi
medida contagem de alocações nem estabelecido limite superior de RSS do produto.

## Correção da primeira medição

A [primeira execução](./collector-open-loop-polling-delay-2026-11-12.json) foi
preservada como diagnóstico e excluída das tabelas. Seu gerador dormia quase até
o próximo envio, atrasando a observação de respostas em aproximadamente 5 ms
no cenário de 200 chegadas/s. Após limitar o sono solicitado a 50 µs e adicionar
verificação desse orçamento, toda a matriz foi repetida sem builds ou testes
concorrentes. Os dados finais usam o binário corrigido identificado pelo hash
no relatório. Ambos os relatórios registram latência até observação do recibo.

## Reprodução, verificações e continuidade

```bash
cargo build -p kiarni-group-commit-spike --bin collector_open_loop --release --locked --offline
python3 benches/group_commit.py --open-loop --output /tmp/collector-open-loop.json
```

O arquivo deve ser novo. A matriz padrão leva aproximadamente três minutos.
`--seconds` admite 1–60 e `--repetitions` admite 1–5, limitando cada rodada a
1 milhão de chegadas. [Metodologia completa](../../benches/README.md).

Passaram `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --offline -- -D warnings`,
`cargo test --workspace --offline` (176 testes, um worker de fixture ignorado),
`python3 -m unittest discover -s benches -p test_http_load.py` (3 testes) e
`python3 scripts/check_product_dependencies.py` (8 pacotes).
Os dois testes novos verificam cronograma, quantis, distribuição, orçamento de polling,
contabilidade das chegadas e recuperação dos documentos após reabertura.

A evidência permite preferir agrupamento sem espera como candidata à integração.
Antes de adotá-la, o próximo incremento é definir no ADR como o coordenador do
produto preservará CRUD, limites, confirmação e ordem sob erros e encerramento,
e validar essa integração por um recorte pequeno. Datasets maiores que cache,
mistura de leituras e escrita, conexões reais e testes de estabilidade por minutos
continuam pendentes. Não há afirmação de liderança de desempenho.
