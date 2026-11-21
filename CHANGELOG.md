# Changelog

## 0.1.0 — em desenvolvimento, sem release estável

Este número corresponde ao Cargo.toml e não declara estabilidade. O aceite
técnico do MVP experimental fica na revisão, condicionado à CI final.
Artifacts experimentais são associados a commits e expiram conforme o CI.
A [revisão de release](./docs/RELEASE_REVIEW.md) registra o gate e as pendências.

- Planner secundário integrado às consultas HTTP, com candidatos carregados no
  mesmo snapshot e avaliação separada. Mantidos semântica, erros, limites e gate;
  métricas/dashboard distinguem id/scan/secondary também em erros. query_read
  exclui avaliação e serialização; removido o carregador de scan duplicado.
- Consolidação do storage: Store único no formato 2 para servidor, writer e
  backup/restore. Removidos backend antigo, fachada IndexedStore e entrypoint
  paralelo de bootstrap. Formato 1 de desenvolvimento é recusado, sem migração
  ou apagamento automático. Índices são mantidos nas mutações HTTP; o planner
  HTTP usa ID/secundário seletivo/scan; administração de índices pela CLI e
  medições de performance permanecem pendentes.
- Removido wrapper kiarni-format-spike; testes de contrato e crash transferidos
  para crates/storage/tests/format.rs. Relatórios históricos preservados.

- Servidor Rust single-node, multi-cliente, com storage JSON persistente,
  coleções, CRUD e IDs gerados/validados. Índice primário por `_id`.
- Consultas AST com filter/sort/limit/project/ID, comparação decimal exata,
  snapshots e limites explícitos de scan/resultado. Sem execução de código cliente.
- Changefeeds SSE de coleção após commit, ordenação local, filas limitadas,
  desconexão de consumidor atrasado e entrega best-effort enquanto conectado.
- API `/v1`, erros estáveis e request IDs, bind loopback, autenticação por arquivo
  privado e comandos offline de rotação/revogação. Perfil HTTPS por proxy local.
- Dashboard embutido com consultas, feed, status, contagem limitada, métricas e
  erros recentes. Logs tipados e métricas HTTP/query/storage/feeds autenticados.
- Backup offline consistente e restauração em nova raiz com novas credenciais,
  validação de hashes/estrutura e publicação sem sobrescrita.
- Pacotes experimentais Linux x86_64 GNU e macOS Apple Silicon, checksums,
  proveniência, MIT/avisos de terceiros e testes do arquivo distribuído.
- Imagem experimental Linux amd64, Compose e Kubernetes de uma réplica/PVC,
  healthcheck autenticado do próprio binário e testes de persistência/recriação.
- Referências de API/CLI/configuração, diagnóstico e contribuição; exemplos
  executados no CI. Quickstart distribuído com seus arquivos de exemplo.
- Avaliação RF-04 de índice secundário string parcial: equivalência com scan,
  testes de atualização/remoção/crash e medições pareadas no ADR 0033. Núcleo
  integrado ao Store; seleção secundária HTTP e medições no produto pendentes.
- Evidências de instalação isolada e observação com Prometheus externo,
  diagnóstico de scan versus ID e consumidor lento, com escopo/duração explícitos.

Limitações conhecidas: [revisão](./docs/RELEASE_REVIEW.md) e
[plano restante](./docs/MVP_REMAINING.md). Não há cluster, replicação, sharding,
replay/exactly-once, garantia de evento para cliente desconectado ou compatibilidade
com ReQL. Não declarar administração de índices pela CLI, SDK completo, upgrade
automático ou ganhos de performance HTTP como implementados. Baselines não são promessa arbitrária de TPS/RSS.
Nenhuma tag, Release ou publicação de imagem em registry é criada por este documento.
