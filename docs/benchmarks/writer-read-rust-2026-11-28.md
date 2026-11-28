# Conclusão da investigação #9 — 2026-11-28

Requisito não funcional de desempenho e RF-09, após M3. A investigação conclui
que a queda histórica de leitura de 4%–6% **não foi confirmada consistentemente
nos ensaios controlados**, mesmo com os dois executáveis originais e depois de
retirar o limite de CPU observado no gerador Python. Isso não prova ausência
de regressão em toda condição e não transforma a observação original em erro.

**Decisão:** manter o código do servidor, seu coordenador e suas garantias;
a menor intervenção justificada foi corrigir o ferramental de medição. O novo
gerador Rust tem validação equivalente para o fixture, limites explícitos,
controle A/A e margem de CPU. Não aplicar um patch especulativo no GET para
obter ganho numérico. A #9 pode ser encerrada como **investigação concluída**,
sem anunciar uma correção de bug ou aumento de capacidade do Kiarni.

## Evidência final

- Calibração: **16 rodadas A/A**, mesmo binário integrado em todos os modos,
  32 clientes, quatro repetições de cinco segundos, **2.445.898 leituras corretas**.
- Comparação: **32 rodadas A/B**, oito e 32 clientes, quatro repetições de dez
  segundos, **9.658.070 leituras corretas**. Inclui controles A/A por versão,
  compartilhando as mesmas rodadas. Não somar controles como amostras extras.
- Todas as 48 rodadas válidas; sem resposta incorreta, recusa, falha de transporte,
  retry ou descarte de outlier. JSON completo validado em todos os GET/warmups e
  todos os documentos conferidos após a carga. Somente leitura na janela.
- Anterior/integrado têm os mesmos SHA-256 dos registros originais; nenhum
  executável foi recompilado para representar o passado. O produto atual de
  formato 2 não é usado como substituto do histórico.

### Calibração A/A do gerador

Razão mediana entre rodadas idênticas: **0,99958**, min–max **0,97813–1,01787**,
oito pares. CPU total do gerador/tempo medido: mediana **2,137**, máximo **2,177**,
de oito CPUs lógicas atribuídas. Há margem no orçamento de CPU em média;
não se demonstrou ausência de toda interferência ou de rajadas locais.

### Comparação integrado/anterior

Razões de throughput são calculadas por par e depois resumidas. Min–max é
amplitude observada, não intervalo de confiança ou teste de equivalência.

| Clientes | Pares A/B | Mediana RPS integrado/anterior | Min–max | Mediana razão p95 |
| --- | --- | --- | --- | --- |
| 8 | 8 | 0.99692 | 0.95177–1.00919 | 1.01122 |
| 32 | 8 | 1.00013 | 0.98854–1.01686 | 1.00029 |

Com oito clientes houve um par com razão 0,95177, que permanece no resumo.
A mediana foi 0,99692 (−0,31%); com 32 clientes, 1,00013 (+0,01%). Não chamar
essas diferenças pequenas de ganho/perda intrínsecos do writer. Não se confirmou
um deslocamento consistente de 4%–6%, nem se identificou causa naquela integração.

RPS/percentis abaixo são medianas de oito rodadas por célula, não percentis
combinados de todas as amostras. CPU/tempo é apenas do gerador, com escopo amplo.

| Clientes | Artefato | Leituras/s | p50 (ms) | p95 (ms) | p99 (ms) | CPU gerador / tempo |
| --- | --- | --- | --- | --- | --- | --- |
| 8 | before | 30334.5 | 0.25217 | 0.34266 | 0.48826 | 2.093 |
| 8 | writer | 30222.7 | 0.25260 | 0.34754 | 0.49455 | 2.100 |
| 32 | before | 30138.8 | 1.03526 | 1.25050 | 1.47456 | 2.105 |
| 32 | writer | 30110.4 | 1.03451 | 1.24971 | 1.46918 | 2.114 |

