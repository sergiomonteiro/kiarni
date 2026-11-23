# Referência da API HTTP

Contratos implementados da alpha, RF-06/07/10, M3. O [aceite técnico](./RELEASE_REVIEW.md)
e a [política pré-1.0](./RELEASE_POLICY.md) não representam release estável. Endereço padrão: `http://127.0.0.1:8711`.
[Primeiro uso](./ALPHA_LOCAL.md), [CLI/configuração](./CLI.md),
[diagnóstico](./TROUBLESHOOTING.md) e [documentação geral](./README.md).

## Autenticação, origem e transporte

Todas as rotas `/v1` exigem um único header `Authorization: Bearer TOKEN`, inclusive
health, status, métricas e erros recentes. Tokens são entregues em arquivo privado
no primeiro serve; rotação e revogação são [offline](./CLI.md). Não há login HTTP,
cookies, chave em query string ou exceção para clientes locais. Tokens ausentes,
inválidos, expirados e revogados recebem 401 sem distinguir a causa.

`Host` deve ser exatamente o endereço configurado, por exemplo `127.0.0.1:8711`.
`localhost:8711` não é equivalente. Se `Origin` estiver presente, deve ser
exatamente `http://127.0.0.1:8711`. Duplicatas ou origem diferente são recusadas.
Com `public_origin`, Host/Origin passam a ser os da origem HTTPS configurada;
[proxy local](./REMOTE_HTTPS.md) termina TLS. `Forwarded`/`X-Forwarded-*` não
concedem confiança. Não há CORS, TLS nativo ou bind público.

HTTP/1.1 com uma requisição por conexão; não há keep-alive/pipelining. Corpos usam
Content-Length; requests com Transfer-Encoding (inclusive chunked) são recusados.
Um proxy pode normalizar framing antes de encaminhar. Não enviar Content-Encoding,
mesmo `identity`. JSON deve ser UTF-8: `application/json`, ou no PATCH
`application/merge-patch+json`; aceita somente parâmetro opcional charset UTF-8.
GET/HEAD/DELETE não aceitam corpo. Falhas de transporte podem fechar o socket
antes de existir envelope de erro ou request ID utilizável.

A raiz `/` e assets fixos do dashboard são públicos, sem dados privados; continuam
sujeitos a Host/Origin, CSP e métodos permitidos. O [dashboard](./DASHBOARD.md)
autentica chamadas `/v1` e mantém token somente em memória.

## Endpoints implementados

`{collection}` e `{id}` são ASCII `[A-Za-z0-9_-]`, de 1 a 128 bytes, sensíveis a
caixa. Sem percent encoding, slash final, segmentos vazios, `.`/`..`, redirects ou
normalização de caminho. Query string só é aceita na listagem de coleções.

| Métodos | Caminho | Sucesso e corpo |
| --- | --- | --- |
| POST | `/v1/collections` | 201, `{"name":"orders"}`; recebe exatamente esse formato e retorna Location. |
| GET, HEAD | `/v1/collections` | 200, `{"collections":["orders"],"next_after":null}`; exige `?limit=1..1000`, aceita `&after=NOME`. |
| DELETE | `/v1/collections/{collection}` | 204 vazio; exige coleção vazia. |
| POST | `/v1/collections/{collection}/documents` | 201, documento completo com `_id`; retorna Location. Não cria coleção implicitamente. |
| GET, HEAD | `/v1/collections/{collection}/documents/{id}` | 200, documento completo. |
| PUT | `/v1/collections/{collection}/documents/{id}` | 200, documento substituído; não faz upsert. |
| PATCH | `/v1/collections/{collection}/documents/{id}` | 200, documento resultante do Merge Patch restrito a objeto. |
| DELETE | `/v1/collections/{collection}/documents/{id}` | 204 vazio. |
| POST | `/v1/collections/{collection}/query` | 200, `{"documents":[...],"count":1}`. |
| GET, HEAD | `/v1/collections/{collection}/count` | 200, contagem limitada da coleção inteira. |
| GET | `/v1/collections/{collection}/changes` | 200, SSE; exige Accept `text/event-stream`. |
| GET, HEAD | `/v1/health/live` | 200, `{"status":"live"}`; não consulta storage. |
| GET, HEAD | `/v1/health/ready` | 200, `{"status":"ready"}`; verifica storage. |
| GET, HEAD | `/v1/status` | 200, versão, uptime HTTP e ocupação/limites de conexões. |
| GET, HEAD | `/v1/metrics` | 200, texto Prometheus. |
| GET, HEAD | `/v1/errors/recent` | 200, histórico limitado de erros concluídos. |

