# ADR 0034 — Contrato e ciclo de vida do índice secundário

- Status: **aceita para o Store único de formato 2**, sem suporte legado.
- Aceite restrito: 2026-11-21, após G1 e validação acumulada do candidato.
  G3.1 estende a composição interna para bootstrap/auth offline, conforme seção final.
  Habilitação no servidor/CLI e homologação do custo permanecem pendentes.
- Data: 2026-11-19.
- Requisito: RF-04; integração posterior ao aceite técnico M3, issue #10.
- Complementa a [avaliação 0033](./0033-avaliacao-indice-secundario.md).

## Revisão de direção — sem legado em uso

Decisão do responsável pelo projeto em 2026-11-21: não há instalações em uso nem
dados de usuários que exijam retrocompatibilidade. A integração passa a visar
**um único backend/formato atual**, com índices, no caminho normal do produto.
Não manter suporte operacional paralelo ao formato 1, construir migração ou
criar comandos separados de instalação apenas para preservar o formato antigo.
As exigências anteriores de manutenção do caminho legado ficam superadas por
esta decisão; as seções abaixo preservam a evolução e as evidências anteriores.

Manter identificação/versionamento em disco para detectar dados incompatíveis.
O número 2 pode continuar identificando o layout atual: não é promessa de suporte
à versão 1. Artefatos de desenvolvimento antigos serão recusados; esta decisão
não autoriza apagá-los, convertê-los ou alterar seus metadados automaticamente.

A consolidação substitui o caminho normal pelo núcleo indexado. Store é o único
backend de servidor, writer, bootstrap e backup/restore. IndexedStore e o
entrypoint separado initialize_indexed_installation foram removidos; operações
normais de escrita mantêm documentos/índices/estatísticas no mesmo commit.
Bootstrap atual é versão 2/store_format 2; envelope de backup mantém schema 1,
com format_version 2. O código de exportação do layout 1 foi removido.

Métricas de primitivas e commits continuam nos limites reais do Store; commits
de manutenção de índice também são contados. O planner HTTP agora seleciona
ID/secundário/scan com observabilidade fixa. Administração offline de índices
pela CLI está integrada conforme a seção final; homologação de desempenho
permanece pendente. Não há ganho HTTP
medido/alegado nesta integração.
As seções seguintes registram o histórico anterior à consolidação.

## Contexto e fronteira desta entrega

O spike demonstrou ganho seletivo, custo de escrita e regressão no caso denso.
Também demonstrou que uma entrada ausente produz falso negativo sem detecção
pelo lookup. O produto usa formato 1, catálogo de coleções com identidade não
reutilizada, primário e scan limitado; seu backup valida exatamente esse layout.
Não basta copiar a tabela experimental para habilitar índices com segurança.

Este ADR aceita o contrato persistido e a API Rust interna IndexedStore, com
engine privado e seleção explícita de formato 2 em diretório novo. Não cria
comandos, endpoints ou migrações. Store e servidor atuais continuam no formato 1;
API HTTP e dados existentes permanecem inalterados. O aceite não libera formato
2 no servidor nem torna o limiar 1/8 uma conclusão de benchmark.

Formato 1 continua abrindo pelo caminho atual; versão 2 só pode ser criada em
destino novo. Não editar manifesto para converter um store nem migrar in-place.
Antes de oferecer upgrade de instalações, implementar e testar conversão offline
de um snapshot para nova raiz, preservando a origem e validando todos os dados
antes de publicar. A interface dessa conversão fica para incremento próprio;
até lá, nenhuma promessa de upgrade/downgrade entre formatos. Binários antigos
devem recusar versão 2 no preflight, sem abrir o engine para escrita.

## Contrato aceito para a fachada Rust

Um índice opcional por coleção, sobre um campo simples da raiz, com nome sujeito
ao mesmo limite ASCII de 1–128 bytes do catálogo de coleções. `_id` continua
exclusivamente primário. O campo é literal, sem interpretação de ponto como path.
Não há unicidade secundária, campos aninhados, composição ou coerção de tipos.

Indexar somente strings decodificadas de até 128 bytes UTF-8, inclusive vazia e
com NUL; preservar distinção Unicode sem normalização. Ausente, null, outros
tipos e strings maiores continuam documentos válidos, sem entrada secundária.
O predicado elegível é igualdade direta entre esse campo e uma string coberta.
Composições e demais operadores usam o caminho existente, mesmo quando uma
otimização pareça possível. O engine mantém comparação JSON exata e a execução
de filter/sort/limit/project; o índice fornece apenas candidatos.

## Formato candidato e compatibilidade

Versão lógica **2**, disponível explicitamente por IndexedStore no crate storage.
Manter manifesto e
metadado interno concordantes, com redb 4.5.0; não adicionar tabelas silenciosamente
ao formato 1. Além das tabelas atuais, propor:

| Tabela | Chave | Valor lógico |
| --- | --- | --- |
| index_catalog | collection_id | revisão de definição, campo, tipo string parcial, limite 128 e estado ready |
| secondary_entries | (collection_id, valor string, _id) | unidade |
| collection_stats | collection_id | nome da coleção, contagem exata de documentos e bytes JSON armazenados |
| secondary_counts | (collection_id, valor string) | contagem exata de entradas, omitida quando zero |

Não reutilizar nomes de coleção como identidade. O nome em collection_stats é
referência inversa para validar propriedade/duplicação de IDs sem guardar todos
os IDs em RAM ou fazer scans quadráticos; não substitui a identidade numérica.
Não aceitar revisões de definição desconhecidas.

Codificação candidata, fixada em `crates/storage/src/format_v2.rs` e na fixture
`crates/storage/tests/fixtures/index-status-v1.bin`:

- O manifesto mantém o JSON estrito do formato 1, com version 2. As tabelas
  originais preservam seus tipos redb; metadados exigem exatamente format_version
  e next_collection_id, ambos u64.
- index_catalog tem chave u64 e valor bytes: revisão 1, tipo string 1, estado ready
  1, limite 128, comprimento do campo em um byte, seguido pelos bytes ASCII do
  campo. Header de cinco bytes, tamanho total 6–133; sem padding ou trailing bytes.
  A fixture status é hexadecimal `01 01 01 80 06 73 74 61 74 75 73`.
