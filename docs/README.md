# Documentação do Kiarni

Kiarni é uma alpha experimental. A [revisão M3](./RELEASE_REVIEW.md) registra
o aceite técnico condicionado à CI final e seus limites.
Os guias abaixo descrevem o comportamento implementado; não representam uma
release publicada ou o MVP completo.

## Começar e usar

- [Quickstart pelo pacote](./QUICKSTART.md): primeiro documento, consulta e evento sem compilar.
- [Experimentar a alpha local](./ALPHA_LOCAL.md): compilar, iniciar, obter a
  credencial e executar documentos, consultas e changefeeds.
- [Binários de desenvolvimento](./DISTRIBUTION.md): plataformas, checksums, licenças e instalação.
- [Docker, Compose e Kubernetes](./CONTAINERS.md): volume, healthcheck e implantação single-node.
- [Referência da API](./API.md): rotas, headers, erros, consultas e feeds.
- [CLI e configuração](./CLI.md): comandos, precedência, defaults e limites.
- [Dashboard](./DASHBOARD.md): autenticação e navegação no mesmo binário.
- [Consultar documentos](./DASHBOARD_QUERIES.md): AST, limites e precisão JSON.
- [Acompanhar mudanças](./DASHBOARD_FEEDS.md): coleção inteira, cancelamento e
  interrupções; sem snapshot, replay ou reconexão automática.
- [Contagem de documentos](./DOCUMENT_COUNTS.md): total exato no orçamento ou
  limite inferior explícito.

## Operar e diagnosticar

- [Validação operacional](./OPERATIONAL_VALIDATION.md): instalação isolada e Prometheus externo.
- [Revisão do candidato/M3](./RELEASE_REVIEW.md): checklist, RFs, evidências e pendências do gate.
- [Administração offline de índices](./INDEXES.md): create/rebuild/drop/audit, limites e recuperação.
- [Diagnóstico operacional](./TROUBLESHOOTING.md): sintomas, códigos e próximos passos.
- [Status HTTP](./SERVER_STATUS.md): versão, uptime e ocupação da admissão.
- [Métricas de storage](./STORAGE_METRICS.md): primitivas, commits e tamanho
  lógico do arquivo, com limites de interpretação.
- [Métricas de consultas](./QUERY_METRICS.md): resultados, espera, execução e
  caminhos ID/scan/secondary, sem conteúdo da AST.
- [Métricas HTTP](./HTTP_METRICS.md): conclusões, erros, transporte e latência
  de respostas/streams, independentes do logger.
- [Métricas dos feeds](./CHANGEFEED_METRICS.md): séries, filas e encerramentos.
- [Métricas no dashboard](./DASHBOARD_METRICS.md): coleta manual e interpretação.
- [Erros recentes](./RECENT_ERRORS.md): histórico limitado em memória, API e painel.
- [Revisão do dashboard](./DASHBOARD_REVIEW.md): cobertura RF-08/09 e limites de validação.
- [Logs de requisições](./REQUEST_LOGS.md): níveis, request IDs, privacidade e
  perdas sob backpressure.
- [Desempenho](./PERFORMANCE.md): medições reproduzíveis e limites conhecidos.

[Backup offline](./BACKUP.md) e [restauração em nova raiz](./RESTORE.md) já
dispõem de comandos e verificações. O [perfil HTTPS por proxy local](./REMOTE_HTTPS.md)
foi implementado com origem explícita e validação de integração. O
[empacotamento nativo](./DISTRIBUTION.md) integra o CI; a
[imagem de desenvolvimento](./CONTAINERS.md) tem testes Docker/Kubernetes.
Validação operacional ampliada permanece pendente. O [plano de fechamento do MVP](./MVP_REMAINING.md)
registra essas entregas e a documentação que deverá acompanhá-las.

## Entender e contribuir

- [Avaliação de índice secundário](./spikes/secondary-index.md): correção, custos e decisão RF-04.
- [Contrato do índice experimental](./adr/0034-lifecycle-indice-secundario.md): Store único, manutenção transacional e planner HTTP; administração CLI integrada; medições no produto pendentes.
- [Revisão da integração do índice](./INDEX_INTEGRATION_REVIEW.md): evidências do candidato e sequência para promoção ao produto.
- [Canal e compatibilidade pré-1.0](./RELEASE_POLICY.md).
- [Contribuir](../CONTRIBUTING.md): ambiente, workspace, testes, fixtures e PRs.
- [PRD](../KIARNI_PRD_v0.1.md): requisitos e critérios de aceite.
- [Arquitetura](../ARCHITECTURE.md): componentes e invariantes.
- [ADRs](./adr/README.md): decisões técnicas e limites das evidências.
- [Revisão M1](./M1_REVIEW.md) e [revisão M2](./M2_REVIEW.md): gates locais.
- [Instruções do repositório](../AGENTS.md): fluxo e verificações de desenvolvimento.

ADRs e relatórios de spikes preservam contexto histórico; não substituem o guia
de uso do produto atual. Use as referências API/CLI acima para o contrato atual
e o guia de contribuição para executar as verificações. A política do [canal pré-1.0](./RELEASE_POLICY.md) separa esse aceite de release estável.

- [Medições do índice no HTTP](./benchmarks/secondary-index-http-2026-11-22.md): ganhos seletivos, custos de escrita, controles e dados brutos.

- [Leitura e custo de telemetria](./benchmarks/telemetry-http-2026-11-26.md): logs, leitores, limites TCP do experimento e estado da #9.

- [Revisita histórica da leitura](./benchmarks/writer-read-revisit-2026-11-26.md): executáveis exatos recuperados, pares A/B, controles A/A e limites da conclusão.

- [Leitura com gerador/servidor separados](./benchmarks/writer-read-pinned-2026-11-27.md): afinidade temporária, janelas maiores e limite de CPU do medidor.
