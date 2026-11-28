# Revisão do candidato e gate M3

Data inicial: 2026-11-15. Revisão após a avaliação RF-04 da issue #8.
**Decisão: gate técnico M3 aprovado para a alpha experimental, condicionado à
CI completa do commit de encerramento.** O aceite remoto fica registrado nas
issues #7/#8 depois de todos os jobs passarem. Sem CI verde, não há aceite.

## Estado após as entregas #9/#10 — 2026-11-28

Issues #1–#10 encerradas e [CI de af4eea1 com dez jobs aprovados](https://github.com/sergiomonteiro/kiarni/actions/runs/46105142261).
O índice secundário já integra Store/HTTP/backup/CLI, com medições no produto;
a investigação de leitura/telemetria concluiu sem queda histórica consistente
confirmada. O [roadmap revisado](./MVP_REMAINING.md) prioriza medição vertical do
servidor atual. Não há GitHub Release publicada nesta revisão; mantêm-se canal
experimental e ação explícita para publicação. A base histórica abaixo explica
o aceite inicial, não pendências atuais de integração do índice.

## Base da decisão

A revisão anterior (409e29f) manteve o gate aberto pela avaliação RF-04 e pelo
encaminhamento do canal/compatibilidade. A avaliação agora tem
[testes e medição pareada](./spikes/secondary-index.md), com decisão no
[ADR 0033](./adr/0033-avaliacao-indice-secundario.md). O RF exige avaliar o campo
simples e comparar correção/custo; isso foi cumprido sem alterar o PRD. A decisão
adiou integração no formato real e mantém o índice primário obrigatório.

O [canal e política pré-1.0](./RELEASE_POLICY.md) formalizam a distribuição
experimental já operada: pacotes e imagem exportada pelos artifacts do CI oficial,
identificados por commit, com retenção de 14 dias. Não se declara publicação de
tag, Release estável ou registry. O [changelog](../CHANGELOG.md) permanece em
desenvolvimento. Fechar a avaliação técnica não executa uma publicação externa.

O quickstart mede o arquivo local disponível até documento, consulta e evento;
leitura humana, download e quarentena não foram medidos. São limites explícitos,
não requisitos novos para prolongar o gate. A pergunta sobre SDK inicial continua
no planejamento do PRD, sem alegar SDK implementado. A API HTTP genérica e o
fluxo de aceite do RF-10 foram documentados e executados.

## Matriz de RFs

“Atendido funcionalmente” descreve o recorte implementado/testado, não aprovação
de release ou operação universal. Os guias históricos M1/M2 preservam suas datas;
a CI atual e os relatórios por commit complementam essas evidências.

| RF | Estado da revisão | Implementação/evidência e limites |
| --- | --- | --- |
| RF-01 Inicialização/configuração | Atendido funcionalmente | [CLI](./CLI.md), precedência estrita, diretórios privados/lock, shutdown/restart; testes reais e exemplos de configuração no CI. |
| RF-02 Documentos/coleções | Atendido funcionalmente | [API](./API.md), CRUD/IDs, duplicatas/tetos, PUT/PATCH/no-op, reinício e recuperação por testes de processo. Não certifica falha física. |
| RF-03 Consultas | Atendido funcionalmente | AST validada com fixtures, JSON exato, filtro/sort/limit/project/ID, snapshots e scans limitados; sem código arbitrário. Métricas explicam caminho/etapas, não plano completo. |
| RF-04 Índices | Avaliação e integração concluídas | Primário e secundário seletivo no Store único, planner HTTP, lifecycle offline e backup/restore; [medições no produto](./benchmarks/secondary-index-http-2026-11-22.md) registram custo de leitura/escrita. ADR 0034 e #10 sucedem a decisão inicial de adiar integração. |
| RF-05 Changefeeds | Atendido no escopo de coleção | [Gate M2](./M2_REVIEW.md), publicação pós-commit, duas conexões, ordem, overflow/cancelamento e no-op. Coleção inteira, best-effort futuro; sem snapshot/replay/exactly-once. |
| RF-06 Rede | Atendido funcionalmente | `/v1`, erros/request IDs, health, concorrência limitada, HTTP/JSON e SSE com testes independentes. Saturação pode aguardar backlog/fechar conexão; não promete sempre envelope. |
| RF-07 Segurança | Atendido no perfil documentado | Loopback, auth para toda API, arquivos privados, logs sem segredos e [HTTPS opcional por proxy local](./REMOTE_HTTPS.md) testado. Não homologa toda operação pública/certificado. |
| RF-08 Dashboard | Atendido no perfil Chromium testado | [Revisão local](./DASHBOARD_REVIEW.md), binário único, estado/consultas/count/feed/métricas/erros, CSP/auth. Não é auditoria de acessibilidade ou homologação de todos os browsers. |
| RF-09 Observabilidade | Instrumentação e validação controlada | Logs tipados, métricas HTTP/query/storage/feeds e anel de erros; [ensaio externo](./OPERATIONAL_VALIDATION.md) com diagnóstico de query/consumidor. Janela/duração só valem quando concluídas; sem CPU/RSS/auditoria durável. |
| RF-10 Distribuição/docs | Atendido no canal experimental | Pacotes Linux/macOS, imagem exportada/Compose/Kubernetes e referências, MIT/avisos e quickstart testado. [Política do canal](./RELEASE_POLICY.md) explicita retenção, versões e compatibilidade; não é release estável. |

