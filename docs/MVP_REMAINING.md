# Roadmap após o aceite técnico da alpha

Revisado em 2026-11-28: issues #1–#10 encerradas, incluindo integração de índices
(#10) e investigação de leitura/telemetria (#9). A
[CI de af4eea1](https://github.com/sergiomonteiro/kiarni/actions/runs/46105142261)
aprovou os dez jobs. O [gate técnico M3](./RELEASE_REVIEW.md) e o canal de
artifacts experimentais não equivalem a release estável: não há GitHub Release
publicada na data da revisão. [Canal e compatibilidade](./RELEASE_POLICY.md)
continuam explícitos. O plano abaixo não amplia o PRD.

## Próxima ordem priorizada

| Ordem | Incremento | Resultado verificável |
| --- | --- | --- |
| 1 | Spike de escalabilidade vertical: leitura por ID no servidor atual | Mesmo binário/dataset com 1, 2 e 4 cores físicos; throughput, latências, recusas, CPU e RSS, com margem no gerador. Resultado positivo, negativo ou inconclusivo documentado. |
| 2 | Memória e cache | Medir RSS e crescimento por tamanho de documento/dataset, conexões e filas; separar memória do servidor/gerador e cenários de cache. Não inferir teto de RSS pelo plano de buffers. |
| 3 | Cargas mistas, escrita e feeds sustentados | Validar concorrência, commits e notificações sob carga, limites/overflow e recuperação, com ferramenta apropriada e sem escritas no-op disfarçadas de throughput. |
| 4 | Uma otimização por gargalo demonstrado | Spike e comparação antes/depois, preservando correção, limites e ordem pós-commit; promover somente com evidência. Não exige concluir toda a matriz antes de agir sobre um gargalo já isolado. |

### Primeiro incremento: medir cores, sem mudar a arquitetura

Requisito não funcional de performance, posterior a M3. Usar o **produto atual**,
sem voltar aos binários históricos da #9. Congelar revisão, build release,
configuração, dataset pequeno conhecido e hashes de servidor/gerador. A primeira
execução usa Linux, onde o medidor já controla afinidade; isso não retira macOS
das plataformas de build/teste nem homologa sua escala por cores.

Começar com GET por ID, 256 documentos de 1 KiB, 8/32 clientes e conjuntos de
1/2/4 cores físicos no servidor. Registrar quais threads SMT pertencem a cada
core; manter fixo e separado o conjunto do gerador. Intercalar configurações,
usar controle A/A, warmup e repetições/janelas declaradas. Medir CPU total do
processo servidor e RSS separadamente do gerador; registrar pico e escopo das
amostras. Distribuição por core só deve ser atribuída ao servidor quando a
instrumentação permitir: CPU global do host não é proxy exclusivo do processo.
Só interpretar a escala quando o gerador tiver margem verificada.

O aceite é um relatório reproduzível com dados brutos e classificação dos
limites/gargalos, não crescimento linear ou um TPS predeterminado. Em um host
compartilhado, as máscaras não reservam cores nem controlam temperatura. Não
extrapolar o ensaio para todos os cores da máquina se eles forem compartilhados
com o gerador; usar outro arranjo de teste para essa extensão.

O gerador Rust atual atende apenas leitura e fixtures limitados. Escrita,
cargas mistas, feeds e datasets maiores precisam de incrementos próprios do
ferramental e de validação correspondente. Não declarar esses workloads
medidos por reutilizar o número de conexões de um teste de GET.

Não escolher pool, async runtime, política nova de escrita ou dependência central
antes do baseline e do perfil. Cada coleta deve produzir uma hipótese acionável
ou uma conclusão explícita; a #9 não será reaberta para perseguir o percentual
histórico sem nova evidência reprodutível.

### Trilhas separadas de adoção e pós-MVP

- **Adoção da alpha:** instalação limpa, feedback de uso e experiência da API
  continuam úteis sem esperar toda otimização. GitHub Release/tag/registry exigem
  ação explícita e artefatos validados; esta revisão não publica esses canais.
- **SDK inicial:** questão ainda aberta do PRD. Avaliar um primeiro SDK e testes
  de conformidade em incremento próprio; TypeScript é candidato, não decisão.
  SDKs adicionais não entram automaticamente no MVP.
- **Replay e escala horizontal:** avaliações futuras separadas. A inspiração na
  distribuição do Erlang precisa ser examinada à luz de dados persistidos,
  consistência e falhas. Cluster, replicação e sharding permanecem fora do MVP;
  nenhum preparo de infraestrutura distribuída é introduzido agora.

## Histórico dos incrementos concluídos

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

## Estado das validações e decisões restantes

- CI Linux/macOS, instalação, métricas externas e operação controlada passaram
  no commit de referência. Novos commits precisam de sua própria CI; esses
  ensaios não certificam toda plataforma/hardware ou operação por tempo indefinido.
- Integração de índice secundário (#10): avaliação RF-04 concluída no ADR 0033.
  O [ADR 0034](./adr/0034-lifecycle-indice-secundario.md) revisado dispensa legado:
  Store único de formato 2 é usado por servidor, writer, bootstrap e backup/restore.
  Removidos IndexedStore, inicialização paralela e wrapper kiarni-format-spike;
  testes de contrato/crash foram transferidos para storage. Mutações HTTP mantêm
  os índices atomicamente; consultas HTTP selecionam ID/secundário/scan com
  métricas e budgets preservados. [Administração offline pela CLI](./INDEXES.md)
  integra lock, auditoria e recuperação restrita. As [medições no produto](./benchmarks/secondary-index-http-2026-11-22.md)
  registram ganhos seletivos e custo de escrita, sem extrapolar o spike. Layouts
  antigos são recusados sem migração ou apagamento automático.
- [Investigação de leitura/telemetria (#9) concluída](./benchmarks/writer-read-rust-2026-11-28.md):
  queda histórica não confirmada consistentemente, gerador validado e decisão
  de manter o runtime sem patch especulativo. CPU/memória seguem para o momento
  combinado com o usuário; não antecipar otimizações amplas.
  O objetivo de aproveitar os cores/RAM de uma máquina está detalhado na etapa
  de [escalabilidade vertical](./PERFORMANCE.md#escalabilidade-vertical--etapa-posterior-à-investigação-9),
  posterior à #9 já encerrada e separada da avaliação futura de escala horizontal.
- Confirmar limites single-node e best-effort em exemplos e materiais de release.
  Cluster, replicação, sharding e replay permanecem fora do MVP.

## Critério de conclusão da documentação

Os comandos publicados devem funcionar na versão distribuída, os exemplos não
podem expor credenciais, os links devem resolver e as limitações devem coincidir
com os testes/ADRs aceitos. Registrar ambiente, comandos e resultados do
quickstart em instalação limpa. Material escrito sem essa execução não comprova
aceite do RF-10 ou conclusão do M3.

## Issues de acompanhamento

As issues abaixo são o histórico concluído (#1–#10). A próxima execução deve
seguir os incrementos priorizados no início deste documento; esta revisão
planeja o spike de cores e não afirma que ele já foi implementado ou medido.

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
| [#9](https://github.com/sergiomonteiro/kiarni/issues/9) | Performance: [investigação concluída](./benchmarks/writer-read-rust-2026-11-28.md), sem queda consistente confirmada |
| [#10](https://github.com/sergiomonteiro/kiarni/issues/10) | Índice secundário integrado ao Store/HTTP/backup/CLI offline; [medições concluídas](./benchmarks/secondary-index-http-2026-11-22.md) |
