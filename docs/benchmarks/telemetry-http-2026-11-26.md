# Leitura, logs e leitores de telemetria — 2026-11-26

Primeiro incremento da issue #9, RF-09 / desempenho, após M3. **Não reproduz
nem resolve a regressão histórica do writer.** Mede o binário atual com logs
off/info e sem/com leitores dos endpoints de métricas e erros recentes.

64 rodadas válidas, quatro repetições por célula, **2.608.436 respostas
esperadas**: 1.293.875 GET 200 e 1.314.561 GET 404 document_not_found. Nenhuma
resposta inesperada, busy ou falha de transporte nessa matriz. Todos os 256
documentos foram conferidos após cada carga. Não há escrita na janela medida;
404 esperado não é operação confirmada 2xx nem evidência de durabilidade.

Com oito clientes, a mediana das razões pareadas info/off sem leitores foi
**0,9847 nas leituras** e **0,9722 nos 404**: reduções observadas de 1,53% e
2,78%. Com leitores, razões 0,9793 e 0,9795. Com um cliente, info ficou entre
5,2% e 7,4% acima de off nas medianas pareadas. Essa inversão impede interpretar
tudo como custo intrínseco do logger: escalonamento/frequência/gerador são
hipóteses, sem perfil para atribuição. Não afirmar que ligar logs acelera o banco.

Leitor a 2 Hz por endpoint (quatro requests/s) não apresentou perda consistente
em todos os pares. Isso não prova custo zero, comportamento em scrape intenso
ou ausência de contenção. Instrumentação fixa permaneceu **ativa em todos os
modos**; não se mediu o custo contra um servidor sem métricas/anel.

## Resultados completos da matriz válida

Cada linha reúne quatro rodadas de 3 segundos. RPS e p95 são medianas entre
rodadas; p95 não é percentil combinado. Requests usam conexão TCP nova e
processos clientes separados. `read` confere documento completo; `missing`
confere 404/código/request ID. Não comparar diretamente read versus missing
como custo do anel: respostas e trabalho de storage são diferentes.

| Clientes | Cenário | Logs | Leitor | Respostas esperadas/s | p95 (ms) |
| --- | --- | --- | --- | --- | --- |
| 1 | read | off | Não | 3123.8 | 0.4304 |
| 1 | read | info | Não | 3343.6 | 0.4004 |
| 1 | read | off | 2 Hz | 3114.4 | 0.4218 |
| 1 | read | info | 2 Hz | 3350.8 | 0.4031 |
| 1 | missing | off | Não | 3280.0 | 0.4081 |
| 1 | missing | info | Não | 3524.2 | 0.3824 |
| 1 | missing | off | 2 Hz | 3295.0 | 0.4009 |
| 1 | missing | info | 2 Hz | 3487.0 | 0.3876 |
| 8 | read | off | Não | 23884.5 | 0.4399 |
| 8 | read | info | Não | 23480.8 | 0.4477 |
| 8 | read | off | 2 Hz | 23371.7 | 0.4477 |
| 8 | read | info | 2 Hz | 23449.9 | 0.4510 |
| 8 | missing | off | Não | 24380.0 | 0.4308 |
| 8 | missing | info | Não | 23701.6 | 0.4537 |
| 8 | missing | off | 2 Hz | 24403.2 | 0.4296 |
| 8 | missing | info | 2 Hz | 23721.3 | 0.4478 |

Razões calculadas dentro da mesma repetição; não são divisão das medianas acima.
Intervalo mínimo–máximo dos quatro pares, sem intervalo de confiança. Valores
abaixo de 1 indicam redução na configuração à direita da comparação.