- secondary_entries usa chave redb `(u64, &str, &str)` e valor unidade `()`;
  collection_stats usa chave u64 e valor `(&str, u64, u64)` (nome, documentos,
  bytes); secondary_counts usa chave `(u64, &str)` e valor u64.
- Inteiros e tuplas usam a implementação tipada de redb 4.5.0, sem codificador
  externo. A fixture fixa o payload da definição, não uma imagem binária redb
  inteira nem compatibilidade com outra versão do engine.

A abertura candidata verifica tabelas exatas e seus tipos, ausência de multimaps,
metadados, nomes/identidades, referência inversa e definição/revisão/estado. Ela
não percorre documentos, entradas ou contagens secundárias para provar
equivalência; auditoria completa e manutenção continuam pendentes.

O binário atual deve continuar recusando versão desconhecida antes de abrir redb.
Um futuro binário poderá ler formato 1 pelo caminho atual, sem índices, e formato
2 pelo novo caminho. Nenhuma abertura ou restauração promove versão implicitamente.
Conversão, se implementada, será offline para **nova raiz**, com validação integral,
publicação sem sobrescrita e preservação da origem; não haverá upgrade in-place
nem promessa de downgrade. Comando, credenciais e publicação dessa conversão
exigem desenho e testes próprios antes de implementação. Não converter arquivos
de spike nem declarar compatibilidade entre releases por esta proposta.

## Criação, reconstrução e exclusão

O primeiro build será **offline**, sob o lock exclusivo já exigido pela instalação.
Servidor ativo implica recusa imediata; não enfileirar uma manutenção longa no
coordenador HTTP. Não oferecer backfill online neste incremento.

1. Validar formato, identidade da coleção e definição. Ler e construir dentro de
   uma única transação de escrita, com limite de 10000 documentos e 8 MiB de JSON
   examinados. Exceder o limite aborta tudo, sem deixar índice parcial.
2. Criar entradas e contagens, conferir equivalência com os documentos nessa
   transação e publicar o catálogo ready no mesmo commit Immediate.
3. Sucesso somente após commit confirmado. Não emitir eventos de documentos por
   manutenção. Falha anterior ao commit mantém o estado anterior; falha após ele
   pode tornar a resposta incerta, exigindo inspeção ao reabrir.

Não persistir building nem cursor de backfill: ausente → ready ocorre atomicamente.
Reconstrução substitui entradas/contagens da coleção na mesma transação limitada;
falha preserva o índice anterior. Exclusão remove catálogo e entradas atomicamente.
Não oferecer build parcial em coleção maior; evoluir esse limite requer outro
incremento com budgets e recuperação demonstrados.

Na abertura, validar presença de tabelas, revisões, referências de catálogo e
estados; tabela faltante é erro, nunca DDL de reparo automático. Recovery normal
de redb é permitido somente após preflight do manifesto conhecido. Um índice
parcial nunca pode virar ready por inferência de que sua tabela existe.

Auditoria integral deve comparar entradas esperadas e efetivas, incluindo ausências,
extras, referências e contagens, em build/rebuild e restauração. Ela deve percorrer
dados em lotes limitados, sem materializar toda a coleção em RAM. Validação
estrutural de startup não prova ausência de corrupção lógica arbitrária: entradas
faltantes introduzidas fora das transações exigem essa auditoria. Não prometer
detecção universal por lookup nem reconstrução automática silenciosa.

## Escritas, gerações e snapshots

Insert, PUT, PATCH e delete atualizam documento, entrada antiga/nova e contagens
na mesma transação, inclusive nos grupos do writer atual. No-op não altera essas
estruturas nem emite evento. Campo inalterado evita reinserção, verificando a
referência esperada. Conflito ou erro deve preservar os invariantes existentes do
grupo; não alterar sua semântica de resultados. Todos os escritores, incluindo
rotinas internas, precisam usar essa manutenção antes de habilitar leitura indexada.

Excluir coleção vazia remove seus metadados secundários e estatísticas no mesmo
commit. Recriar o nome recebe outra identidade; handle antigo não acessa o novo
índice. Overflow de contadores aborta, sem wraparound. Consultas leem catálogo,
contagens, entradas e documentos no mesmo snapshot, com validação das referências.
Inconsistência detectada é erro de storage; não mascarar corrupção com fallback.
Feeds continuam exclusivamente pós-commit, na ordem já estabelecida.

## Escolha de plano e orçamentos

Prioridade: primário por ID, secundário elegível e seletivo, scan. Consultar a
contagem exata do valor sem percorrer candidatos. Proposta inicial conservadora:
usar secundário somente quando seus candidatos forem no máximo 1/8 da coleção;
zero candidatos pode produzir resultado vazio. Esse limiar é hipótese a medir no
produto, não conclusão do spike nem promessa de latência.

Preservar o contrato atual de erro: antes de escolher secundário, estatísticas do
mesmo snapshot devem provar que a coleção inteira cabe em 10000 documentos/8 MiB.
Se não couber, usar o scan atual e seu erro de limite; o índice não pode transformar
essa consulta em sucesso por examinar menos documentos. Busca por ID mantém sua
exceção atual. Também permanecem os limites de resultado, filas e admissão.
Contagens/bytes exatos são necessários para essa decisão; estimativas não bastam.

Registrar a escolha por categoria finita em observabilidade e testes, sem valores
de campo, IDs ou rótulos de cardinalidade ilimitada. O contrato dessa métrica será
definido no incremento do planner; não alterar a resposta HTTP nesta etapa.

## Backup e restauração

Exportar documentos, catálogo, entradas e contagens de um único snapshot; exportar
somente documentos perderia a definição. Destino usa formato correspondente à
origem. Manter staging privado, lotes limitados, hashes e publicação sem sobrescrita.
Restore deve validar equivalência integral antes de publicar a nova raiz; hashes
sozinhos não validam integridade lógica. Credenciais continuam fora do backup.
Formato 1 permanece aceito pelo caminho documentado, sem inventar índice ausente.
Não habilitar formato 2 antes de completar backup/restore e seus testes.

