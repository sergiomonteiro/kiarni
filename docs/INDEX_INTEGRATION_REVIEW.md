# Revisão de integração do índice secundário — issue #10

- Data: 2026-11-20; RF-04, evolução posterior ao aceite técnico M3.
- Base do candidato: `7fde91c6f4eb5a0ca232951a0232269ee054779a`.
- Contrato: [ADR 0034 aceito](./adr/0034-lifecycle-indice-secundario.md).
- Resultado: evidência suficiente para iniciar promoção **incremental**;
  promoção para Store único concluída no incremento de consolidação abaixo;
  planner secundário HTTP e administração offline pela CLI integrados;
  medições no produto concluídas conforme relatório de 2026-11-22 abaixo.

## Estado atual: Store único no caminho normal

A decisão do responsável em 2026-11-21 retirou a exigência de retrocompatibilidade:
não há bancos em uso. A consolidação usa Store de formato 2 no servidor, writer,
bootstrap e backup/restore. Remove IndexedStore, o entrypoint separado de
inicialização, a implementação de export do layout 1 e o wrapper do spike de
formato. Seus testes de contrato/crash estão em crates/storage/tests/format.rs.

Bootstrap só aceita versão 2/store_format 2; envelope de backup continua schema
1 com format_version 2. Layouts antigos são recusados antes do engine e não são
apagados/convertidos automaticamente. Store preserva telemetria de primitivas,
commits, no-op e grupos. Abertura exige auditoria lógica; IndexedMaintenance é
recuperação restrita, não outro backend de produção.

Um teste black-box cobre backup/restore de banco com índice, primeiro serve com
credencial independente, insert/PATCH HTTP, auditoria e rebuild após shutdown e
reinício. Os testes anteriores de hashes, publicação, disputa, kill/restart,
backpressure e grupos passam a exercitar o mesmo Store normal.

Planner secundário integrado ao carregamento HTTP, preservando admissão, erros,
orçamento da coleção inteira e tempos de storage/engine. Métricas e dashboard
distinguem id/scan/secondary, inclusive em falha posterior à escolha. A
[administração offline pela CLI](./INDEXES.md) integra lock e recuperação restrita.
As [medições no produto](./benchmarks/secondary-index-http-2026-11-22.md)
concluem o recorte: 270 rodadas / 135 pares válidos, duas distribuições de tags,
dois tamanhos de dataset/documento, leitura seletiva/densa e custo de escrita.
Todos os membros foram auditados e verificados após reinício; recusas 503 são
explicitadas. Ganhos seletivos e regressões de escrita são locais, não promessa
de capacidade geral. O limiar 1/8 é mantido conservadoramente, sem prova de
optimalidade. Critérios 1–6 da issue #10 atendidos; CPU/RSS e #9 seguem separados.

As tabelas e etapas G1–G3.1 abaixo preservam o histórico e suas evidências. A
exigência antiga de manter formato 1 está superada pelo ADR 0034 revisado.

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
O núcleo sob format_v2 foi promovido para código normal por IndexedStore;
envelope/instalação continuam em cfg(test). O servidor ainda não oferece índice
secundário. O aceite restrito do ADR não conclui os itens 2–6 de integração da issue.

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
| G1 — Codec compartilhado (entregue) | Codificação/preflight compartilhados pelo Store atual e candidato, com política explícita de versão | Fixtures 1/2, formatos inválidos e recusa de 2 sem abrir o banco; API/CLI atuais intactas |
| G2 — Fachada de storage (entregue no crate) | IndexedStore encapsula formato 2, CRUD, grupos, query, auditoria/export e recuperação restrita; Store atual permanece 1 | Testes externos usando somente API pública, auditoria antes de abrir para consulta e regressão do formato 1 |
| G3 — Composição operacional (parcial: bootstrap Rust) | Lock/bootstrap/auth compostos pela API Rust; administração/CLI e envelope backup/restore ainda pendentes | Restauração + primeiro serve com credencial nova, duas publicações, nomes em disputa, origem intacta e sem migração implícita |
| G4 — Query/writer/feeds no servidor | Integrar seleção tipada, carregamento de candidatos, métricas e confirmação/publicação no writer | Clientes independentes, limites de admissão/erro, captura/ordem de feeds, backpressure e nenhum evento pré-commit |
| G5 — Aceite do recurso | Medir binário integrado e revisar contrato/documentação/ADRs | Casos seletivo/denso, leitura/escrita, comandos/ambiente/dataset reproduzíveis e CI completa do commit final |

G1/G2, G3.1 e a consolidação do Store normal estão entregues. A próxima
implementação segue o estado atual descrito acima, sem backend legado ou CLI
paralela.
Cada etapa precisa preservar a recusa de caminhos incompletos. Esta
sequência não cria flags/endpoints ainda inexistentes nem exige implementar
todas as etapas na mesma sessão.

