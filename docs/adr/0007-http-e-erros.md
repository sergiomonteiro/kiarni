# 0007 — Contrato HTTP e erros estáveis

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md))
- Data: 2026-11-10
- Requisitos e marco: M0; RF-02/RF-03/RF-06/RF-07 e preparação para RF-09.

## Contexto e alternativas

Os contratos de documentos, consulta e stream ainda são propostas. Precisamos
alinhar rotas e erros antes de implementar handlers. As alternativas são erros
livres versus catálogo estático e atualização ambígua versus PUT/PATCH distintos.
Propor catálogo estático e JSON Merge Patch restrito a objetos, preservando `_id`.
Este incremento não escolhe framework HTTP e não inicia servidor.

## Rotas e respostas de sucesso propostas

| Método | Rota | Sucesso | Corpo |
|---|---|---|---|
| POST | /v1/collections | 201 | `{"name":"orders"}` |
| GET/HEAD | /v1/collections | 200 | `{"collections":["orders"],"next_after":null}` |
| DELETE | /v1/collections/{collection} | 204 | vazio; coleção deve estar vazia |
| POST | /v1/collections/{collection}/documents | 201 | documento completo com `_id` |
| GET/HEAD | /v1/collections/{collection}/documents/{id} | 200 | documento completo |
| PUT | /v1/collections/{collection}/documents/{id} | 200 | documento substituído |
| PATCH | /v1/collections/{collection}/documents/{id} | 200 | documento atualizado |
| DELETE | /v1/collections/{collection}/documents/{id} | 204 | vazio |
| POST | /v1/collections/{collection}/query | 200 | `{"documents":[...],"count":1}` |
| GET | /v1/collections/{collection}/changes | 200 SSE | conforme ADR 0005 |
| GET/HEAD | /v1/health/live | 200 | `{"status":"live"}` |
| GET/HEAD | /v1/health/ready | 200 | `{"status":"ready"}` se storage utilizável |

- Todas as rotas da API exigem autenticação antes de revelar rota/recurso ou
  alocar assinatura. Sem exceção remota implícita para health ou loopback atrás
  de proxy. Modelo completo de token ainda exige ADR próprio.
- Coleção não é criada implicitamente ao inserir. Criar coleção já existente é
  409; documento ausente é 404; inserir ID duplicado na coleção é 409.
- A proposta de DELETE de coleção vazia evita cascade implícito. Coleção não
  vazia retorna 409/collection_not_empty, sem excluir nada. Não há endpoint extra
  de cascade neste recorte; a operação precisa de verificação transacional futura.
- POST de coleção recebe somente `{"name":"orders"}`. A listagem exige limit
  de 1–1000 e aceita after como nome de coleção, exclusivo; ordem lexical crescente.
  next_after é o último nome retornado somente se existir outra página. Paginação
  entre requests não é snapshot consistente; concorrência pode alterar a listagem.
  Parâmetros extras/duplicados são rejeitados. Parsing desses parâmetros ainda
  não foi implementado pelo modelo.
- POST de documento usa o ADR 0001/0002. PUT é substituição de documento existente,
  nunca upsert: `_id` ausente no corpo será obtido da rota, ID presente deve ser
  igual ao da rota. Divergência retorna 422/id_mismatch.
- PATCH usa `application/merge-patch+json`, com raiz objeto e `_id` da raiz proibido,
  mesmo com valor igual ou null. Campos null removem propriedades; arrays são
  substituídos como unidade e objetos mesclam recursivamente. Para gravar null
  literal, usar substituição PUT. `_id` aninhado continua dado comum.
