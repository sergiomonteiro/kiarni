# Índice secundário no HTTP — 2026-11-22

RF-04 / issue #10, após M3. Medição do binário real: mesmo Store formato 2,
sem índice versus índice em `tag`. Não é resultado do spike.

Nas três repetições com 4.096 documentos de 1 KiB, tags espalhadas e um cliente,
a mediana das razões pareadas de throughput foi **53,32×** para igualdade com um
candidato e **4,42×** para 512 candidatos (1/8). Com 1.024 documentos de 256 bytes,
essas razões foram **8,43×** e **2,48×**. A escrita que altera `tag` teve custo:
throughput **14,4% menor** no primeiro cenário e **11,6% menor** no menor.
Com oito clientes e tags espalhadas, essa redução foi **16,4%**. O índice deve ser
opcional; ganhos de leitura não eliminam custo de manutenção.

**270 rodadas, 135 pares válidos, 1.080.963 operações confirmadas**, todas com
auditoria offline e verificação após reinício. Houve **2.155 respostas 503
`service_unavailable`**, exclusivamente nas consultas de oito clientes que fazem
scan. Nenhum erro de transporte ou resultado de escrita desconhecido; nenhuma
rodada foi descartada. As recusas estão separadas dos sucessos nas tabelas e nos
JSON; throughput conta apenas respostas corretas confirmadas.

## Resultados

RPS é a mediana das taxas de três rodadas por modo. p95 é a mediana dos p95 de
cada rodada, em ms, não um percentil combinado. Razão é a mediana das **razões
pareadas**, portanto pode diferir da razão entre as medianas de RPS. Intervalo
mostra mínimo–máximo dos três pares, não intervalo de confiança. 503 soma as três
rodadas por modo; denominadores, p50/p99, falhas e commits/grupos estão nos JSON.

`needle`: 1 candidato; `sparse`: N/64; `medium`: N/16; `edge`: N/8;
`over`: N/8+1; `dense`: restante; `absent`: zero. `compound` força scan e `id`
controla prioridade do primário. PUT indexado alterna tag a cada confirmação;
PUT outro altera revision/padding mantendo tag. Churn alterna POST e DELETE
em um ID por cliente; conta cada operação confirmada, não um ciclo como uma só.

### 4.096 × 1.024 bytes, tags contíguas

144 rodadas de 2 s; três repetições por célula.