| Clientes | Cenário | Comparação (direita/esquerda) | Mediana | Min–max |
| --- | --- | --- | --- | --- |
| 1 | read | info/off sem leitor | 1.0718 | 1.0610–1.0834 |
| 1 | read | info/off com leitor | 1.0700 | 1.0495–1.0986 |
| 1 | read | com/sem leitor, off | 1.0067 | 0.9801–1.0142 |
| 1 | read | com/sem leitor, info | 1.0010 | 0.9889–1.0164 |
| 1 | missing | info/off sem leitor | 1.0744 | 1.0525–1.0972 |
| 1 | missing | info/off com leitor | 1.0518 | 1.0393–1.0829 |
| 1 | missing | com/sem leitor, off | 1.0017 | 0.9949–1.0245 |
| 1 | missing | com/sem leitor, info | 0.9952 | 0.9432–1.0226 |
| 8 | read | info/off sem leitor | 0.9847 | 0.9777–0.9920 |
| 8 | read | info/off com leitor | 0.9793 | 0.9643–1.0222 |
| 8 | read | com/sem leitor, off | 0.9964 | 0.9713–1.0163 |
| 8 | read | com/sem leitor, info | 0.9953 | 0.9897–1.0045 |
| 8 | missing | info/off sem leitor | 0.9722 | 0.9682–0.9768 |
| 8 | missing | info/off com leitor | 0.9795 | 0.9564–0.9886 |
| 8 | missing | com/sem leitor, off | 1.0009 | 0.9871–1.0058 |
| 8 | missing | com/sem leitor, info | 1.0080 | 0.9852–1.0086 |

No cenário missing com oito clientes, os contadores do anel somaram **1.081
perdas por contenção** nos 16 ciclos; com um cliente, zero. Substituição de
entrada antiga não é perda nesse contador. Logs info reportaram **47 descartes**
nesses ciclos; demais células, zero descartes reportados. Os contadores cobrem
o ciclo inteiro, incluindo warmup, e os logs incluem seed/verificação/shutdown.
`dropped_before` não garante contagem de perdas finais. São mecanismos
best-effort limitados, não auditoria; sem bloqueio por consumidor lento.

## Tentativas abortadas e limite do experimento

Duas tentativas completas anteriores foram interrompidas; **não entram nas
razões da matriz válida** e não foram combinadas com ela:

1. Harness `73a7548`, porta aleatória por servidor. 25 rodadas concluídas; falha
   de conexão na conferência pós-carga seguinte. O controle antigo registrou
   somente status 0, sem tipo/errno. [Log preservado](./telemetry-http-aborted-first-2026-11-26.log).
2. Harness `baf8311`, também porta aleatória. [Journal de 26 rodadas concluídas](./telemetry-http-aborted-progress-2026-11-26.jsonl).
   A rodada 26 teve 3.530 respostas 200 em 3,2003 s, p95 0,4514 ms e máximo
   3.053,8944 ms: não descartar esse outlier. A coleta seguinte abortou no leitor,
   com TimeoutError em socket.connect. [Log/trace preservado](./telemetry-http-aborted-second-2026-11-26.log).

Nenhum JSON final dessas matrizes foi produzido. O journal não tem o cabeçalho
completo de ambiente da execução abortada; não tratá-lo como baseline autônomo.
Smokes curtos de 16 rodadas serviram apenas à validação funcional do medidor.

Uma [coleta diagnóstica posterior de oito rodadas](./telemetry-http-random-port-diagnostic-2026-11-26.json),
harness `752355b`, registrou 598.936 respostas esperadas, sem falhas, mas estoque
TCP TIME_WAIT crescendo de 12 a **65.536**, o limite configurado, e **248.124**
novos TCPTimeWaitOverflow nos ciclos. Portanto, overflow não implica timeout em
cada rodada. Esses contadores globais não provam a causa dos abortos anteriores;
também incluem outras aplicações do host.

Ajuste apenas do medidor/helper: reutilizar uma porta loopback entre servidores
com instalações novas, mantendo conexão nova por request. Na matriz completa
com essa política, TIME_WAIT máximo observado foi **14.177** e o delta de
TCPTimeWaitOverflow, ListenOverflows e ListenDrops foi **zero** em todos os
ciclos. O contador global TCPTimeouts cresceu 96; como não houve falha de
transporte nos clientes medidos, não atribuir esse total ao workload nem
interpretá-lo como contador exclusivo do servidor. Não houve alteração de
sysctl, API, servidor Rust ou dados persistidos.
O helper conserva escolha de porta livre por padrão para seus outros usuários.
Não afirmar causalidade isolada sem ensaio controlado adicional; a nova coleta
é evidência do experimento com a política registrada, não reparo do produto.