O ADR 0034 foi aceito para a fachada Rust experimental e diretório novo, distinguindo
contrato persistido, habilitação operacional e hipótese de custo. Medição não substitui
segurança de dados e testes de regressão. O checklist da issue continua com um
item entregue e cinco de integração/validação pendentes, com avanços parciais.

## G1 — Codec no caminho de produção

[format.rs](../crates/storage/src/format.rs) é código normal do crate storage,
usado por Store::create/open para formato 1 e pelas rotinas do candidato para
formato 2. Substitui o manifesto/preflight duplicados. Codificação reconhece as
duas versões; cada caller deve exigir sua versão antes de inspecionar data.redb
ou abrir/reparar redb. Reconhecer versão 2 não autoriza Store::open a usá-la.

Cinco [testes novos](../crates/storage/src/format/tests.rs) fixam os bytes das duas
versões, incluindo igualdade do manifesto criado pelo Store real com a fixture
v1. Exercitam 1024/1025 bytes, schema/duplicados/tipos inválidos, magic/engine ou
versão desconhecida, forma dos arquivos/symlink e política cruzada entre versões.
Formato 2 é recusado pelo Store real mesmo com data.redb ausente ou contendo bytes
que não podem ser abertos como banco. Nenhum arquivo recusado é inicializado ou
reparado. Store atual é criado/reaberto normalmente e preserva seus dados.

Verificações locais de G1: fmt, clippy, 355 testes do workspace, dez workers
subordinados ignorados como testes diretos/exercitados pelos pais, documentação,
fronteira de nove pacotes de produto e diff sem erros. A suíte completa precisou
de execução fora do sandbox para sockets locais. CI da base bdcc896 aprovada;
a CI deste incremento precisa ser observada separadamente. Sem dependências,
migração, endpoint ou comando novos, sem ganho de consulta alegado.

## G2 — API de storage indexado compilada normalmente

[IndexedStore](../crates/storage/src/format_v2/facade.rs) cria/abre explicitamente
formato 2, sem expor Database, WriteTransaction ou tabelas ao caller. Store
continua no formato 1 e o servidor não seleciona o novo backend. Não há migração
nem mudança de defaults. A API Rust é interna ao workspace experimental, sem
promessa de compatibilidade entre releases ou SDK publicado.

A fachada controla coleção/lista/contagem, inserção preparada, leitura, PUT/PATCH,
delete/captura do estado anterior, grupos e transições intermediárias, lifecycle
de índices, consultas e export/restore do store. Snapshots mantêm catálogo,
dados, entradas/contagens e estatísticas da mesma transação, inclusive ao exportar
depois de escritas concorrentes. Erro de commit/cópia incerta segue tipado, sem
retornar captura para publicação quando a confirmação falha. Não emite feeds.

Open faz auditoria lógica integral antes de admitir consulta, com cursores e
cache configurado. Esse custo de startup cresce com a base; precisa ser medido
antes da habilitação no servidor. `check_readable` é verificação estrutural;
`audit` é a verificação completa. Manutenção e abertura são síncronas, não tarefas
despachadas pelo servidor neste incremento.

IndexedMaintenance é um handle separado, com engine privado, para abrir um store
estruturalmente válido que falha na auditoria lógica. Só oferece identificação
de coleção, auditoria, rebuild/drop de índice e conversão para IndexedStore após
auditoria completa. Não oferece query nem mutações comuns. Não repara catálogo
inválido/documentos inválidos, não ignora budgets de manutenção, nem transforma
um índice incompleto em pronto por existir uma tabela.

Três testes de integração em
[indexed_store.rs](../crates/storage/tests/indexed_store.rs) usam a API pública,
sem redb/tabelas: consulta seletiva real (1 de 16 candidatos), CRUD/grupos/captura,
snapshots, export/restore, reopen, gerações, paginação e concorrência. Um teste
privado injeta perda de entrada **e** contagem, demonstra recusa de open/into_store
e recuperação explícita por rebuild. As suites anteriores de falha/crash/budget
continuam exercitando o mesmo núcleo agora compilado normalmente.

G2 não implementa telemetria/admissão de servidor, ativação de auth, comandos ou
endpoint de índice. Administração exposta ao operador ainda deve ser offline sob
lock da instalação, conforme ADR. O limiar de planner 1/8 permanece hipótese;
não há ganho HTTP alegado. A divisão entre carregamento/storage e engine/tempos
HTTP precisa ser preservada em G4, em vez de copiar o wrapper de query inteiro.

## Verificações da fronteira tipada (bdcc896)

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

## Verificações de G2

