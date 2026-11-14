# Carga HTTP do Kiarni

Este benchmark inicia um binário release em loopback com diretórios temporários
privados. Os clientes são processos Python independentes, no mesmo computador do
servidor. Usa apenas a biblioteca padrão do Python.

```bash
cargo build -p kiarni --release --locked
mkdir -p docs/benchmarks
python3 benches/http_load.py --output docs/benchmarks/http-baseline.json
```

O arquivo de saída deve ser novo. A execução padrão contém 36 rodadas, com
quatro cenários, 1/8/32 clientes e três repetições. Cada rodada dura aproximadamente
dois segundos, além do warmup e da última requisição em trânsito. A saída parcial
mostra metadados agregados de cada rodada.

## Dataset e cenários

São 256 documentos de exatamente 1024 bytes, com IDs explícitos, score crescente,
grupo par/ímpar, revision e padding ASCII. O relatório inclui o hash do dataset
inicial. A cardinalidade permanece constante durante a medição.

| Cenário | Operação |
|---|---|
| read | GET por ID, percorrendo deterministicamente o dataset |
| write | PUT de documento existente, com revision nova a cada tentativa |
| query | Filtro composto, sort decrescente, limit 10 e project de score |
| mixed | 80% de tentativas GET e 20% de tentativas PUT |

Cada escritor tem seu próprio documento. A contenção medida vem da coordenação
global do servidor. A query usa os campos imutáveis score e group, com resultado
esperado conhecido. Warmup faz cinco tentativas por cliente, antes da barreira
que sincroniza a janela medida.

## Medição

O modelo é closed-loop. Cada cliente espera a resposta antes de enviar outra
requisição. Cada requisição usa uma conexão TCP nova, conforme o comportamento
HTTP atual do servidor. A latência inclui conexão, resposta e verificação do
resultado. A janela usada para throughput começa no primeiro cliente liberado
pela barreira e termina na última requisição concluída.

O relatório distingue tentativas por segundo e operações confirmadas por segundo.
Respostas 503 e erros de transporte ficam fora do throughput confirmado e dos
percentis de sucesso. A latência de falhas e os códigos de erro são reportados
separadamente. Não há retry automático nas operações medidas.

Percentis usam nearest-rank. Escritas com resposta 200 são verificadas por leitura
depois da rodada. Falha de transporte ou erro de servidor pode deixar uma escrita
com resultado desconhecido, contado em uncertain_write_outcomes.

Setup, warmup, inicialização dos processos, verificações posteriores e shutdown
ficam fora da janela medida. Eles continuam consumindo recursos do mesmo host.
O relatório registra CPU dos clientes na janela, hardware, filesystem dos dados,
versões, commit, estado do workspace e hashes do binário e fontes.

## Limites da conclusão

As rodadas são curtas e o dataset cabe no cache. Servidor e geradores de carga
compartilham CPU, e afinidade/governor não são fixados pelo harness. A coleta
está sujeita à carga e ao estado térmico do computador. A amostra p99 de uma
rodada com poucas confirmações deve ser interpretada junto da contagem.

O resultado descreve o binário e a configuração registrados. Latência em rede
remota, TLS/proxy, datasets maiores, duração prolongada e outros hardwares exigem
novas execuções. Os testes de correção e recuperação continuam na suíte Rust.

## Verificação do harness

```bash
python3 -m unittest discover -s benches -p test_http_load.py
```

Os testes verificam tamanho do dataset, cálculo dos percentis e separação de
rejeições do throughput confirmado. Um ensaio curto pode usar --clients 1 2,
--repeat 1, --seconds 0.2, --warmup 1 e --documents 16.
# Perfil opcional de PUT e agrupamento

Para medir validação e admissão HTTP, compile com
`cargo build -p kiarni --release --features kiarni-server/profiling --locked --offline`.
Depois execute `python3 benches/http_load.py --scenarios write --repeat 1 --seconds 2 --output REPORT.json`.
O benchmark captura os contadores do servidor após o encerramento. Eles incluem
warmup, somam tempo de parede e CPU da própria thread e não fornecem percentis
por fase nem contagem de alocações. O recurso vem desabilitado por padrão e
não registra documentos ou credenciais. `--scenarios` também permite selecionar
outros cenários existentes, sem mudar a matriz padrão.

Após a integração do ADR 0017, put_admission mede submissão à fila. As fases
de storage instrumentadas ainda pertencem à API individual; seus contadores
de commit não representam os novos grupos HTTP. Os perfis anteriores documentam
o caminho vigente na data da medição. A comparação atual do HTTP usa binários
sem instrumentação e conta confirmações/latências de clientes reais.

Para o experimento separado, compile
`cargo build -p kiarni-group-commit-spike --release --locked --offline` e execute
`python3 benches/group_commit.py --output REPORT.json`. Use um arquivo novo.
Não execute compilação, testes ou outras cargas simultaneamente. São 3 repetições
por tamanho de grupo (1, 8, 32), cada uma com 4096 substituições de 1 KiB sobre
256 documentos, cache de 8 MiB, grupos pré-formados e commit Immediate.
Criação, preparação de entradas e verificação após reabertura ficam fora do tempo.
O p95 é a latência do grupo pronto, sem espera de coleta nem HTTP.
O relatório inclui ambiente, filesystem temporário e hashes de fonte/binário.
Esse throughput não representa capacidade da API.
O executável padrão usa grupos pré-formados e não o coletor concorrente descrito
no [README do spike](../spikes/group-commit/README.md).

## Medição do coletor concorrente

```bash
cargo build -p kiarni-group-commit-spike --bin collector_bench --release --locked --offline
python3 benches/group_commit.py --collector --output /tmp/collector.json
```

