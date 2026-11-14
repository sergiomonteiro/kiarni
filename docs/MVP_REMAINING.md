# Plano de fechamento do MVP

Atualizado em 2026-11-14, após o fechamento funcional do dashboard local. M1 e M2 foram validados
localmente em Linux. M3 permanece aberto. Este plano organiza pendências do
[PRD](../KIARNI_PRD_v0.1.md), sem ampliar o escopo ou aprovar o gate de release.

## Ordem dos incrementos

1. Dashboard local concluído: [revisão RF-08/09](./DASHBOARD_REVIEW.md),
   [erros recentes](./RECENT_ERRORS.md) e métricas de HTTP, queries, storage e
   feeds. Manter correções por defeito/requisito concreto. Validação operacional
   prolongada e coletor externo continuam no gate de release.
2. Consolidar um checkpoint no repositório antes de iniciar novos recursos:
   revisar arquivos e exclusões (artefatos, dados locais e credenciais), organizar
   commits revisáveis e registrar a validação realmente executada. Depois,
   publicar no remoto e abrir issues pequenas para as pendências deste plano,
   com requisito/marco, escopo, critérios de aceite e verificações esperadas.
   Esse checkpoint registra o estado do desenvolvimento; não aprova uma release.
3. Implementar backup/restauração operacional conforme
   [ADR 0013](./adr/0013-backup-e-restauracao.md), com integridade, interrupção,
   publicação segura e novas credenciais após restauração.
4. Definir e validar operação remota segura, configuração de exposição e
   HTTPS/proxy. Loopback permanece o perfil atual; não recomendar HTTP público.
5. Preparar distribuição: binários, Docker, volume/healthcheck, execução não-root,
   Compose e manifesto Kubernetes básico, sem alegar alta disponibilidade.
6. Executar instalação limpa, quickstart, CI/plataformas e checklist de release;
   revisar explicitamente o atendimento dos RFs e aprovar o gate M3 com evidências.

Documentação acompanha cada incremento acima. Sua consolidação e validação são
uma entrega própria do RF-10, não uma atividade opcional depois da release.

## Entrega documental obrigatória

| Material | Estado atual e trabalho restante |
| --- | --- |
| Entrada e navegação | [Índice](./README.md) disponível; manter caminhos claros para usuários, operadores e contribuidores. |
| Instalação e quickstart | [Roteiro local](./ALPHA_LOCAL.md) disponível; consolidar instalação por artefato/Docker e validar primeiro documento, consulta e feed em até dez minutos em ambiente limpo. |
| Referência da API | Contratos espalhados em guias/ADRs; consolidar endpoints implementados, autenticação, métodos, erros/request IDs, exemplos e limites em uma referência atual. Não apresentar rotas ilustrativas do PRD como prontas. |
| CLI e configuração | Consolidar comandos, opções, precedência, defaults, intervalos, variáveis de ambiente, arquivos e falhas comuns; conferir com código/testes. |
| Consultas e changefeeds | Guias atuais disponíveis; consolidar semântica de JSON/números, AST, custo de scan, ordem e política de overflow, sem prometer replay/exactly-once. |
| Operação | Criar procedimentos reproduzíveis de backup, restauração, credenciais, reinício, upgrade/compatibilidade e operação remota conforme forem implementados. Não documentar simples cópia de banco aberto como backup. |
| Diagnóstico | Guias de status/métricas/logs disponíveis; ampliar troubleshooting com sintomas, códigos, limites, consumidor lento e consulta lenta, sem exemplos com segredos reais. |
| Contribuição | Criar guia para pessoas com setup, workspace, testes, fixtures, benchmarks e fluxo de ADR; AGENTS.md não substitui esse guia. |
| Release e licenças | Consolidar changelog, limitações conhecidas, checklist, plataformas realmente validadas, MIT e avisos de terceiros. Publicar junto dos artefatos. |

## Validação e decisões ainda abertas

- Observar resultados de CI Linux/macOS e validar operação prolongada e coleta
  externa; não transformar passagem local em evidência de outra plataforma.
- Fechar documentalmente a avaliação do índice secundário simples do RF-04,
  condicionada à prova de conceito. Não declarar implementação obrigatória já
  escolhida ou retirar o requisito sem decisão.
- Investigar regressão de leitura e medir custo dos logs. CPU/memória seguem
  para o momento combinado com o usuário; não antecipar otimizações amplas.
- Confirmar limites single-node e best-effort em exemplos e materiais de release.
  Cluster, replicação, sharding e replay permanecem fora do MVP.

## Critério de conclusão da documentação

Os comandos publicados devem funcionar na versão distribuída, os exemplos não
podem expor credenciais, os links devem resolver e as limitações devem coincidir
com os testes/ADRs aceitos. Registrar ambiente, comandos e resultados do
quickstart em instalação limpa. Material escrito sem essa execução não comprova
aceite do RF-10 ou conclusão do M3.
