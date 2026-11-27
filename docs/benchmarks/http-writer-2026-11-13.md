# Integração HTTP do coordenador — 2026-11-13

O coordenador comum está ativo em todas as rotas mutantes. PUT/PATCH consecutivos
podem compartilhar commit Immediate, sem espera deliberada; inserções, exclusões
e catálogo são barreiras na mesma fila. Contratos HTTP e formato persistido
foram preservados. A escrita concorrente melhorou neste perfil, com regressão
de throughput de leitura que permanece registrada para investigação posterior.

## Atualização — revisita de 2026-11-26

Os dois executáveis exatos foram recuperados por hash. A
[revisita intercalada com controles A/A](./writer-read-revisit-2026-11-26.md)
registra diferenças medianas menores e dispersão importante; não confirmou
uma queda consistente de 4%–6% e não demonstrou ausência de regressão. As
matrizes e conclusões históricas abaixo são preservadas, sem atribuição causal.

## Comparação

Fontes: [binário anterior](./http-before-writer-2026-11-13.json),
[envio vetorizado inicial](./http-writer-vectored-2026-11-13.json) e
[integração final](./http-writer-2026-11-13.json).
O anterior foi compilado antes das alterações HTTP e copiado para
`/tmp/kiarni-before-writer`. Os relatórios identificam hashes de fonte/binário,
ambiente, configuração e dataset. Workspace local com alterações, sem CI remoto.

Cada matriz contém 36 rodadas: três repetições de 2 segundos, cenários read,
write, query e mixed, com 1/8/32 processos clientes. Dataset de 256 documentos
de 1 KiB, em cache, loopback, conexão nova por operação e durabilidade Immediate.
Mixed é 80% GET e 20% PUT. Ambiente Linux/ext4, Ryzen 7 6800H, 16 CPUs lógicas,
Rust/Cargo 1.99.1. Não houve builds ou testes concorrentes às medições.
As matrizes foram sequenciais, não intercaladas; rodadas curtas e estado térmico
do computador limitam a atribuição causal de diferenças pequenas.

Valores são medianas entre três repetições; p95 é a mediana dos p95 por rodada,
não um percentil de todas as amostras reunidas. Throughput conta confirmações.

| Cenário | Clientes | Antes, ops/s | Depois, ops/s | p95 antes | p95 depois |
|---|---:|---:|---:|---:|---:|
| read | 1 | 3158 | 3168 | 0,414 ms | 0,419 ms |
| read | 8 | 25695 | 24164 | 0,406 ms | 0,436 ms |
| read | 32 | 21548 | 20617 | 1,969 ms | 2,025 ms |
| write | 1 | 593 | 619 | 2,909 ms | 2,862 ms |
| write | 8 | 823 | 3308 | 11,749 ms | 3,769 ms |
| write | 32 | 805 | 6873 | 44,285 ms | 6,498 ms |
| query | 1 | 599 | 624 | 2,175 ms | 2,129 ms |
| query | 8 | 866 | 894 | 10,700 ms | 10,357 ms |
| query | 32 | 887 | 917 | 39,566 ms | 37,906 ms |
| mixed | 1 | 1559 | 1525 | 1,505 ms | 1,586 ms |
| mixed | 8 | 4479 | 10753 | 8,308 ms | 3,132 ms |
| mixed | 32 | 4302 | 15164 | 36,772 ms | 5,177 ms |

Em 32 clientes, write cresceu **8,54 vezes** e mixed **3,53 vezes**. O p95 de
write caiu de 44,3 para 6,5 ms. Com um cliente, write permaneceu na mesma ordem
de grandeza, sem espera artificial para formar grupos.

Read caiu **5,96% com 8 clientes** e **4,32% com 32**, enquanto mixed com um
cliente caiu 2,24%. Esses resultados não são omitidos nem tratados como ganho.
O primeiro envio vetorizado em todas as respostas também apresentou queda de
leitura. O caminho final restaura write comum para respostas de até 4 KiB e
mantém write_vectored para corpos maiores, mas não eliminou a diferença.
Não foi demonstrado que writev, o worker ou a stack são a causa. A investigação
de CPU e memória permanece para a etapa combinada, sem alegação de liderança.

O baseline confirmou 394640 operações; a integração final, **531243**. Em ambas
as matrizes, não houve rejeições, falhas de transporte ou escritas de resultado
desconhecido. Os documentos das escritas confirmadas foram verificados pelo
harness. A matriz intermediária foi preservada, sem substituir a comparação final.

## Correção e recursos

A conexão mantém sua permissão de admissão durante leitura, execução e envio.
O comando compartilha essa permissão via Arc, protegendo trabalho que sobreviva
ao descarte do handler. O Reply permanece vivo enquanto sua cópia HTTP é enviada.
Fechamento cancela comandos ainda não iniciados; grupos iniciados concluem.
O writer termina antes da liberação da raiz de dados. Nenhuma fila é ilimitada.

Sem cópia integral de wire para corpos grandes, o plano por request passou de
4 MiB para 3 MiB, conservando as reservas de resultado/query. Foram contabilizados
768 KiB de stack, 64 KiB de metadados fixos e 1 KiB por request. A desigualdade
`832 KiB + N KiB <= N MiB`, para N>=1, mantém o novo plano abaixo do antigo.
O teto anterior de 512 MiB e as validações de configuração continuam verificados.
O plano usa o teto persistido de 1 MiB mesmo quando a escrita aceita menos.
Isso é reserva de buffers explícitos, não limite superior de RSS.
[Fórmula e propriedade dos buffers](../../crates/server/README.md).

Os quatro testes adicionais cobrem plano em N=1..128, profundidade JSON máxima
com corpo próximo de 1 MiB em PUT/PATCH na stack configurada, permissão retida
após descarte do handler e 32 clientes TCP alterando o mesmo ID. Cada cliente
recebe seu próprio estado; consultas/health avançam durante a carga e todas as
alterações confirmadas sobrevivem ao restart. HEAD, exclusão, drop/recreate e
reinserção também são verificados. A suíte anterior mantém os testes de erro,
no-op, corpo incompleto, limites reduzidos, shutdown e crash do storage.

## Reprodução e verificações

```bash
cargo build -p kiarni --release --locked --offline
python3 benches/http_load.py --output /tmp/http-writer.json
```

O arquivo deve ser novo. Para comparar versões, preserve os dois executáveis e
passe `--binary /caminho/do/binario`. O baseline exige checkout/código anterior
à integração; o binário temporário citado acima não é artefato distribuído.
Regras completas: [metodologia](../../benches/README.md).

Passaram fmt, clippy, fronteiras de dependências e 201 testes Rust, com dois
workers de fixture ignorados e acionados pelos testes pais. A suíte também foi
executada com o feature opcional de profiling. Esse feature ainda instrumenta
fases do storage individual, não commits em grupo; não se devem usar seus
contadores de commit para interpretar o novo caminho HTTP. As matrizes foram
executadas sem instrumentação.

EIO, disco cheio, falha física, energia, datasets fora do cache e estabilidade
prolongada não foram simulados nesta integração. Changefeeds continuam fora
deste incremento e exigem retenção de estados e ordenação pós-commit próprias.