CPU máxima do gerador/tempo entre todas as rodadas A/B: **2,117** de oito CPUs
lógicas. A medida inclui launcher, aquecimento e resumo, além da janela: é
conservadora para verificar margem. No teste Python anterior, os workers
chegaram a 7,94/8 medidos só na janela; os escopos não são idênticos. Não declarar
uma redução percentual exata de CPU ou atribuir mais RPS a melhoria do servidor:
os artefatos de servidor são os mesmos, o gerador mudou.

### Controles A/A dentro da comparação

| Clientes | Controle | Pares | Mediana RPS | Min–max |
| --- | --- | --- | --- | --- |
| 8 | before_AA | 4 | 0.99555 | 0.98594–1.01762 |
| 8 | writer_AA | 4 | 1.00686 | 0.98855–1.02199 |
| 32 | before_AA | 4 | 0.98856 | 0.98546–0.99808 |
| 32 | writer_AA | 4 | 0.99565 | 0.98671–1.00591 |

## Menor intervenção adotada

[read-load](../../apps/kiarni/examples/read_load.rs) é exemplo de desenvolvimento,
compilado separadamente, usando serde_json já presente nas dependências de teste.
Nenhuma dependência nova ou serviço obrigatório no runtime. Não muda API,
formato persistido, limites do servidor, ordem pós-commit ou backpressure.

- Um processo com até 32 threads fixos, stack de 256 KiB por thread. Até 100000
  tentativas por cliente; entrada stdin até 8 MiB, 1024 documentos de 4096 bytes,
  headers de 8192 bytes, resposta de 64 KiB e prazo total de cinco segundos por
  request. O limite de corpo é específico do fixture, não contrato da API.
- Conexão TCP nova por GET, requests pré-compostos e validação JSON completa.
  Sem código arbitrário do cliente, escrita ou retry. Content-Length estrito;
  framing ambíguo, truncamento e documento incorreto invalidam a resposta.
- Token entra por stdin, nunca nos argumentos/resultado. Saída só com métricas
  agregadas. Falha de aquecimento invalida e libera as threads, sem deadlock.
- Execução sob as mesmas máscaras temporárias e inspeção do hash do gerador
  antes/depois. Aplicação do banco permanece com afinidade normal por padrão.

A CPU é obtida pelo pai via RUSAGE_CHILDREN entre iniciar e aguardar o gerador.
O servidor ainda está em execução e não foi reaped nessa etapa, portanto não
entra na diferença. Inclui todas as threads, launcher, warmup, carga e resumo;
não é perfil de CPU/RSS do servidor nem prova de uso de cores do produto.

## Ambiente, método e limites

Linux 7.2.6-76070206-generic, Ryzen 7 6800H, oito cores físicos/16 threads,
15.898.722.304 bytes de RAM; ext4/bloco 4096 bytes. Python 3.12.3 e Rust/Cargo
1.99.1 no ambiente. Controller com CPUs 0–15, servidor 0–7 (cores físicos 0–3),
gerador 8–15 (cores físicos 4–7), SMT em pares. Todas as máscaras observadas
coincidiram com o solicitado. Não há reserva contra outras aplicações.

Governor powersave, frequência/temperatura não fixadas, host compartilhado,
closed-loop e loopback. Somente o fixture 256 × 1 KiB em cache, com 100 requests
de aquecimento por cliente. Sem comparar bancos externos, dados fora do cache,
tráfego de rede real ou escala horizontal. Quantidade de CPUs não foi variada;
a avaliação ampla de escalabilidade vertical continua separada.

As coletas históricas de 2026-11-13 eram matrizes sequenciais, três repetições
de dois segundos e cenários misturados. As atuais intercalam versões, usam seed
novo por membro, porta fixa, validação mais forte e gerador diferente. O cliente
Rust usa TcpStream padrão; não força TCP_NODELAY como http.client. São GETs
pré-compostos em conexões novas. Não é repetição bit a bit da geração antiga;
a comparação final mantém o mesmo cliente nos dois lados.

