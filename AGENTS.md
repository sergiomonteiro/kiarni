# AGENTS.md — Kiarni

Instruções para agentes de desenvolvimento (incluindo Codex) que atuem neste repositório.

## 1. Missão

Construir **Kiarni**, um banco de documentos JSON reativo, open source sob **MIT**, escrito em **Rust**, executado como servidor **single-node multi-cliente** em **um único binário**, com consultas composicionais, changefeeds nativos, dashboard e observabilidade embutidos.

**Princípio do produto:** Simplicidade para começar. Reatividade por natureza. Complexidade somente quando necessária.

**Tagline:** One binary. Live queries. Built-in observability.

## 2. Leia antes de alterar o código

Ordem de autoridade:

1. [`KIARNI_PRD_v0.1.md`](./KIARNI_PRD_v0.1.md): requisitos, limites do MVP, critérios de aceite e roadmap.
2. [`ARCHITECTURE.md`](./ARCHITECTURE.md): arquitetura de referência, invariantes e decisões pendentes.
3. ADRs aceitos em `docs/adr/`: decisões técnicas específicas.
4. Código e testes existentes: comportamento implementado; divergências devem ser identificadas, não tratadas automaticamente como requisito.

Se um documento estiver ausente ou houver contradição relevante, registre o problema e peça decisão antes de alterar o contrato de produto.

## 3. Regras inegociáveis

- **Não expandir o MVP** com cluster, replicação, sharding, SQL completo, busca vetorial, full-text avançado ou serviço cloud.
- **Não prometer compatibilidade** com RethinkDB/ReQL ou drivers existentes.
- **Não escolher `redb` como definitivo** sem spike e ADR; é candidato de persistência.
- **Não escolher SSE ou WebSocket por conveniência** sem registrar trade-offs e decisão.
- **Nunca emitir changefeed antes de commit confirmado.**
- **Não prometer replay ou exactly-once** para eventos do MVP.
- **Não bloquear indefinidamente writers** por assinantes lentos.
- **Não expor a API em rede pública por padrão**; bind em `127.0.0.1`.
- **Não adicionar serviços obrigatórios** em runtime; dashboard deve funcionar dentro do binário.
- **Não registrar credenciais nem payloads de documentos** em logs por padrão.
- **Não alterar licença MIT** ou remover avisos de terceiros.

## 4. Forma de trabalhar

Para cada issue ou tarefa:

1. **Ler** os trechos relevantes do PRD, da arquitetura e dos ADRs.
2. **Identificar o marco** (M0, M1, M2 ou M3), os requisitos afetados e o que está fora de escopo.
3. **Inspecionar o código atual** antes de propor mudanças; não presumir estrutura inexistente.
4. **Planejar em pequenos incrementos**, com resultado verificável e testes correspondentes.
5. **Implementar a menor solução correta**, mantendo as fronteiras dos componentes.
6. **Executar testes e verificações** relevantes; informar claramente os comandos executados e os que não puderam ser executados.
7. **Atualizar documentação** quando mudar API, configuração, comportamento ou contrato.
8. **Resumir entrega, riscos e pendências**; não declarar uma funcionalidade pronta sem evidências.

Não iniciar reestruturação ampla, renomeação de módulos, troca de dependências centrais ou mudança de protocolo sem necessidade da tarefa.

## 5. Arquitetura e qualidade de código

- Linguagem principal: Rust estável; priorizar segurança de tipos, erros explícitos e ownership claro.
- Componentes lógicos: core, storage, engine, feeds, server, binário e dashboard; a divisão física em crates pode evoluir conforme ADR.
- Mantenha o domínio independente de HTTP, UI e mecanismo concreto de persistência.
- Evite `unwrap()`/`expect()` em caminhos que recebem entrada externa ou podem falhar em produção; trate erros com contexto e sem vazamento de dados sensíveis.
- Evite `unsafe` salvo justificativa excepcional documentada e revisada.
- Prefira dependências pequenas, mantidas e compatíveis com MIT; verifique licenças transitivas antes de adicionar bibliotecas.
- Não introduza padrões genéricos, plugins ou abstrações para requisitos hipotéticos.
- Configure limites explícitos de memória, filas, payloads, resultados e concorrência.
- Não introduza bloqueio síncrono pesado em tarefas assíncronas sem análise de impacto.
- Não use `spawn` ilimitado, canais sem limite ou buffers infinitos para feeds.

