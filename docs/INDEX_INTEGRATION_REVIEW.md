# Revisão de integração do índice secundário — issue #10

- Data: 2026-11-20; RF-04, evolução posterior ao aceite técnico M3.
- Base do candidato: `7fde91c6f4eb5a0ca232951a0232269ee054779a`.
- Contrato: [ADR 0034 proposto](./adr/0034-lifecycle-indice-secundario.md).
- Resultado: evidência suficiente para iniciar promoção **incremental**;
  habilitação do formato 2 no servidor **ainda não aprovada por esta revisão**.

## Evidência e seu limite

A [CI da base](https://github.com/sergiomonteiro/kiarni/actions/runs/44855206657)
concluiu os dez jobs com sucesso: Rust Linux/macOS, HTTPS/dashboard, Chromium,
dois builds de pacote, dois testes de artefatos, operação/métricas externas e
contêiner/Kubernetes. A base tem 347 testes Rust aprovados e dez workers
subordinados ignorados como testes diretos e exercitados pelos pais.
Passagem da base não certifica um commit futuro ou qualquer filesystem/hardware.

| Área | Evidência no candidato | Pendência para o produto |
| --- | --- | --- |
| Formato/catálogo | Schema estrito, tipos redb/fixture e recusa de formato incompatível antes do engine | Fachada versionada, preservando comportamento e dados do formato 1 |
| Lifecycle | Build/rebuild/remoção, auditoria, budgets, gerações, falhas e kill/restart | Administração offline sob lock e recuperação integradas ao servidor |
| Escritas | CRUD/no-op e grupos de PUT/PATCH com comparação ao formato 1, snapshots e captura | Writer real, telemetria e publicação de feeds somente após commit confirmado |
| Backup/restore | Snapshot das sete tabelas, hashes, descriptors, rename sem substituição, bootstrap e entrega vazia | Integração com comandos e primeiro serve restaurado com novas credenciais |
| Consulta | ID/índice/scan, tipos/Unicode, sort/projeção/limite e equivalência de erros/budgets | Caminho HTTP/admissão e categorias de métricas, inclusive em erros |
| Desempenho | Spike seletivo/denso e custos registrados no ADR 0033 | Medições reproduzíveis no binário integrado; limiar 1/8 ainda é hipótese |

Fontes: [formato candidato](../crates/storage/src/format_v2.rs),
[grupos](../crates/storage/src/format_v2/groups.rs),
[planner](../crates/storage/src/format_v2/queries.rs) e testes relacionados.
Os arquivos sob format_v2 continuam em `cfg(test)`. Não correspondem a um
servidor com índice secundário disponível nem a cumprimento integral dos itens
2–6 da issue. O item 1 exige propor o ADR, não aceitá-lo implicitamente.

## Fronteiras identificadas na revisão

1. **AST tipada.** O planner serializava a consulta validada para inspecionar um
   objeto JSON. Neste incremento passou a usar Query::direct_string_equality do
   engine. O acesso apenas devolve campo literal e string decodificada de eq
   direto; não extrai conjunções/ranges nem escolhe plano. Mantém strings maiores
   que 128 bytes: esse limite pertence ao storage. Campos com ponto/Unicode
   continuam literais, sem interpretação de path ou nova restrição na API.
   Três testes verificam escapes/NUL/Unicode, tipos/composições, serialização
   inalterada, ID e resultado numérico exato. Não há benchmark de ganho por isso.
2. **Abertura e escrita.** Store::open, health, CRUD, grupos e export atuais
   assumem formato 1. Não basta aceitar manifesto 2 ou remover cfg(test): uma
   escrita sem manutenção de índice introduziria falsos negativos. Nenhum
   caminho de escrita pode ser habilitado parcialmente sobre um store 2.
3. **Encapsulamento.** O candidato usa Database diretamente em fixtures. A
   fachada de produção deve impedir que callers alterem documentos/catalogação
   sem manter entradas/estatísticas, e separar envelope de instalação do domínio
   de storage. sha2/rustix do envelope continuam dev-dependencies no storage;
   o servidor já tem essas dependências para a composição operacional.
4. **Consulta/observabilidade.** O servidor hoje carrega documentos no storage,
   executa o engine em seguida e registra ID/scan. A integração deve preservar
   essa separação de tempos, admissão e erros, registrando o acesso secundário
   também quando a execução falhar. O wrapper do candidato avalia o engine para
   testar equivalência; não deve substituir silenciosamente toda a telemetria.
5. **Administração e compatibilidade.** A primeira manutenção é offline. Sua
   interface concreta ainda precisa ser documentada/testada antes de exposição.
   Não adicionar backfill online, alteração de formato in-place ou conversão
   automática. Formato 1 e backups atuais continuam no caminho suportado.
6. **Integridade.** Lookup não prova ausência de corrupção lógica arbitrária.
   Ready depende de lifecycle/auditoria e manutenção atômicos. Corrupção detectada
   deve abortar; recriar tabelas faltantes, reconstruir silenciosamente no startup
   ou mascarar divergência com scan não são estratégias aceitas.

## Sequência de promoção verificável

| Incremento | Resultado revisável | Verificação necessária antes do seguinte |
| --- | --- | --- |
| G1 — Codec compartilhado | Extrair codificação/preflight versionados para módulo compartilhado, mantendo create/open atuais restritos ao formato 1 | Fixtures 1/2, formatos inválidos e recusa de 2 sem tocar no banco; API/CLI atuais intactas |
| G2 — Fachada completa de storage | Encapsular catálogo, CRUD, grupos, auditoria e export por versão, sem expor Database ou habilitar o servidor sobre 2 | Escritas e grupos mantêm invariantes; budgets, snapshot e crash/restart; regressão do formato 1 |
| G3 — Composição operacional | Definir administração offline, lock/bootstrap/auth e envelope backup/restore versionados | Restauração + primeiro serve com credencial nova, duas publicações, nomes em disputa, origem intacta e sem migração implícita |
| G4 — Query/writer/feeds no servidor | Integrar seleção tipada, carregamento de candidatos, métricas e confirmação/publicação no writer | Clientes independentes, limites de admissão/erro, captura/ordem de feeds, backpressure e nenhum evento pré-commit |
| G5 — Aceite do recurso | Medir binário integrado e revisar contrato/documentação/ADRs | Casos seletivo/denso, leitura/escrita, comandos/ambiente/dataset reproduzíveis e CI completa do commit final |

G1 é o próximo incremento: pode ser revisado sem abrir dados de formato 2 no
produto. Cada etapa precisa preservar a recusa de caminhos incompletos. Esta
sequência não cria flags/endpoints ainda inexistentes nem exige implementar
todas as etapas na mesma sessão.

O ADR 0034 permanece proposto. Seu aceite deve distinguir contrato persistido,
habilitação operacional e hipótese de custo; uma medição favorável não substitui
segurança de dados e testes de regressão. O checklist da issue continua com um
item entregue e cinco de integração/validação pendentes, com avanços parciais.

## Verificações deste incremento

A fronteira tipada tem três testes em
[query_access.rs](../crates/engine/tests/query_access.rs), além dos testes do
planner usando o mesmo engine. Verificações locais deste incremento:

- `cargo fmt --all -- --check`: aprovado.
- `cargo clippy --workspace --all-targets -- -D warnings`: aprovado.
- `cargo test --workspace`: 350 aprovados; dez workers ignorados como testes
  diretos e exercitados pelos pais. Executado fora do sandbox para sockets locais.
- `python3 scripts/check_docs.py` e
  `python3 scripts/check_product_dependencies.py`: aprovados.
- `git diff --check`: aprovado.

A CI deste incremento é distinta da CI da base citada acima; observar o commit
enviado antes de usar sua passagem como evidência de plataforma.
Não repetir as medições antigas como se fossem feitas nesta revisão: os
relatórios do spike preservam fonte, ambiente e resultados históricos.