A/B: sem novos TCPTimeWaitOverflow, ListenOverflows ou ListenDrops nos ciclos;
TIME_WAIT máximo 26.419. TCPTimeouts global cresceu 58, sem erro de transporte
nos clientes medidos: não atribuir esse contador global ao workload. Metadados
de topologia, frequência pontual e loadavg estão nos JSON, sem médias inferidas.

## Proveniência e reprodução

Fonte do gerador/medidor: `dbd188ffce774f1e4d8942ce5a9804908dc52196`, árvore limpa.
Hash do gerador release:
`25a5e27784463fe5de6804424957fa8dd28ecb636ce9117a8f55337d0a4b2fc2`.
Fontes e executáveis conferidos antes/depois. Nenhum build/teste local durante
as medições. Os fingerprints de checkout identificam o medidor, não o código
que gerou os binários históricos em workspace então alterado.

- [Calibração A/A bruta](./read-generator-aa-2026-11-28.json).
- [Comparação A/B e controles brutos](./writer-read-rust-2026-11-28.json).
- [Manifesto dos executáveis históricos](./writer-read-artifacts-2026-11-26.json).

Os JSON foram copiados sem alteração. Artefatos históricos preservados localmente
em .bench-artifacts/writer-2026-11-26, não publicados como release. Checkout novo
precisa recebê-los com os hashes do manifesto; o medidor recusa outros binários.

```bash
cargo build -p kiarni --example read-load --release --locked --offline
python3 benches/writer_history.py --before .bench-artifacts/writer-2026-11-26/before --writer .bench-artifacts/writer-2026-11-26/writer --rust-driver target/release/examples/read-load --aa-only --output /tmp/writer-rust-AA-NOVO.json --clients 32 --repeat 4 --seconds 5 --warmup 100 --server-cpus 0 1 2 3 4 5 6 7 --client-cpus 8 9 10 11 12 13 14 15
python3 benches/writer_history.py --before .bench-artifacts/writer-2026-11-26/before --writer .bench-artifacts/writer-2026-11-26/writer --rust-driver target/release/examples/read-load --output /tmp/writer-rust-AB-NOVO.json --clients 8 32 --repeat 4 --seconds 10 --warmup 100 --server-cpus 0 1 2 3 4 5 6 7 --client-cpus 8 9 10 11 12 13 14 15
```

Saídas/journals devem ser novos. [Metodologia](../../benches/README.md).

## Conclusão dos quatro itens da #9

1. **Investigação da queda concluída:** tentativa de reprodução com artefatos
   exatos, comparação controlada e gerador com margem; resultado não confirmado
   consistentemente. Não marcar a queda como reproduzida nem como bug corrigido.
2. **Telemetria medida:** [off/info e leitores de métricas/erros](./telemetry-http-2026-11-26.md),
   com resultados, perdas best-effort e limites publicados. Não promete overhead
   universal ou custo isolado da instrumentação contra servidor sem métricas.
3. **Medições rastreáveis:** histórico, tentativas abortadas, dados brutos,
   datasets, hashes, comandos, ambiente, latências e recusas preservados.
4. **Intervenção mínima decidida e validada:** substituir o gerador de leitura
   saturado e manter o runtime. Não há justificativa para um patch especulativo
   no GET; filas, limites, confirmação/pós-commit e feeds permanecem preservados.

O fechamento é da investigação, com resultado negativo para a hipótese histórica,
sem alegação de inexistência de regressão em outras condições. Nova evidência
reprodutível pode motivar nova investigação. Não repetir indefinidamente o mesmo
ensaio nem bloquear o roadmap tentando obter um percentual predeterminado.

Verificações locais: 379 testes Rust aprovados (incluindo sete do gerador),
18 testes Python, fmt/clippy, links/inventários e fronteiras de dependências.
Smokes curtos só validaram a ferramenta, sem uso como evidência de capacidade.
A CI final será registrada na issue antes do encerramento.