## Alternativas e consequências

Build online com dual-write, cursor e reconciliação permite bases maiores, mas
adiciona estados persistentes e concorrência sem evidência atual. Um build offline
limitado permite demonstrar atomicidade primeiro, ao custo de indisponibilidade
durante manutenção e limite explícito de tamanho. Criar índice implicitamente no
primeiro eq altera custo e formato de maneira surpreendente; descartado.

Contagens exatas adicionam trabalho de escrita e espaço. Seleção conservadora
evita forçar o caso denso, mas não garante ganho em toda distribuição. O limite de
build e a preservação do orçamento de scan restringem utilidade inicial; relaxá-los
exige decisão explícita sobre o contrato de consulta. Nenhuma dependência externa
ou serviço adicional é necessário.

## Condições de habilitação no servidor e sequência de implementação

1. Fixar codificação/fixtures do formato 2 e implementar criação em destino novo,
   recusa de formatos inválidos e validação estrutural, ainda sem habilitar no HTTP.
2. Demonstrar manutenção transacional, build/rebuild/drop e auditoria, com erros,
   rollback, kill antes/depois do commit, reabertura e exclusão/recriação da coleção.
3. Integrar export/restore, incluindo backups sem índice, catálogo divergente,
   entrada faltante/extra, contagem inválida e falhas nas janelas de publicação.
4. Integrar planner e observabilidade com equivalência de sucesso **e erros** contra
   scan: tipos, Unicode, precisão numérica, fronteiras de budgets, ID, no-op,
   snapshots concorrentes e grupos de escrita. Medir cenário seletivo e denso,
   custo de estatísticas/escrita e leitura HTTP antes de escolher o limiar final.

O item 1 tem a evidência candidata descrita abaixo, sem habilitação no servidor.
Documento, estatísticas e entradas já têm manutenção transacional candidata,
descrita abaixo. Construção/reconstrução, remoção do índice e auditoria também
têm evidência isolada. Exclusão/recriação de coleção e interrupção abrupta das
mutações têm os testes adicionais abaixo. Export/restore do storage candidato
também têm evidência isolada. O envelope com hashes/publicação tem o candidato
Unix descrito abaixo. A instalação restaurada também tem publicação candidata
com lock e entrega vazia de credenciais. Grupos internos de PUT/PATCH e seleção
ID/índice/scan também têm evidência candidata. A [revisão de integração](../INDEX_INTEGRATION_REVIEW.md)
consolida o gate e a promoção incremental. Codec/preflight já são compartilhados
no código de produção, com a abertura padrão restrita ao formato 1. O próximo
passo de fachada está entregue por IndexedStore, sem selecionar esse backend no
servidor. A composição operacional e integração HTTP ainda precisam ser concluídas.
Não é migração de instalações existentes. Os testes do [spike](../spikes/secondary-index.md) sustentam
a hipótese de indexação, mas não validam este lifecycle, formato ou planner.
O aceite é restrito ao formato/fachada internos; issue #10 continua aberta até
concluir integração e medições. O [ADR 0011](./0011-formato-inicial-e-catalogo.md)
segue vigente para Store e servidor de formato 1.

As seções de evidência abaixo preservam a sequência histórica do candidato.
Menções a cfg(test)/proposta descrevem aquelas etapas; a seção G2 final registra
quais partes já passaram a código normal do crate.

### Evidência do primeiro incremento

Seis testes em format_v2 exercitam fixture e validação estrita, criação/reabertura,
recusa mútua entre formato 1 e candidato 2 sem alterar o arquivo recusado,
destino existente/parcial, tabela ausente sem recriação, tabela extra/tipo errado,
metadados divergentes, catálogo órfão/definição inválida, IDs duplicados/referências
inversas inconsistentes e preflight de manifesto inválido antes do engine.
Esse código está sob `cfg(test)`: nenhum caminho do binário cria ou abre formato 2.
Não demonstra ainda kill/restart durante criação, energia,
backup/restore do novo formato ou ganho de performance no produto.

### Evidência da manutenção transacional candidata

`crates/storage/src/format_v2/mutations.rs`, também somente em `cfg(test)`,
implementa inserção preparada pelo core, PUT/PATCH preparados pelo engine e
exclusão, atualizando documento, entrada secundária, contagem por valor e
contagem/bytes da coleção no mesmo commit Immediate. Coleções sem índice também
mantêm estatísticas. No-op preserva os bytes existentes e aborta sem commit;
campo indexado inalterado verifica a referência existente e evita reinserção.
Subtração inválida e overflow de contadores abortam a operação. A enumeração
inicial de tabelas retém no máximo oito nomes para recusar layouts diferentes
das sete tabelas esperadas sem recriá-las no caminho de escrita.

Erros de validação/preparo/checkpoints anteriores ao commit retornam Aborted.
Erro no commit ou perda da confirmação posterior retorna CommitUncertain;
não deve ser interpretado como rollback nem autorização para retry automático.
Não existe integração com feeds, HTTP, admissão ou grupos de escrita nesta etapa.

Cinco testes adicionais cobrem ciclo de vida com/sem índice, erro de entrada,
no-op/mesma chave, tipos não indexados, limites UTF-8, strings vazias/NUL,
Unicode distinto, número JSON exato, conflitos, geração antiga, corrupção de
referência/contagem, overflow e tabela faltante sem reparo. Para insert/update/delete,
dezoito combinações injetam falha em documento, remoção/adição de entrada,
estatísticas e antes/depois do commit, comparando rollback ou persistência após
reabertura. Quatro writers com vinte updates cada e leitores verificam equivalência
em snapshots, inclusive nos checkpoints dentro da transação.

O comparador de referência dos testes reconstrói entradas/contagens a partir de
JSON em pequenos fixtures, com decoder independente da extração usada na escrita.
Ele materializa mapas em RAM **somente no teste**; não é a auditoria limitada
para bases arbitrárias exigida por este ADR. Erros injetados não substituem kill
do processo, falha física ou queda de energia. A evidência adicional de lifecycle
abaixo cobre build/rebuild/remoção do índice e auditoria no candidato, mas não
substitui os testes de crash das mutações, backup/restore ou integração do planner.

