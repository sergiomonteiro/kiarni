# Perfil de PUT e potencial de agrupamento — 2026-11-12

Investigação local do M1/RF-02. O servidor continua com um commit Immediate por
mutação. O agrupamento existe somente em `spikes/group-commit`; o ADR 0016
permanece em experimento. Nenhuma API transacional foi adicionada ao produto.

## Perfil do servidor

Fonte: [relatório de PUT instrumentado](./http-write-profile-2026-11-12.json).
Linux, Ryzen 7 6800H, 16 CPUs lógicas, ext4, Rust 1.99.1; detalhes e hashes no
JSON. Uma rodada de 2 segundos para cada concorrência de 1, 8 e 32 clientes,
256 documentos de 1 KiB. Os contadores incluem 4398 operações medidas e 205
de warmup, totalizando 4603 PUTs. Todas as operações medidas retornaram 200.

| Fase | Parede média por PUT | CPU da thread média por PUT |
|---|---:|---:|
| Validação HTTP | 17,87 µs | 15,68 µs |
| Espera de admissão | 8212,58 µs | 15,82 µs |
| Início da transação | 7,40 µs | 5,80 µs |
| Lookup | 11,09 µs | 9,58 µs |
| Transformação | 45,31 µs | 43,41 µs |
| Staging | 12,21 µs | 10,71 µs |
| Commit | 1090,55 µs | 221,52 µs |

O commit domina as fases de serviço observadas e a fila cresce sob concorrência.
Isso justifica investigar amortização da sincronização. As médias agregam as
três concorrências; não são percentis nem um perfil completo do processo.
Não foram medidas alocações, parsing de todas as rotas ou custo total de threads.

Uma rodada separada com `strace -D -f -c -w -e trace=fdatasync,fsync,pwrite64`
registrou 981 `fdatasync` (1,005 s), 6796 `pwrite64` (0,103 s) e 29 `fsync`
(0,045 s). `fdatasync` foi 87,18% **do tempo das chamadas selecionadas**,
incluindo inicialização e seed. Não é porcentagem do tempo total do programa.
O tracing altera o desempenho; seus 352,71 PUT/s não são resultado de capacidade.
Fontes: [resumo de syscalls](./http-write-strace-2026-11-12.txt) e
[rodada rastreada](./http-write-strace-2026-11-12.json).
O hash de binário dessa rodada identifica o wrapper de strace; o executável
instrumentado usado pelo wrapper é o do relatório de PUT, SHA-256
`7e3350fb2125f27cf5a55cc143218acd74afedb5dd25488a0e85254882f525f8`.

## Experimento isolado

Fonte: [grupos pré-formados](./group-commit-2026-11-12.json). Cada célula faz
4096 substituições, com 256 documentos de 1 KiB, cache de 8 MiB, três
repetições e commit Immediate por grupo. Entradas e grupos são preparados antes
da medição. Cada repetição usa raiz nova e verifica todos os documentos após
reabertura. As nove verificações passaram, sem rejeições ou no-ops.

| Operações por commit | Mediana de operações confirmadas/s | Faixa das repetições | Mediana do p95 do grupo |
|---|---:|---:|---:|
| 1 | 1398 | 711–1476 | 0,951 ms |
| 8 | 8221 | 3711–8607 | 1,406 ms |
| 32 | 18019 | 10777–24422 | 2,249 ms |

Grupos de 32 produziram aproximadamente 12,9 vezes a mediana de operações
por segundo dos grupos de 1 neste microbenchmark. As repetições curtas exibem
variação grande; isso é evidência de potencial, insuficiente para meta de release.
O p95 mede o grupo já pronto: não inclui espera de coleta. Não há HTTP,
autenticação, múltiplos clientes ou publicação de eventos. O resultado não pode
ser usado como throughput da plataforma nem comparado diretamente ao benchmark HTTP.

A [primeira execução](./group-commit-shared-host-2026-11-12.json) coincidiu
com testes e compilação. Ela foi preservada para auditoria e excluída da tabela;
a execução da tabela ocorreu depois da conclusão desses processos.
Ambas medem um workspace local com alterações, sem validação de CI remota.

## Correção e reprodução

O spike limita grupos a 32 operações e 8 MiB de corpos, valida documentos com
o engine existente e distingue rejeição individual de erro transacional.
Recibos só retornam após commit ou aborto de no-op. Erros após commit retornam
resultado incerto. Testes demonstram snapshot anterior ao commit, rollback,
ordem no mesmo ID, limites, recusa de raiz estrangeira e kill/restart atômico
antes/depois da confirmação. Não foi simulada queda de energia ou falha física.

```bash
cargo build -p kiarni --release --features kiarni-server/profiling --locked --offline
python3 benches/http_load.py --scenarios write --repeat 1 --seconds 2 --output /tmp/put-profile.json
cargo build -p kiarni-group-commit-spike --release --locked --offline
python3 benches/group_commit.py --output /tmp/group-commit.json
cargo build -p kiarni --release --locked --offline
```

Para reproduzir o tracing, use um wrapper executável que invoque
`exec strace -D -f -c -w -e trace=fdatasync,fsync,pwrite64 -o SUMMARY.txt /absolute/path/to/target/release/kiarni "$@"`;
passe esse wrapper em `http_load.py --binary` junto de
`--scenarios write --clients 1 --repeat 1 --seconds 2 --output REPORT.json`.
O `-D` preserva o PID do servidor para o encerramento pelo harness.
Use caminhos novos para os relatórios. O último build restaura o binário sem
instrumentação, que permanece desabilitada na configuração padrão.

Próximo incremento: avaliar coleta com fila, bytes e janela explicitamente
limitados, sob clientes concorrentes, medindo latência completa e recuperação.
A integração de CRUD completo e eventos ainda exige decisão e testes próprios.
