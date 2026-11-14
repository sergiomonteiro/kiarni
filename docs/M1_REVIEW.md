# Revisão do gate M1

Data da revisão: 2026-11-11.

**Decisão: M1 aprovado no workspace local em Linux.** O servidor implementa o
recorte de banco funcional e os testes exercitam múltiplas conexões independentes,
concorrência e recuperação após encerramento abrupto. O gate autoriza a entrada
nos incrementos do M2. [PRD, §10](../KIARNI_PRD_v0.1.md)

## Evidências

| Recorte do M1 | Evidência |
|---|---|
| RF-01, configuração e startup | CLI/env/arquivo/defaults, diretórios privados, lock exclusivo e fases de retomada. Testes de precedência, segundo processo recusado e retomada de store/auth. |
| RF-02, coleções e documentos | CRUD HTTP autenticado, IDs explícitos/gerados, conflitos, paginação, PUT/Merge Patch e no-op. Testes de erros e preservação após restart. |
| RF-03, consultas básicas | AST validada, filtro, sort, limit, project e lookup por ID. Fixtures determinísticas, comparação decimal e consultas após kill/restart. |
| RF-04, índice primário | Chave composta de identidade da coleção e _id, unicidade na transação e lookup direto para leitura e consulta com id. Scan limitado para as demais consultas. |
| RF-06, recorte HTTP | Endpoints /v1, request IDs, erros estáveis, liveness/readiness autenticadas, limites de conexão, corpo e prazos de I/O. |
| RF-07, perfil local | Bind loopback, Bearer, entrega privada e comandos offline de rotação/revogação. Testes de recusa e recuperação após expiração. |
| Gate multi-cliente | Clientes em conexões TCP distintas disputam a criação de coleção/documento. Um vencedor confirma e os demais recebem conflito ou contenção explícita. Patches concorrentes preservam ambos os campos no storage. |
| Gate de recuperação | Processo real recebe kill após respostas confirmadas. Coleções, documentos, updates, exclusões e queries preservam o estado após restart. Falha injetada antes de commit mantém o estado anterior. |

Os testes HTTP ficam em apps/kiarni/tests/serve.rs. Contratos de biblioteca
acompanham os crates. A checagem de dependências confirma a separação entre
produto e experimentos. [Testes HTTP](../apps/kiarni/tests/serve.rs) ·
[Storage](../crates/storage/src/lib.rs) ·
[Organização](./adr/0014-organizacao-do-workspace.md)

## Ajuste encontrado na revisão

Readiness passa a executar uma leitura atual dos metadados e tabelas do store.
Contenção ou falha retorna 503/storage_unavailable. Liveness mantém sua
verificação de processo. Os novos testes verificam contenção, recuperação após
liberação e detecção de schema indisponível depois da abertura.

## Verificação

| Comando | Resultado local |
|---|---|
| cargo fmt --all -- --check | passou |
| cargo clippy --workspace --all-targets --locked --offline -- -D warnings | passou |
| cargo test --workspace --locked --offline | 155 testes Rust passaram |
| python3 scripts/check_product_dependencies.py | passou para oito pacotes do produto |
| git diff --check | passou |

Os testes TCP executaram fora do sandbox. CI remoto e macOS aguardam execução.
Os resultados se referem ao workspace com mudanças locais. HEAD continua
674ac71 e há arquivos ainda não rastreados. A vinculação a um commit de revisão
ou release depende do versionamento desse conjunto.

## Capacidade e desempenho

O produto admite múltiplas conexões com vagas limitadas. Os defaults são
32 conexões, uma consulta ativa e serialização de escritas pelo coordenador.
A consulta captura os documentos em snapshot antes de executar filtro e sort.
O scan tem orçamento de 10.000 documentos e 8 MiB de JSON. Esses limites e a
reserva de buffers descrevem o perfil operacional atual.

O primeiro baseline HTTP foi executado com 1, 8 e 32 processos clientes e três
repetições por cenário. A medição identificou latência próxima de 25 ms em
baixa concorrência e rejeições por contenção em escrita/query. Os dados e os
comandos reproduzíveis orientam o próximo incremento de otimização.
[Baseline HTTP](./benchmarks/http-baseline-2026-11-12.md)

## Verificações dos marcos seguintes

M2 integra dispatcher pós-commit, assinaturas de coleção, backpressure e
métricas. Os testes do produto devem demonstrar ordem de eventos, dois clientes,
consumidor lento, cancelamento, reconexão e shutdown.

M3 entrega dashboard, distribuição, backup/restauração operacional e exposição
remota segura. A recuperação testada no M1 cobre falhas de processo e os
checkpoints descritos. Queda de energia, falha física, disco cheio e ambientes
de proxy/TLS exigem verificações próprias.

A coordenação foi evoluída após a revisão para snapshots concorrentes e espera
FIFO limitada. [ADR 0015](./adr/0015-coordenacao-concorrente.md)

Em 2026-11-13, a escrita HTTP passou ao coordenador do
[ADR 0017](./adr/0017-integracao-coordenador-de-escrita.md), com grupos PUT/PATCH,
barreiras de CRUD e reservas compartilhadas com o transporte. Passaram 201
testes locais. A comparação mostra ganho de escrita e regressão de leitura
de 4%–6% em concorrência, registrada para investigação posterior. Essa evolução
não amplia as garantias de crash para falha física ou validação de outras plataformas.

O M2 já integra `crates/feeds` ao writer com publicação pós-commit, estados
intermediários e barreiras de inscrição. SSE autenticado e testes de streams TCP
já permitem a alpha reativa local. [Guia de uso](./ALPHA_LOCAL.md).
[ADR 0018](./adr/0018-dispatcher-do-produto.md).
Investigações fora do MVP ficam em [FUTURE_RESEARCH.md](./FUTURE_RESEARCH.md),
incluindo o spike futuro de escalabilidade horizontal solicitado pelo usuário.

O recorte de métricas dos feeds foi integrado em `/v1/metrics`, com autenticação,
filas/assinaturas, publicação, encerramentos e histograma confirmação→fanout.
Passaram 234 testes locais. O [gate M2 foi aprovado localmente](./M2_REVIEW.md);
observabilidade completa de queries/requests/storage e logs continua no RF-09.
[ADR 0019](./adr/0019-metricas-de-changefeeds.md).
