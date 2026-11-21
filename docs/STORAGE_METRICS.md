# Métricas de storage

Recorte M3/RF-09. GET/HEAD `/v1/metrics` acrescenta famílias `kiarni_storage_`,
com os mesmos Bearer, Host/Origin, request ID e limites. Sem nova dependência,
formato persistido ou mudança de durabilidade/publicação. Contadores pertencem
ao handle Store aberto e reiniciam ao reabrir; bootstrap/recovery do engine não
são contados como chamadas feitas nesse handle.

## Chamadas lógicas e tempos

`kiarni_storage_operations_total{operation,outcome="success"|"error"}` conta
conclusões nas fronteiras instrumentadas abaixo. Success significa que a
primitiva retornou Ok, não que um documento foi alterado ou entregue. Error
inclui conflitos/validação, não somente falha física. Buscar documento ausente
retorna Ok(None) no storage e pode produzir 404 no HTTP.

| Operation | Fronteira |
| --- | --- |
| health | check_readable, incluindo validação do layout acessado. |
| catalog_list | list_collections e sua leitura limitada. |
| catalog_lookup | collection, lookup de identidade/geração. |
| count | count_documents, exata ou limite inferior válido. |
| query_read | query_candidates, seleção, leitura e parsing num snapshot limitado; exclui avaliação da AST e serialização. |
| document_read | get_document, incluindo validação do JSON/chave no snapshot. |
| insert | insert_document ou insert_fixture, com manutenção transacional de índice. |
| update | update_before_commit, usado pelo update_document individual. |
| delete | delete_inner, com ou sem captura para feeds. |
| collection_create | create_collection. |
| collection_drop | drop_collection. |
| update_group | apply_update_group, com ou sem captura para feeds. |

Chamadas aninhadas não contam duas vezes a mesma primitiva: get_fixture direto
não é medido; insert_document/insert_fixture medem cada inserção uma vez. Chamadas diretas
ao redb em fixtures/spikes não entram. Uma query HTTP pode fazer catalog_lookup
e query_read; esses valores não são número de requisições, documentos ou páginas.

`kiarni_storage_operation_duration_seconds{operation}` inclui validação,
transação, parsing/transformação e eventual commit da primitiva, inclusive erro.
Expõe buckets cumulativos 0.001, 0.01, 0.1, 1, 5, 30 e +Inf, sum e count. Soma
acumula micros inteiros saturantes e é exposta em segundos. Não é tempo de disco,
CPU ou fsync isolado. O commit abaixo é parte dessa duração; não some ambos como
etapas disjuntas. Não há percentis ou taxas calculadas no servidor.

## Commit e grupos

`kiarni_storage_commits_total{outcome="confirmed"|"uncertain"}` conta somente
chamadas de commit no Store aberto. Confirmed significa retorno Ok do engine;
uncertain significa erro ao confirmar, sem presumir rollback. No-op, abort e
rejeição antes do commit não incrementam essas séries. O bootstrap inicial usa
outra instância/transação e não entra. A durabilidade continua Immediate, com
as mesmas evidências e limitações já documentadas.

`kiarni_storage_commit_duration_seconds` mede a chamada do engine, com os mesmos
buckets, sum/count. Não comprova entrega HTTP nem publicação de evento. Uma
falha após o commit pode produzir resposta de erro e nenhum receipt, enquanto
a métrica de commit permanece confirmed. Essa distinção é intencional.

`kiarni_storage_group_receipts_total{commit="confirmed"|"not_required"}` mede
receipts efetivamente retornados. `kiarni_storage_group_items_total` separa
changed, noop e rejected somente nesses receipts. Um grupo misto pode retornar
receipt com itens recusados e um único commit para os alterados. Operações
intermediárias sobre o mesmo ID contam por item; não são quantidade de documentos
únicos, páginas ou HTTPs confirmados. Grupo inteiro abortado/uncerto não gera
receipt/itens bem-sucedidos nesses contadores.

## Tamanho do arquivo