| Clientes | Caso | RPS sem / com | p95 sem / com (ms) | Razão pareada [min–max] | 503 sem / com |
| --- | --- | --- | --- | --- | --- |
| 1 | absent | 57.1 / 2944.3 | 19.16 / 0.46 | 50.701 [48.829–52.859] | 0 / 0 |
| 1 | churn | 574.3 / 523.0 | 3.15 / 3.16 | 0.911 [0.889–0.929] | 0 / 0 |
| 1 | compound | 53.9 / 51.9 | 21.07 / 21.01 | 0.975 [0.918–0.978] | 0 / 0 |
| 1 | dense | 55.1 / 54.7 | 20.07 / 20.52 | 0.991 [0.977–0.993] | 0 / 0 |
| 1 | edge | 56.7 / 251.8 | 19.73 / 4.86 | 4.468 [4.343–4.622] | 0 / 0 |
| 1 | id | 2943.2 / 2946.2 | 0.45 / 0.45 | 0.998 [0.971–1.025] | 0 / 0 |
| 1 | medium | 57.4 / 412.9 | 19.53 / 3.23 | 7.201 [7.177–7.573] | 0 / 0 |
| 1 | needle | 59.4 / 2759.4 | 19.22 / 0.49 | 46.152 [46.097–50.610] | 0 / 0 |
| 1 | over | 57.7 / 56.9 | 19.54 / 19.60 | 0.985 [0.982–1.009] | 0 / 0 |
| 1 | put_indexed | 550.0 / 503.1 | 3.10 / 3.13 | 0.907 [0.906–0.953] | 0 / 0 |
| 1 | put_other | 565.2 / 553.9 | 3.14 / 3.06 | 1.004 [0.980–1.031] | 0 / 0 |
| 1 | sparse | 59.3 / 975.2 | 18.71 / 1.33 | 16.321 [15.955–18.247] | 0 / 0 |
| 8 | absent | 61.4 / 22138.9 | 116.56 / 0.47 | 360.308 [356.630–375.057] | 88 / 0 |
| 8 | churn | 817.4 / 716.1 | 12.46 / 12.67 | 0.868 [0.718–0.876] | 0 / 0 |
| 8 | compound | 53.9 / 52.6 | 116.77 / 118.78 | 0.991 [0.975–1.017] | 120 / 126 |
| 8 | dense | 56.2 / 54.3 | 117.41 / 116.94 | 0.997 [0.966–1.049] | 119 / 117 |
| 8 | edge | 58.3 / 293.6 | 116.97 / 29.56 | 5.013 [4.961–5.295] | 105 / 0 |
| 8 | id | 20905.5 / 21023.2 | 0.52 / 0.53 | 1.010 [1.006–1.010] | 0 / 0 |
| 8 | medium | 56.9 / 520.0 | 116.44 / 17.69 | 9.193 [8.008–9.259] | 97 / 0 |
| 8 | needle | 60.8 / 20446.3 | 116.27 / 0.52 | 336.426 [335.234–356.000] | 87 / 0 |
| 8 | over | 58.8 / 59.0 | 116.20 / 117.45 | 1.003 [0.962–1.009] | 100 / 95 |
| 8 | put_indexed | 2826.7 / 2503.1 | 4.29 / 4.48 | 0.924 [0.825–0.925] | 0 / 0 |
| 8 | put_other | 2751.5 / 2706.6 | 4.35 / 4.34 | 0.959 [0.684–0.984] | 0 / 0 |
| 8 | sparse | 60.7 / 1893.0 | 116.39 / 5.49 | 30.390 [29.512–33.606] | 87 / 0 |

### 4.096 × 1.024 bytes, tags espalhadas

84 rodadas de 3 s; três repetições por célula.

| Clientes | Caso | RPS sem / com | p95 sem / com (ms) | Razão pareada [min–max] | 503 sem / com |
| --- | --- | --- | --- | --- | --- |
| 1 | churn | 559.8 / 514.6 | 3.09 / 3.14 | 0.903 [0.876–0.946] | 0 / 0 |
| 1 | dense | 54.0 / 53.6 | 20.71 / 20.65 | 0.993 [0.991–1.008] | 0 / 0 |
| 1 | edge | 56.7 / 244.2 | 19.61 / 4.99 | 4.415 [4.307–4.811] | 0 / 0 |
| 1 | needle | 52.2 / 2786.2 | 22.23 / 0.49 | 53.320 [47.448–58.048] | 0 / 0 |
| 1 | over | 56.2 / 56.0 | 19.70 / 19.95 | 0.997 [0.996–1.016] | 0 / 0 |
| 1 | put_indexed | 538.5 / 469.8 | 3.17 / 3.38 | 0.856 [0.667–0.872] | 0 / 0 |
| 1 | put_other | 533.1 / 519.6 | 3.15 / 3.27 | 0.999 [0.958–1.021] | 0 / 0 |
| 8 | churn | 853.2 / 746.4 | 11.20 / 12.70 | 0.868 [0.834–0.901] | 0 / 0 |
| 8 | dense | 51.8 / 52.4 | 120.41 / 119.23 | 0.988 [0.973–1.012] | 184 / 195 |
| 8 | edge | 54.8 / 286.1 | 118.54 / 30.11 | 5.226 [4.754–5.500] | 160 / 0 |
| 8 | needle | 56.3 / 19234.0 | 116.73 / 0.58 | 341.650 [333.130–342.211] | 157 / 0 |
| 8 | over | 55.5 / 56.9 | 117.58 / 116.76 | 1.025 [0.992–1.027] | 161 / 157 |
| 8 | put_indexed | 2863.3 / 2405.4 | 3.99 / 4.39 | 0.836 [0.827–0.852] | 0 / 0 |
| 8 | put_other | 2793.6 / 2773.5 | 4.25 / 4.28 | 1.012 [0.956–1.014] | 0 / 0 |

