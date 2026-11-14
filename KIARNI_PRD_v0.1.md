# Kiarni — Product Requirements Document (PRD)

**Versão:** 0.1\
**Status:** Proposta para implementação e validação técnica\
**Data:** 2026-11-08\
**Licença:** MIT\
**Tagline:** One binary. Live queries. Built-in observability.

## 1. Resumo executivo

Kiarni é um banco de dados de documentos JSON, open source, escrito em Rust, concebido para aplicações que precisam armazenar, consultar e reagir a mudanças de dados sem operar múltiplos serviços. O MVP é um servidor **single-node, multi-cliente**, distribuído como **um único executável**, com persistência local, consultas composicionais, changefeeds nativos, API de rede e dashboard administrativo embutido.

**Princípio:** Simplicidade para começar. Reatividade por natureza. Complexidade somente quando necessária.

A inspiração em RethinkDB refere-se à experiência de consultas e changefeeds, **não** à compatibilidade com ReQL, drivers ou protocolo existentes.

## 2. Problema e oportunidade

Aplicações reativas frequentemente combinam banco de dados, mecanismo de publicação de eventos, componentes de sincronização, ferramentas de administração e observabilidade. Essa composição aumenta a carga operacional para casos de uso pequenos e médios. Kiarni propõe uma alternativa integrada, sem prometer substituir sistemas especializados em todos os cenários.

### Público-alvo
- Desenvolvedores backend e full-stack construindo aplicações com atualização em tempo real.
- Equipes pequenas que valorizam deploy simples e observabilidade imediata.
- Produtos internos, protótipos e serviços de produção com carga compatível com um único nó.

### Casos de uso prioritários
- Painéis operacionais com atualizações em tempo real.
- Aplicações colaborativas simples e notificações de alteração.
- Backends de produtos que consultam documentos JSON e acompanham alterações filtradas.

### Não objetivos do MVP
- Clusterização, sharding, replicação ou failover automático.
- Compatibilidade binária ou de API com RethinkDB.
- Transações distribuídas, SQL completo, busca textual avançada, vetores ou analytics distribuído.
- Garantia de entrega exatamente uma vez em changefeeds.
- Serviço gerenciado ou console cloud separado.

## 3. Proposta de valor e princípios

1. **One binary:** servidor e dashboard integrados; sem dependências externas obrigatórias para iniciar.
2. **Live by default:** mudanças observáveis por assinaturas nativas.
3. **Developer-first:** API previsível, consultas composicionais, erros claros e quickstart curto.
4. **Operational clarity:** métricas, logs e diagnóstico disponíveis desde o primeiro boot.
5. **Safe foundations:** consistência e durabilidade documentadas, sem promessas não verificadas.
6. **Complexidade progressiva:** recursos avançados apenas quando sustentados por necessidades reais.

## 4. Escopo funcional do MVP

### RF-01 — Inicialização e configuração
- Executar `kiarni serve --data-dir ./data` com configuração mínima.
- Configuração via CLI e variáveis de ambiente; precedência documentada.
- Criar diretório de dados e metadados necessários; falhar com erro claro se houver conflito de lock ou permissões.
- Encerramento gracioso com fechamento de conexões e flush conforme contrato do mecanismo de persistência.

**Aceite:** instalação local e inicialização documentadas; reiniciar o processo preserva dados confirmados.

### RF-02 — Banco de documentos e coleções
- Criar, listar e excluir coleções.
- Inserir, obter por ID, atualizar/substituir e excluir documentos JSON.
- Identificador único por documento, gerado pelo servidor quando ausente; campo reservado `_id`.
- Rejeitar JSON inválido, IDs duplicados e documentos acima de limite configurável.
- Documentar regras de comparação de tipos, campos ausentes, valores `null` e limites de tamanho.

**Aceite:** suíte de testes CRUD, incluindo erros e reinício com persistência.

### RF-03 — Consultas composicionais
- API de consultas com composição de operações: `filter`, `sort`, `limit`, `project` e busca por ID.
- Contrato de consulta independente de linguagem, com representação serializável para transporte de rede.
- Sem execução arbitrária de código no servidor.
- Paginação ou limites obrigatórios para evitar respostas ilimitadas.
- Definir semântica de operadores, ordenação, coerção e comportamento de campos ausentes.

**Aceite:** consultas determinísticas e testadas sobre conjuntos fixos de documentos; plano ou explicação simples de execução quando viável.