HEAD mantém status e headers do GET, inclusive Content-Length da representação,
mas não envia corpo. HEAD de changes é 405 e não abre assinatura. Métodos não
listados de rota conhecida recebem 405/Allow; rota desconhecida recebe 404 após
autenticação. Não existe GET do recurso individual de coleção: Location de sua
criação identifica o recurso administrado por DELETE.

Listagem é lexical crescente; after é exclusivo e next_after só aparece com
valor não-null se houver outra página. O cliente precisa enviar limit em cada
request. Parâmetros extras, repetidos, percent-encoded ou inválidos são recusados.
Paginação entre requests não é snapshot consistente sob escrita concorrente.

## Documentos e alterações

Raiz objeto JSON UTF-8; duplicatas de chaves são rejeitadas em qualquer nível,
inclusive escapes equivalentes. Ausente difere de null. Documentos preservam
números JSON brutos, sem conversão para f64; não há garantia de preservar todo
whitespace/ordem de membros após gerar ID ou transformar documento.

`_id` ausente no POST gera 32 hexadecimais minúsculos aleatórios. `_id:null`, número,
string vazia ou caracteres fora da gramática são inválidos. Unicidade é por
coleção; ID já existente recebe 409/duplicate_id. Não é UUID nem ordenação temporal.

PUT substitui os campos do documento existente. Pode omitir `_id`, que será
injetado da rota; ID diferente recebe 422/id_mismatch. PATCH não pode conter
`_id` na raiz, mesmo igual ou null: 422/immutable_id. Campos null de PATCH removem
propriedades, objetos mesclam recursivamente e arrays são substituídos inteiros.
Para gravar null literal em um membro, use PUT. `_id` aninhado é dado comum.

No-op retorna 200 com o documento original, sem commit de mutação ou evento.
Ordem de membros, whitespace e escapes equivalentes de strings não alteram a
comparação estrutural; ordem de arrays e **tokens numéricos** alteram. Assim,
`1` → `1.0` gera alteração, embora sejam iguais numa comparação numérica de query.
Uma escrita efetiva valida tamanho final após injetar ID/aplicar patch.

Reduzir max_document_bytes não apaga documentos anteriores. GET/HEAD, DELETE e
before/after de eventos seguem o teto persistido de 1 MiB. Corpo de PUT/PATCH segue
o limite vigente; resultado de no-op pode devolver documento maior que esse
limite, e uma alteração efetiva deve caber nele. Não há transação multi-documento,
upsert, revisão otimista ou idempotency key.

## Consultas

Corpo JSON: `from` obrigatório, igual à coleção da rota; `limit` inteiro obrigatório
1–1000. Opcionais: `id`, `filter`, `sort`, `project`. Campos desconhecidos/duplicados
são rejeitados. [Exemplo executável](./examples/query.json):

```json
{"from":"orders","filter":{"op":"gte","field":"total","value":9007199254740993},"sort":[{"field":"total","direction":"desc"}],"limit":10,"project":["status","total"]}
```

| Operador | Campos e semântica |
| --- | --- |
| eq | field/value; igualdade escalar, sem coerção entre tipos. |
| lt, lte, gt, gte | field/value; somente número/número ou string/string. |
| exists | field; null existe, campo ausente não. |
| all, any | args não vazio, composição booleana com curto-circuito. |
| not | arg; negação booleana, inclusive de comparação falsa por ausência. |

Fields são nomes literais da raiz, UTF-8, 1–128 bytes, sem controle ASCII; `a.b`
é uma chave literal, não caminho. Valores comparados são null, bool, número ou
string. Objetos/arrays como literais são recusados; como valores do documento não
satisfazem comparação escalar. `eq null` não seleciona ausentes. Faixa com tipos
incompatíveis retorna false. Strings são sensíveis a caixa e sem normalização.

Números usam comparação decimal exata: `1 = 1.0 = 1e0`, `-0 = 0`. Tokens comparados
até 256 bytes e expoente explícito absoluto até 10000. Documentos podem armazenar
números fora disso; `unsupported_number` surge quando forem examinados em
comparação/sort, inclusive conforme curto-circuito, sem arredondamento silencioso.