### 1.024 × 256 bytes, tags espalhadas

42 rodadas de 1 s; três repetições por célula.

| Clientes | Caso | RPS sem / com | p95 sem / com (ms) | Razão pareada [min–max] | 503 sem / com |
| --- | --- | --- | --- | --- | --- |
| 1 | churn | 596.6 / 512.2 | 3.05 / 3.20 | 0.859 [0.801–0.870] | 0 / 0 |
| 1 | dense | 305.2 / 301.8 | 4.06 / 4.14 | 0.999 [0.977–1.007] | 0 / 0 |
| 1 | edge | 325.3 / 810.9 | 3.89 / 1.61 | 2.481 [2.479–2.492] | 0 / 0 |
| 1 | needle | 333.8 / 2814.0 | 3.76 / 0.46 | 8.430 [8.170–8.645] | 0 / 0 |
| 1 | over | 317.5 / 316.9 | 3.99 / 3.97 | 0.998 [0.987–1.001] | 0 / 0 |
| 1 | put_indexed | 577.8 / 513.7 | 3.04 / 3.12 | 0.884 [0.880–0.898] | 0 / 0 |
| 1 | put_other | 561.4 / 549.7 | 3.14 / 3.11 | 0.979 [0.904–1.059] | 0 / 0 |

## Interpretação e decisão

Manter o limiar conservador de candidatos <= N/8: houve benefício em todas as
rodadas `edge`, inclusive com tags espalhadas e documentos menores. Acima do
limiar (`over`) e no caso denso, o produto preservou scan, com razões medianas
próximas de 1. Isso valida o comportamento e o benefício nos casos medidos,
**não prova que 1/8 seja o ponto ótimo**: não se forçou índice acima dele.

A razão muito alta com oito clientes inclui a eliminação da contenção do gate
serial de consulta: scans têm espera e recusas, enquanto consultas seletivas
terminam rapidamente. Não é comparação com outro banco nem capacidade geral do
Kiarni. p95 de sucesso não inclui as recusas; percentis de falha estão separados.
São rodadas curtas, sobretudo para scans com poucas amostras; p99 não é SLO.

Custos variam: PUT que preserva tag teve razões medianas entre 0,959 e 1,012;
PUT que altera tag, entre 0,836 e 0,924; churn, entre 0,859 e 0,911. Há dispersão
importante: um par de PUT indexado deu 0,667 e um de PUT outro 0,684. Não remover
esses pares nem atribuir causalidade a ruído sem perfil. O relatório registra
custo observado, sem alegar isolamento de cada componente do commit.

## Ambiente, artefatos e custo offline

Linux 7.2.6-76070206-generic x86_64; Ryzen 7 6800H, 16 CPUs lógicas disponíveis,
15.898.722.304 bytes de RAM total. Dados temporários em ext4, bloco de 4.096
bytes, `/dev/nvme0n1p3[/tmp]`. Rust 1.99.1, Cargo 1.99.1, Python 3.12.3,
redb 4.5.0, cache do engine de 8 MiB. Cgroups observados sem quota finita de
CPU/memória. Host compartilhado; carga e afinidade registradas por matriz.
Não foram fixados governor/afinidade nem descartados caches; abertura/auditoria
e warmup precedem a medição. Não demonstra cold-cache ou limite de RAM.