`kiarni_storage_file_logical_bytes` é o comprimento lógico de data.redb,
consultado por metadata em um descritor de leitura mantido pelo Store. Não é
uso físico de blocos, espaço livre, payload dos documentos ou checagem de
integridade. Pode crescer com alocação do engine e não diminuir ao excluir
um documento; não indica, por si só, commit ou dados efetivamente confirmados.

`kiarni_storage_file_bytes_available` vale 1 quando metadata está disponível;
em falha, vale 0 e o tamanho é NaN, sem inventar tamanho zero. As outras séries
continuam disponíveis. O descritor é aberto após validação e fechado depois do
Database, sem adquirir locks adicionais nem abrir/fechar arquivo a cada scrape.
Isso consome um descritor extra por Store. Fstat é sujeito ao filesystem; não
há promessa de timeout físico. Não há scan do banco nem transação de escrita
para produzir o tamanho.

## Concorrência, privacidade e limites

Estado fixo de contadores atômicos saturantes e histogramas. Buckets são
atualizados do maior para o menor com release e lidos do menor para o maior com
acquire; +Inf e count compartilham a mesma observação. Não há filas, mapas por
coleção, logger ou gate no scrape. O estado HTTP/query/storage e o descritor
cabem na reserva nominal de telemetria já existente de 4 KiB. O corpo de
exposição pertence à reserva HTTP de resposta; isso não é limite de RSS.

Coleta concorrente não é snapshot transacional entre todas as famílias. Os
valores podem divergir transitoriamente; panic/encerramento abrupto não garantem
registro. Nenhum rótulo contém caminho, coleção, ID, payload, token ou erro
dinâmico. Não há histórico de documentos ou amostras de latência retidas.

O [dashboard](./DASHBOARD_METRICS.md) apresenta storage junto de HTTP/query/feeds:
resumo de arquivo/commits/grupos, duração de commit expansível e uma primitiva
por vez em seletor. A seleção reutiliza a coleta atual, sem fetch adicional.
As 237 séries são validadas antes de renderizar. Tamanho NaN com disponibilidade
zero é exibido como Indisponível, mantendo os outros valores. A exposição
completa continua dentro do leitor de 64 KiB. Métricas de páginas, cache, fsync isolado e documentos examinados não
foram implementadas neste recorte.

## Verificação

Testes de storage cobrem leitura presente/ausente, erros, no-op individual,
grupos mistos e sem alterações, commits, reinício e tamanho lógico. O scrape é
exercitado com transação de escrita aberta. Injeção de falha antes/depois do
commit demonstra confirmação sem receipt; a classificação uncertain do retorno
do engine é testada no contador, sem alegar injeção real de falha física.

HTTP real verifica autenticação, CRUD/no-op, grupos, privacidade, tamanho de
exposição e reset. Os testes existentes de concorrência, locks e crash/restart
continuam sendo verificações necessárias. [ADR 0026](./adr/0026-metricas-de-storage.md).
Não houve benchmark novo, Prometheus externo/promtool ou injeção real de falha física de disco
neste incremento. RF-09 segue parcial.

Validação local em 2026-11-14: 258 testes Rust passaram no perfil padrão e a
suíte com profiling também passou, incluindo locks, concorrência e crash/restart
existentes. Passaram fmt, clippy nos dois perfis, build release, fronteira de
nove pacotes, Chromium real, links locais e `git diff --check`. Dois workers de
fixture continuam ignorados diretamente e executados pelos testes pais. Foram
adicionados dois testes de storage e uma integração HTTP; a injeção de falha
pós-commit ganhou verificações de métricas.

A primeira execução completa revelou uma premissa de tempo na fixture de
agrupamento: a preparação de 32 produtores com writer intencionalmente parado
podia consumir a lease de 100 ms. O teste de ordem/agrupamento usa agora uma
lease lógica limitada à fixture, mantendo a exigência de grupo de 32 e os
resultados individuais. O timeout de produção e seus testes próprios continuam
em 100 ms. O caso isolado e as suítes finais passaram. CI remoto e outras
plataformas não foram executados nesta entrega.
