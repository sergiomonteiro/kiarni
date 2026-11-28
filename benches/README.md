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

## Índice secundário no produto HTTP

`index_http.py` compara o mesmo binário/formato 2 com índice opcional ausente ou
presente. Não usa o exemplo Rust do spike nem uma versão antiga do servidor.
Os dois modos mantêm estatísticas, telemetria e o writer normal do produto.

```bash
cargo build -p kiarni --release --locked --offline
python3 benches/index_http.py --output /tmp/index-http-main-NOVO.json --layout spread --documents 4096 --document-bytes 1024 --clients 1 8 --repeat 3 --seconds 2 --warmup 5
python3 benches/index_http.py --output /tmp/index-http-small-NOVO.json --layout spread --documents 1024 --document-bytes 256 --clients 1 --repeat 3 --seconds 1 --warmup 5 --cases needle edge over dense put_indexed put_other churn
```

Saídas precisam ser novas. O primeiro comando mede 144 rodadas (72 pares); o
segundo, 42 rodadas (21 pares). Não executar builds, testes ou outras cargas
junto das medições. Setup/warmup, restauração, auditoria e verificação após
reinício ficam fora da janela de throughput/latência.

O dataset tem documentos de tamanho exato, `_id`, score, tag, revision e padding.
Buckets de tag contêm 1 documento, N/64, N/16, N/8, N/8+1 e o restante denso.
`--layout spread` (padrão) distribui tags por permutação multiplicativa de stride
coprimo com N; IDs continuam ordenados. `--layout contiguous` agrupa cada tag
por faixa de ID, permitindo comparar o efeito da localidade. Ambos mantêm a
mesma contagem de cada tag; cada par compartilha exatamente seu dataset.
Consultas eq tag também medem valor ausente; há controle por ID e filtro composto
que exige scan. Sort por score decrescente, limit 10 e project preservam retorno
pequeno conhecido. O relatório verifica o caminho id/scan/secondary por diferença
de contadores do servidor, excluindo warmup por barreira com o processo pai.

Há PUT que muda tag a cada confirmação e PUT que muda só revision/padding;
cada cliente tem seu próprio ID. Churn alterna POST/DELETE num ID reservado por
cliente, mantendo cardinalidade entre N e N+clientes. Recusa busy preserva o
estado conhecido para a próxima tentativa; resultado incerto interrompe o
cliente e invalida a rodada, sem retry de uma escrita desconhecida. Inserts e
deletes têm percentis separados além do agregado.

Clientes são processos Python em closed-loop, uma conexão TCP por requisição.
Latência inclui HTTP, decode e validação da resposta; construção do payload é
anterior ao timer, mas consome CPU e afeta throughput. A janela agregada vai do
primeiro cliente liberado ao último encerrado. Somente respostas corretas
200/201/204 entram em confirmações/percentis de sucesso; falhas, códigos,
incertezas e percentis de falha são separados. Há teto de 100000 tentativas por
cliente/rodada, que invalida a medição se atingido.

Cada membro do par usa nova instalação restaurada pela CLI de um dos dois
artefatos equivalentes. A ordem se inverte entre repetições de cada célula e
alterna entre cenários/clientes. Depois da carga, encerra serve, audita pela CLI,
reinicia e verifica o estado de cada escritor ou o resultado da consulta. O
relatório inclui commits/grupos, tamanho lógico do arquivo, hashes do dataset,
binário/fontes e metadata de host/toolchain/filesystem. Não contém credenciais.
A criação/auditoria offline do seed é medida separadamente, incluindo abertura
e auditoria; não é tempo isolado de backfill.

Resumo usa a mediana das razões de pares completos e válidos; não é percentil
agrupado nem intervalo de confiança. Ausência/duplicação de membro ou falha não
vira ganho por exclusão silenciosa. O dataset respeita os budgets do produto.
Caches não são descartados, afinidade/governor não são fixados, host e clientes
são compartilhados. Não demonstra cold-cache, limite de CPU/RSS, capacidade em
outro hardware ou optimalidade do limiar 1/8 fora dos casos medidos.

