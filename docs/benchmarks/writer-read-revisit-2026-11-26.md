# Revisita da leitura nos binários históricos do writer — 2026-11-26

Issue #9 / requisito não funcional de desempenho, após M3. A busca ampliada no
cache do Cargo encontrou o executável integrado exato, antes considerado não
localizado. A recuperação permite comparar os **dois binários medidos em
2026-11-13**, sem usar o servidor atual como substituto.

**48 rodadas válidas, 2.081.656 leituras confirmadas**, sem recusas, falhas de
transporte, respostas incorretas ou exclusão de rodadas. Dataset conferido em
todas as leituras/warmups e após cada carga. Nenhuma otimização Rust nesta entrega.

As medianas pareadas integrado/anterior foram **0,9920 com oito clientes** e
**0,9943 com 32**: quedas observadas de 0,80% e 0,57%, menores que os 5,96% e
4,32% da medição original. Há grande dispersão nos pares A/B e também em A/A.
**A revisita não confirmou uma queda consistente de 4%–6%; não prova ausência
de regressão nem atribui causa ao writer.** Não aplicar uma correção de GET
baseada apenas naquele percentual histórico.

## Comparação e repetibilidade

Dataset inicial de 256 documentos × 1.024 bytes, mesmo SHA da coleta original:
`85fb688b70f90d073c5fe1bda93328937f85c4489435f2f9d90fda9aefa01ac4`.
Somente GET de documentos existentes; sort/query/mixed/escrita não entram na
janela. Por quantidade de clientes: quatro repetições × quatro modos = 16
rodadas, oito por versão, três segundos por rodada.

RPS, p50 e p95 abaixo são medianas entre oito rodadas, não percentis reunidos.
RPS conta apenas HTTP 200 com o documento completo correto.

| Clientes | RPS anterior / integrado | p50 anterior / integrado (ms) | p95 anterior / integrado (ms) |
| --- | --- | --- | --- |
| 1 | 3035.7 / 3093.3 | 0.3121 / 0.3072 | 0.4585 / 0.4458 |
| 8 | 22318.5 / 22484.1 | 0.3326 / 0.3306 | 0.4966 / 0.4933 |
| 32 | 18264.3 / 18401.4 | 1.6787 / 1.6713 | 2.3697 / 2.3350 |

Razões são calculadas por par e depois resumidas; não são divisões das medianas
acima. A/B sempre significa integrado/anterior. Min–max é amplitude observada,
não intervalo de confiança ou teto universal de ruído.

| Clientes | Pares A/B | Mediana RPS integrado/anterior | Min–max | Mediana p95 integrado/anterior |
| --- | --- | --- | --- | --- |
| 1 | 8 | 1.0173 | 0.9358–1.0460 | 0.9693 |
| 8 | 8 | 0.9920 | 0.7747–1.0564 | 1.0079 |
| 32 | 8 | 0.9943 | 0.9243–1.1128 | 1.0145 |

Controles A/A usam duas rodadas do mesmo executável, em cada repetição. As
rodadas são compartilhadas com A/B; não contar esses controles como amostras
independentes extras nem usar sua amplitude para demonstrar equivalência.

| Clientes | Controle | Pares | Mediana RPS | Min–max |
| --- | --- | --- | --- | --- |
| 1 | before_AA | 4 | 1.0068 | 0.9329–1.0153 |
| 1 | writer_AA | 4 | 1.0054 | 1.0000–1.0343 |
| 8 | before_AA | 4 | 1.0491 | 0.9890–1.0768 |
| 8 | writer_AA | 4 | 1.0313 | 0.9405–1.3283 |
| 32 | before_AA | 4 | 0.9748 | 0.9268–1.0288 |
| 32 | writer_AA | 4 | 1.0047 | 0.9849–1.0353 |

Exemplo preservado de dispersão: integrado, oito clientes, repetição 1/modo 1,
17.679,621 confirmações/s, p95 0,6227 ms e p99 4,335 ms. Foram 53.049 respostas
200 corretas; a rodada permanece no resumo. Nenhuma conclusão usa seleção de
pares favoráveis. Não há perfil que explique essa oscilação.

## Proveniência recuperada e preservação

[Manifesto de recuperação](./writer-read-artifacts-2026-11-26.json):

| Artefato | SHA-256 | Bytes | Recuperado de |
| --- | --- | --- | --- |
| Anterior | `0a7cfc9978237587c00b71dd2060b13ff33b4f95cc720ed3df3b1074228ce125` | 2970336 | `/tmp/kiarni-before-writer` |
| Integrado | `aec28f53b10ebc336e47227fa6fc1b78f344ac4b6841249608e7d14d55aa117e` | 3065640 | `target/release/deps/kiarni-8cdd944d79c91365` |

Os hashes coincidem exatamente com os dois JSON originais, não apenas nomes ou
versões. Cópias foram preservadas localmente em `.bench-artifacts/writer-2026-11-26/`,
fora de target e de /tmp, com acesso privado; pasta ignorada pelo Git. Binários
não foram publicados como release/pacote. O manifesto e os resultados são
versionados; **um checkout limpo precisa receber os executáveis com esses
hashes para reproduzir a medição**. Não prometer disponibilidade remota dos
artefatos ou reconstrução exata só a partir do commit antigo.