Baseline reproduzível de performance já está em [PERFORMANCE.md](./PERFORMANCE.md),
com dados/hardware/comandos. A regressão de leitura/custo de logs na
[issue #9](https://github.com/sergiomonteiro/kiarni/issues/9) foram investigados e
[concluídos](./benchmarks/writer-read-rust-2026-11-28.md), sem confirmação consistente da queda.
Conforme combinado, esta entrega não antecipa otimizações amplas de CPU/memória.

## Checklist técnico e operacional

| Item | Evidência/estado |
| --- | --- |
| Instalar o binário distribuído | Verificar checksum externo e interno, extrair com limites e executar fora do checkout; sem build tools/serviços no runtime. CI por target e relatório de quickstart. |
| Primeiro documento, consulta e SSE | Executar comandos do Markdown incluído no pacote e preservar 9007199254740993. Medir marcos com teto 600 s; não incluir tempos não medidos. |
| Linux/macOS | Somente baselines declaradas: Linux x86_64 GNU/glibc 2.39 e macOS 15 Apple Silicon. Consultar jobs completos do commit, não passagem local de outra plataforma. |
| CLI e API documentadas | [CLI](./CLI.md), [API](./API.md), exemplos reais e checker de rotas/erros/campos/links. Sem compatibilidade ReQL/driver externo. |
| Backup offline | [Contrato](./BACKUP.md): lock exclusivo, um snapshot, hashes, staging/publicação sem sobrescrita. Destino novo e pai privado; não copiar redb aberto. |
| Restore independente | [Contrato](./RESTORE.md): validar bytes/estrutura, raiz nova, novas credenciais; antigas recebem 401. Testes de corrupção/interrupção e comandos do pacote/imagem. |
| Credenciais | Primeiro boot privado, rotate/revoke offline e expiração. Probe autenticado não renova chaves. Sem token em argv/env/logs/relatórios públicos. |
| Rede remota | Loopback default, Host/Origin exatos e proxy HTTPS no mesmo namespace. Nenhuma exposição pública automática por Docker/Ingress. |
| Contêiner | UID 10001, volume pai incluindo credenciais, root read-only, limites e probes; testes reais de Docker e Kubernetes/PVC com uma réplica. Sem HA. |
| Logs/métricas/diagnóstico | Coletor Prometheus opcional e externo; custos ID/scan e overflow correlacionados por request ID. Relatório com fonte/duração/cobertura, sem payload/segredo. |
| Licença | MIT preservada, grafo normal/build por target, textos integrais e avisos Rust std. [Distribuição](./DISTRIBUTION.md) e [dependências](./dependencies.md); catálogo std contém componentes condicionais, não muda licença Kiarni. |
| Changelog/limitações | [CHANGELOG](../CHANGELOG.md) em desenvolvimento e esta revisão; não confundir número Cargo com release aprovada. |
| Upgrade/rollback | Formato persistido 1, sem migração/upgrade automático ou compatibilidade entre versões prometida. Não houve teste de salto entre releases distintas. Backup testado é requisito antes de ensaiar troca em raiz independente. |
| RF-04 | Avaliação concluída; ADR 0033 e resultados pareados versionados. Integração segue incremento separado, sem retirar requisito de avaliação. |
| Aceite técnico/publicação | Gate técnico aceito após CI completa do commit de encerramento; canal experimental/compatibilidade em RELEASE_POLICY. Tags, Release e registry não executados. SDK permanece pergunta do roadmap, sem novo requisito nesta avaliação. |

## Evidências reproduzíveis

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 scripts/check_docs.py
python3 scripts/check_product_dependencies.py
python3 -m unittest discover -s scripts -p test_release.py
```

Para arquivos distribuídos, seguir [validação operacional](./OPERATIONAL_VALIDATION.md)
e [distribuição](./DISTRIBUTION.md). O CI acrescenta quickstart em cada runner
nativo novo e uma observação Linux de 120 s com Prometheus real; publica somente
JSON sanitizado, não credenciais, banco/backup, raw logs ou TSDB.

A execução base [43771865935](https://github.com/sergiomonteiro/kiarni/actions/runs/43771865935)
validou os nove jobs do commit 54f2165, incluindo binários Linux/macOS,
Chromium/HTTPS e Docker/Kubernetes. É a origem do pacote limpo usado na observação
local inicial. Os jobs novos desta entrega precisam passar no **commit final**;
relatórios da execução anterior não certificam arquivos novos. Resultados e
ambientes reais adicionais devem ser registrados na
[issue #7](https://github.com/sergiomonteiro/kiarni/issues/7), com fonte/hashes e
sem transformar ensaio iniciado ou QA dirty em aceite de release.

A janela longa local de referência é 900 s; conferir na issue #7 o relatório
efetivamente concluído, com fonte/duração. Uma execução iniciada não é evidência. A automação é limitada e não certifica dias de operação, hardware/rede
arbitrários, queda de energia, armazenamento cheio ou todos os provisionadores.
A decisão atual combina essas evidências com a avaliação RF-04 e a política
do canal. Não depende de remover limitações ou de prometer desempenho/produção.

## Resultados concluídos desta avaliação

Relatórios sanitizados versionados, sem token/payload/paths privados:

| Execução | Fonte e resultado |
| --- | --- |
| Quickstart Linux | [JSON](./validation/quickstart-b1b2604-x86_64-unknown-linux-gnu.json), pacote b1b2604: documento/evento em 0,131 s, query em 0,143 s; total 0,222 s. |
| Quickstart macOS | [JSON](./validation/quickstart-b1b2604-aarch64-apple-darwin.json), pacote b1b2604: documento/evento em 0,213 s, query em 0,231 s; total 0,330 s. |
| Observação externa Linux | [JSON](./validation/operational-observation-54f2165.json), pacote 54f2165: 900 s observados, 449 escritas de progresso confirmadas, 902 scrapes com mínimo up=1 e 12 eventos no leitor rápido; restart preservou documento/credencial. |

Os quickstarts foram executados nos runners novos da
[CI 43777659564](https://github.com/sergiomonteiro/kiarni/actions/runs/43777659564).
Os tempos são de uma execução automatizada do arquivo já disponível, não incluem
download, leitura humana ou Gatekeeper e não são benchmark de latência do banco.
A observação longa usa o pacote anterior explicitamente identificado; conferir
hash do binário nos relatórios em vez de presumir equivalência entre builds.
Para esses pacotes Linux 54f2165 e b1b2604, os SHA-256 dos executáveis são iguais;
isso não promete reprodutibilidade bit a bit de qualquer rebuild futuro.
A CI do commit final deve passar separadamente e permanece vinculada na issue #7.
Esses resultados compõem o aceite técnico junto da avaliação RF-04 e da CI final.

## Encerramento e próximos incrementos

A issue #8 fecha a avaliação prevista no RF-04. A issue #7 pode ser encerrada com
esta decisão técnica após a CI do commit final, sem esperar uma publicação estável
não solicitada. Os links e resultados remotos de encerramento ficam nas issues.

Próximos incrementos: lifecycle durável/seleção de plano para integrar o índice
secundário conforme ADR 0033 e a investigação #9 no momento combinado. O gate
não transforma esses trabalhos em funcionalidades existentes ou autoriza mudança
de formato sem ADR. CPU/RSS e falhas físicas continuam fora das alegações desta alpha.