### Evidência de construção, reconstrução e auditoria

`crates/storage/src/format_v2/lifecycle.rs`, somente em `cfg(test)`, implementa
build sobre documentos existentes, rebuild e remoção do índice em uma transação
Immediate. Publica ready apenas no mesmo commit das entradas e contagens, após
auditoria na própria transação. Rebuild recompõe também contagem/bytes da coleção
a partir dos documentos; não repara documentos inválidos nem identidade/catálogo
inválido. A remoção é do índice, não da coleção. Ainda não há comando offline,
lock da raiz integrado à manutenção, backfill online ou conexão com o servidor.

Build/rebuild/remoção examinam no máximo 10000 documentos e 8 MiB de JSON. A
limpeza de entradas/contagens antigas tem limite de 10000 registros por tabela;
excesso aborta toda a manutenção e preserva o estado anterior. Prefixos usam o
intervalo entre collection_id e a próxima identidade, evitando excluir valores
que começam com o maior scalar Unicode e têm sufixo. A operação não toca outra
coleção, mesmo quando documentos e valores têm nomes iguais.

A auditoria confere, em um snapshot, documentos contra entradas esperadas,
referências de cada entrada, contagens exatas por valor e contagem/bytes da
coleção. Percorre cursores e retém apenas um registro/grupo por vez, com parsing
limitado ao tamanho de documento; não materializa mapas da coleção. O custo
percorre todos os registros auditados e faz lookups, sem promessa de latência ou
medição de RSS neste incremento. Não aplica o limite de cardinalidade do build
à auditoria: ela pode verificar um índice que cresceu além desse limite.

Sete testes adicionais e um worker subordinado cobrem:

- build/rebuild/remoção, preservação de snapshots antigos e isolamento de coleção;
- detecção de entrada ausente/extra, referência errada, contagem ausente/zero/extra
  ou divergente e estatística inválida, com reparo por rebuild quando aplicável;
- rejeição por definição, geração ou documento inválido e limites de limpeza;
  aceite exato de 10000 documentos/8 MiB, seguido de recusa ao exceder o orçamento;
- 21 combinações de erro retornado e 28 de subprocesso morto em sete checkpoints
  (limpeza, progresso, entradas construídas, ready, auditoria, pré e pós-commit).

Nos subprocessos, criação, reconstrução, remoção e reparo são interrompidos com
kill após confirmação do checkpoint, com deadline de 15 segundos e coleta do
processo. O worker encerra se o pai desaparecer. Reabertura e auditoria verificam
o estado anterior ou o commit confirmado. Reparo interrompido antes do commit
preserva a corrupção preexistente e a auditoria continua recusando-a; não
transforma esse índice em confiável. Erro após commit permanece CommitUncertain.

Essas evidências demonstram recuperação nos checkpoints testados do candidato,
não queda de energia, interrupção em cada instrução do commit ou liberação do
recurso no produto. Os testes de coleção e mutações abaixo complementam essa
evidência; coordenação de grupos, backup/restore e planner continuam pendentes.
ADR permanece proposto.

### Evidência do ciclo de coleções e crash de mutações

`crates/storage/src/format_v2/collections.rs`, somente em `cfg(test)`, cria
coleção com contador monotônico e estatística vazia na mesma transação Immediate.
Destino de identidade com dados/metadados órfãos é recusado. Conflito, nome
inválido e overflow não consomem identidade. A exclusão exige ausência de
documentos e estatísticas vazias coerentes, recusando entradas/contagens órfãs;
remove nome, estatística e definição secundária no mesmo commit, sem resetar o
contador. Recriar o nome recebe outra identidade e começa sem índice secundário.

Cinco testes adicionais e um worker subordinado exercitam:

- uma coleção antes populada, esvaziada por delete e então excluída/recriada,
  com mesmo ID de documento reutilizado somente na nova identidade; handles
  antigos são recusados na mutação, auditoria, rebuild e exclusão;
- preservação de snapshot antigo, reabertura e ausência de entradas/contagens
  antigas; criação concorrente duplicada confirma apenas um contador;
- coleção não vazia, estatística/catálogo inválido, tabela ausente, órfãos e
  contador inválido/esgotado, com recusa sem exclusão ou adoção silenciosa;
- dez combinações de erro retornado e dez de kill/restart nos cinco checkpoints
  de criação/exclusão, validando catálogo, estatística e contador conjuntamente;
- 24 combinações de kill/restart em insert, PUT, PATCH e delete nos seis
  checkpoints de documento, entradas, estatísticas, pré e pós-commit.

Os subprocessos usam confirmação do checkpoint, deadline de 15 segundos, kill e
coleta, com término por EOF se o pai desaparecer. Depois de recovery, auditoria,
catálogo/contador e conteúdo do documento confirmam rollback ou novo estado
completo, preservando o número JSON exato do fixture. Falha de confirmação após
commit permanece incerta, nunca autorização para repetir a mutação.

As rotinas compartilham a validação de schema antes de abrir tabelas para escrita,
evitando reparo implícito de tabelas faltantes. Continuam sem comandos, servidor,
eventos ou alteração do formato 1. Não demonstram energia, falha dentro de cada
instrução do commit, concorrência de grupos ou compatibilidade de backup. Antes
de habilitar formato 2, ainda concluir export/restore, grupos, planner e revisão
da proposta. Nenhuma migração ou decisão de estabilidade foi adicionada.

### Evidência de cópia e restauração do snapshot de storage

`crates/storage/src/format_v2/backup.rs`, somente em `cfg(test)`, copia as sete
tabelas tipadas de um único ReadTransaction: metadados, coleções, documentos,
catálogo, entradas, estatísticas e contagens secundárias. Audita a origem nesse
snapshot antes de criar o destino e a cópia antes de marcá-la como reconhecível.
A auditoria global complementa a de coleção com rejeição de documentos, entradas
e contagens pertencentes a identidades inexistentes.

