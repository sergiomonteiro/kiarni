# Kiarni — Architecture

**Versão:** 0.1\
**Status:** Arquitetura de referência; decisões pendentes identificadas\
**Fonte de requisitos:** [`KIARNI_PRD_v0.1.md`](./KIARNI_PRD_v0.1.md)\
**Licença do projeto:** MIT

> **Princípio:** Simplicidade para começar. Reatividade por natureza. Complexidade somente quando necessária.

## 1. Objetivo e limites

Kiarni é um servidor de documentos JSON, multi-cliente e single-node, escrito em Rust, com consultas composicionais, changefeeds nativos, dashboard embutido e observabilidade. É distribuído como um único executável com armazenamento persistente local.

O MVP **não** implementa clusterização, replicação, sharding, failover automático, compatibilidade com protocolo ReQL, SQL completo, entrega exatamente uma vez ou replay durável de changefeeds. O PRD define os requisitos; este documento propõe uma implementação verificável e evolutiva.

### Restrições arquiteturais

1. Um único processo para API, storage, dispatcher e dashboard; sem serviços obrigatórios externos.
2. Persistência confirmada antes da emissão de eventos.
3. Sem executar código fornecido pelo usuário no servidor.
4. Limites explícitos para documentos, consultas, conexões, filas e tempo de execução.
5. Segurança por padrão: bind em loopback e autenticação para acesso remoto.
6. Interfaces internas claras, mas sem microserviços ou abstrações especulativas.

## 2. Visão de componentes

```text
SDKs / CLI / Browser
        |
        v
+-------------------------------------------------------+
| Kiarni (single binary)                                |
|                                                       |
| HTTP API / Streaming / Embedded UI                    |
|        |                 |                            |
|   Auth + limits     Health / metrics / logs            |
|        |                                              |
|  Request handlers                                     |
|        |                                              |
|  Query AST -> Validation -> Planner -> Executor        |
|        |                    |                         |
|        |             Collection / Index service        |
|        |                    |                         |
|        +-------------> Storage interface               |
|                              |                        |
|                        redb (candidate)                |
|                              |                        |
|                      successful commit                |
|                              |                        |
|                      Changefeed dispatcher ---------->| streaming subscribers
+-------------------------------------------------------+
                       |
                 Persistent volume
```

A seta de commit representa uma **fronteira semântica**, não uma promessa de que uma publicação em memória é atômica com a transação persistida. Essa limitação é deliberada no MVP.

## 3. Organização do código do produto

O workspace separa bibliotecas do produto, composição do binário e harnesses
experimentais. A promoção dos componentes está registrada no ADR 0014.

| Local | Responsabilidade |
|---|---|
| apps/kiarni | CLI, fontes de configuração e chamada do servidor |
| crates/core | Validação de documentos e preparação de inserções |
| crates/engine | AST, avaliação de consultas e transformações PUT/PATCH |
| crates/storage | Formato redb, catálogo, snapshots e mutações transacionais |
| crates/auth | Credenciais, autoridade e journal de ativação |
| crates/config | Resolução de configuração e admissão limitada |
| crates/http | Rotas, validação de corpos, request IDs e respostas de erro |
| crates/server | Bootstrap, coordenação, sockets e handlers |
| crates/feeds | Dispatcher integrado ao writer e framing SSE limitado |
| spikes | Experimentos de storage/feeds/transporte e entradas de compatibilidade |
| docs | Requisitos, ADRs, operação e evidências |
| scripts | Verificação da fronteira de dependências |

Core e engine permanecem independentes de HTTP e persistência. Storage usa os
contratos de domínio e mantém consistência na transação. Server compõe as
bibliotecas e coordena as requisições. Os testes HTTP black-box ficam em
apps/kiarni/tests. Contratos de biblioteca acompanham seus crates.

As dependências normais e de build de apps/crates devem alcançar somente
componentes do produto e bibliotecas externas. Os harnesses podem depender
do produto. A checagem de workspace está no CI.
[ADR 0014](./docs/adr/0014-organizacao-do-workspace.md)

## 4. Modelo de dados e persistência

### Documentos e coleções

- Coleções são namespaces de documentos JSON com `_id` único por coleção.
- `_id` é reservado; geração no servidor quando ausente. O formato final do ID exige ADR.
- Inserção, substituição, atualização parcial e exclusão são operações distintas.
- JSON inválido, documentos grandes e identificadores duplicados retornam erros tipados.
- A semântica de `null`, campo ausente, números, comparação e ordenação será especificada em testes de contrato antes de SDKs.

