# ADR 0034 — Contrato e ciclo de vida do índice secundário

- Status: **proposta**, ainda sem autorização de mudança do formato do produto.
- Data: 2026-11-19.
- Requisito: RF-04; integração posterior ao aceite técnico M3, issue #10.
- Complementa a [avaliação 0033](./0033-avaliacao-indice-secundario.md).

## Contexto e fronteira desta entrega

O spike demonstrou ganho seletivo, custo de escrita e regressão no caso denso.
Também demonstrou que uma entrada ausente produz falso negativo sem detecção
pelo lookup. O produto usa formato 1, catálogo de coleções com identidade não
reutilizada, primário e scan limitado; seu backup valida exatamente esse layout.
Não basta copiar a tabela experimental para habilitar índices com segurança.

Esta proposta especifica o primeiro lifecycle implementável. Não cria comandos,
endpoints, tabelas ou migrações. API e dados existentes continuam inalterados.
A escolha abaixo precisa ser aceita e demonstrada antes de habilitar o recurso.

Formato 1 continua abrindo pelo caminho atual; versão 2 só pode ser criada em
destino novo. Não editar manifesto para converter um store nem migrar in-place.
Antes de oferecer upgrade de instalações, implementar e testar conversão offline
de um snapshot para nova raiz, preservando a origem e validando todos os dados
antes de publicar. A interface dessa conversão fica para incremento próprio;
até lá, nenhuma promessa de upgrade/downgrade entre formatos. Binários antigos
devem recusar versão 2 no preflight, sem abrir o engine para escrita.

## Contrato proposto

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

Reservar uma versão lógica **2**, ainda não implementada. Manter manifesto e
metadado interno concordantes, com redb 4.5.0; não adicionar tabelas silenciosamente
ao formato 1. Além das tabelas atuais, propor:

| Tabela | Chave | Valor lógico |
| --- | --- | --- |
| index_catalog | collection_id | revisão de definição, campo, tipo string parcial, limite 128 e estado ready |
| secondary_entries | (collection_id, valor string, _id) | unidade |
| collection_stats | collection_id | contagem exata de documentos e bytes JSON armazenados |
| secondary_counts | (collection_id, valor string) | contagem exata de entradas, omitida quando zero |

Não reutilizar nomes de coleção como identidade. Não aceitar revisões de
definição desconhecidas. Codificação física dos valores e validação estrita devem
ser fixadas com fixtures antes de aceitar este ADR; a tabela descreve o contrato
lógico, não autoriza escolher um serializador implicitamente.

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

## Condições de aceitação e sequência de implementação

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

O próximo incremento verificável é o item 1 em destino novo; não é migração de
instalações existentes. Os testes do [spike](../spikes/secondary-index.md) sustentam
a hipótese de indexação, mas não validam este lifecycle, formato ou planner.
Este ADR permanece proposto até resolver a codificação e revisar essas condições;
issue #10 continua aberta. O [ADR 0011](./0011-formato-inicial-e-catalogo.md) segue
vigente para o produto atual.
