# Índices: administração offline

RF-04, evolução após M3, issue #10. [Contrato](./adr/0034-lifecycle-indice-secundario.md),
[consultas HTTP](./API.md), [métricas](./QUERY_METRICS.md).

## Criar, reconstruir, remover e auditar

Pare `serve` com Ctrl+C ou SIGTERM e aguarde encerrar. A instalação deve estar
inicializada e a coleção já deve existir. Os comandos não criam raiz, coleção
ou credencial. Na sequência abaixo, `./data` é uma instalação existente e
`orders` é a coleção criada nos exemplos da API.

Cada coleção aceita um índice secundário opcional, sem nome separado, sobre
um campo simples da raiz. Campo e coleção: 1–128 bytes ASCII alfanuméricos,
underscore ou hífen. `_id` é exclusivamente primário; campo com ponto não é um
caminho aninhado aceito para criar esse índice. São indexadas strings de até
128 bytes UTF-8, sem normalização; outros valores continuam documentos válidos.

<!-- verify:index -->
```bash
kiarni index create --data-dir ./data --collection orders --field status
kiarni index audit --data-dir ./data
kiarni index rebuild --data-dir ./data --collection orders
kiarni index drop --data-dir ./data --collection orders
kiarni index audit --data-dir ./data
```

Esse exemplo demonstra o ciclo completo e termina sem índice. Para usar o índice,
execute create e audit, depois inicie `serve` na mesma raiz. O planner HTTP pode
selecioná-lo em igualdade direta seletiva; não é usado em todo filtro. A métrica
`kiarni_query_execution_total{access="secondary"}` registra a seleção, inclusive
em erro posterior. Não há índice criado automaticamente por consulta.

- create constrói e ativa o índice no mesmo commit; índice existente é erro,
  mesmo quando o campo pedido é o mesmo.
- rebuild reconstrói o índice existente e preserva sua definição.
- drop remove somente o índice e mantém documentos e coleções.
- audit verifica todo o banco e informa coleções/documentos; não recebe coleção
  nem campo. Verifica também ausência/excesso de entradas e contagens divergentes.

`--field` só é aceito em create. `--collection` é obrigatório nas três mutações.
Opções de operação são únicas, somente CLI, e não são campos JSON ou variáveis
de ambiente. A configuração comum segue CLI > ambiente > arquivo > defaults;
até 32 pares são aceitos. Opções de rede são validadas, mas não há bind nem HTTP.
Não é necessário passar token: acesso offline depende dos arquivos privados do
usuário e do lock exclusivo, como nos comandos existentes de backup/auth.

## Lock, limites e recuperação

Outro servidor ou comando offline na mesma instalação causa
`data_directory_locked`. O lock da raiz é adquirido antes de abrir o engine e
mantido até fechá-lo. Não remova o lock nem execute manutenção com servidor ativo.
Bootstrap precisa estar em initialized; um estado incompleto é recusado, sem
inicialização automática. A operação não prepara/recupera auth, renova token ou
altera sua seleção; credenciais expiradas/revogadas permanecem assim.

Criação abre o Store com auditoria completa antes de construir. Rebuild/drop
usam a abertura restrita de manutenção: assim é possível reparar um índice que
impede a abertura normal. O catálogo e a estrutura ainda precisam ser válidos.
Não recria tabelas faltantes, inventa definições ou aceita documentos inválidos.

Cada manutenção examina até **10.000 documentos e 8 MiB de JSON da coleção**,
inclusive drop. A limpeza das estruturas antigas também tem teto de 10.000
registros por tabela de entradas/contagens. Estouro aborta toda a transação e
preserva o índice anterior; não publica estado parcial. Reduzir limit da query
ou max_document_bytes não muda esses tetos de leitura do formato.

Audit global e abertura normal verificam a base inteira por cursores; não têm o
mesmo teto de backfill nem prazo global prometido. Cache configurado de 8 MiB
do engine não é promessa de RSS total. Abertura pode executar recovery do engine;
audit não faz mutações lógicas de documentos/índices nem promete arquivo físico
intocado. Nenhum comando emite changefeeds por manutenção.

Quando o problema for uma entrada/contagem secundária ausente ou divergente,
pare o servidor, execute rebuild na coleção afetada e audit antes de iniciar
serve. Drop é a alternativa explícita para retirar o índice. Sucesso de uma
mutação confirma a coleção tratada; não prova que outras coleções estejam
íntegras. A auditoria global e a abertura normal continuam necessárias.

Não há backfill online, operação parcelada, `--force`, migração ou apagamento de
dados incompatíveis. [Backup](./BACKUP.md) e [restauração](./RESTORE.md) preservam
definições, entradas e contagens no snapshot consistente.

## Diagnóstico

Sucesso retorna código 0 e resumo sem dados dos documentos. Erro retorna código
1, stdout vazio e um código curto em stderr; argumentos privados não são ecoados.

| Código | Significado |
| --- | --- |
| `unsupported_index_command` | Subcomando ausente ou diferente de create/rebuild/drop/audit. |
| `missing_index_collection`, `missing_index_field` | Opção obrigatória ausente. |
| `duplicate_index_argument`, `unexpected_index_argument` | Opção repetida ou indevida para o subcomando. |
| `invalid_index_collection`, `invalid_index_field` | Nome inválido, campo `_id` ou texto não UTF-8. |
| `index_already_exists`, `index_not_found` | Definição já existe ou não existe. Não há idempotência implícita nesses casos. |
| `collection_not_found` | Coleção não existe na instalação selecionada. |
| `installation_not_initialized` | Bootstrap incompleto; comando não o conclui. |
| `index_maintenance_limit` | Orçamento excedido; nenhuma alteração foi confirmada. |
| `invalid_storage_layout` | Estrutura ou integridade inválida; audit/rebuild não são reparo universal. |
| `commit_uncertain` | A confirmação foi perdida; a alteração pode estar persistida. Não presuma rollback nem repita automaticamente. Preserve o estado e audite antes de decidir a próxima ação. |

Falhas de configuração, permissões e lock seguem a [referência da CLI](./CLI.md).

## Evidência e limites

Testes com o binário real cobrem create/rebuild/drop/audit, recusa de servidor
ativo, bind ocupado sem abertura de rede, índice usado pelo HTTP, escritas e
restart com credenciais preservadas. Testes de recuperação removem entrada e
contagem, reconstroem/removem pela abertura restrita e recusam tabelas faltantes.
Casos acima de 8 MiB verificam rollback e preservação da definição anterior.

Subprocessos são mortos antes/depois do commit de cada mutação e na abertura
do comando audit:
demonstram lock retido, recuperação de processo e nenhuma nova credencial.
As janelas internas de build/rebuild/drop continuam cobertas pelos testes do
mesmo núcleo transacional. Isso não simula queda de energia ou falha física.
Benchmarks no produto e homologação do limiar de seletividade permanecem pendentes.