Destino deve ser novo. Sua inicialização privada mantém formato ausente durante
a cópia e os commits em lotes de 256 registros ou 4 MiB contabilizados, mais o
registro que cruza o limiar. Não carrega a coleção inteira em RAM. Somente após
auditoria, fechamento do engine e sync dos dados/diretório grava format.json,
sincroniza arquivo e diretórios. Erro anterior é Incomplete e pode deixar staging
parcial sem manifesto, nunca reutilizado automaticamente. Erro a partir da
tentativa de gravar o manifesto é PublicationUncertain: a cópia completa pode
existir; não interpretar como rollback nem sobrescrever ao repetir.

A restauração candidata abre uma cópia limpa de storage com redb read-only e
reexporta o snapshot para outro diretório novo. Recusa origem em uso/incompatível
e destino dentro da origem; não executa recovery ou escreve no banco original.
Mantém formato 2, IDs/gaps do contador, JSON bruto e definições de índices, sem
rebuild que esconda inconsistência. A origem permanece byte a byte intacta nos
testes. Não converte formato 1 nem modifica o restore atual.

Seis testes adicionais e um worker subordinado demonstram:

- comparação exata das sete tabelas, com 607 documentos, coleção vazia e coleção
  sem índice, JSON de precisão exata e mais de 5 MiB em múltiplos lotes; mudanças
  de documento e catálogo na origem entre lotes não entram na cópia do snapshot;
- cópia/restauração de store sem índice, preservação de identidades e recusa de
  destinos existentes/nested, origem ativa e formato 1, sem modificar a origem;
- nove corrupções recusadas antes de criar destino: entrada ausente/extra,
  contagem incorreta, definição inválida, estatística divergente, identidades
  órfãs em documentos/entradas/contagens e coleção com identidade duplicada;
- quatro erros retornados e quatro kill/restart em lote, auditoria, pré-manifesto
  e pós-publicação do manifesto, com deadline de 15 segundos, coleta do worker,
  auditoria da cópia reconhecível e confirmação de origem intacta.

Esta primitiva de storage não implementa sozinha o artefato dos ADRs 0028/0029.
O envelope candidato abaixo complementa hashes/leitura/publicação de store, mas
não habilita restauração de instalação/credenciais nem CLI. Manifesto não é
assinatura e não há prova de queda de energia. Os comandos atuais só aceitam
formato 1; grupos de escrita, planner e revisão deste ADR continuam pendentes.

### Evidência do envelope de backup candidato

`crates/storage/src/format_v2/artifact.rs`, Unix e somente em `cfg(test)`, usa o
layout dos ADRs 0028/0029: backup.json e store/{format.json,data.redb}. A proposta
mantém versão 1 do schema JSON e declara **format_version 2**; não altera o
contrato aceito para os comandos atuais, que recusam esse formato. A fixture
`crates/storage/tests/fixtures/backup-format2.json` fixa o schema/ordem de escrita,
não representa um backup com dados válidos.

Manifesto limitado a 1 KiB, com campos/duplicados estritos, magic/engine conhecidos,
contagens/tamanhos decimais u64 em strings, exatamente dois caminhos fixos em ordem
e SHA-256 em 64 caracteres hexadecimais minúsculos. O export audita o snapshot,
calcula hashes após fechar o store exportado, grava/sincroniza o manifesto e
sincroniza o diretório privado antes de rename NOREPLACE. Diretório de staging
é criado explicitamente com 0700, arquivos com 0600. O pai deve pertencer ao
usuário e não permitir escrita por grupo/outros. Recusa plataformas/FS sem rename
sem substituição; não há fallback check-then-rename.

Restauração do **store**, ainda sem instalação/auth: abre diretórios e filhos por
descriptors, com NOFOLLOW/NONBLOCK, valida proprietário/modo/tipo/número de links,
enumera entradas exatas por descriptor e copia somente os dois arquivos fixos para
staging privado. Usa buffer de 64 KiB, exige tamanho exato, relê os bytes copiados
para comparar hashes e só então abre a cópia para auditoria/reexport. Hash correto
não dispensa integridade lógica nem validação das contagens do manifesto.
Publica o store auditado por rename sem substituição, sem escrever na origem.
Não restaura dentro do artefato nem importa credenciais.

Antes do rename, erro não publica o destino; um staging próprio pode permanecer.
Depois dele, falha de confirmação ou sync do pai é PublicationUncertain, sem
inferir ausência ou repetir com sobrescrita. A cópia intermediária de restauração
é removida por seu guard privado no término normal. Kill pode deixar staging
privado; não há limpeza automática de caminhos preexistentes ou de terceiros.

Seis testes novos e um worker subordinado cobrem roundtrip das sete tabelas
com 607 documentos e store vazio, fixture estrita/duplicados/limites e vetor
conhecido SHA-256 de abc, alteração de conteúdo com mesmo tamanho, tamanho
divergente, contagem incorreta e corrupção lógica com hash recalculado. Dez
formas inseguras (symlink, hardlink, modos e entradas extras) são recusadas.
Testes também recusam pai gravável por grupo, destino interno ao artefato e
disputa do nome após preflight, preservando o destino concorrente. Oito erros
retornados e oito kill/restart em cópia/verificação/pré/pós-rename de backup e
restore confirmam destino ausente ou completo e origem byte a byte intacta.

sha2 0.11 e rustix 1.1 foram adicionados **somente como dependências de teste** do
storage, reutilizando pacotes/versões já presentes no produto. Licenças locais
verificadas: sha2 MIT OR Apache-2.0 e rustix inclui alternativa MIT. Nenhum pacote
novo ou dependência normal do binário foi introduzido.

Este envelope não é API/CLI disponível, assinatura criptográfica nem prova contra
processo malicioso do próprio usuário ou energia. A publicação de instalação
complementar tem a evidência abaixo. Faltam integração com o bootstrap/CLI real,
grupos do writer, planner e aceite da proposta antes de habilitar formato 2.
A CI ainda precisa demonstrar a execução deste incremento
em cada plataforma; passagem local não certifica macOS ou outro filesystem.