### RF-04 — Índices essenciais
- Índice primário por `_id`.
- Avaliar índice secundário de campo simples como parte do MVP, condicionado à prova de conceito do storage.
- Sem índices complexos, compostos ou geoespaciais no primeiro marco.

**Aceite:** correção de resultados com e sem índice; benchmarks registram custo de leitura e escrita.

### RF-05 — Changefeeds nativos
- Cliente pode assinar alterações de uma coleção e, se tecnicamente viável, de uma consulta filtrada.
- Eventos mínimos: `insert`, `update`, `delete`, com identificador e dados relevantes.
- Definir explicitamente o ponto de publicação: eventos emitidos **após commit confirmado**.
- Ordem garantida **por nó e dentro do escopo definido da assinatura**; formalizar semântica no contrato.
- Conexões encerradas e consumidores lentos não podem bloquear indefinidamente a escrita.
- MVP: entrega **best-effort, enquanto conectado**; sem replay durável ou retomada garantida após desconexão.
- Assinaturas filtradas devem aplicar o predicado tanto ao estado anterior quanto ao novo para identificar entrada/saída do conjunto de resultados, ou restringir o recurso até essa semântica estar implementada.

**Aceite:** duas conexões clientes recebem alterações após commit; teste de consumidor lento, desconexão, reconexão e encerramento.

### RF-06 — Servidor de rede
- API HTTP/JSON versionada para operações de administração e dados.
- Streaming de eventos por WebSocket ou SSE, a decidir por spike; documentar decisão.
- Suporte a clientes concorrentes com limites configuráveis de conexões e filas.
- Endpoints de saúde: liveness e readiness; readiness depende da disponibilidade efetiva do storage.
- Erros com códigos e formato estáveis.

**Aceite:** múltiplos clientes independentes executam CRUD e assinaturas simultaneamente.

### RF-07 — Segurança mínima
- Bind padrão em `127.0.0.1` para evitar exposição acidental.
- Autenticação obrigatória para acesso remoto; mecanismo simples de token/API key a definir.
- Credenciais não aparecem em logs e não são incluídas em exemplos públicos.
- TLS: documentar suporte nativo ou operação segura atrás de proxy; acesso remoto sem proteção não deve ser recomendado.
- Autorização granular por coleção e multi-tenancy ficam fora do MVP.

**Aceite:** sem credencial, chamadas remotas protegidas são recusadas; testes de exposição e logs.

### RF-08 — Dashboard web embutido
- Servido pelo mesmo processo, sem servidor frontend separado.
- Exibir status, versão, uptime, coleções, contagem aproximada/exata conforme contrato, conexões e atividade de changefeeds.
- Navegar documentos e executar consultas limitadas.
- Visualizar métricas básicas e erros recentes, respeitando políticas de segurança.
- A interface administrativa compartilha os controles de autenticação do servidor.

**Aceite:** após iniciar o binário, usuário acessa dashboard local, consulta coleção e acompanha mudanças.

### RF-09 — Observabilidade
- Logs estruturados com níveis configuráveis e identificação de requisições.
- Métricas: requisições, latência, erros, conexões, assinaturas, fila de eventos, uso de storage e operações de leitura/escrita.
- Endpoint de métricas em formato interoperável (preferencialmente Prometheus), com controle de exposição.
- Não registrar payloads sensíveis de documentos por padrão.

**Aceite:** métricas e logs permitem diagnosticar uma consulta lenta e um consumidor atrasado.

### RF-10 — Distribuição e documentação
- Binários para Linux e macOS conforme viabilidade do pipeline; outras plataformas por demanda.
- Imagem Docker oficial com volume persistente, healthcheck e execução não-root quando possível.
- Exemplos de Docker Compose e manifesto básico para Kubernetes com volume persistente, sem alegar alta disponibilidade.
- Quickstart com primeiro documento, consulta e changefeed em até 10 minutos para usuário familiarizado com CLI.
- Licença MIT no repositório e avisos de terceiros preservados.

**Aceite:** instalação limpa executa o quickstart sem serviços externos.

## 5. Contrato de dados e API — proposta inicial

**Modelo:** coleções de documentos JSON; `_id` é único dentro da coleção. A semântica de atualização parcial versus substituição deve ser explícita e separada.

**API ilustrativa, não definitiva:**

```http
POST /v1/collections
POST /v1/collections/orders/documents
GET  /v1/collections/orders/documents/{id}
PATCH /v1/collections/orders/documents/{id}
DELETE /v1/collections/orders/documents/{id}
POST /v1/collections/orders/query
GET  /v1/health/live
GET  /v1/health/ready
```

