# Revisão do candidato e gate M3

Data inicial: 2026-11-15. RF-08/09/10 e revisão final do
[PRD](../KIARNI_PRD_v0.1.md). **Decisão: gate M3 não aprovado nesta revisão.**
Não criar tag, Release estável ou publicar imagem em registry a partir deste
relatório. O resultado de uma avaliação pode ser negativo mesmo com CI verde.

## Motivos e fronteira da entrega

O RF-04 exige avaliar índice secundário simples e registrar correção/custos de
leitura e escrita; a [issue #8](https://github.com/sergiomonteiro/kiarni/issues/8)
continua aberta. O índice primário do produto está implementado, mas isso não
retira a avaliação condicionada do PRD. Não há decisão de adoção/adiamento nesta
entrega, nem mudança de formato persistido ou implementação de novo índice.

A distribuição ainda é **experimental**, por artifacts temporários de Actions.
O pacote/imagem tem validações técnicas, mas a escolha de versão/canal de release,
publicação oficial, notas finais e revisão de compatibilidade pré-1.0 exigem
encaminhamento explícito. O número 0.1.0 no Cargo.toml não declara estabilidade.
O [changelog](../CHANGELOG.md) registra estado em desenvolvimento.

O quickstart automatizado mede o arquivo local disponível até o primeiro
resultado/evento. Download, quarentena macOS e leitura humana não foram medidos;
não há evidência de sessão humana em máquina limpa concluindo tudo em dez minutos.
A pergunta do PRD sobre SDK inicial também continua sem decisão: documentar HTTP
para clientes genéricos não é prova de SDK implementado. Esses pontos devem ser
resolvidos explicitamente na revisão final, sem ampliar/retirar escopo em silêncio.

## Matriz de RFs

“Atendido funcionalmente” descreve o recorte implementado/testado, não aprovação
de release ou operação universal. Os guias históricos M1/M2 preservam suas datas;
a CI atual e os relatórios por commit complementam essas evidências.

| RF | Estado da revisão | Implementação/evidência e limites |
| --- | --- | --- |
| RF-01 Inicialização/configuração | Atendido funcionalmente | [CLI](./CLI.md), precedência estrita, diretórios privados/lock, shutdown/restart; testes reais e exemplos de configuração no CI. |
| RF-02 Documentos/coleções | Atendido funcionalmente | [API](./API.md), CRUD/IDs, duplicatas/tetos, PUT/PATCH/no-op, reinício e recuperação por testes de processo. Não certifica falha física. |
| RF-03 Consultas | Atendido funcionalmente | AST validada com fixtures, JSON exato, filtro/sort/limit/project/ID, snapshots e scans limitados; sem código arbitrário. Métricas explicam caminho/etapas, não plano completo. |
| RF-04 Índices | Parcial; pendência do gate | Índice primário consistente/lookup; avaliação de campo secundário e decisão fundamentada ainda na #8. Não declarar equivalência com/sem índice secundário como entregue. |
| RF-05 Changefeeds | Atendido no escopo de coleção | [Gate M2](./M2_REVIEW.md), publicação pós-commit, duas conexões, ordem, overflow/cancelamento e no-op. Coleção inteira, best-effort futuro; sem snapshot/replay/exactly-once. |
| RF-06 Rede | Atendido funcionalmente | `/v1`, erros/request IDs, health, concorrência limitada, HTTP/JSON e SSE com testes independentes. Saturação pode aguardar backlog/fechar conexão; não promete sempre envelope. |
| RF-07 Segurança | Atendido no perfil documentado | Loopback, auth para toda API, arquivos privados, logs sem segredos e [HTTPS opcional por proxy local](./REMOTE_HTTPS.md) testado. Não homologa toda operação pública/certificado. |
| RF-08 Dashboard | Atendido no perfil Chromium testado | [Revisão local](./DASHBOARD_REVIEW.md), binário único, estado/consultas/count/feed/métricas/erros, CSP/auth. Não é auditoria de acessibilidade ou homologação de todos os browsers. |
| RF-09 Observabilidade | Instrumentação e validação controlada | Logs tipados, métricas HTTP/query/storage/feeds e anel de erros; [ensaio externo](./OPERATIONAL_VALIDATION.md) com diagnóstico de query/consumidor. Janela/duração só valem quando concluídas; sem CPU/RSS/auditoria durável. |
| RF-10 Distribuição/docs | Parcial para release final | Pacotes Linux/macOS, imagem/Compose/Kubernetes e [referências](./README.md); [quickstart empacotado](./QUICKSTART.md) com relatório. Artifact experimental não é canal estável; aceite humano/publicação permanecem em revisão. |

Baseline reproduzível de performance já está em [PERFORMANCE.md](./PERFORMANCE.md),
com dados/hardware/comandos. A regressão de leitura/custo de logs na
[issue #9](https://github.com/sergiomonteiro/kiarni/issues/9) continua registrada.
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
| RF-04 | Em aberto na #8; revisar decisão antes de fechar gate. |
| Aceite final/publicação | Em aberto: revisar resultados do commit, experiência humana/limites macOS, canal/versionamento e encaminhar a questão do SDK. Não foi executada publicação estável. |

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
A decisão negativa do gate independe de essas verificações técnicas passarem:
continuam existindo as pendências explícitas acima.

## Próximo incremento

Concluir a avaliação condicionada do RF-04 na #8, preservando a política de
spikes/ADR e sem alterar formato persistido por conveniência. Depois reavaliar
esta matriz com as evidências de instalação/operação e decisões de release.
A issue #7 permanece aberta como acompanhamento do gate até esse aceite final.