### Evidência de publicação da instalação restaurada

`crates/storage/src/format_v2/artifact/installation.rs`, Unix e somente em
`cfg(test)`, compõe o restore de store já auditado com uma nova raiz privada.
Cria serve.lock 0600 e adquire flock exclusivo não bloqueante antes de abrir
qualquer engine copiado; mantém esse descriptor até terminar a publicação.
Grava/sincroniza bootstrap.json `{"version":1,"phase":"store_ready"}` depois
da validação do store, sem criar auth ou emitir credenciais.

A estrutura candidata segue ADRs 0009/0029: raiz 0700 com store, serve.lock e
bootstrap.json, e diretório irmão `<raiz>.credentials` 0700, vazio. Os dois nomes
devem estar ausentes, inclusive diretórios vazios, arquivos ou symlinks existentes.
Staging, arquivos e diretórios são sincronizados antes de publicação. Publica a
entrega primeiro, confirma sync do pai e só então publica a raiz, ambos com
rename NOREPLACE. Não há afirmação de atomicidade dos dois diretórios juntos.

Falha antes de publicar a entrega não publica a raiz. A partir de entrega
publicada, erros de confirmação, disputa do nome da raiz ou sync retornam
PublicationUncertain; entrega e destinos existentes não são removidos. Repetição
recusa os nomes já existentes. Staging próprio pode permanecer no erro ou kill;
sucesso remove o staging vazio. O lock segue a raiz renomeada e só é liberado
após terminar a operação; kill o libera pelo fechamento dos descriptors.

Cinco testes novos e um worker subordinado verificam roundtrip das sete tabelas,
bootstrap privado/fase correta, ausência de auth e entrega vazia, origem intacta,
lock bloqueando outro descriptor e processo durante a publicação e disponível
após sucesso/kill. Também exercitam nomes existentes/symlinks e disputas da
entrega/raiz sem sobrescrita ou limpeza de dados concorrentes, origem inválida,
destino dentro do artefato, quatro erros retornados e quatro kill/restart nas
janelas preparado, entrega publicada, pré-raiz e raiz publicada. Os subprocessos
têm deadline de 15 segundos, kill/coleta e término por EOF quando o pai some.

Isso comprova o esqueleto restaurado nos checkpoints do candidato, não um serve
de formato 2: os comandos atuais continuam recusando esse formato. O registro
store_ready não habilita uma nova versão de storage implicitamente. Ativação de
auth/geração de novas credenciais no primeiro serve, integração CLI/writer,
planner e aceite da proposta permanecem pendentes. Nenhuma credencial
da origem é copiada ou mostrada. Não há nova garantia de energia ou migração.

### Evidência de manutenção em grupos de escrita

`crates/storage/src/format_v2/groups.rs`, somente em `cfg(test)`, reutiliza os
tipos internos Update, CapturedReceipt e GroupError do produto, com os mesmos
limites de 32 operações/8 MiB de corpos e limite individual de documento. Não
cria API de transação multi-documento pública, worker, fila ou integração HTTP.
Resolve nomes/identidades dentro da transação, processa PUT/PATCH em ordem e
aplica a mesma manutenção de documento/índice/estatísticas das escritas simples.
O preparo do engine é reaproveitado, sem preparar a transformação duas vezes.

Rejeições de domínio são resultados individuais, antes de alterar aquela
operação; outras operações válidas podem confirmar. Erro de layout, engine,
integridade ou encoding interno aborta o grupo inteiro, incluindo alterações
anteriores. Grupo sem mudança aborta sem commit e sem transições. Havendo
mudança, um único commit Immediate confirma dados, entradas e contagens.
Erro no commit ou na confirmação posterior continua CommitUncertain.

Captura opcional preserva os estados before/after por operação, inclusive os
intermediários do mesmo documento, mas só entrega receipt/transições depois de
commit confirmado. As referências compartilhadas seguem os tipos existentes do
produto. Não emite eventos; integração com o dispatcher permanece fora desta
etapa. Os limites de resultados retidos seguem a quantidade de operações e o
tamanho máximo dos documentos; 8 MiB continua sendo limite de **corpos recebidos**,
não promessa de memória total/RSS do grupo.

Seis testes adicionais e um worker subordinado demonstram:

- resultados ordenados, erros individuais, no-op e transições do mesmo ID,
  comparados diretamente com update_group_captured do formato 1 e com aplicação
  sequencial; resultado final das sete tabelas, snapshot antigo e JSON exato;
- bordas de zero/32/33 operações, captura incompatível, limite individual inválido,
  aceite de exatamente 8 MiB de corpos e recusa de excesso sem escrita;
- falha de integridade depois de uma operação válida abortando tudo, incluindo
  contador que estoura na segunda atualização e tabela ausente sem recriação;
- quatro erros retornados e quatro kill/restart depois das operações 0/1,
  grupo preparado e commit confirmado, auditando duas coleções/índice/contagens;
- quatro writers com doze grupos cada e leitores verificando que mudanças nas
  duas coleções aparecem juntas no mesmo snapshot, sem visibilidade parcial.

O helper transacional compartilhado continua compilado somente em testes. O
writer/coordenador e seus limites de admissão, métricas e feeds do produto não
foram alterados. Integração e validação HTTP do planner, ativação auth/CLI e revisão
do ADR ainda são necessárias antes de habilitar formato 2. Falhas injetadas/kill
nos checkpoints não certificam energia, desempenho ou falha em cada instrução.

### Evidência de seleção de acesso e equivalência de consultas

`crates/storage/src/format_v2/queries.rs`, somente em `cfg(test)`, recebe Query
validada e uma transação de leitura. Valida identidade/geração e mantém catálogo,
estatísticas, contagens, entradas e documentos no mesmo snapshot. Prioridade:
lookup primário por ID; igualdade direta elegível de string sobre o campo da
definição; scan. A versão inicial inspecionava a serialização da AST; após a
revisão passou a usar o acesso tipado Query::direct_string_equality, sem expor
novos operadores ou modificar a semântica de avaliação/serialização do engine.