**Consulta ilustrativa:**

```json
{
  "from": "orders",
  "filter": { "status": { "eq": "paid" } },
  "sort": [{ "field": "created_at", "direction": "desc" }],
  "limit": 50
}
```

**Evento ilustrativo:**

```json
{
  "type": "update",
  "collection": "orders",
  "id": "ord_123",
  "sequence": 1042,
  "after": { "_id": "ord_123", "status": "paid" }
}
```

`sequence` é identificador de ordenação local, **não** promessa de replay após reconexão. Formatos definitivos devem ser congelados antes do primeiro SDK.

## 6. Arquitetura de referência

```text
Clientes / SDKs / CLI
        |
        v
API HTTP/JSON + Streaming (SSE ou WebSocket)
        |
        +---- Auth / Limits / Request lifecycle
        |
        v
Query Parser / Planner / Executor
        |
        +---- Collections / Documents / Indexes
        |              |
        |              v
        |        Storage Abstraction
        |              |
        |              v
        |          redb (candidato)
        |
        +---- Changefeed Dispatcher (post-commit)
        |
        +---- Metrics / Logs / Health
        |
        +---- Embedded Admin UI
```

**Decisões consolidadas:** Rust; single-node; binário único; documentos JSON; consultas composicionais; changefeeds; dashboard integrado; MIT.

**Hipóteses a validar:** redb como engine; modelo de índices; SSE versus WebSocket; estratégia de concorrência; atomicidade entre mutações, índices e eventos; limites de tamanho e throughput.

### Spike obrigatório de persistência
Construir prova de conceito que avalie redb para: operações transacionais, recuperação após interrupção abrupta, persistência de índices, crescimento do arquivo, concorrência de leitura/escrita e integração com publicação post-commit. Comparar com ao menos uma alternativa ou justificar tecnicamente a permanência.

## 7. Semântica e confiabilidade

- **Consistência:** documentar visibilidade de leituras após escrita confirmada e isolamento oferecido pelo storage.
- **Durabilidade:** testar recuperação de dados confirmados após encerramento abrupto; não prometer proteção contra perda física sem backups.
- **Atomicidade:** mutações de documento e índices devem ser consistentes; em falha, não deixar índices apontando para dados inexistentes.
- **Changefeeds:** notificação posterior ao commit, não substitui log durável de eventos; não garante entrega a clientes desconectados.
- **Backpressure:** filas limitadas e política explícita de desconexão/sinalização de overflow.
- **Backup:** procedimento documentado de backup consistente e restauração testada antes de declarar o MVP pronto para produção.

## 8. Requisitos não funcionais

| Dimensão | Requisito MVP | Verificação |
|---|---|---|
| Instalação | Um binário e diretório persistente | Quickstart automatizado |
| Confiabilidade | Dados confirmados sobrevivem a reinício e crash testado | Testes de falha e recuperação |
| Segurança | Sem exposição remota insegura por padrão | Testes de bind e autenticação |
| Performance | Estabelecer baseline, sem meta arbitrária de TPS | Benchmark reprodutível com hardware descrito |
| Operação | Health, métricas e logs estruturados | Testes de integração |
| Portabilidade | Build e execução em plataformas-alvo | CI e testes de instalação |
| Qualidade | Testes unitários, integração e propriedades críticas | Pipeline de CI |
| Compatibilidade | API `/v1` documentada; política de mudanças pré-1.0 | Testes de contrato |

## 9. Experiência do desenvolvedor

Fluxo desejado:

```bash
kiarni serve --data-dir ./kiarni-data
# acessar dashboard local
# criar coleção e inserir documento
# executar consulta
# abrir assinatura e observar alteração
```

SDK inicial: priorizar **um** SDK com base em facilidade de adoção e testes de contrato (TypeScript é candidato), sem exigir SDKs em múltiplas linguagens no MVP. Documentar API HTTP para clientes genéricos.

## 10. Roadmap e gates

### M0 — Spikes e contratos
- Validar redb, modelo de transações e evento pós-commit.
- Definir contrato de consulta, API, segurança e streaming.
- Criar benchmarks-base e testes de recuperação.

**Gate:** decisões técnicas documentadas em ADRs; riscos bloqueadores resolvidos.

