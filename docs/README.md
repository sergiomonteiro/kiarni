# Documentação do Kiarni

Kiarni está em alpha local: M1 e M2 validados em Linux, M3 em desenvolvimento.
Os guias abaixo descrevem o comportamento implementado; não representam uma
release publicada ou o MVP completo.

## Começar e usar

- [Experimentar a alpha local](./ALPHA_LOCAL.md): compilar, iniciar, obter a
  credencial e executar documentos, consultas e changefeeds.
- [Dashboard](./DASHBOARD.md): autenticação e navegação no mesmo binário.
- [Consultar documentos](./DASHBOARD_QUERIES.md): AST, limites e precisão JSON.
- [Acompanhar mudanças](./DASHBOARD_FEEDS.md): coleção inteira, cancelamento e
  interrupções; sem snapshot, replay ou reconexão automática.
- [Contagem de documentos](./DOCUMENT_COUNTS.md): total exato no orçamento ou
  limite inferior explícito.

## Operar e diagnosticar

- [Status HTTP](./SERVER_STATUS.md): versão, uptime e ocupação da admissão.
- [Métricas de storage](./STORAGE_METRICS.md): primitivas, commits e tamanho
  lógico do arquivo, com limites de interpretação.
- [Métricas de consultas](./QUERY_METRICS.md): resultados, espera, execução e
  caminhos ID/scan, sem conteúdo da AST.
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
foi implementado com origem explícita e validação de integração. Distribuição e
validação operacional ampliada permanecem pendentes. O [plano de fechamento do MVP](./MVP_REMAINING.md)
registra essas entregas e a documentação que deverá acompanhá-las.

## Entender e contribuir

- [PRD](../KIARNI_PRD_v0.1.md): requisitos e critérios de aceite.
- [Arquitetura](../ARCHITECTURE.md): componentes e invariantes.
- [ADRs](./adr/README.md): decisões técnicas e limites das evidências.
- [Revisão M1](./M1_REVIEW.md) e [revisão M2](./M2_REVIEW.md): gates locais.
- [Instruções do repositório](../AGENTS.md): fluxo e verificações de desenvolvimento.

ADRs e relatórios de spikes preservam contexto histórico; não substituem o guia
de uso do produto atual. Uma referência consolidada de API/CLI/configuração e
um guia de contribuição para pessoas continuam como entregas documentais.