Usa secundário somente se a coleção inteira couber em 10000 documentos/8 MiB e
a contagem exata do valor for no máximo `documentos / 8`. Zero candidatos também
é elegível, inclusive coleção vazia. A razão 1/8 permanece hipótese conservadora,
não limiar homologado por benchmark do produto. Tipos/valores fora do índice,
campo diferente, ranges, composições, coleção sem definição e caso denso usam
scan. Fora do orçamento da coleção, executa o scan que preserva seu erro de
limite, sem sucesso silencioso por reduzir candidatos. ID mantém sua exceção,
inclusive no modo de referência que desabilita somente o secundário.

Toda entrada usada precisa apontar para documento existente com chave/_id e
campo consistentes. Contagem zero persistida, contagem maior que a coleção,
linha extra ou quantidade retornada diferente da contagem produzem InvalidLayout,
sem mascarar erro com fallback. O intervalo aceita todos os IDs ASCII válidos.
Documentos carregados seguem os mesmos limites e são entregues ao engine real
para filtro/sort/limit/project, comparação exata, erro de sort e limite de saída.

O resultado interno registra uma categoria finita Primary/Secondary/Scan,
documentos examinados e bytes carregados; não coloca valores/IDs em labels ou
logs. Não altera HTTP, resposta pública, admissão ou métricas do produto.

Seis testes adicionais verificam:

- escolha seletiva/densa/vazia, ausência de índice e prioridade de ID, incluindo
  o limiar exato de uma ocorrência em 7 versus 8 documentos;
- equivalência com scan em tipos mistos, missing/null, campos compostos armazenados,
  strings vazias/NUL, Unicode distinto/maior scalar com sufixo e 128/130 bytes;
  arrays/objetos como literais continuam recusados pela AST, sem ampliar a API;
- fallback para exists/range/all/not, ordenação/projeção e JSON numérico exato;
  erros de sort/UnsupportedNumber só nos selecionados e ResultTooLarge preservado;
- bordas exatas de 10000 documentos/8 MiB, recusa ao exceder e busca por ID válida
  em coleção maior, com comparação de sucesso **e erro** ao caminho de scan;
- referência faltante/extra/incorreta, contagem ausente/zero/excessiva e handle
  antigo recusados nos casos detectáveis, sem fallback de integridade;
- quatro writers com 80 mutações e leitores comparando planos no mesmo snapshot,
  preservando também snapshot anterior e auditoria após as escritas.

A equivalência pressupõe store íntegro, criado/auditado pelo lifecycle e mantido
nas transações já testadas. Query não varre os documentos excluídos pelo índice
para procurar corrupção lógica arbitrária; apagar entrada e contagem juntas ou
falsificar estatísticas ainda exige auditoria. Não há garantia universal de
detecção por lookup, como demonstrado desde ADR 0033. O teste não homologa
performance HTTP, limiar de custo, RSS ou integração dos endpoints. A proposta
segue pendente de revisão e habilitação explícita no produto, sem migração.

### Primeiro incremento de promoção ao produto

O manifesto/preflight está em `crates/storage/src/format.rs`, módulo normal
compartilhado pelo Store atual e candidato. Elimina as duas implementações
duplicadas, preserva os bytes do formato 1 e exige versão explícita do caller.
Store::create/open continuam fixados na versão 1; reconhecer/serializar versão 2
no codec não inicia seu engine nem migra dados. O candidato exige versão 2.