Ambos os registros originais apontavam para `674ac71` com workspace alterado,
com fingerprints de fonte diferentes. Recuperar os executáveis resolve a
lacuna do artefato, não transforma aquele checkout em fonte imutável de build.
Fingerprints originais estão em `artifacts` no novo JSON. O `git_head` atual
e o `source_sha256` do ambiente identificam o checkout/medidor da revisita,
**não** o código-fonte que gerou os binários históricos.

Medidor em `e26ba1106c263d1181fbb952ea7449d5e4d66702`, árvore limpa; fontes e
executáveis conferidos antes/depois. [JSON bruto](./writer-read-revisit-2026-11-26.json)
copiado sem alteração. Nenhum build/teste concorrente às medições.

## Método, ambiente e diferenças da medição original

Linux 7.2.6-76070206-generic x86_64, Ryzen 7 6800H, 16 CPUs lógicas disponíveis,
15.898.722.304 bytes de RAM, ext4/bloco 4.096 bytes. Rust/Cargo 1.99.1 e Python
3.12.3 no ambiente do medidor; executáveis históricos não foram recompilados.
Coleta UTC 2026-11-27, noite local de 2026-11-26 (America/Sao_Paulo).

- Cada modo usa instalação temporária nova e o mesmo seed, sem dados existentes.
  Configuração original: max_requests 32, max_streams 8, feed_queue 4,
  documento 1 MiB, query 16 KiB, resultado 4 MiB, timeouts 5 s, heartbeat 15 s.
- Não se adicionam flags recentes, como log-level, ao binário antigo. Mesmas
  opções originais nos dois artefatos; sem profiling opcional ou assinantes.
- Quatro modos: 0/2 anterior, 1/3 integrado. Ordem Williams, equilibrada em
  quatro repetições; cada par alterna sua ordem. São 24 pares A/B e controles
  A/A por versão. Clientes Python em processos, closed-loop, TCP novo/request.
- Warmup de cinco requests por cliente e barreira entre clientes precedem a
  janela. Latência inclui HTTP/decode/validação; processo/seed/conferência final
  ficam fora da janela. Validação JSON mais forte aumenta trabalho do cliente
  em relação à coleta antiga, igualmente nas duas versões.
- Porta loopback fixa entre instalações; sem alterar sysctls. Máximo TIME_WAIT
  observado: 14.150; delta de TCPTimeWaitOverflow, ListenOverflows e ListenDrops
  zero nos ciclos. TCPTimeouts global cresceu 83; não atribuir esse contador do
  host aos clientes medidos, que não registraram falha de transporte.
- Host compartilhado, sem fixar frequência/afinidade nem descartar caches.
  Não foram perfilados CPU/RSS, temperatura ou interferência do gerador. Janelas
  curtas e poucas repetições limitam conclusões de diferenças pequenas.

A medição original tinha três repetições de dois segundos e matrizes inteiras
sequenciais, com read/write/query/mixed e estado acumulado no mesmo servidor.
Agora há quatro repetições de três segundos, intercalação, somente leitura,
seed novo por membro e controle de porta. Mesmo binário/dataset/opções não
significa repetição bit a bit do experimento original. Não usar a revisita
para afirmar evolução de desempenho do produto atual (Store único formato 2).

## Reprodução

```bash
python3 benches/writer_history.py --before .bench-artifacts/writer-2026-11-26/before --writer .bench-artifacts/writer-2026-11-26/writer --output /tmp/writer-history-NOVO.json --clients 1 8 32 --repeat 4 --seconds 3 --warmup 5
```

Saída e journal devem ser novos; ambiente sem outras cargas de build/teste.
O medidor recusa hashes diferentes dos registros originais. [Metodologia](../../benches/README.md)
e [relatório original](./http-writer-2026-11-13.md) preservam o contexto.

## Atualização — coleta longa com afinidade

A [campanha seguinte](./writer-read-pinned-2026-11-27.md) usou dez segundos por
rodada e conjuntos de cores separados. Houve menor amplitude em alguns
controles, mas o gerador aproximou-se do orçamento de CPU em 32 clientes.
A próxima hipótese a remover é esse limite do cliente, antes de interpretar
a medição como capacidade do servidor. Preservadas as conclusões anteriores.

## Atualização — conclusão da #9

A [calibração e comparação finais com gerador Rust](./writer-read-rust-2026-11-28.md)
concluem a investigação, com margem de CPU no cliente. A queda histórica não
foi confirmada consistentemente; a intervenção justificada foi no ferramental,
sem patch especulativo no runtime. O estado abaixo preserva o histórico deste
incremento, não uma pendência atual de localização ou medição.

## Decisão e próximo incremento

Não alterar transporte, buffers, coordenador ou configuração pública para
corrigir uma queda de 4%–6% ainda não isolada. Menor passo justificável: reduzir
interferência/variabilidade do experimento com janelas mais longas e controles
do gerador/transporte antes de escolher uma intervenção no GET. Preservar o
histórico; não declarar o writer inocentado nem a regressão corrigida.

A issue #9 permanece aberta: a revisita com artefatos exatos foi executada, mas
a queda histórica não foi reproduzida de forma consistente e nenhuma mudança
Rust de desempenho foi homologada. Escalabilidade vertical de cores/RAM e
escala horizontal continuam nas etapas separadas já combinadas.

Verificações: fmt, clippy, build release do produto atual, 372 testes Rust
aprovados (13 workers exercitados pelos pais), 14 testes Python, links/inventários
e diff. Smoke dos binários históricos: oito rodadas curtas válidas, sem uso
como evidência de desempenho. Nenhuma mudança de API/formato persistido ou
nova dependência runtime nesta entrega.
