# Baseline HTTP — 2026-11-12

Foram executadas 36 rodadas com 1, 8 e 32 processos clientes, três repetições
por cenário, cinco tentativas de warmup por cliente e janela medida de dois
segundos por rodada. O dataset contém 256 documentos de 1024 bytes e cabe no
cache. O servidor e os clientes rodam no mesmo computador, em loopback.
[Dados completos](./http-baseline-2026-11-12.json) ·
[Harness e metodologia](../../benches/README.md)

## Ambiente e reprodução

- AMD Ryzen 7 6800H, 16 CPUs lógicas, aproximadamente 15,9 GB de memória física.
- Linux 7.2.6-76070206-generic, x86_64, dados temporários em ext4.
- rustc 1.99.1, Cargo 1.99.1, Python 3.12.3, binário release.
- Perfil explícito de 32 conexões, oito vagas de stream e limites do servidor
  registrados no JSON. A query tem uma vaga global no executor atual.
- HEAD 674ac71, workspace com mudanças locais. Hashes do binário, fontes e
  dataset estão no artefato. A execução em CI remoto/macOS permanece pendente.

```bash
cargo build -p kiarni --release --locked --offline
python3 benches/http_load.py --output docs/benchmarks/http-repro.json
```

Use um nome de saída novo em cada reprodução. Nenhuma credencial é gravada no
relatório. O servidor temporário recebe SIGTERM ao concluir. Os diretórios do
dataset e da entrega de credenciais são removidos pelo contexto temporário.

## Resultados

Confirmadas/s e percentis abaixo são as medianas das três rodadas. A coluna
503 usa a proporção agregada de tentativas das três rodadas. Os percentis são
calculados somente sobre sucessos; as latências das falhas também estão no JSON.
Os percentis por rodada permanecem disponíveis para inspeção.

| Clientes | Cenário | Confirmadas/s | 503 (%) | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---|---|---|---|---|---|
| 1 | read | 39.51 | 0.00 | 25.264 | 25.768 | 26.026 |
| 1 | write | 39.51 | 0.00 | 25.189 | 25.960 | 28.704 |
| 1 | query | 39.53 | 0.00 | 25.222 | 26.033 | 26.636 |
| 1 | mixed | 39.48 | 0.00 | 25.259 | 27.228 | 27.898 |
| 8 | read | 212.00 | 32.23 | 25.596 | 26.012 | 26.195 |
| 8 | write | 39.12 | 87.50 | 26.556 | 26.947 | 30.645 |
| 8 | query | 39.09 | 87.50 | 27.106 | 27.608 | 27.779 |
| 8 | mixed | 179.28 | 43.93 | 25.579 | 26.259 | 27.455 |
| 32 | read | 19623.41 | 7.99 | 1.425 | 1.922 | 2.397 |
| 32 | write | 1030.79 | 94.51 | 2.510 | 3.244 | 3.830 |
| 32 | query | 671.98 | 96.40 | 3.120 | 3.952 | 4.646 |
| 32 | mixed | 4431.55 | 81.59 | 1.557 | 2.629 | 3.059 |

Houve 476.880 tentativas, 147.904 confirmações e 328.976 respostas 503.
Os códigos de erro foram write_busy e service_unavailable. Nenhuma falha de
transporte ou escrita de resultado desconhecido foi registrada. As últimas
escritas confirmadas de cada cliente ficaram visíveis na verificação posterior.
Os resultados das queries coincidiram com a seleção esperada.

O cenário mixed com 32 clientes variou de 2.396 a 4.505 confirmações/s, e read
variou de 15.630 a 20.299. As rodadas são curtas e compartilham CPU com o gerador
de carga. Esses intervalos devem acompanhar qualquer uso dos valores medianos.

## Diagnóstico e próximo experimento

A latência de aproximadamente 25 ms com um cliente coincide com a espera fixa
de 25 ms no loop de accept quando o listener retorna WouldBlock. Sob tráfego
contínuo com 32 clientes, o backlog pode manter o loop ativo e evitar essa
espera. Essa é a hipótese principal para a diferença entre os perfis de carga.
A causalidade deve ser confirmada com uma alteração isolada e nova medição.

O mutex do store é adquirido com try_lock inclusive em leituras. Contenção
retorna write_busy imediatamente. A query também exige sua vaga global e
retorna service_unavailable quando ocupada. O alto percentual de 503 em
write/query mostra o custo dessa política na carga concorrente.
[Servidor medido](../../crates/server/src/http.rs)

O próximo incremento deve trocar a espera fixa de accept por espera de eventos
de I/O, preservando o encerramento por sinal e os limites de admissão. Repetir
o mesmo benchmark permitirá comparar binários e confirmar o impacto. Depois,
avaliar a coordenação das leituras e escritas com novas evidências, preservando
ordem de commit, limites de memória e a política de backpressure.

Os números descrevem este dataset, hardware e configuração. O benchmark usa
uma conexão por requisição e um modelo closed-loop. Novas medições são necessárias
para duração prolongada, carga de chegada independente, datasets maiores e
rede remota. A taxa de tentativas inclui rejeições e deve ser analisada junto
da taxa de confirmações.

## Verificações

O build release, o ensaio curto e a medição completa passaram. Os três testes
do harness verificam tamanho de documentos, nearest-rank e exclusão de falhas
do throughput confirmado. O CI executa esses testes de medição, enquanto a
coleta de desempenho permanece uma execução explícita.

Os 155 testes Rust passaram. Fmt, clippy, checagem de dependências e
git diff --check também passaram. O hash das fontes no artefato foi conferido
contra o workspace após a medição, e os contadores de status foram validados.