## Ambiente e reprodução

Linux 7.2.6-76070206-generic x86_64, Ryzen 7 6800H, 16 CPUs lógicas disponíveis,
15.898.722.304 bytes de RAM. ext4, bloco 4.096 bytes. Rust/Cargo 1.99.1,
Python 3.12.3. Dataset 256 × 1.024 bytes, sem índice secundário, cache do engine
8 MiB; SHA do dataset e metadata constam do JSON. Host compartilhado, sem
fixar governor/afinidade ou descartar caches. Não mede cold-cache, CPU/RSS ou
capacidade fora desse perfil. Carga do host antes/depois está no JSON.

Mesmo binário release/default features, fonte Rust de produção `2790394`, SHA:
`18b8343941d02387446aa13e5d8f0560f797082ca0ae9f9cffb13a9e58c8243f`.
Harness final `95b1271be80f84406a9f64978c50ef60de776d8e`, árvore limpa;
[JSON bruto](./telemetry-http-2026-11-26.json) copiado sem alteração. Sem builds
e testes concorrentes às janelas medidas.

```bash
cargo build -p kiarni --release --locked --offline
# Matriz completa, no harness 95b1271:
python3 benches/telemetry_http.py --output /tmp/kiarni-telemetry-fixed-20261126.json --clients 1 8 --repeat 4 --seconds 3 --documents 256 --document-bytes 1024 --warmup 5 --reader-hz 2
# Diagnóstico anterior, no harness 752355b (porta aleatória por ciclo):
python3 benches/telemetry_http.py --output /tmp/kiarni-telemetry-diagnostic-20261126.json --clients 8 --repeat 1 --seconds 3 --documents 256 --document-bytes 1024 --warmup 5 --reader-hz 2
```

Usar caminhos novos. [Metodologia](../../benches/README.md): ordem Williams
balanceada em quatro repetições, nova instalação por modo, barreira pós-warmup,
leitor em processo separado com ticks perdidos sem rajadas, sem retry oculto.
stdout/stderr em arquivo temporário comum, não pipe/sink artificialmente lento.
Latência inclui HTTP/decode/validação. Saída bruta não contém logs/credenciais;
bytes/registros/perdas são agregados. Dados/journals privados durante coleta.

## Estado da issue #9 e próximo incremento

- Comparação off/info e sem/com leitores no produto atual documentada.
- Reprodução da queda histórica de 4%–6% **pendente**: binário anterior local
  tem SHA `0a7cfc9978237587c00b71dd2060b13ff33b4f95cc720ed3df3b1074228ce125`,
  mas não foi encontrado o integrado exato, SHA
  `aec28f53b10ebc336e47227fa6fc1b78f344ac4b6841249608e7d14d55aa117e`.
  Matrizes históricas eram sequenciais e fontes com workspace alterado.
  Comparar o produto atual ao antigo misturaria várias mudanças. Não declarar
  regressão reproduzida, resolvida ou causada pela telemetria.
- Nenhuma otimização Rust escolhida neste incremento. Próximo recorte: estabelecer
  comparação de leitura com proveniência suficiente para investigar a regressão,
  preservando o histórico; se o artefato exato não puder ser recuperado, registrar
  o limite da reprodução e definir baseline novo, sem fingir equivalência.
- Aproveitamento de cores/RAM segue a etapa específica de
  [escalabilidade vertical](../PERFORMANCE.md#escalabilidade-vertical--etapa-posterior-à-investigação-9),
  conforme combinado. Não é concluído por múltiplos clientes de loopback.

Verificações locais: build release, fmt, clippy, 372 testes Rust aprovados
(13 workers ignorados isoladamente e exercitados pelos pais), 12 testes Python,
links/inventários e diff. A tentativa Rust dentro do sandbox falhou por falta de
permissão para sockets; execução com loopback permitido passou. Nenhuma nova
dependência runtime, mudança de contrato público ou formato persistido.