São 75 rodadas: 3 repetições de 2 segundos para cinco políticas e cinco cargas.
As políticas são um commit por operação e lotes de até 32 com janelas de
0/50/200/1000 µs. As cargas usam 1/8/32/128 threads com uma operação em voo
por cliente e um cenário de baixa carga com 1 cliente que pensa 5 ms após cada
resposta. A admissão permite 64 operações e 8 MiB de IDs/corpos, contando o
grupo ativo; corpos por grupo têm limite de 8 MiB. O dataset é de 256 documentos
de 1 KiB e o cache é de 8 MiB. Cada cliente altera seu próprio ID com uma revisão
única a cada tentativa. Seed, criação do worker e verificação pós-reabertura
ficam fora da medição. Não há warmup; o cache vem da criação do seed.

A ordem das cinco políticas gira entre repetições para reduzir viés de ordem.
Cada rodada usa raiz nova. O throughput divide confirmações pelo tempo da
liberação da barreira até o join dos produtores, incluindo a conclusão das
últimas operações e pausas finais. É um gerador closed-loop dentro do mesmo
processo, sem HTTP, autenticação ou eventos. Não mede capacidade do servidor.

A latência completa começa antes da construção do corpo e vai até a confirmação
recebida pelo cliente. Fila/coleta vai da admissão até o início do processamento
do grupo. Serviço vai desse início até o retorno do armazenamento. Percentis
nearest-rank são calculados por operação confirmada; serviço repetido em cada
membro do grupo é ponderado por operações, não por grupos. A média de operações
por grupo usa grupos distintos identificados por sequência local. O relatório
também expõe a média de tamanho ponderada por operações, de interpretação diferente.

Rejeições `Full` são contadas com latências separadas e não entram no throughput
confirmado. Após rejeição, o produtor espera 100 µs antes de uma nova revisão;
não repete a operação recusada. Isso limita busy loops, mas também muda a carga
oferecida em saturação. Há um teto de 100 mil tentativas por cliente/rodada que
aborta o benchmark se ultrapassado. Falhas de execução são contadas; o documento
final de cada cliente é comparado com sua última confirmação após reabertura.

`--seconds` (1–30) e `--repetitions` (1–10) permitem alongar as rodadas do coletor.
Os resultados curtos são exploratórios. Não execute testes, builds ou outras
cargas junto da medição. O arquivo de saída precisa ser novo.

## Chegadas independentes das respostas

```bash
cargo build -p kiarni-group-commit-spike --bin collector_open_loop --release --locked --offline
python3 benches/group_commit.py --open-loop --output /tmp/collector-open-loop.json
```

A matriz padrão tem 18 rodadas: 3 repetições, taxas de 200/5000/15000 chegadas/s
e janelas de 0/1000 µs, por 10 segundos cada. Cada rodada ocorre em subprocesso
novo, com 32 clientes virtuais distribuídos em round-robin por um único gerador.
O envio não espera confirmações anteriores. Esses clientes não são threads nem
conexões de rede. Há até 256 tickets no gerador, separados do limite de 64
operações/8 MiB do coletor; ambos os limites incluem suas estruturas em uso.
Seed e cache são os mesmos da matriz closed-loop, com commit Immediate e sem
warmup. A ordem das duas janelas alterna entre repetições.

O cronograma é uniforme, calculado a partir de um instante inicial e uma taxa
fixa. Chegadas atrasadas por um intervalo inteiro são descartadas e contadas
como `generator_skipped`; não viram uma rajada de compensação nem uma recusa do
coletor. O mesmo contador inclui chegadas que esbarram no teto de tickets do
gerador. Por rodada, chegadas previstas equivalem à soma de confirmações,
recusas Full, chegadas perdidas pelo gerador e falhas de execução.
Não há retry ou backoff por Full: a próxima tentativa segue o cronograma.

O gerador intercala polling não bloqueante de confirmações com novas admissões.
Espera por sono de no máximo 50 µs quando faltam mais de 100 µs para a próxima
chegada e usa spin/polling no trecho final, consumindo CPU do mesmo processo.
Esse teto mantém a observação de respostas ativa mesmo em baixa carga; o SO
pode alongar o tempo de sono. A coleta das
respostas também compete por CPU com storage. O tempo desde o horário previsto
até observar a confirmação é `scheduled_latency`; `actual_latency` começa no
envio efetivo, antes da construção do documento. Ambos incluem o atraso de
observação das respostas. O relatório também contém percentis de atraso do
gerador e tempo de fila/coleta. Throughput confirmado inclui drenagem final.

São registrados confirmações, recusas e perdas do gerador por cliente virtual,
clientes sem confirmação e índice de Jain sobre contagens confirmadas. Esse
índice isolado não demonstra justiça: perdas do gerador podem afetar a distribuição
das tentativas, e as 32 identidades passam pelo mesmo produtor serial.
O documento de cada identidade é verificado após reabertura, usando a maior
sequência confirmada, já que a observação por polling pode mudar a ordem dos recibos.

Em Linux, VmRSS de `/proc/self/status` é amostrado a cada 100 ms, além do começo
e fim da medição. `rss_sampled_peak_bytes` é máximo **observado**, não limite
superior de RSS, contagem de alocações ou medição exclusiva do coletor. Inclui
redb, threads e buffers crescentes do benchmark. Cada célula em processo novo
evita herdar o histórico do allocator da célula anterior. Em outras plataformas,
os campos de RSS podem ser nulos. Valores de `--seconds` vão de 1 a 60 e
`--repetitions` de 1 a 5; o teto de amostras é de 1 milhão por rodada.
