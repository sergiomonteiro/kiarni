# Decisões arquiteturais

ADRs registram contexto, alternativas, decisão, consequências e evidências.
As decisões em aberto no PRD e na arquitetura continuam em aberto até que
um ADR seja revisado e aceito.

Copie [TEMPLATE.md](./TEMPLATE.md) para `NNNN-titulo.md`, usando numeração
sequencial a partir de `0001`. Status: proposta, aceita, rejeitada ou substituída.
Uma proposta não representa decisão aceita. Ao substituir uma decisão, preserve
o ADR anterior e inclua links entre os documentos.

A [deliberação M0-F4](../M0_DELIBERACAO.md) registra as decisões aceitas e os marcos de verificação.

## ADRs aceitos

- [0001 — Entrada de documentos e IDs explícitos](./0001-entrada-de-documentos.md).
- [0002 — Geração de IDs e preparação de inserções](./0002-geracao-de-ids.md).
- [0003 — Spike de persistência](./0003-spike-de-persistencia.md).
- [0004 — Ordenação pós-commit e overflow](./0004-ordenacao-e-overflow.md).
- [0005 — Streaming e autenticação do stream](./0005-streaming-e-autenticacao.md).
- [0006 — AST e comparação JSON](./0006-ast-e-comparacao-json.md).
- [0007 — Contrato HTTP e erros](./0007-http-e-erros.md).
- [0008 — Autenticação e ciclo de vida de tokens](./0008-autenticacao-e-tokens.md).
- [0009 — Bootstrap e ativação persistida de auth](./0009-bootstrap-e-ativacao-auth.md).
- [0010 — Substituição, Merge Patch e no-op](./0010-substituicao-merge-patch-e-no-op.md).
- [0011 — Formato inicial e catálogo de coleções](./0011-formato-inicial-e-catalogo.md).
- [0012 — Configuração, limites e admissão inicial](./0012-configuracao-e-admissao.md).
- [0013 — Backup consistente e restauração em nova raiz](./0013-backup-e-restauracao.md).
- [0014 — Organização do workspace no M1](./0014-organizacao-do-workspace.md).

- [0015 — Coordenação concorrente e admissão com espera limitada](./0015-coordenacao-concorrente.md).

Proposta em experimento:

- [0016 — Avaliação de agrupamento de commits](./0016-group-commit-spike.md).

Integração aceita no perfil Linux local, com limitações de desempenho registradas:

- [0017 — Integração do agrupamento no coordenador de escrita](./0017-integracao-coordenador-de-escrita.md).

Integração local do M2, com publicação no writer e transporte SSE autenticado:

- [0018 — Dispatcher de coleção no produto](./0018-dispatcher-do-produto.md).

Observabilidade incremental do M2/RF-09:

- [0019 — Métricas de changefeeds](./0019-metricas-de-changefeeds.md).

Primeiro incremento do M3/RF-08:

- [0020 — Dashboard embutido e sessão em memória](./0020-dashboard-embutido.md).

- [0021 — Status do serviço HTTP e ocupação da admissão](./0021-status-do-servidor.md).

- [0022 — Contagem de documentos com orçamento explícito](./0022-contagem-limitada-de-documentos.md).

- [0023 — Logs de requisições com fila e saída limitadas](./0023-logs-de-requisicoes.md).

- [0024 — Métricas agregadas de conclusão HTTP](./0024-metricas-http.md).

- [0025 — Métricas de processamento de consultas](./0025-metricas-de-consultas.md).

- [0026 — Métricas de primitivas e commits de storage](./0026-metricas-de-storage.md).

- [0027 — Erros recentes em memória](./0027-erros-recentes.md).
