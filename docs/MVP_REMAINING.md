# Plano de fechamento do MVP

Atualizado após a avaliação RF-04: o [gate técnico M3](./RELEASE_REVIEW.md)
é aceito para a alpha experimental quando a CI do commit final passar.
[Canal e compatibilidade](./RELEASE_POLICY.md) permanecem explícitos; não há
publicação estável automática. Este documento preserva a sequência e os trabalhos
posteriores ao aceite, sem ampliar o PRD.

## Ordem dos incrementos

1. Dashboard local concluído: [revisão RF-08/09](./DASHBOARD_REVIEW.md),
   [erros recentes](./RECENT_ERRORS.md) e métricas de HTTP, queries, storage e
   feeds. Manter correções por defeito/requisito concreto. Validação operacional
   prolongada e coletor externo continuam no gate de release.
2. Checkpoint publicado: [5aea314](https://github.com/sergiomonteiro/kiarni/commit/5aea314),
   com implementação, testes, ADRs, spikes e evidências locais. Pendências
   registradas nas issues abaixo. Esse checkpoint não aprova uma release.
3. [Backup offline implementado](./BACKUP.md), conforme
   [ADR 0013](./adr/0013-backup-e-restauracao.md), com hashes, interrupção e
   publicação sem sobrescrita. [Restauração operacional implementada](./RESTORE.md):
   artefato validado, raiz nova e novas credenciais no primeiro serve. As issues
   #1/#2 não aprovam o gate de release.
4. [Perfil HTTPS por proxy local implementado](./REMOTE_HTTPS.md), com origem
   pública explícita e integração de auth/dashboard/métricas/SSE. Backend
   permanece loopback; certificado público/renovação e operação prolongada
   continuam responsabilidades de operação e fechamento da release.
5. [Pipeline de binários nativos](./DISTRIBUTION.md): Linux x86_64/glibc 2.39 e
   macOS 15/Apple Silicon, checksums, licenças e smoke do pacote em runner novo.
   [Imagem Docker, Compose e Kubernetes](./CONTAINERS.md) com volume, UID 10001,
   healthcheck autenticado e testes reais de recriação/persistência. A validação
   depende de CI verde do commit; artifacts são experimentais, sem publicação
   automática em registry e sem alegar alta disponibilidade.
6. [Referência da API](./API.md), [CLI/configuração](./CLI.md),
   [diagnóstico](./TROUBLESHOOTING.md) e [contribuição](../CONTRIBUTING.md)
   consolidados, com exemplos executados e verificação de links/inventários no CI.
7. [Revisão M3](./RELEASE_REVIEW.md), quickstart e observação externa concluídos;
   [avaliação RF-04](./spikes/secondary-index.md) e ADR 0033 fecham a pendência de
   índice. CI completa do commit final permite encerrar #8 e #7 com gate técnico
   da alpha. Publicação estável/registry continua ação explícita separada.

Documentação acompanha cada incremento acima. Sua consolidação e validação são
uma entrega própria do RF-10, não uma atividade opcional depois da release.

## Entrega documental obrigatória

| Material | Estado atual e trabalho restante |
| --- | --- |
| Entrada e navegação | [Índice](./README.md) disponível; manter caminhos claros para usuários, operadores e contribuidores. |
| Instalação e quickstart | [Roteiro local](./ALPHA_LOCAL.md) disponível; binários/contêiner e quickstart empacotado documentados; automação mede arquivo local disponível até documento/consulta/evento. Download, leitura humana e quarentena ainda não medidos. |
| Referência da API | [Referência](./API.md) consolidada, com métodos/headers/corpos, erros/request IDs, limites e exemplos testados. Manter sincronizada com contrato implementado. |
| CLI e configuração | [Referência CLI](./CLI.md) consolidada; exemplos de precedência e probe testados com servidor real. |
| Consultas e changefeeds | [API](./API.md) consolida JSON/números, AST, custo de scan, ordem e overflow; guias do dashboard preservam detalhes de UI. Sem replay/exactly-once. |
| Operação | Guias de backup, restore, credenciais, reinício e HTTPS disponíveis; compatibilidade/limites em RELEASE_POLICY. Upgrade entre releases distintas não é certificado nesta alpha. |
| Diagnóstico | [Diagnóstico](./TROUBLESHOOTING.md) relaciona sintomas/códigos a métricas, consumidor lento e consulta lenta, com links operacionais. |
| Contribuição | [CONTRIBUTING.md](../CONTRIBUTING.md) disponível com setup, workspace, testes/fixtures, benchmarks e fluxo de PR/ADR. |
| Release e licenças | Changelog, checklist, canal pré-1.0, baselines, MIT e avisos disponíveis/versionados. Publicação estável ou registry permanece ação explícita separada. |

## Validação e decisões ainda abertas

- Observar resultados de CI Linux/macOS e validar operação prolongada e coleta
  externa; não transformar passagem local em evidência de outra plataforma.
- Integrar índice secundário em incremento próprio conforme ADR 0033: a
  avaliação está concluída, e lifecycle/planner/formato precisam de desenho e
  testes antes de mudar o produto. Não tratar o ganho do spike como ganho HTTP.
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

## Issues de acompanhamento

A ordem de implementação continua sendo a dos incrementos acima. A investigação
de performance aguarda o momento combinado; registrar uma issue não a antecipa.

| Issue | Entrega |
| --- | --- |
| [#1](https://github.com/sergiomonteiro/kiarni/issues/1) | M3: implementar backup offline consistente |
| [#2](https://github.com/sergiomonteiro/kiarni/issues/2) | M3: restaurar backup validado em nova raiz |
| [#3](https://github.com/sergiomonteiro/kiarni/issues/3) | M3: definir e validar operação remota segura |
| [#4](https://github.com/sergiomonteiro/kiarni/issues/4) | M3: automatizar artefatos Linux e macOS |
| [#5](https://github.com/sergiomonteiro/kiarni/issues/5) | M3: distribuir imagem Docker e exemplos de implantação |
| [#6](https://github.com/sergiomonteiro/kiarni/issues/6) | M3: consolidar referência da API e configuração |
| [#7](https://github.com/sergiomonteiro/kiarni/issues/7) | M3: validar instalação limpa e gate do MVP |
| [#8](https://github.com/sergiomonteiro/kiarni/issues/8) | RF-04: concluir avaliação de índice secundário simples |
| [#9](https://github.com/sergiomonteiro/kiarni/issues/9) | Performance: investigar regressão de leitura e custo da telemetria |