Execução: id → filter → sort → limit → project. Sort aceita até quatro chaves
únicas, direction asc/desc obrigatório. Asc: ausente < null < bool < número < string;
desc inverte. Objeto/array em valor ordenado gera unsupported_sort_value.
Desempate final sempre `_id` crescente; sem sort também `_id` crescente.
Project aceita até 32 nomes únicos e preserva `_id`; lista vazia retorna só `_id`.
Ausência de project devolve documento inteiro. Não altera os campos de filter/sort.

ID usa índice primário e pode combinar filtro. Sem ID, o planner usa um índice
secundário existente somente para igualdade direta de string de até 128 bytes
UTF-8 no campo simples indexado, quando a contagem de candidatos é no máximo
1/8 da coleção. Casos densos, tipos/operadores não elegíveis e consultas sem
índice usam scan. Tudo é lido de um único snapshot e avaliado pelo mesmo engine.
Esse limiar conservador teve benefício nos
[fixtures medidos](./benchmarks/secondary-index-http-2026-11-22.md),
sem demonstrar ponto ótimo universal ou garantir menor latência em qualquer carga.

O orçamento de **10.000 documentos/8 MiB de JSON da coleção inteira** continua
valendo antes da avaliação: índice/filtro/limit seletivo não transforma coleção
acima do orçamento em consulta aceita. Nesse caso, o scan retorna
422/query_too_complex sem resultado parcial. A busca por ID mantém sua exceção.
Uma query/contagem executa por vez; admissão FIFO espera até 100 ms antes de
503/service_unavailable. Resultado deve caber no envelope configurado; count é
quantidade **retornada**, não total da coleção. Não há paginação de documentos.

O caminho selecionado aparece em métricas fixas id/scan/secondary, inclusive
quando a execução falha; não muda o envelope da query. Definições são administradas pelos [comandos offline de índice](./INDEXES.md);
consultas não criam índices automaticamente. Contratos: [ADR 0006](./adr/0006-ast-e-comparacao-json.md)
e [ADR 0034](./adr/0034-lifecycle-indice-secundario.md).

### Contagem da coleção

```json
{"count":"30","exact":true,"limits":{"documents":10000,"document_bytes":8388608}}
```

Count aqui é string decimal. exact:false significa **pelo menos N**, nunca total
exato nem estimativa estatística. Lê snapshot do índice primário, com lookahead
limitado; não aplica a query do cliente e não compara o JSON. Não é contador de
custo constante. [Custo e consistência](./DOCUMENT_COUNTS.md).

## Changefeeds

GET da rota changes, sem corpo/query, um único Accept exatamente
`text/event-stream` (caixa ignorada). Authorization continua obrigatório.
Last-Event-ID, mesmo vazio, é recusado com 400/resume_not_supported.

```text
event: ready
data: {"snapshot":false,"replay":false}

: heartbeat

event: change
data: {"type":"insert","collection":"orders","id":"first","sequence":"1","before":null,"after":{"_id":"first","status":"open"}}

```

`ready` confirma inscrição, não snapshot. Insert tem before:null; delete tem
after:null; update contém ambos. Mudanças são publicadas **após commit confirmado**
e ordenadas localmente no processo. Sequence é string decimal, não cursor durável
ou token de retomada; reinício não preserva essa sequência. No-op e operações
rejeitadas não geram evento. O feed é da coleção inteira: não aceita AST/filtro.

Fila cheia encerra assinante lento, sem bloquear writer indefinidamente. Terminal
best-effort: `event: stream_end`, data `{"code":"overflow"}`, `shutdown` ou
`interrupted`; pode haver somente EOF, especialmente com frame parcial/rede lenta.
Frame SSE tem teto de 3 MiB codificados; overhead de linhas pode fazer um documento
válido excedê-lo. Parser deve juntar linhas data com newline e preservar números.
Não há campos SSE id/retry, replay, exactly-once ou reconexão automática garantida.

Consulta seguida de assinatura não é snapshot+stream atômico. Para estado derivado,
EOF/interrupção/overflow exigem reconciliação; uma nova assinatura recebe apenas
mudanças futuras. Não interprete EOF como prova de stream íntegro. O dashboard usa
fetch com header Bearer; EventSource nativo não oferece esse header arbitrário.
[Guia do feed](./DASHBOARD_FEEDS.md) e [métricas](./CHANGEFEED_METRICS.md).

## Telemetria