### Storage interface (conceitual)

```rust
// Pseudocódigo de contrato; não é uma API Rust estabilizada.
trait Storage {
    fn get(&self, collection: &str, id: &str) -> Result<Option<Document>>;
    fn scan(&self, collection: &str, bounds: ScanBounds) -> Result<DocumentStream>;
    fn commit(&self, mutation: MutationBatch) -> Result<CommitOutcome>;
}
```

A implementação deve oferecer **atomicidade entre dados e metadados/índices** dentro da mesma unidade de commit. Não prometer operações multi-documento transacionais na API pública sem requisito e validação específicos.

### redb: hipótese, não decisão final

Executar spike M0 para validar: concorrência entre leitores e escritores; durabilidade após crash; transações e atualizações de índices; tamanho/crescimento do arquivo; scans; backups consistentes; latência de commit; compatibilidade com distribuição em binário único. Comparar pelo menos uma alternativa ou registrar justificativa técnica para não comparar. Registrar resultado em ADR antes de depender do engine em APIs públicas.

### Índices

- Índice primário `_id` é obrigatório.
- Índice secundário simples é candidato condicionado ao spike.
- Índices devem ser atualizados na mesma transação da mutação, quando suportado.
- Query planner começa com lookup por ID ou scan limitado; usar índice apenas quando houver equivalência comprovada por testes.
- Sem índices compostos, full-text ou geoespaciais no MVP.

## 5. Query engine

Pipeline: **parse -> validate -> normalize AST -> plan -> execute -> serialize**.

A API de transporte recebe uma AST serializável, independente de SDK. Operações iniciais: busca por ID, `filter`, `sort`, `limit`, `project`. Consultas devem ser determinísticas sob regras documentadas e ter limite máximo de resultados. Não executar JavaScript, SQL arbitrário nem expressões enviadas como código.

Exemplo **ilustrativo** (não contrato congelado):

```json
{
  "from": "orders",
  "filter": { "status": { "eq": "paid" } },
  "sort": [{ "field": "created_at", "direction": "desc" }],
  "limit": 50
}
```

O planejador inicial pode privilegiar correção e simplicidade sobre otimizações. Testar filtros combinados, ordenação estável, valores ausentes, limites e comparação de resultados indexados versus scans.

## 6. Caminho de escrita e changefeeds

```text
HTTP mutation
  -> authentication + validation + limits
  -> collection service
  -> storage transaction (document + affected indexes)
  -> successful durable commit
  -> construct change event
  -> dispatch to in-memory bounded subscriber queues
  -> SSE/WebSocket clients (protocol TBD)
```

### Contrato mínimo

- Eventos `insert`, `update`, `delete`, com coleção, ID e conteúdo necessário para o consumidor.
- **Nunca publicar antes do commit confirmado.**
- Ordem de eventos definida por nó e escopo de assinatura; sequência local não implica replay.
- Entrega **best-effort enquanto conectado**. Crash entre commit e publicação pode causar evento perdido; desconexões também podem causar lacunas.
- Nenhum replay durável, cursor retomável ou exactly-once no MVP.
- Assinaturas de coleção são a primeira entrega; assinaturas de consulta filtrada exigem avaliação de estado anterior e posterior para detectar entrada, permanência e saída do resultado. Caso não estejam corretas, adiar esse recurso.
- Consumidor lento não bloqueia indefinidamente escrita: fila por assinante com capacidade fixa; política de overflow definida em ADR (por exemplo, encerrar assinatura com erro explícito).
- O dispatcher não deve reter histórico ilimitado nem payloads indefinidamente.

### Concorrência e ordenação

O spike deve decidir se há um serializador lógico de commits ou outro mecanismo para preservar ordem de publicação sob escritas concorrentes. **Não assumir** que `commit()` seguido de `publish()` em tarefas concorrentes preserva a ordem dos commits. Criar teste com concorrência e injeção de atrasos para validar a solução.

## 7. API e streaming

- HTTP/JSON versionado em `/v1`; rotas iniciais ilustrativas no PRD.
- Modelo de erro estável: código de máquina, mensagem segura, request ID e status HTTP coerente.
- SSE versus WebSocket é decisão de spike M0/M2: comparar simplicidade, autenticação, reconexão, limites, bidirecionalidade necessária e ergonomia de SDK.
- Timeouts, tamanho de request, limite de resultados, conexões e assinaturas configuráveis.
- Liveness responde sobre processo; readiness somente quando storage pode atender requisições.
  GET/HEAD `/v1/status` autenticado informa versão, uptime monotônico do serviço
  HTTP e ocupação da admissão; não substitui readiness.
  [ADR 0021](./docs/adr/0021-status-do-servidor.md).
  GET/HEAD `/v1/collections/{collection}/count` conta em snapshot limitado do
  índice primário; fora do orçamento retorna limite inferior explícito. Usa
  o gate das consultas, sem alterar formato persistido ou escritas.
  [ADR 0022](./docs/adr/0022-contagem-limitada-de-documentos.md).