O harness reutiliza transporte, percentis e metadata de `http_load.py` e o ciclo
de processo de `scripts/release_smoke.py`. Os testes de medição acima cobrem
partição/tamanho do dataset, pares/ordem, seleção esperada e separação de busy,
falha e confirmação. Um smoke completo curto pode usar N=128, tamanho 128,
clientes 1/2, repeat 1, seconds 0.15 e warmup 2; ele valida o medidor, não fornece
uma baseline de desempenho.

## Custo de logs e leitores de telemetria

`telemetry_http.py` compara quatro modos no mesmo binário atual: logs off/info,
cada um sem/com um processo leitor. Não desliga a instrumentação fixa do produto
e não isola seu custo contra um servidor sem métricas. Não é reprodução do
binário histórico do writer.

```bash
cargo build -p kiarni --release --locked --offline
python3 benches/telemetry_http.py --output /tmp/telemetry-NOVO.json --clients 1 8 --repeat 4 --seconds 3 --documents 256 --document-bytes 1024 --warmup 5 --reader-hz 2
```

Esse comando mede 64 rodadas. Quatro repetições equilibram a posição de cada
modo e seus predecessores imediatos por célula. A ordem também varia entre
cenários/clientes. Cada rodada inicia instalação nova, com o mesmo dataset
somente de leitura: GET de documento existente, ou GET de ID ausente (404
document_not_found esperado, que exercita captura de erros). O 404 esperado é
contado como resposta esperada, nunca como escrita/operação confirmada 2xx.
Falhas de transporte, busy, resposta incorreta e limite de tentativas invalidam
a rodada; não são descartadas silenciosamente. Percentis esperados/inesperados
e status/códigos são separados. Compara razões por repetição, sem pressupor
significância estatística de diferenças pequenas.

Clientes são processos em closed-loop, nova conexão por requisição. Warmup e
preparação ficam fora da janela, com barreira entre clientes, leitor e pai.
O leitor faz uma chamada a metrics e uma a errors/recent por ciclo: 2 Hz são
quatro requests/s no total. Não tenta recuperar ticks perdidos com rajadas;
registra ciclos atrasados e contagem/latência/bytes por endpoint. Leitor também
valida respostas. Todos os documentos do seed são conferidos após a carga,
fora da janela; não há escrita concorrente, subscribers ou teste de crash.

stdout/stderr do servidor vão para arquivo temporário comum no filesystem de
dados, sem pipe bloqueado nem coletor externo. Logs são contados após shutdown;
bytes, registros, dropped_before e linhas JSON malformadas cobrem **todo o
ciclo**, inclusive seed/warmup/verificação. Não inferir taxa de perda só da
janela medida: descartes finais podem não ter próximo registro para reportar.
O contador de perdas do anel é coletado após a carga e inclui warmup; substituição
normal de entrada antiga não é perda por contenção. Nenhum log bruto, request ID
ou credencial é publicado. O arquivo tem orçamento pós-coleta de 128 MiB;
tentativas são limitadas a 100000 por cliente/rodada. O sink é arquivo local,
sem simular sink lento; esse caso mantém os testes existentes do logger.

Dados temporários são removidos ao fim de cada rodada. Relatórios usam criação
exclusiva, hashes de fonte/binário/dataset e ambiente. Não rodar builds/testes
concorrentes. Caches/governor/afinidade não são fixados; host e CPU dos clientes
são compartilhados. É diagnóstico local, sem promessa de capacidade, CPU/RSS
ou causalidade de pequenas diferenças. Os testes de medição incluem respostas
404 esperadas versus falhas, equilíbrio das ordens, quartetos completos e
validação limitada do histórico.