- Essa é a restrição do modelo de documentos ao
  [RFC 7396](https://www.rfc-editor.org/rfc/rfc7396.html), que também admite patches
  não objetos. Não alegar suporte irrestrito ao RFC: o spike valida o envelope,
  não executa merge. Merge transacional, limite final, no-op e eventos de PATCH
  precisam de implementação/testes antes de aceitar esse contrato.
- Query exige from igual à coleção da rota. Divergência é 422/collection_mismatch.
  Resultados seguem ADR 0006; números são JSON bruto, sem conversão a f64. Count
  é quantidade retornada, não total global. Não há paginação/replay implícitos.
- Criar coleção/documento deve retornar Location para o recurso, montado somente
  a partir de identificadores validados. HEAD usa mesmo status/headers do GET,
  com corpo vazio; HEAD de changes não cria assinatura e retorna 405.
- Readiness indisponível retorna 503/storage_unavailable; liveness não consulta
  storage. Health não inclui caminhos, documentos, credenciais ou diagnóstico.

## Erros e request IDs

```json
{
  "error": {"code":"duplicate_id","message":"The document ID already exists in this collection."},
  "request_id":"req_00000000000000000000000000000000"
}
```

- Status HTTP segue o [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110.html).
  Sintaxe JSON inválida e chaves duplicadas: 400. Validação semântica: 422.
  Corpo acima do limite: 413; media type: 415; conflito: 409; recurso: 404;
  limite de assinaturas: 429; writer/storage indisponível: 503; erro interno: 500.
- Catálogo de códigos/status/mensagens é estático e exercitado por fixtures.
  Não retornar mensagens de parser, storage, filesystem, panic, documento ou token.
  Erros exclusivos do harness de query viram internal_error, não contrato público.
- Response JSON usa Content-Type application/json, Cache-Control no-store,
  Content-Length e X-Request-ID. 401 inclui WWW-Authenticate: Bearer;
  405 inclui Allow exato da rota; 429/503 incluem Retry-After: 1.
- Request ID é gerado no servidor: prefixo req_ e 16 bytes aleatórios em hex.
  Não aceitar/ecoar X-Request-ID externo como identificação confiável. O header e
  o envelope usam o mesmo ID. O modelo não recebe headers externos; o adapter
  futuro deve garantir a substituição e não registrar o valor recebido.
- Se a geração do request ID falhar, o adapter deve encerrar a requisição sem
  fabricar ID ou expor detalhes; a representação dessa falha de infraestrutura
  precisa de integração, fora da garantia do envelope de aplicação.
- HEAD de erro mantém os headers da representação JSON, incluindo Content-Length,
  mas não envia seu corpo. Falhas de parsing HTTP/TLS anteriores à aplicação
  podem encerrar conexão sem envelope; não há promessa impossível de request ID
  para toda falha de transporte.
- Retry-After informa espera, não segurança de repetir mutação. Erro de I/O,
  timeout ou desconexão pode deixar resultado de commit desconhecido. Nenhum
  retry automático de POST, idempotency key, transação distribuída ou exactly-once.
- Depois de iniciar SSE, não trocar status nem inserir envelope JSON no stream.
  Usar stream_end best-effort ou encerrar a conexão, conforme ADR 0005.

## Caminhos, corpos e limites

- Em PUT/PATCH, autenticar e validar rota, media type e corpo antes de consultar
  o recurso. Com entrada válida, documento ausente retorna 404. Estado persistido
  inconsistente retorna `internal_error`. A transformação e a decisão de no-op
  seguem o ADR 0010. GET/HEAD por ID usam o teto do formato, inclusive para
  documentos acima do limite vigente de escrita. DELETE continua disponível.
  O orçamento de respostas de consulta permanece sujeito ao ADR 0012.

- Coleções/IDs na rota: ASCII `[A-Za-z0-9_-]`, 1–128 bytes. Caminhos canônicos,
  sem percent encoding, segmentos vazios, ponto, traversal, barra invertida ou
  redirects para normalização. O matcher recebe raw path antes de decoding,
  separado da query string. Rota desconhecida: 404; método de rota conhecida: 405.
- Bodies JSON: create collection até 1 KiB; query até 16 KiB; insert/PUT/PATCH
  até 1 MiB. Tamanho final após gerar/injetar `_id` ou aplicar patch deve ser
  verificado de novo no caminho de escrita. GET/HEAD/DELETE não aceitam corpo.
- Adapter deve normalizar media type, validar UTF-8 e recusar Content-Encoding
  comprimido no primeiro recorte. O modelo recebe somente a essência normalizada
  do media type; não implementa parser de headers ou framing HTTP.
- Orçamento proposto: path 2 KiB, headers 8 KiB, resposta JSON 4 MiB incluindo
  envelope, e leitura de request em até 5 s. Até 32 requests em curso, com streams
  incluídos e até 8 assinaturas. Esses limites de rede ainda não estão implementados
  nem congelados como defaults; exigir admissão/buffers limitados no servidor.
- O 408 do catálogo é para request não recebido no prazo. Timeout de engine ou
  commit não prova rollback; não cancelar commit presumindo que nada foi persistido.
- Validar limites antes de acumular corpos; validar/serializar resposta limitada
  antes dos headers. O teto de saída do avaliador de query não inclui envelope
  HTTP, então o adapter deve conferir novamente o total final.

## Evidências e pendências

`spikes/http-contract` modela rotas, validação de corpos e respostas de erro; não
tem sockets, autenticação, persistência ou handlers de sucesso. Código/status de
erros do core e dos spikes têm mapeamento explícito. Fixtures fixam rotas e corpo
de erro; testes verificam HEAD, headers, identidade e erros sem conteúdo sensível.

Executar `cargo test -p kiarni-http-contract-spike --locked`. Resultados e limites
em `docs/spikes/http-contract.md`. A integração do produto exige autenticação,
handlers reais, leitura limitada, respostas de sucesso e testes black-box. Este
ADR não fecha M0 e não estabiliza automaticamente uma API pública.

## Incremento posterior

O [ADR 0010](./0010-substituicao-merge-patch-e-no-op.md) propõe transformação pura,
limite final e igualdade para no-op, com fixtures no spike de escrita. Não integra
handlers/transações nem demonstra os efeitos sobre commit e publicação.

O [ADR 0008](./0008-autenticacao-e-tokens.md) exercita lifecycle local de tokens e
um guard no modelo HTTP. Isso complementa a proposta; não implementa o servidor
nem elimina as pendências de integração descritas acima.