- Não confundir endpoint de métricas com API pública de dados.

## 8. Segurança

- Bind padrão: `127.0.0.1`.
- Acesso remoto usa Bearer existente e proxy HTTPS opcional no mesmo host; backend
  continua loopback. Origem pública HTTPS explícita rege Host/Origin; headers de
  encaminhamento não concedem confiança. [ADR 0030](./docs/adr/0030-https-por-proxy-local.md),
  [perfil e evidências](./docs/REMOTE_HTTPS.md).
- Não expor dashboard administrativo sem o mesmo controle de acesso da API.
- TLS nativo ou reverse proxy seguro: documentar opção adotada; nunca sugerir exposição de credenciais em HTTP público.
- Não registrar segredos, tokens ou conteúdo de documentos por padrão.
- Validar nomes de coleções, IDs, filtros e caminhos; impedir traversal e consumo não limitado de recursos.
- Autorização granular por coleção e multi-tenancy fora do MVP.

## 9. Observabilidade e dashboard

- Logs estruturados de requisições com request ID, código estável, duração e
  nível configurável já implementados em stderr, com fila limitada e descarte
  sob backpressure. Mensagens internas detalhadas não são retidas por padrão.
  [ADR 0023](./docs/adr/0023-logs-de-requisicoes.md).
- Métricas de conclusão HTTP, erros por classe e latência separada de respostas
  e streams já implementadas, independentes do logger.
  [ADR 0024](./docs/adr/0024-metricas-http.md). [Métricas de queries](./docs/QUERY_METRICS.md) já separam processamento total,
  espera, execução e limites, sem rótulos de AST. [Métricas de primitivas/commit de storage](./docs/STORAGE_METRICS.md) e
  tamanho lógico do arquivo já estão na API e no dashboard; páginas/cache, I/O físico e
  documentos examinados não fazem parte desse recorte. Admissão, assinaturas e
  profundidade/overflow de filas têm seus contratos próprios.
- Erros recentes: anel fixo de oito conclusões, captura sem espera por lock,
  independente do logger, sem payloads e apagado no reinício. API autenticada
  GET/HEAD `/v1/errors/recent` e coleta manual no dashboard.
  [ADR 0027](./docs/adr/0027-erros-recentes.md).
- Health endpoints e métricas interoperáveis (preferência Prometheus) com exposição controlada.
- Dashboard estático compilado/embutido no executável; não requer Node.js em runtime.
  Primeiro recorte implementado: shell público sem dados, leituras autenticadas de
  health, coleções e consultas limitadas; token somente em memória.
  O [painel de mudanças](./docs/DASHBOARD_FEEDS.md) usa fetch/SSE de coleção,
  com último evento limitado e sem replay ou reconciliação automática.
  O [painel de métricas](./docs/DASHBOARD_METRICS.md) coleta manualmente as
  séries agregadas HTTP, de queries, storage e feeds, sem histórico ou conversão de contadores u64
  na exibição. Separa histogramas de respostas, streams encerrados, fanout e
  processamento total/espera/execução de queries.
  O [editor de consultas](./docs/DASHBOARD_QUERIES.md) preserva JSON numérico
  original no envio e na exibição. [ADR 0020](./docs/adr/0020-dashboard-embutido.md).
- Dashboard: status, coleções, navegação de documentos, consultas limitadas e atividade de feeds. Evitar queries ilimitadas no navegador.
- Não armazenar payloads sensíveis em telemetria por padrão.

## 10. Ciclo de vida e operação

### Formato persistido atual

Store usa um único backend, formato lógico 2, com documentos, catálogo de
coleções, definições/entradas de índices e estatísticas transacionais. O layout
1 de desenvolvimento não é suportado; não há migração ou apagamento automático.
[Decisão e invariantes](./docs/adr/0034-lifecycle-indice-secundario.md).
Bootstrap versão 2 fixa store_format 2 antes de criar o banco. Startup audita a
integridade lógica antes de preparar auth. Backup/restore preservam as sete
tabelas e restauram numa nova instalação com credenciais independentes.
O planner HTTP ainda usa ID/scan; a seleção secundária e sua observabilidade
serão conectadas em incremento próprio, sem atribuir ganhos do spike ao HTTP.