| Dataset | Criação offline (ms) | Auditoria offline (ms) | data.redb sem / com (bytes) |
| --- | --- | --- | --- |
| Contíguo 4096 | 69.23 | 39.33 | 8425472 / 8425472 |
| Espalhado 4096 | 90.78 | 39.77 | 8425472 / 8425472 |
| Espalhado 1024 | 22.01 | 11.27 | 1056768 / 1056768 |

Criação é uma execução do comando completo, incluindo abertura/auditoria;
não é backfill isolado nem mediana de manutenção. Arquivo de mesmo tamanho não
significa índice gratuito: redb aloca/reutiliza páginas; não se mediu ocupação
útil nem tamanho físico exclusivo do índice. Tamanhos antes/depois de cada
workload constam dos JSON. Os dois modos já mantêm estatísticas: não mede seu
custo em comparação ao formato 1 removido.

Binário release/default features, fonte de produção `2790394`, SHA-256 comum:
`18b8343941d02387446aa13e5d8f0560f797082ca0ae9f9cffb13a9e58c8243f`.
Harness contíguo no commit `64414ec20a048f82f41ca497fe026b55f646871a`;
espalhado/menor em `fc1eff3d692d1989c4dc9bc6d0e1e90b842dffad`. Ambos com árvore
limpa, sem mudanças Rust entre eles. Dataset/fontes/manifests têm hashes nos JSON.
Coleta ocorreu na noite local de 22/11 (timestamps UTC podem indicar 23/11).

## Reprodução e controles

```bash
cargo build -p kiarni --release --locked --offline
# No harness 64414ec, layout contíguo era implícito:
python3 benches/index_http.py --output /tmp/kiarni-index-http-main-20261122.json --documents 4096 --document-bytes 1024 --clients 1 8 --repeat 3 --seconds 2 --warmup 5
# No harness fc1eff3:
python3 benches/index_http.py --output /tmp/kiarni-index-http-spread-20261122.json --layout spread --documents 4096 --document-bytes 1024 --clients 1 8 --repeat 3 --seconds 3 --warmup 5 --cases needle edge over dense put_indexed put_other churn
python3 benches/index_http.py --output /tmp/kiarni-index-http-small-20261122.json --layout spread --documents 1024 --document-bytes 256 --clients 1 --repeat 3 --seconds 1 --warmup 5 --cases needle edge over dense put_indexed put_other churn
```

Usar caminhos novos; no harness atual, adicionar `--layout contiguous` ao primeiro
comando. Rodadas executadas serialmente, sem builds/testes concorrentes.
[Metodologia do harness](../../benches/README.md) detalha seed, barreira que exclui
warmup, ordem alternada, restauração por membro, classificação e validação.
Cada cliente é um processo Python em closed-loop, nova conexão TCP por request,
loopback autenticado, respostas pequenas (sort/limit 10/project), sem assinantes.
Latência inclui HTTP/decode/validação; construção do payload fica fora do timer,
mas consome CPU e afeta RPS. Não é carga de chegadas independentes nem medição
isolada de servidor. Commit Immediate, writer normal, logs off; telemetria ativa.

Resultados brutos, copiados sem alteração da saída do medidor:

- [144 rodadas contíguas](./secondary-index-http-contiguous-2026-11-22.json).
- [84 rodadas espalhadas](./secondary-index-http-spread-2026-11-22.json).
- [42 rodadas menores](./secondary-index-http-small-2026-11-22.json).

A equivalência e os caminhos selecionados são verificados em cada rodada por
respostas e deltas de métricas; escrituras têm estado final conferido após
reinício. Os testes existentes cobrem falhas/crash/concorrência, inclusive feeds
pós-commit e recuperação; o medidor não injeta power-loss. Sete testes Python
validam o harness. Rust: 372 testes aprovados; fmt/clippy aprovados. CPU/RSS,
concorrência mista sustentada, datasets fora do cache e investigação ampla da
issue #9 continuam no planejamento separado. Nenhuma dependência runtime nova,
mudança de API ou formato persistido nesta medição.
