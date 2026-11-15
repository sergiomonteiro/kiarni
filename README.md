# Kiarni

One binary. Live queries. Built-in observability.

Banco de documentos JSON em Rust, single-node e multi-cliente, com consultas
composicionais, changefeeds, dashboard e observabilidade no mesmo executável.
Licença [MIT](./LICENSE).

## Estado atual

**M1 — banco funcional validado localmente em Linux.** `kiarni serve` abre o store
versionado, ativa autenticação e atende health, operações de coleção e
CRUD de documentos e consultas composicionais em loopback. O M2 já oferece
changefeeds de coleção por SSE autenticado, com publicação pós-commit, filas
limitadas e testes de clientes reais. O **gate M2 foi aprovado localmente em
Linux**, incluindo métricas autenticadas dos feeds. Há uma
[alpha reativa local para experimentar](./docs/ALPHA_LOCAL.md); observabilidade
operacional ampliada e distribuição continuam no roadmap. O primeiro recorte do
[dashboard embutido](./docs/DASHBOARD.md) já permite autenticar, verificar health,
listar coleções, consultar documentos com limites, acompanhar mudanças da
coleção e visualizar métricas agregadas dos feeds pelo navegador. O
[status do serviço](./docs/SERVER_STATUS.md) informa versão, uptime e vagas
HTTP/stream ocupadas. A [contagem por coleção](./docs/DOCUMENT_COUNTS.md) é
exata dentro de um orçamento de leitura, ou informa um limite inferior. Não há
snapshot/replay. Há [logs estruturados de requisições](./docs/REQUEST_LOGS.md),
com nível configurável e saída limitada. O endpoint autenticado de métricas
também fornece [totais e latências HTTP](./docs/HTTP_METRICS.md), já apresentados
no dashboard com histogramas separados de respostas e streams encerrados.
[Métricas de consultas](./docs/QUERY_METRICS.md) distinguem validação, admissão,
execução e limites atingidos na API e no dashboard.
[Métricas de storage](./docs/STORAGE_METRICS.md) expõem primitivas, tentativas de
commit e tamanho lógico do arquivo na API autenticada e no dashboard.
[Erros recentes](./docs/RECENT_ERRORS.md) têm retenção limitada em memória, sem
payloads. O [dashboard local foi revisado](./docs/DASHBOARD_REVIEW.md); M3 e
release continuam abertos. [Revisão do gate M1](./docs/M1_REVIEW.md) e
[revisão do gate M2](./docs/M2_REVIEW.md).

O [backup offline](./docs/BACKUP.md) exporta um snapshot consistente para destino
novo, sem credenciais, com hashes e publicação atômica sem sobrescrita.
Restauração operacional permanece pendente na issue #2.

O baseline HTTP com múltiplos clientes registra throughput confirmado,
latências e rejeições por contenção. [Medição de desempenho](./docs/benchmarks/http-baseline-2026-11-12.md)

O primeiro ciclo de otimização retirou a espera fixa de accept e adotou
snapshots concorrentes e admissão FIFO limitada. [Resultados comparativos](./docs/benchmarks/http-performance-2026-11-12.md)

```bash
cargo run -p kiarni --locked -- serve --data-dir ./data
```

O primeiro startup informa o caminho da credencial inicial, guardada fora de
`data/` em um arquivo privado. O bind padrão é `127.0.0.1:8711`. GET e HEAD em
`/v1/health/live` e `/v1/health/ready` exigem `Authorization: Bearer`. Use o
endereço numérico do bind como Host. SIGINT e SIGTERM encerram a admissão e
liberam o store e o lock da raiz. [Operação do servidor local](./docs/M1_SERVE.md)

Com o servidor parado, `kiarni auth rotate --data-dir ./data` emite uma
credencial nova e `kiarni auth revoke-all --data-dir ./data` revoga as credenciais
da instalação.

## Desenvolvimento

Pré-requisitos: toolchain Rust estável com Cargo, rustfmt e Clippy.
Com uma instalação via rustup:

```bash
rustup component add rustfmt clippy
cargo build --workspace --locked
cargo run -p kiarni --locked -- --version
```

O binário e os testes HTTP ficam em `apps/kiarni`. As bibliotecas do produto
ficam em `crates/`, com core, engine, storage, auth, config, http e server.
Os experimentos e seus harnesses ficam em `spikes/` e usam as bibliotecas
promovidas. `Cargo.lock` fixa a resolução das dependências.
[Organização do workspace](./ARCHITECTURE.md#3-organização-do-código-no-m1)

Verificações locais, também configuradas no CI para Linux e macOS:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 scripts/check_product_dependencies.py
```

## Referências e próximos incrementos

- [Documentação de uso e operação](./docs/README.md).
- [Plano de fechamento do MVP e entrega documental](./docs/MVP_REMAINING.md).

- [PRD](./KIARNI_PRD_v0.1.md): escopo, requisitos e gates de M0 a M3.
- [Arquitetura](./ARCHITECTURE.md): fronteiras, invariantes e hipóteses técnicas.
- [Instruções de contribuição para agentes](./AGENTS.md).
- [ADRs](./docs/adr/README.md): processo e modelo para registrar decisões.

O M0 exercitou documentos/IDs, transações, recuperação, publicação pós-commit,
AST, transporte TCP/Chromium e ativação de auth. Os relatórios preservam as
evidências e os limites dos experimentos. O gate foi aprovado para entrada no M1.
[Revisão M0](./docs/M0_REVIEW.md) · [Deliberação M0-F4](./docs/M0_DELIBERACAO.md)

Para os testes do cliente de referência (Node 22, sem pacotes npm):

```bash
node spikes/streaming/client.test.mjs
```

Para experimentar o servidor e os changefeeds autenticados, consulte o
[roteiro da alpha reativa local](docs/ALPHA_LOCAL.md).
