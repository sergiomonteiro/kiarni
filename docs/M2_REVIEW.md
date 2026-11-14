# Revisão do gate M2 — banco reativo

Data: 2026-11-13. Resultado: **gate aprovado localmente em Linux**, no recorte
de assinaturas de coleção. A aprovação permite iniciar incrementos do M3;
não declara o MVP completo nem uma release pronta para produção.

## Requisitos e evidências

O [PRD](../KIARNI_PRD_v0.1.md) exige no M2 dispatcher pós-commit, assinaturas,
semântica de eventos, backpressure e métricas. Seu gate pede testes de
concorrência, desconexão e consumidores lentos. O RF-05 permite restringir
assinaturas a coleções enquanto não houver semântica de entrada/saída de
consultas filtradas.

| Critério | Evidência verificável |
| --- | --- |
| Publicação somente após commit confirmado | `nothing_is_published_before_commit_or_before_confirmed_publication` pausa antes do storage e entre confirmação e publicação. Falhas antes e depois do commit têm testes próprios, sem confirmação indevida ao cliente. |
| Insert, update, delete e ordem | `grouped_transitions_are_ordered_and_noops_or_rejections_do_not_publish` verifica estados intermediários; `concurrent_writers_produce_identical_ordered_transitions_for_two_real_streams` compara duas conexões SSE reais sob escritores concorrentes. |
| Inscrição, desconexão e reconexão | `subscription_barrier_has_no_snapshot_or_replay` e os testes SSE de reconexão e liberação de capacidade demonstram inscrição somente em mudanças futuras, sem replay. |
| Consumidor lento e memória limitada | `slow_consumer_overflow_never_stops_other_consumers_or_writes` força overflow preservando outro consumidor; `nonreading_client_does_not_block_writes_and_its_slot_is_released` exercita o transporte real. |
| Encerramento e falhas explícitas | Testes cobrem remoção da coleção, falha pós-commit, expiração de credencial e frame grande demais. Um delete confirmado permanece persistido mesmo quando seu frame não pode ser enviado. |
| Métricas dos feeds | Testes de backlog, encerramento único e histogramas; `metrics_do_not_wait_for_a_blocked_writer`; integração de `/v1/metrics` com stream real, autenticação, publicação, no-op e cancelamento. |

Fontes dos testes: [writer/feeds](../crates/server/src/writer/tests/feeds.rs),
[streams HTTP](../apps/kiarni/tests/serve/streams.rs),
[dispatcher](../crates/feeds/src/lib.rs),
[métricas](../crates/feeds/src/metrics.rs) e
[framing SSE](../crates/feeds/src/sse.rs).

As decisões estão nos ADRs
[0018 — dispatcher](./adr/0018-dispatcher-do-produto.md) e
[0019 — métricas](./adr/0019-metricas-de-changefeeds.md).
O contrato de uso está no [guia da alpha local](./ALPHA_LOCAL.md) e no
[guia das métricas](./CHANGEFEED_METRICS.md).

## Verificações desta revisão

| Comando | Resultado |
| --- | --- |
| `cargo fmt --all -- --check` | Passou. |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passou. |
| `cargo test --workspace --locked --offline` | 234 passaram; 2 workers de fixture ignorados diretamente, executados pelos testes pais. |
| `node spikes/streaming/client.test.mjs` | 12 passaram; cliente de referência, não navegador conectado ao produto. |
| `python3 scripts/check_product_dependencies.py` | Passou; fronteira de nove pacotes de produto. |

Ambiente: Linux x86_64, kernel `7.2.6-76070206-generic`, Rust
`1.99.1 (58cf8f9c3 2026-10-13)` e Cargo
`1.99.1 (5f94df478 2026-08-27)`.
HEAD: `674ac71a2b6cc4dd768e206c5a999bd2ecb95b9b`, com alterações locais
e arquivos ainda não rastreados; o HEAD sozinho não identifica esta entrega.

SHA-256 do recorte Rust do produto:
`9a0e190735b7cd0fe1cf819f394ddf3385e82edb9baa03358fcb94fe54b6a808`.
Para reproduzir: ordenar por caminho `Cargo.toml`, `Cargo.lock` e todos os
arquivos `*.rs` e `Cargo.toml` sob `apps/` e `crates/`; alimentar SHA-256 com
`caminho UTF-8 + NUL + conteúdo + NUL` para cada arquivo. Esse hash não inclui
spikes, scripts, fixtures externas ou documentação.

## Limites e pendências

- Entrega best-effort enquanto conectado, sem snapshot inicial, replay,
  retomada ou exactly-once. A sequência é local ao processo. Após EOF, o
  cliente precisa reconciliar seu estado; o terminal SSE é best-effort.
- Filas e admissão são limitadas. O orçamento de buffers não equivale a um
  teto comprovado de RSS. CPU, memória e estabilidade prolongada continuam
  pendentes, assim como a investigação da regressão de leitura registrada
  em [PERFORMANCE.md](./PERFORMANCE.md).
- O RF-09 completo continua aberto: métricas de requests, consultas e storage,
  além de logs para diagnosticar consultas lentas. O histograma atual mede
  confirmação do storage até fanout, não latência até o cliente.
- Esta revisão não executou CI remoto/macOS, navegador contra o produto,
  promtool/Prometheus externo, proxy/TLS ou testes de falha física de disco.
  Testes de referência do spike não substituem essas validações.
- Dashboard, distribuição, operação de backup/restauração e checklist de
  release seguem no M3. Escalabilidade horizontal permanece pesquisa futura,
  fora do MVP.

## Próximo incremento

Iniciar o dashboard embutido com um recorte somente de leitura: autenticação
same-origin, health e listagem de coleções, servido pelo mesmo binário e sem
serviço adicional em runtime. Validar esse fluxo no navegador antes de ampliar
para documentos, changefeeds e visualização de métricas. Esta revisão não
implementa o dashboard nem altera contratos públicos.