- `cargo fmt --all -- --check`: aprovado.
- `cargo clippy --workspace --all-targets -- -D warnings`: aprovado.
- `cargo test --workspace --locked`: 359 aprovados, zero falhas e dez workers
  ignorados como testes diretos, exercitados pelos pais. Executado fora do
  sandbox para permitir os testes com sockets locais.
- `python3 scripts/check_docs.py`: aprovado.
- `python3 scripts/check_product_dependencies.py`: aprovado, nove pacotes.
- `git diff --check`: aprovado.

A CI do commit de G2 deve ser consultada separadamente destas evidências locais.

## G3.1 — Bootstrap indexado com lock e autenticação

[initialize_indexed_installation](../crates/server/src/lib.rs) é um entrypoint
Rust experimental e offline para criar ou retomar uma instalação indexada.
Não abre rede nem muda a CLI. Reusa [Root](../crates/server/src/bootstrap.rs),
as fases da ADR 0009 e o journal real de autenticação. Mantém o lock exclusivo
da raiz até fechar o store e concluir o bootstrap. Não expõe Database ou store
para escapar desse ciclo de vida; retorna somente epoch e eventual caminho da
entrega privada, sem conteúdo de credencial.

A escolha de formato é durável antes da criação do store. Bootstrap legado
continua JSON versão 1, sem novos campos. A instalação indexada usa versão 2,
store_format 2 e as mesmas fases, com schema estrito e teto de 1 KiB. Cada
entrypoint exige sua versão antes de criar/abrir dados ou preparar auth. Assim,
crash em initializing_store não permite que serve legado retome criando formato
1. Não converte raízes ou registros experimentais anteriores implicitamente.

Antes de preparar/retomar auth, a abertura indexada audita todo o store. Uma
entrada e sua contagem faltantes recusam startup, sem emitir credencial. Estado
com auth ausente/incompleta falha fechado; instalação expirada ou revogada não
recebe token automaticamente. Snapshot de store instalado numa nova raiz
store_ready recebe credencial independente da origem, que permanece utilizável.

