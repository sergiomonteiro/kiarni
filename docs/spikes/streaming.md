# Spike M0 — SSE versus WebSocket

O [ADR 0005](../adr/0005-streaming-e-autenticacao.md) recomenda SSE via fetch
como proposta. A comparação de WebSocket/EventSource é documental, apoiada nos
padrões; o código experimenta somente framing SSE e cliente com streams simulados.
Não há escolha aceita nem servidor de produto implementado. O incremento
[M0-F1](./transport.md) posteriormente exercitou TCP, autenticação e Chromium reais;
este relatório descreve a comparação e o primeiro experimento de framing/mocks.

## Reprodução

```bash
cargo test -p kiarni-feed-spike --locked
node spikes/streaming/client.test.mjs
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Ambiente local: Linux x86_64, Rust/Cargo 1.99.1, Node 22.23.4. O cliente usa
somente APIs Fetch/ReadableStream/TextDecoder e os testes usam `node:test`.
Não há instalação npm. Node é necessário somente para esse experimento e CI,
não para executar o futuro binário. O arquivo de testes pode ser executado
diretamente para exibir individualmente os casos de `node:test`.

## Artefatos e evidências

- `spikes/feeds/src/sse.rs`: encoder com limite de 3 MiB, estados JSON brutos,
  eventos nomeados, sequência como string e sem campos `id`/`retry`.
- Fixtures de ready/change/overflow compartilhadas entre Rust e JavaScript.
  O teste Rust compara byte a byte a saída; o teste JavaScript consome as mesmas
  fixtures fragmentadas em chunks de um byte.
- O encoder trata CR/LF de whitespace JSON como linhas data e escapa metadados;
  strings não podem injetar novos campos de framing. Números grandes não são
  convertidos a f64 na serialização.
- A referência retorna `{event, data}`, com data como texto JSON. Parsing/schema
  de documentos, números lossless, ready antes de change e códigos terminais
  devem ser validados pela futura camada de protocolo/SDK.
- Os testes verificam headers Bearer e opções de fetch com mocks. Isso demonstra
  o pedido construído pelo cliente, não valida implementação de CORS/autenticação.
- UTF-8 fragmentado, LF/CRLF, multiline, comentários, cancelamento, limites de
  bytes, resposta 401 e tipo de conteúdo incorreto são exercitados.
- EOF inesperado interrompe, não reconecta e não entrega frame incompleto.
  Um evento terminal cancela o reader e impede consumir mudanças posteriores.

## Pendências antes de um contrato aceito

1. Servidor HTTP real que autentica e limita antes de iniciar stream, com request
   IDs, erros estáveis, origem/host permitidos e rejeição de Last-Event-ID.
2. Browser real na mesma origem para comprovar Authorization, cancelamento,
   credenciais omitidas e política de CORS/redirect.
3. Duas conexões independentes, commits reais, overflow e transporte lento, com
   deadline de escrita, heartbeat e buffers limitados. A fila do coordenador não
   limita automaticamente buffers HTTP/TCP/proxy.
4. Reverse proxy com HTTPS e buffering desativado; medir flush, desconexões e
   limites de conexão. Não há comparação de performance entre transportes.
5. Modelo completo de token, provisionamento e rotação/revogação, além de proteção
   do dashboard, métricas e endpoints administrativos.
6. Revisar a proposta e contratos de consulta/API ainda pendentes para o gate M0.
