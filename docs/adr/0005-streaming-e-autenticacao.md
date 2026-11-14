# 0005 — SSE via fetch e autenticação do stream

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md))
- Data: 2026-11-09
- Requisitos e marco: M0; RF-05, RF-06, RF-07 e preparação para RF-08.

## Contexto

O cliente altera documentos pela API HTTP e recebe mudanças de coleção. O MVP
não exige mensagens bidirecionais na mesma conexão nem multiplexação de consultas.
O ADR 0004 fornece ordenação e overflow em memória; falta definir como transportar
isso sem prometer replay ou colocar credenciais em URLs.

## Opções consideradas

| Aspecto | SSE com EventSource | SSE via fetch + ReadableStream | WebSocket nativo do browser |
|---|---|---|---|
| Fluxo | Servidor → cliente | Servidor → cliente | Bidirecional |
| Bearer no header pelo construtor/API | Não oferece headers arbitrários | Permite Authorization | Construtor não oferece headers arbitrários |
| Autenticação alternativa no browser | Cookies/sessão ou outro fluxo | Mesmo Bearer da API HTTP | Cookies, ticket temporário ou autenticação por mensagem, com limites prévios |
| Reconexão | Automática, com semântica EventSource | Controlada explicitamente pelo cliente | Precisa ser implementada pelo cliente |
| Framing | Padronizado e decodificado pelo browser | Mesmo framing, parser explícito | Frames, close e ping/pong próprios |
| Comandos de assinatura/multiplexação | Novas requisições | Novas requisições | Podem compartilhar conexão, exigindo protocolo adicional |
| Overflow | Evento terminal e fechamento | Evento terminal e fechamento | Mensagem/close e fechamento |