Dois [testes externos históricos](https://github.com/sergiomonteiro/kiarni/blob/f355eef/crates/server/tests/indexed_bootstrap.rs) exercitam o
entrypoint público, emissão única, reabertura e recusa de legado/raiz insegura ou
desconhecida. Quatro [testes internos](../crates/server/src/bootstrap/indexed_tests.rs)
cobrem schema/versionamento, auditoria antes de auth, expiração/revogação e cópia
com credencial independente. Um deles mata um subprocesso em cinco estados
duráveis de bootstrap, verifica disputa real do lock, recusa cruzada e retomada.
O worker ignorado isoladamente é executado por esse teste pai. São fixtures de
interrupção de processo, não prova de queda de energia ou de RSS/latência.

G3 permanece parcial: envelope de backup/restore de formato 2, publicação da
instalação restaurada, administração offline, comandos e primeiro serve indexado
ainda não estão integrados. HTTP, feeds, métricas e medições continuam pendentes.
Os checks originais da issue 10 permanecem um concluído e cinco parciais.

Verificações locais de G3.1: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace --locked` (365 aprovados, zero falhas, onze workers
ignorados isoladamente e exercitados pelos pais), `python3 scripts/check_docs.py`,
`python3 scripts/check_product_dependencies.py` e `git diff --check`: aprovados.
A suíte completa foi executada fora do sandbox para sockets locais. CI do commit
é evidência separada, a acompanhar após envio.

## Verificações da consolidação

- `cargo fmt --all -- --check`: aprovado.
- `cargo clippy --workspace --all-targets -- -D warnings`: aprovado.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: aprovado.
- `cargo test --workspace --locked`: 363 aprovados, zero falhas.
- `cargo test --workspace --all-features --locked`: 363 aprovados, zero falhas.
  Doze workers ignorados como testes diretos são exercitados pelos pais. A suíte
  completa usa sockets locais e foi executada fora do sandbox.
- `cargo test -p kiarni-storage --test format --locked`: oito aprovados, um
  worker ignorado isoladamente e exercitado pelo pai, após portar o contrato.
- `python3 scripts/check_docs.py`,
  `python3 scripts/check_product_dependencies.py` e `git diff --check`: aprovados.

Dois testes exclusivos do entrypoint experimental removido saíram junto dele;
bootstrap normal segue coberto por testes de processo e da API HTTP. O núcleo
mantém seus testes transacionais/crash; não se descartou a proteção desses
invariantes. Storage/server e o wrapper de formato têm redução líquida de 560
linhas Rust em src, incluindo testes internos, em relação à base f355eef. O crate
de compatibilidade deixou o workspace; nenhuma dependência de runtime foi adicionada.
CI do novo commit deve ser consultada separadamente das evidências locais.

## Planner no fluxo HTTP

Store::query_candidates seleciona e carrega candidatos num snapshot, sem avaliar
a AST. O engine continua avaliando no handler HTTP e montando o mesmo envelope.
QueryReadError preserva o plano selecionado quando a carga falha depois da
seleção. query_read cobre apenas seleção/carga/parsing; sort/resultado inválido
não transforma uma carga bem-sucedida em falha de storage. O carregador antigo
query_documents foi removido; testes de fixtures usam o mesmo núcleo atual.

Um teste black-box executa as mesmas oito consultas antes/depois de criar o
índice offline: compara envelopes, JSON numérico exato, sort/projeção, resultado
vazio, ID, literal escapado, fallback denso/tipo não elegível e erros de
sort/limite HTTP. Demonstra caminhos secondary=5/scan=2/id=1 na execução indexada,
com seis sucessos e duas falhas posteriores à leitura. Contadores de leitura
permanecem oito sucessos, sem incluir avaliação/serialização. Reinício zera as
métricas e preserva o índice.

Outro teste HTTP cresce uma coleção indexada além de 8 MiB: filtro seletivo
continua falhando por orçamento, enquanto lookup por ID projetado permanece
aceito. Teste privado injeta entrada ausente após startup, confirma falha de
carga com plano Secondary e contabilização de erro de query_read. Nenhuma falha
ou corrupção é mascarada como fallback. Gate FIFO e resposta pública não mudam.

Dashboard valida 238 séries fixas, 42 de query, e exibe o novo caminho em quinze
linhas de totais/caminhos. Parser mantém validação de u64, histograma, presença e
duplicação. Chromium real verifica o painel completo. Administração offline
pela CLI foi entregue no incremento abaixo; consultas não criam índices implicitamente.

Verificações locais do planner HTTP: fmt, Clippy do workspace com e sem todas
as features, `cargo test --workspace --locked` e
`cargo test --workspace --all-features --locked` (365 aprovados em cada execução,
zero falhas; doze workers exercitados pelos pais), parser do dashboard (onze
aprovados), Chromium real (um cenário aprovado), documentação, fronteira de
nove pacotes e diff: aprovados. Testes com sockets/Chromium executados fora do
sandbox. CI do commit novo é evidência separada, a observar após envio.

## Administração offline de índices

[manage_index](../crates/server/src/indexes.rs) mantém o lock da instalação até
fechar o engine. O entrypoint de [CLI](../apps/kiarni/src/cli.rs) aceita
create/rebuild/drop/audit com argumentos próprios únicos e limitados; usa a mesma
configuração, sem novos campos JSON/env e sem bind. [Guia operacional](./INDEXES.md).

Criação abre o Store auditado; rebuild/drop e audit usam IndexedMaintenance,
que agora permite ler apenas o campo da definição como metadado. Não expõe
queries, CRUD ou engine. A raiz precisa estar initialized. Nenhum desses caminhos
chama start/ativação de auth ou emite credencial. Reparo de uma coleção não
transforma o resto da instalação em pronto; audit global e open normal permanecem.

Cinco testes internos cobrem recuperação, tabela ausente, autenticação revogada
inalterada, instalação incompleta, limite de 8 MiB, falhas retornadas e kill em
sete fronteiras de operação (antes/depois de cada mutação e na abertura de audit).
O worker é ignorado isoladamente e executado pelo pai. Testes anteriores do
núcleo continuam cobrindo as janelas internas da transação e o teto de documentos.

Uma integração CLI valida sintaxe/privacidade/limites sem criar raiz. Outra usa
binário + servidor reais: recusa as quatro operações com serve ativo, opera com
porta ocupada sem bind, cria índice usado por query HTTP, mantém índice em
escritas, reconstrói/remove e reinicia com documentos/credenciais preservados.
Os cinco comandos do guia são executados pelo teste documental junto do fluxo
API já existente. Sem nova dependência ou mudança do formato persistido.

Item 2 da issue passa a atendido; faltam workloads e medições no produto (item 6).
A escolha inicial 1/8 ainda não é conclusão de benchmark nem ganho HTTP alegado.

Verificações locais da administração CLI: `cargo fmt --all -- --check`,
Clippy com e sem todas as features e as duas execuções completas do workspace
(default e all-features, com --locked): aprovados, 372 testes em cada execução,
zero falhas, treze workers exercitados pelos pais. `scripts/docs_smoke.py`
executou quinze comandos curl publicados, cinco comandos offline de índice,
HEADs/SSE/credenciais e precedência de configuração. Documentação, fronteira de
nove pacotes e diff aprovados. Testes com sockets executados fora do sandbox.
CI do novo commit deve ser consultada separadamente dessas evidências locais.