### Startup

1. Carregar CLI, variáveis de ambiente e configuração; aplicar precedência documentada.
2. Validar data directory, permissões e lock exclusivo.
3. Abrir/recuperar storage e validar versão de formato.
4. Inicializar engine, dispatcher, telemetria e API.
5. Sinalizar readiness somente após storage utilizável.

### Shutdown

Parar novos requests, encerrar feeds com sinalização quando possível, aguardar operações em curso dentro de timeout, concluir commits permitidos e liberar recursos/lock. Não prometer que uma interrupção forçada publica todos os eventos já confirmados.

### Backup e restauração

Backup offline implementado em `kiarni backup`: lock exclusivo, um snapshot,
exportação em lotes para novo store, hashes e publicação atômica sem sobrescrita.
[Contrato e evidências](./docs/BACKUP.md), [ADR 0028](./docs/adr/0028-artefato-de-backup-offline.md).
Restauração em `kiarni restore` confere hashes dos bytes copiados e estrutura,
publica entrega vazia e raiz nova em store_ready sem sobrescrita; primeiro serve
emite novas credenciais. As duas publicações têm janelas explícitas de
interrupção. [Guia](./docs/RESTORE.md), [ADR 0029](./docs/adr/0029-restauracao-em-nova-raiz.md). Não recomendar cópia arbitrária de arquivo aberto sem garantia de consistência. Documentar também limitações do modo single-node.

### Distribuição

Build release para plataformas suportadas, imagem Docker com volume, exemplo Compose e manifesto Kubernetes com PVC e **uma réplica**. Um único binário não significa alta disponibilidade.
[Distribuição experimental](./docs/CONTAINERS.md): UID 10001, volume pai de dados
e credenciais, probe autenticado do próprio binário; Compose host em Linux e
Kubernetes sem Service, preservando loopback ([ADR 0032](./docs/adr/0032-container-single-node.md)).

## 11. Testes e quality gates

| Área | Evidência mínima |
|---|---|
| Documentos | CRUD, limites, erros, reinício, `_id` único |
| Persistência | crash/restart, índices consistentes, lock exclusivo |
| Query | fixtures determinísticas; scan vs. índice |
| Changefeeds | post-commit, concorrência, slow consumer, desconexão |
| Rede | múltiplos clientes, timeout, respostas de erro |
| Segurança | loopback padrão, auth remoto, segredo fora de logs |
| Operação | health, métricas, shutdown e restore |
| Distribuição | quickstart em ambiente limpo, Docker com volume |

Executar `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` e `cargo test --workspace` quando o workspace existir. Benchmarks precisam registrar hardware, versão, dataset e metodologia; não inventar metas de throughput.

## 12. Roadmap e decisões arquiteturais

- **M0 — Spikes e contratos:** storage, protocolo de streaming, ID, formato de erro, semântica JSON, ordenação post-commit.
- **M1 — Banco funcional:** storage, coleções, CRUD, consultas, índice primário e API.
- **M2 — Banco reativo:** dispatcher, subscriptions, limites, testes de concorrência.
- **M3 — Experiência integrada:** dashboard, observabilidade, documentação, binários e Docker.

Registrar ADRs em `docs/adr/NNNN-titulo.md` com: contexto, opções, decisão, consequências, evidências/testes e status. ADRs iniciais sugeridos:

1. Storage engine e atomicidade de índices.
2. Semântica e formato de IDs.
3. Representação da AST e regras de comparação JSON.
4. SSE vs. WebSocket e autenticação de streams.
5. Ordenação de commits/eventos e política de overflow.
6. Modelo de autenticação e exposição de métricas/dashboard.
7. Estratégia de backup consistente.

## 13. Política de evolução

Antes de `1.0`, mudanças incompatíveis são possíveis, mas devem ser documentadas e cobertas por testes de contrato. Não introduzir features pós-MVP para “preparar” clusterização, event sourcing ou extensibilidade sem necessidade demonstrada. Priorizar interfaces pequenas e substituíveis, não abstrações genéricas.

---

**Relação com outros documentos:** o PRD é a fonte de verdade sobre escopo e critérios de aceite; este arquivo é a referência técnica; `AGENTS.md` define o comportamento de agentes e contribuidores automatizados. Conflitos de requisitos devem ser resolvidos no PRD, não por suposições no código.
