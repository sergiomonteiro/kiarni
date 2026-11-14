# Spike M0 — contrato HTTP e erros

O [ADR 0007](../adr/0007-http-e-erros.md) permanece em proposta. O experimento
`spikes/http-contract` não abre sockets, não altera dados e não implementa handlers
de sucesso. O incremento de [auth](./auth.md) adiciona um guard modelado que
autentica antes de resolver a rota, sem integração com servidor HTTP.

O incremento [M0-F1](./transport.md) usa o guard, os erros e GET de documento em
uma fixture TCP/Chromium. Isso não implementa os handlers completos deste contrato.

## Reprodução

```bash
cargo test -p kiarni-http-contract-spike --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Execução local: Linux x86_64, Rust/Cargo 1.99.1. Não há nova biblioteca externa:
o modelo usa getrandom/serde_json e os contratos/spikes locais já existentes.
Os testes estão incluídos no workspace e no CI Rust. CI remoto/macOS ainda não
foi executado nesta entrega.

## Evidências

- Fixtures de rotas distinguem operação reconhecida, 404, 405 com Allow e caminho
  inválido. HEAD é permitido nos GETs JSON propostos, mas não cria assinatura SSE.
- Matcher recebe raw path sem query string e rejeita percent encoding, traversal,
  barras repetidas, nomes inválidos e identificadores acima de 128 bytes.
- Target tem campos privados, construídos pelo matcher; o adapter consulta seus
  acessores e não monta um target arbitrário que contorne validação.
- Fixtures de corpos distinguem POST, PUT e PATCH, Content-Type normalizado,
  sintaxe/duplicatas JSON, ID divergente/imutável e coleção da query divergente.
- Orçamentos de bytes são inclusivos e verificados antes do media type: 1 KiB
  para coleção, 16 KiB para query e 1 MiB para documentos/patch. Corpos de GET,
  HEAD e DELETE são rejeitados quando não vazios.
- Catálogo independente de códigos/status é comparado com todos os códigos do
  modelo. Uma fixture completa fixa o envelope de duplicate_id; mensagens vêm
  somente do catálogo estático, nunca de strings de erros de dependências.
- Mapeamentos explícitos cobrem todos os erros atuais do core, query, storage e
  coordenador. Guardas de dataset do harness viram internal_error, não um contrato
  de scan/limite do servidor.
- Request IDs têm prefixo req_ e 32 caracteres hexadecimais gerados localmente.
  Há teste determinístico do formato, fonte real do SO e falha injetada da fonte
  sem fallback. Não existe construtor a partir de headers recebidos.
- Header X-Request-ID e envelope usam o mesmo ID. São exercitados Content-Type,
  Cache-Control, Content-Length, WWW-Authenticate, Retry-After e Allow.
- HEAD mantém status/headers da representação e não envia corpo. Um 405 sem
  contexto de Allow é recusado pelo construtor de resposta.
- Debug de Target/Response omite caminho, IDs de documento e corpo. O teste de
  divergência de ID verifica que os valores sensíveis da entrada não são ecoados.

## Limites e pendências

As respostas de sucesso e a paginação de coleções são documentadas, não handlers
implementados. O código não faz leitura limitada de sockets, normalização de
Content-Type, parsing de query string/headers, autenticação, validação de origem,
Accept, Last-Event-ID, probing de readiness, timers ou admissão de conexões.
Essas verificações ainda exigem implementação e testes black-box.

O validador de PATCH aceita somente o envelope objeto e proíbe `_id` na raiz;
não executa merge recursivo, preservação da identidade, atomicidade ou eventos.
PUT sem `_id` é aceito para futura injeção do ID da rota, que ainda não ocorre.
Limites do documento final e da resposta com envelope precisam ser revalidados
depois dessas transformações pelo futuro handler.

O guard modelado autentica antes do matcher. O adapter real ainda deve preservar
essa ordem, sobrescrever
request IDs externos e evitar registrar headers/payloads sensíveis. Ter um tipo
seguro de request ID não demonstra sozinho esses controles na camada HTTP.

O storage experimental usa um índice sintético de status e agrupa alguns erros
de validação/geração em InvalidDocument. O mapeamento deste modelo não recupera
informações já perdidas por esse spike: os handlers reais devem validar nos
componentes apropriados e preservar contexto tipado, sem herdar a exigência
experimental de status string como regra de documento do produto.

Falhas de commit ou perda da resposta não autorizam inferir rollback. Não há
autorretry de mutação, idempotency key ou garantia de exactly-once. Depois de
iniciar SSE, status/envelope JSON não são usados para falhas do stream.

Próximo incremento: modelo completo de autenticação, provisionamento e rotação
de tokens, depois revisar evidências/pendências e ADRs antes de declarar o gate M0.