Um journal `.progress.jsonl`, criado com permissão privada e sem sobrescrita,
preserva as rodadas concluídas se uma etapa posterior interromper a execução.
Conferência pós-carga não repete requests: falha invalida a rodada e registra
tipo/errno ou status, sem mensagem livre, payload ou token. Não excluir rodadas
inválidas para calcular ganhos; o relatório só compara quartetos completos de
uma matriz válida. Tentativas abortadas devem ser declaradas junto da repetição
completa, sem juntar células escolhidas de execuções diferentes.

Em Linux, snapshots de TCP antes/depois de cada ciclo registram TIME_WAIT,
overflow, timeout, backlog e limites disponíveis, sem alterar sysctls. São
contadores globais do host, incluindo outras aplicações; não atribuem falhas
ao Kiarni nem isolam o custo do logger. Timeouts durante leitura de telemetria
invalidam a rodada e ficam no resultado, sem retry. Um smoke/diagnóstico com
uma repetição pode identificar limites do experimento, mas não substitui as
quatro repetições da comparação de custo.

O harness atual reutiliza uma porta loopback entre servidores temporários,
mantendo instalações independentes e uma conexão nova por request. Evita
adicionar um novo conjunto de tuplas TCP por porta a cada ciclo. Essa política
é registrada no JSON; versões anteriores com porta aleatória a cada servidor
não são misturadas na mesma comparação. Não altera sysctls ou protocolo HTTP.

## Revisita dos artefatos históricos do writer

`writer_history.py` exige os dois binários com hashes exatos dos relatórios de
2026-11-13. Não recompila fonte atual para representar a versão histórica nem
abre um banco existente. Cada membro inicia instalação temporária nova, usando
as opções HTTP originais. Não adiciona flags recentes ao executável antigo.

```bash
python3 benches/writer_history.py --before /caminho/binario-anterior --writer /caminho/binario-integrado --output /tmp/writer-history-NOVO.json --clients 1 8 32 --repeat 4 --seconds 3 --warmup 5
```

São 48 rodadas, 24 pares integrado/anterior, além de controles A/A: modos 0/2
usam anterior e 1/3 integrado; ordem Williams, equilibrada em quatro repetições.
Comparações sempre integrado/anterior; A/A compara 2/0 e 3/1. Esses controles
compartilham rodadas com os pares A/B: não são amostras independentes extras.

Reutiliza `http_load.py` com porta fixa (`--port`) e `--verify-full-read`, válido
somente no cenário read. Confere JSON completo a cada leitura e warmup, além
dos 256 documentos após a carga. Dataset inicial de 256 × 1 KiB, idêntico entre
membros; não mistura escritas/query entre rodadas. Read-only, nova conexão TCP
por request e um processo por cliente. Mantém budgets, durabilidade e limites
de configuração originais. Sem alteração no protocolo/binário do servidor.

Criação exclusiva e journal preservam resultados concluídos; qualquer falha
invalida a matriz, sem exclusão de pares para obter ganho. Hashes dos binários
e do medidor são conferidos antes/depois; diagnósticos globais de TCP incluem
seed/warmup/carga/conferência. Sem builds/testes durante a medição.

É uma revisita controlada da leitura nos executáveis exatos, não repetição
bit a bit das matrizes originais: agora há intercalação, validação mais forte,
instalação nova por membro e controle de porta. Não representa desempenho do
produto atual nem prova que uma queda histórica foi causada por uma função.
Janelas curtas e host compartilhado limitam diferenças pequenas; amplitude
de A/A não é intervalo de confiança nem limite universal de ruído.

### Separação opcional entre gerador e servidor (Linux)

`http_load.py` e `writer_history.py` aceitam `--server-cpus` e `--client-cpus`
em conjunto. As listas devem ser não vazias, distintas entre si e contidas na
afinidade disponível. O launcher Python aplica a máscara **antes do exec** do
servidor, para que seus threads a herdem; os clientes aplicam sua máscara no
initializer, antes do warmup. Máscaras observadas são verificadas e registradas.
Sem as opções, a execução mantém a afinidade normal e funciona em macOS.