Cinco testes fixam fixtures das versões 1/2, limite de 1 KiB, schema/duplicados,
distinção dos erros, formas de arquivo e rejeição cruzada antes do banco,
incluindo Store::open real sobre manifesto 2 com banco ausente/inválido. A
[revisão G1](../INDEX_INTEGRATION_REVIEW.md#g1--codec-no-caminho-de-produção)
registra a evidência e a próxima fachada G2. CRUD/indexação, planner, artefato e
instalação de formato 2 ainda estão somente no candidato; nenhum servidor passou
a usar índice secundário por essa extração. O ADR continua proposto.

### G2 — Aceite e promoção da fachada Rust interna

Em 2026-11-21 o contrato de persistência/lifecycle aqui definido é aceito para
IndexedStore, selecionado explicitamente em diretório novo. O aceite se apoia na
evidência acumulada e CI de G1 (40560d0), sem aceitar ativação no servidor, migração
ou o limiar 1/8 como decisão de desempenho. Os registros anteriores de proposta
e cfg(test) são históricos; esta seção descreve o estado atual.

O núcleo de formato 2 (coleções, mutações, grupos, planner e export de snapshot)
passou a código normal do crate. IndexedStore tem Database privado e controla
todas as escritas. IndexedSnapshot mantém uma transação sem expor o engine.
Os tipos de resultado distinguem aborto, commit incerto e publicação incerta.
Captura de grupos/delete só sai da fachada após commit confirmado; não emite
eventos, altera o writer do servidor ou registra payloads por padrão.

IndexedStore::open exige auditoria lógica completa. IndexedMaintenance permite
recuperação explícita de um store estruturalmente válido, sem expor consultas
ou CRUD, e só produz um IndexedStore depois de nova auditoria. Não garante
recuperação de catálogo/documentos arbitrariamente corrompidos nem remove os
limites de build/rebuild. A auditoria de abertura usa memória limitada, mas seu
custo proporcional aos registros ainda requer medição operacional.

Store::create/open e os comandos atuais continuam no formato 1, recusando 2 antes
de abrir redb. Novas APIs Rust são aditivas e experimentais, sem compromisso de
SDK/compatibilidade entre releases. Artefato/instalação candidatos continuam em
cfg(test); integração de bootstrap/auth/CLI, métricas/admissão/writer/feeds e
medições HTTP ainda faltam. A issue #10 permanece aberta.

A [revisão G2](../INDEX_INTEGRATION_REVIEW.md#g2--api-de-storage-indexado-compilada-normalmente)
registra três testes externos da API pública e um teste privado de recusa/reparo
de índice incompleto, além de todas as suites anteriores reutilizadas. Nenhuma
dependência normal ou serviço obrigatório foi adicionado por esta promoção.

## G3.1 — Composição interna com bootstrap e auth

Aceite restrito à API Rust de preparação offline de instalação indexada nova ou
retomada da mesma instalação, sem abrir listener. Reusar o protocolo de fases,
lock exclusivo e journal da ADR 0009. Versionar o registro de bootstrap para
fixar a escolha antes da criação do store: versão 1 continua exclusiva do legado;
versão 2 exige store_format: 2 e a mesma enumeração de fases. JSON estrito e
limitado a 1 KiB. Caminho legado recusa registro 2 antes de qualquer criação ou
auth, inclusive após crash em initializing_store com store ainda ausente. Não
converter registros nem aceitar implicitamente esqueletos experimentais antigos.
A escolha do backend é explícita no entrypoint Rust, nunca inferida como upgrade.

Antes de preparar ou recuperar auth, abrir o store pela fachada IndexedStore,
com auditoria lógica integral. Recusar formato 1, manifesto/layout inválido ou
índice inconsistente antes de emitir credencial. Manter o lock da raiz até fechar
o store e concluir a autenticação. Retomada de auth inicializada, expirada ou
revogada não emite automaticamente outra credencial. Estado restaurado store_ready
pode completar auth com nova entrega, sem copiar credenciais de origem.

Esta composição ainda não habilita formato 2 em serve, auth, backup ou restore
pela CLI; esses caminhos continuam exigindo formato 1. A API Rust experimental
é etapa de integração operacional, não nova recomendação de instalação ao usuário.
Testar exclusão entre processos, retomada das fases, erros anteriores à emissão,
credenciais independentes após cópia e regressão do bootstrap legado.

## Consolidação — caminho normal único

Servidor e comandos existentes criam/abrem somente formato 2. Abertura executa
auditoria lógica integral; falha não é reparada ou mascarada por scan. A
recuperação restrita IndexedMaintenance permanece, pois é uma capacidade
distinta necessária para rebuild/drop antes de voltar ao Store auditado.
Snapshot continua distinto do Store por representar uma transação de leitura.

A exportação normal copia as sete tabelas pelo núcleo antes validado, em lotes
limitados, audita origem/destino e publica manifesto somente após fechar/sync.
O restore operacional valida hashes da cópia, abre/audita o formato atual e
reexporta antes de publicar raiz e entrega de credenciais. O primeiro serve usa
o mesmo Store, mantém credenciais novas e índices; mutações HTTP subsequentes
mantêm esses índices e podem ser auditadas após shutdown.

O wrapper kiarni-format-spike foi removido. Seus testes de contrato e crash
foram movidos para crates/storage/tests/format.rs, preservando as verificações
úteis e adaptando somente o layout esperado. Relatórios antigos são históricos.
Não removeram-se os testes de consistência, concorrência e falhas do núcleo.

## Integração do planner HTTP

Usar o planner existente no caminho HTTP admitido, separando seleção/carregamento
num snapshot da avaliação pelo engine. A primitiva query_read mede só planning e
carregamento; avaliação/serialização continuam na execução HTTP, como antes.
Carregamento retorna caminho escolhido também em erro posterior à seleção.
Antes de escolher plano, manter a classificação inicial id/scan já documentada
na ADR 0025 (inclusive coleção inexistente); seleção secundária substitui scan.

Adicionar somente access="secondary" à família fixa kiarni_query_execution_total,
incluindo falhas posteriores à seleção. Sem nomes de campos/coleções/valores nos
rótulos. Atualizar parser e apresentação do dashboard para a nova série fixa.
Não mudar AST, envelope, admissões, publicação de feeds ou formatos persistidos.
Preservar fallback denso/não elegível, budgets da coleção inteira e exceção de ID;
limiar 1/8 continua hipótese de custo a medir no produto. Não é promessa de ganho.

## Administração offline pela CLI

Comandos aceitos neste incremento, sem endpoint HTTP de manutenção:

- `kiarni index create --collection NOME --field CAMPO --data-dir RAIZ`.
- `kiarni index rebuild --collection NOME --data-dir RAIZ`.
- `kiarni index drop --collection NOME --data-dir RAIZ`.
- `kiarni index audit --data-dir RAIZ` (auditoria integral, sem mutações).

Opções de operação são pares CLI, únicas e limitadas; não são novos campos de
configuração JSON/ambiente. Campo/coleção usam 1–128 bytes ASCII alfanuméricos,
underscore ou hífen; campo _id é recusado. Um índice opcional por coleção, sem
nome separado. Rebuild preserva a definição. Campo só é aceito em create;
coleção é obrigatória nas mutações e não é aceita no audit global.

Exigir raiz existente com fase initialized e arquivos privados, lock exclusivo
não bloqueante antes do engine e até fechá-lo. Não criar instalação, abrir rede,
iniciar/recuperar auth ou emitir token. Criação abre Store com auditoria completa;
rebuild/drop e audit usam IndexedMaintenance, estruturalmente validado. Ler a
definição é metadado permitido nesse handle restrito; não autoriza consultas ou
CRUD. Catálogo inválido, tabelas faltantes e documentos inválidos não são reparados
implicitamente. Estado não initialized exige diagnóstico, sem bootstrap automático.

Sucesso de mutação refere-se ao índice da coleção após commit confirmado e sua
auditoria transacional. Não declarar o banco inteiro recuperado por reparar uma
coleção; auditoria global explícita e abertura normal continuam necessárias.
Reusar os budgets de manutenção: 10000 documentos/8 MiB de JSON, inclusive para
remoção, e até 10000 registros antigos por tabela de entradas/contagens.
Orçamento excedido aborta tudo. Auditoria integral é streaming, mas não tem teto
igual ao backfill nem prazo físico prometido.

CLI distingue index_already_exists, index_not_found, index_maintenance_limit,
invalid_index_collection/field e falhas de sintaxe, sem ecoar valores privados.
Erro commit_uncertain exige inspeção/auditoria antes de repetir; não presumir
rollback nem apagar dados/locks. Não emitir changefeeds por manutenção offline.