Status: uptime_seconds em string decimal; admitted inclui a própria requisição,
conexões incompletas e streams. Streaming é subconjunto de admitted; não são
clientes únicos. Reinício zera uptime/contadores. [Campos](./SERVER_STATUS.md).

Metrics usa `text/plain; version=0.0.4; charset=utf-8`. HTTP, queries, storage e
feeds têm [guias de interpretação](./README.md#operar-e-diagnosticar). Uma coleta
não é snapshot atômico de todos os componentes; concluir escrita local não prova
recebimento pelo cliente. Não há métricas de CPU/RSS nesta entrega.

Errors/recent tem até oito registros em memória, recentes primeiro; dropped_total
mede perdas por contenção, não substituição normal. Inclui conclusões 4xx/5xx,
falhas de escrita e overflow/interrupção de SSE; não é auditoria. Strings decimais
preservam timestamp/duração/contadores; campos podem ser null. Não contém payload,
URL, coleção, IDs de documento ou credenciais. [Schema e limites](./RECENT_ERRORS.md).

## Headers e erros

JSON usa Content-Type application/json, Cache-Control no-store e X-Request-ID.
204 não envia corpo nem Content-Type/Content-Length. Criação envia Location.
X-Request-ID tem formato `req_` + 32 hexadecimais e é gerado pelo servidor:
X-Request-ID do cliente não é ecoado como identidade confiável.

```json
{"error":{"code":"duplicate_id","message":"The document ID already exists in this collection."},"request_id":"req_00000000000000000000000000000000"}
```

O ID acima é ilustrativo. Correlacione o ID real com logs/erros recentes. Mensagens
são estáticas, sem parser/paths/token. 401 envia WWW-Authenticate: Bearer; 405,
Allow; 429/503, Retry-After: 1. Um Retry-After ou timeout **não autoriza repetir uma
mutação com segurança**: desconexão pode deixar commit com resultado desconhecido.
Após começar SSE, falhas não se tornam envelope JSON ou novo status HTTP.

<!-- error-catalog:start -->
| Código | HTTP | Mensagem estática |
| --- | --- | --- |
| `invalid_path` | 400 | The request path is invalid. |
| `route_not_found` | 404 | The route does not exist. |
| `method_not_allowed` | 405 | The method is not allowed for this route. |
| `invalid_json` | 400 | The JSON input is invalid. |
| `unexpected_body` | 400 | This operation does not accept a body. |
| `invalid_document` | 422 | The document is invalid. |
| `invalid_id` | 422 | The document ID is invalid. |
| `document_must_be_object` | 422 | The document must be an object. |
| `document_too_deep` | 422 | The document exceeds the nesting limit. |
| `invalid_collection` | 422 | The collection name is invalid. |
| `invalid_query` | 422 | The query is invalid. |
| `invalid_limit` | 422 | The result limit is invalid. |
| `query_too_complex` | 422 | The query exceeds the complexity limit. |
| `unsupported_number` | 422 | A compared number exceeds the supported range. |
| `unsupported_sort_value` | 422 | The sort value type is unsupported. |
| `id_mismatch` | 422 | The body ID differs from the route ID. |
| `immutable_id` | 422 | The patch must not include the reserved ID field. |
| `collection_mismatch` | 422 | The query collection differs from the route collection. |
| `duplicate_id` | 409 | The document ID already exists in this collection. |
| `collection_exists` | 409 | The collection already exists. |
| `collection_not_found` | 404 | The collection does not exist. |
| `document_not_found` | 404 | The document does not exist. |
| `collection_not_empty` | 409 | The collection must be empty before deletion. |
| `document_too_large` | 413 | The document exceeds the byte limit. |
| `query_too_large` | 413 | The query exceeds the byte limit. |
| `request_too_large` | 413 | The request body exceeds the byte limit. |
| `result_too_large` | 422 | The result exceeds the response byte limit. |
| `unsupported_media_type` | 415 | The request media type is unsupported. |
| `not_acceptable` | 406 | The requested response media type is unsupported. |
| `unauthorized` | 401 | Authentication is required or invalid. |
| `forbidden` | 403 | The request is not permitted. |
| `resume_not_supported` | 400 | Stream resumption is unsupported. |
| `subscription_limit` | 429 | The subscription limit has been reached. |
| `write_busy` | 503 | The writer is busy. |
| `storage_unavailable` | 503 | Storage is unavailable. |
| `service_unavailable` | 503 | The service is unavailable. |
| `shutting_down` | 503 | The server is shutting down. |
| `id_generation_failed` | 503 | Document ID generation is unavailable. |
| `id_generation_exhausted` | 503 | Document ID generation attempts were exhausted. |
| `sequence_exhausted` | 503 | The local event sequence is exhausted. |
| `internal_error` | 500 | An internal error occurred. |
| `request_timeout` | 408 | The request was not received in time. |
| `headers_too_large` | 431 | The request headers exceed the byte limit. |
<!-- error-catalog:end -->

## Limites efetivos

| Recurso | Limite |
| --- | --- |
| Caminho/headers | 2048 bytes / 8192 bytes, até 32 headers |
| Criação de coleção | 1024 bytes |
| Documento | escrita até max_document_bytes; formato persistido 1048576 bytes; profundidade 64 containers |
| Query | até max_query_bytes (máximo 16384); filtro 64 nós/profundidade 8; sort 4 campos; project 32 |
| Resultado | até max_result_bytes (máximo 4194304), incluindo envelope |
| Coleções por página / documentos por query | 1–1000, limite explícito obrigatório |
| Scan / contagem | 10000 documentos e 8388608 bytes, com lookahead limitado na contagem |
| SSE | max_streams dentro de max_requests; feed_queue eventos; 3145728 bytes por frame |
| Admissão writer | fila/input limitados, até 8 MiB de input e espera de admissão 100 ms |

Defaults, intervalos, prazos e restrições combinadas: [CLI/configuração](./CLI.md).
Limites de buffers/admissão não são teto de RSS nem garantia de latência.

## Exemplos verificáveis

Na raiz do repositório, após iniciar serve com raiz nova, configure o arquivo
privado real e carregue a função. A URL deve coincidir com o bind configurado:

```bash
token_file='./kiarni-data.credentials/token-EXEMPLO.txt'
source docs/examples/client.sh
```

Execute os blocos abaixo em ordem, numa coleção ainda inexistente. Eles são
executados pelo teste `scripts/docs_smoke.py`; as respostas permanecem JSON bruto.
Não envie tokens em argumentos/URL, não use `set -x`, `curl -v` ou jq/parse que
converta números para float ao processar documentos precisos.

Criar coleção e documento (201, 201):

<!-- verify:api -->
```bash
kc -H 'Content-Type: application/json' --data-binary @docs/examples/collection.json "$api_base/v1/collections"
kc -H 'Content-Type: application/json' --data-binary @docs/examples/document.json "$api_base/v1/collections/orders/documents"
```

Listar, ler e consultar (200); a consulta retorna apenas `_id`, status e total:

<!-- verify:api -->
```bash
kc "$api_base/v1/collections?limit=10"
kc "$api_base/v1/collections/orders/documents/first"
kc -H 'Content-Type: application/json' --data-binary @docs/examples/query.json "$api_base/v1/collections/orders/query"
```

Substituir e aplicar patch (200, 200). O PUT preserva ID da rota; PATCH remove optional:

<!-- verify:api -->
```bash
kc -X PUT -H 'Content-Type: application/json' --data-binary @docs/examples/replace.json "$api_base/v1/collections/orders/documents/first"
kc -X PATCH -H 'Content-Type: application/merge-patch+json' --data-binary @docs/examples/patch.json "$api_base/v1/collections/orders/documents/first"
```

Consultar contagem, saúde e telemetria (200):

<!-- verify:api -->
```bash
kc "$api_base/v1/collections/orders/count"
kc "$api_base/v1/health/live"
kc "$api_base/v1/health/ready"
kc "$api_base/v1/status"
kc "$api_base/v1/metrics"
kc "$api_base/v1/errors/recent"
```

Para observar SSE, **antes de excluir**, em outro terminal com a mesma preparação:

```bash
kc --max-time 60 --no-buffer -H 'Accept: text/event-stream' "$api_base/v1/collections/orders/changes"
```

O prazo de 60 s é do cliente, não duração garantida do feed. Aguarde ready e faça
uma alteração pelo outro terminal; Ctrl+C cancela o leitor. O teste abre SSE e
verifica um evento real antes de executar a limpeza:

<!-- verify:api -->
```bash
kc -X DELETE "$api_base/v1/collections/orders/documents/first"
kc -X DELETE "$api_base/v1/collections/orders"
```