Escolher conjuntos que não compartilhem cores físicos/SMT, conferindo a topologia
registrada. Disjunção lógica sozinha não garante disjunção física. O medidor não
reserva cores contra outras aplicações e não muda governor, sysctls ou a
afinidade do Kiarni fora dessas execuções temporárias. Não é benchmark de
escalabilidade por quantidade de cores nem otimização multi-core do produto.

Exemplo somente para host com CPUs 0–7 e 8–15 em cores físicos distintos:

```bash
python3 benches/writer_history.py --before .bench-artifacts/writer-2026-11-26/before --writer .bench-artifacts/writer-2026-11-26/writer --output /tmp/writer-pinned-NOVO.json --clients 8 32 --repeat 4 --seconds 10 --warmup 100 --server-cpus 0 1 2 3 4 5 6 7 --client-cpus 8 9 10 11 12 13 14 15
```

São 32 rodadas / 16 pares A/B, mais os controles A/A compartilhando as mesmas
rodadas. Topologia, governor/frequência pontual e loadavg são lidos antes/depois
dos ciclos. Frequências pontuais não são médias nem controle de temperatura.
CPU agregada dos processos clientes já é registrada; não equivale a perfil
CPU/RSS do servidor. O novo particionamento muda as condições: manter os dados
anteriores, sem atribuir melhoria da dispersão só a afinidade ou só à duração.

## Gerador Rust de leitura

`read-load` é um exemplo de desenvolvimento de kiarni, sem dependência nova e
sem ligação no binário do servidor. Compila com dependências de teste existentes:

```bash
cargo build -p kiarni --example read-load --release --locked --offline
```

`--rust-driver target/release/examples/read-load` seleciona o gerador no
http_load.py (somente read com --verify-full-read) ou writer_history.py.
Mantém conexão nova por request, validação JSON completa e limite de 32 clientes;
usa um processo com threads fixos, 256 KiB de stack por thread e no máximo
100000 tentativas por cliente. Entrada por stdin limitada a 8 MiB, até 1024
documentos de 4096 bytes; respostas até 64 KiB, headers 8192 bytes e prazo total
de cinco segundos por request. Esse limite de resposta é deliberadamente
restrito ao fixture de leitura, não contrato da API. Não é gerador de escrita.

As requisições são preparadas antes da janela. Aquecimento usa barreira antes
do relógio compartilhado; falha de warmup invalida a rodada e libera todos os
workers. Token entra somente por stdin, não por argv/resultado. Respostas
inválidas e erros nunca entram nos percentis de sucesso. Não há retry oculto
ou conexão persistente. Testes do exemplo rodam em cargo test --workspace.

CPU do gerador Rust é medida pelo pai via RUSAGE_CHILDREN antes/depois de
aguardar seu processo. Inclui launcher, warmup, carga e resumo, portanto é
escopo mais amplo que a CPU medida só durante a janela nos workers Python.
O servidor ainda não foi reaped nesse ponto e não entra nessa diferença.
Dividir esse total pelo tempo medido é conservador para verificar margem;
não usar como perfil de CPU do servidor ou comparação exata de overhead.

Calibração A/A (todos os modos usam o mesmo writer e seu hash é conferido):

```bash
python3 benches/writer_history.py --before .bench-artifacts/writer-2026-11-26/before --writer .bench-artifacts/writer-2026-11-26/writer --rust-driver target/release/examples/read-load --aa-only --output /tmp/writer-rust-AA-NOVO.json --clients 32 --repeat 4 --seconds 5 --warmup 100 --server-cpus 0 1 2 3 4 5 6 7 --client-cpus 8 9 10 11 12 13 14 15
```

Depois, a comparação A/B usa o mesmo comando sem --aa-only e com --clients
8 32 --seconds 10. Escolher máscaras compatíveis com o host. O JSON registra
o hash do gerador, além de fontes, executáveis de servidor e CPU observada.
Não misturar geradores/escopos como se a melhoria fosse do produto.