## 6. Contratos que exigem cuidado especial

### Storage

Mutação de documento e atualização de índices devem permanecer consistentes. Teste reinício, interrupção abrupta, locks e recuperação. Não declare durabilidade sem teste. Backup exige snapshot consistente, não simples cópia de arquivo aberto sem garantias.

### Query engine

Trabalhe sobre AST serializável e validada; não execute código arbitrário do cliente. Garanta semântica documentada para campos ausentes, `null`, números, ordenação e limites. Testes com fixtures determinísticas são obrigatórios.

### Changefeeds

Publicação somente depois de commit confirmado. A ordem sob escrita concorrente deve ser demonstrada por testes. Filtragem reativa precisa considerar estado anterior e posterior; caso contrário, mantenha somente assinaturas de coleção. Overflow de assinante deve ser explícito e limitado. Uma sequência local **não** é um cursor de retomada durável.

### API e segurança

Versione endpoints em `/v1`; use erros estáveis e request IDs. Preserve loopback como padrão. Proteja dashboard e endpoints sensíveis. Nunca exponha tokens em logs, exemplos reais ou mensagens de erro.

## 7. Testes e comandos

Quando houver workspace Rust configurado, executar ao menos:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Quando pertinente, incluir testes de integração black-box (servidor real + clientes independentes), testes de crash/restart, testes de concorrência e backpressure. Não substituir testes por demonstrações manuais. Benchmarks devem registrar dataset, ambiente, hardware, versões e comandos reproduzíveis.

Se uma verificação não puder ser executada, diga **por quê** e não afirme que passou.

## 8. Critérios para considerar uma issue concluída

- Requisito/aceite do PRD identificado e atendido, ou desvio explicitamente aprovado.
- Testes de sucesso, erro e casos-limite relevantes adicionados.
- `fmt`, `clippy` e testes aplicáveis executados (ou impedimento declarado).
- Sem novas dependências obrigatórias em runtime.
- Sem exposição insegura de rede, credenciais ou dados sensíveis.
- Documentação e exemplos atualizados, se necessário.
- Alterações pequenas, revisáveis e coerentes com o marco atual.

## 9. Decisões: quando criar ADR ou pedir confirmação

Criar/propor ADR antes de decidir: engine de storage, formato de IDs, serialização de AST, semântica de comparação, protocolo de streaming, política de overflow, ordenação de eventos, autenticação, backup e alterações de formato persistido.

**Peça confirmação** quando a tarefa exigir mudança de escopo do PRD, compromisso de compatibilidade, dependência externa obrigatória, alteração da licença, quebra de API pública ou risco de perda de dados.

Para decisões locais reversíveis (nome de função, estrutura interna, testes), aja autonomamente e documente no PR.

## 10. Formato de entrega esperado do agente

Ao concluir cada tarefa, responda com:

1. **Objetivo e requisito:** qual RF/marco foi atendido.
2. **Alterações:** arquivos/componentes relevantes e comportamento entregue.
3. **Verificações:** comandos executados e resultados; falhas/limitações.
4. **Decisões e riscos:** ADRs criados, hipóteses e efeitos conhecidos.
5. **Próximo passo:** menor incremento lógico seguinte, sem ampliar escopo automaticamente.

## 11. Primeiras tarefas recomendadas

Comece por **M0**, nesta ordem sugerida:

1. Inicializar repositório/workspace Rust, licença MIT, CI mínimo e documentação.
2. Definir fixtures e testes de contrato para documentos, `_id`, JSON e erros.
3. Spike de `redb` (atomicidade, índices, concorrência, crash/restart e backup), com ADR.
4. Spike do fluxo commit -> evento, com concorrência e injeção de falhas.
5. Comparar SSE e WebSocket, definindo protocolo e autenticação por ADR.
6. Congelar apenas os contratos necessários para iniciar M1.

**Não implemente M1–M3 integralmente em uma única sessão.** Trabalhe em issues pequenas, com evidência verificável a cada etapa.

---

Este arquivo orienta agentes, mas não substitui o PRD nem os ADRs aceitos.