### M1 — Banco funcional
- Processo servidor, storage, coleções, CRUD, consultas básicas e índice primário.
- API versionada, configuração, health e testes.

**Gate:** operações multi-cliente e recuperação validadas.

### M2 — Banco reativo
- Dispatcher pós-commit, assinaturas, semântica de eventos, backpressure e métricas.
- Consultas filtradas reativas apenas após semântica correta de entrada/saída do conjunto.

**Gate:** testes concorrentes, desconexão e consumidores lentos passam.

### M3 — Experiência integrada
- Dashboard embutido, documentação, CLI, Docker, backup/restauração e segurança de acesso remoto.

**Gate:** quickstart reproduzível; checklist de release e licença completos.

### Pós-MVP (não comprometido)
- Índices compostos e otimizador de consultas mais sofisticado.
- Resume tokens e replay durável de changefeeds.
- Autorização granular, SDKs adicionais, replicação e clusterização.

## 11. Métricas de sucesso

- **Time to first document:** tempo entre baixar o binário e persistir o primeiro documento.
- **Time to first live update:** tempo até observar primeira mudança via assinatura.
- **Quickstart success rate:** proporção de execuções automatizadas que concluem o tutorial.
- **Correctness:** zero violações conhecidas de invariantes de CRUD/índices em testes de falha.
- **Changefeed health:** assinaturas ativas, desconexões por overflow e latência commit→notificação.
- **Operational usability:** capacidade de identificar erros e gargalos apenas com dashboard/logs/métricas.

Metas quantitativas de desempenho serão definidas **após** M0, com ambiente e cargas de referência.

## 12. Riscos e mitigação

| Risco | Impacto | Mitigação |
|---|---|---|
| Limitações de redb para padrões de escrita/índice | Alto | Spike e abstração de storage |
| Inconsistência entre commit e eventos | Alto | Publicar somente após commit e testar falhas |
| Consultas reativas com semântica incorreta | Alto | Definir regras de entrada/saída e limitar escopo |
| Consumidor lento causa pressão de memória | Alto | Filas limitadas e política de overflow |
| Dashboard amplia superfície de ataque | Alto | Bind local, autenticação e controles de exposição |
| Escopo de query engine cresce demais | Alto | Operadores MVP fechados; sem compatibilidade ReQL |
| Percepção indevida de HA | Médio | Comunicar explicitamente single-node |
| Nome/marca sem liberação formal | Médio | Validar marcas e identificadores antes de lançamento público |

## 13. Questões em aberto (para ADRs)

1. redb atende requisitos de recuperação, índices e concorrência? Qual alternativa de contingência?
2. SSE ou WebSocket para changefeeds? Quais requisitos reais de bidirecionalidade?
3. Qual estratégia de serialização e validação de JSON, incluindo números e ordenação?
4. Qual modelo de autenticação remota e rotação de credenciais?
5. Como definir snapshot inicial de uma consulta reativa sem perder eventos entre snapshot e assinatura?
6. Quais limites padrão de documentos, consultas, conexões e filas?
7. Qual mecanismo de backup consistente com o storage escolhido?
8. Qual SDK inicial e como serão feitos testes de conformidade?

## 14. Definição de pronto para o MVP

O MVP só será considerado pronto quando: (a) CRUD e consultas operarem em clientes independentes; (b) persistência e recuperação forem testadas; (c) changefeeds pós-commit tiverem contrato documentado e testes de backpressure; (d) acesso remoto exigir configuração segura; (e) dashboard e observabilidade funcionarem no mesmo executável; (f) backup e restauração forem demonstrados; (g) Docker, quickstart e licença MIT estiverem publicados; e (h) limitações single-node e de entrega de eventos estiverem claramente documentadas.

## 15. Identidade e comunicação

**Nome:** Kiarni (derivado criativamente do islandês *kjarni*, “núcleo/essência”).\
**Posicionamento:** The essential database.\
**Assinatura técnica:** One binary. Live queries. Built-in observability.\
**Visual:** minimalismo nórdico, símbolo K geométrico, Midnight `#101820`, Aurora `#39D6B4`, Frost `#E9F0ED`; tipografia limpa e foco em legibilidade.\
**Licença:** MIT. O arquivo `LICENSE` deverá conter o texto integral da licença e o titular/ano de copyright corretos, definidos pelos mantenedores.

---

**Controle de mudanças:** v0.1 — primeira consolidação de produto. Decisões de implementação ainda abertas devem ser registradas em ADRs, não assumidas como requisitos definitivos.