As características das APIs são documentadas nos padrões
[EventSource](https://html.spec.whatwg.org/multipage/server-sent-events.html#the-eventsource-interface),
[WebSocket](https://websockets.spec.whatwg.org/#the-websocket-interface) e
[Fetch](https://fetch.spec.whatwg.org/). O protocolo WebSocket é definido no
[RFC 6455](https://www.rfc-editor.org/rfc/rfc6455.html).
Usar SSE não torna autenticação ou backpressure automáticos. O fluxo atual não
justifica adicionar comandos bidirecionais apenas para autenticar uma conexão.

## Decisão proposta

Recomendar **SSE consumido via fetch** para a primeira assinatura de coleção.
EventSource e WebSocket foram as alternativas avaliadas.
A evidência inclui framing/cliente e o [spike real M0-F1](../spikes/transport.md)
em TCP/Chromium. Não há servidor do produto ou benchmark comparativo de transportes.

### Autenticação e exposição

- Bearer no header Authorization, validado antes de alocar assinatura e enviar
  headers de sucesso. Exigir autenticação no stream inclusive em loopback; assim
  um reverse proxy local não cria bypass de autenticação remota.
- Não transportar token em query, fragmento de URL ou subprotocolo WebSocket.
  Não registrar Authorization, cookies, token ou payloads em logs.
- Dashboard na mesma origem do servidor. Referência usa `mode: same-origin`,
  `credentials: omit` e `redirect: error`. Não habilitar CORS no primeiro recorte.
- Token do dashboard somente em memória durante a sessão; sem URL, localStorage
  ou embedding no HTML. Isso não elimina riscos de XSS; CSP e segurança da UI
  ainda precisam de implementação/testes.
- Bind permanece 127.0.0.1. Acesso remoto requer HTTPS por proxy configurado e
  autenticação, sem recomendar HTTP público. HTTP loopback é apenas desenvolvimento.
- Geração, provisionamento, armazenamento, comparação segura, rotação/revogação e
  expiração de tokens ficam para ADR de segurança completo antes do acesso remoto.
  Não há auth server implementado neste incremento. Sessões de cookie ficam fora
  deste experimento, evitando introduzir agora um contrato de sessão/CSRF.

O header e os cuidados de transporte seguem a orientação de Bearer do
[RFC 6750](https://www.rfc-editor.org/rfc/rfc6750.html#section-2.1), sem adotar um
servidor OAuth ou alegar conformidade OAuth completa.

### Contrato de stream proposto

- Rota ilustrativa: `GET /v1/collections/{collection}/changes`;
  Accept `text/event-stream`. Implementado somente pela fixture M0-F1, sem
  integração ao binário do produto.
- Antes de iniciar: autenticar; validar coleção, origem/host permitidos e limites;
  recusar Last-Event-ID com 400/`resume_not_supported`, nunca ignorá-lo silenciosamente.
  Credencial ausente/inválida retorna 401 com `WWW-Authenticate: Bearer`.
  Limite de assinaturas retorna 429. Envelope de erro/request ID exige contrato de API.
- Resposta de sucesso: 200, `Content-Type: text/event-stream; charset=utf-8`,
  `Cache-Control: no-store`, sem compressão/buffering no primeiro experimento.
  O proxy precisa permitir flush de stream; validar sua configuração em integração.
- Inscrever antes de enviar `ready`, depois drenar a fila. `ready` indica assinatura
  registrada; não significa snapshot consistente dos documentos existentes.
- Eventos nomeados: `ready`, `change`, `stream_end`. Comentários de heartbeat não
  são eventos de mudança. Estados before/after seguem o spike de feeds.
- `sequence` é string decimal no JSON do evento, evitando arredondamento de u64
  em clientes JavaScript. A API numérica do PRD é ilustrativa; não havia contrato
  público congelado. O valor continua local e reinicia com a instância.
- Não emitir campos SSE `id` ou `retry`. Sequência dentro de data não é cursor.
  Não conectar automaticamente após EOF, overflow, erro ou shutdown. Uma nova
  assinatura recebe só eventos futuros; estado local derivado pode estar incompleto.
- `stream_end` usa códigos `overflow`, `shutdown`, `interrupted`. O consumidor
  encerra a leitura e descarta o handle; frame terminal é best-effort. Se não chegar,
  EOF/erro de transporte é interrupção, sem deduzir fechamento limpo.
- Limite de frame codificado: 3 MiB, além do teto de 1 MiB por estado de documento.
  Expansão por linhas de whitespace pode exceder esse limite; encerrar stream como
  interrompido sem desfazer o commit. Não truncar JSON nem bloquear o writer.
- Reutilizar até 8 handles de assinatura e filas de até 8 eventos do ADR 0004.
  Um worker de transporte por assinatura, sem criar buffers adicionais ilimitados.
  Como parâmetros a validar: heartbeat a cada 15 s e deadline de escrita de 5 s.
  Esses timers não estão implementados e não são defaults públicos congelados.

## Consequências

Preserva HTTP para mutações e usa um stream unidirecional autenticado. Exige parser
no cliente e política explícita para reconexão; haverá conexões por assinatura,
com limites de browser/HTTP e proxies a medir. WebSocket pode ser reconsiderado se
multiplexação/bidirecionalidade justificar o protocolo adicional.

`spikes/streaming/client.mjs` retorna o JSON de data como texto, pois JSON.parse
convencional pode arredondar números de documentos. O contrato numérico de query
e a estratégia de um SDK lossless ainda estão pendentes. A referência aceita LF
e CRLF, não implementa toda a gramática SSE nem substitui um EventSource genérico.

## Evidências e limites

- Encoder Rust em `spikes/feeds/src/sse.rs`, fixtures compartilhadas em
  `spikes/streaming/fixtures` e testes JavaScript sem pacotes npm.
- Testes cobrem fragmentação byte a byte, Unicode, multiline, limites, erro de
  UTF-8, interrupção, cancelamento, headers e política de URL/reconexão.
- M0-F1 exercitou sockets, autenticação e Chromium reais, incluindo prazo de escrita,
  overflow e revogação. Seu perfil HTTP GET restrito/threads não escolhe o framework
  de produção. Timers e capacidade reduzida servem aos testes, não são defaults.
- Não houve TLS, proxy, EventSource nativo ou handshake WebSocket. As restrições
  dessas APIs foram comparadas pelos padrões; isso não é comparação de performance.
- Detalhes e comandos: [relatório do spike](../spikes/streaming.md).
- A evidência local com clientes independentes/browser/consumidor lento está no
  [relatório M0-F1](../spikes/transport.md). A integração do produto fica no M2. Proxy/TLS e segurança devem ser
  verificados antes da exposição remota.

## Integração no produto — 2026-11-13

O endpoint foi integrado ao HTTP com inscrição por barreira no writer antes de
ready, Bearer, Host/Origin, recusa de Last-Event-ID e revalidação da sessão.
Defaults/tetos de streams, fila e timers seguem o ADR 0012. Não há thread
adicional por evento ou buffers ilimitados. Cada conexão usa seu worker já
admitido e sua vaga promovida, compartilhada com o recibo da assinatura.

O encoder preserva tokens numéricos, prefixa cada linha de whitespace e verifica
3 MiB codificados antes do primeiro byte do change. Em vez de copiar frame inteiro,
usa slices do evento mais buffer de 8 KiB. Evento em processamento (até 2 MiB)
e buffer cabem na reserva de 3 MiB, além da reserva separada da fila. Isso não
é um teto de RSS. Deadline por frame e heartbeat usam a configuração existente.

Erro em frame parcial encerra por EOF, sem anexar terminal corrompido.
Overflow/shutdown/interrupted são best-effort; uma tentativa terminal dura até
25 ms. Expiração também encerra como interrupted. Rotação/revogação permanecem
offline e não são reload de autoridade durante execução.

Sete testes TCP do produto verificam dois streams, oito writers concorrentes,
ordem/transições, no-op, validações, reconexão sem replay, slots/heartbeat,
consumidor que não lê, frame excessivo sem rollback, expiração e shutdown.
Dois testes do encoder verificam framing/precisão e expansão excessiva.
Passaram 229 testes locais em Linux. Browser, TLS/proxy e métricas completas
continuam pendentes. [Alpha local](../ALPHA_LOCAL.md).
