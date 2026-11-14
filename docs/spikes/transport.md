# Spike M0-F1 — transporte autenticado real em loopback

Data: 2026-11-10. Requisitos relacionados: RF-05/RF-06/RF-07.
Complementa os ADRs [0004](../adr/0004-ordenacao-e-overflow.md),
[0005](../adr/0005-streaming-e-autenticacao.md),
[0007](../adr/0007-http-e-erros.md) e [0008](../adr/0008-autenticacao-e-tokens.md).
Todos continuam propostas; este experimento não aceita automaticamente SSE ou redb.

## Reprodução

```bash
cargo test -p kiarni-transport-spike --locked -- --test-threads=1
cargo build -p kiarni-transport-spike --example fixture_server --locked
npm ci --prefix spikes/transport --ignore-scripts --no-audit --no-fund
spikes/transport/node_modules/.bin/playwright install chromium
node spikes/transport/browser.test.mjs
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

O browser precisa das bibliotecas de sistema compatíveis; no CI Linux a instalação
usa `playwright install --with-deps chromium`. Localmente foi reutilizado Chromium
já instalado. Playwright 1.57.0 está travado para reproduzir esse browser, não como
recomendação de versão atual ou dependência do produto. npm/Node/browser são apenas
ferramentas de teste; não integram o runtime do futuro servidor.

Os testes precisam abrir sockets locais e lançar processo de navegador. O sandbox
local bloqueou bind; as execuções com sockets/Chromium foram feitas fora dele,
com autorização, sem substituir falhas por skips. Nenhum serviço foi publicado.

## Implementação experimental

- `spikes/transport`: fixture Unix HTTP/1.1, obrigatoriamente 127.0.0.1 e porta
  efêmera. GET de SSE/consulta de um documento e assets mínimos de teste.
  Sem mutações HTTP, framework definitivo, TLS, proxy, dashboard ou SDK.
- `httparse` processa cabeçalhos. Perfil restrito: um request por conexão,
  Connection close, até 8 KiB/32 headers e 2 s para recebê-los; sem corpo,
  transfer encoding, pipelining ou negociação completa de media types.
- 8 workers fixos, fila de sockets aceita com capacidade 8 e até 4 streams, para
  reservar capacidade aos demais requests. Sob excesso, 429/503; sem spawn por
  request ou fila ilimitada. Backlog e memória do kernel não foram medidos.
- Auth pelo modelo existente antes de resolver rotas da API; Authorization
  duplicado é recusado. Host exato e Origin, quando presente, devem corresponder
  à fixture. Cookies não autenticam. Request IDs externos não são refletidos.
- Inscrição ocorre antes de ready, e eventos usam o encoder Rust existente.
  Frames não usam SSE id/retry; Last-Event-ID é recusado antes de criar assinatura.
- GET autenticado de documento passa pelo coordenador e lê redb. O método get
  não permite mutações fora da seção crítica existente.
- Filas usam capacidade 1 nos testes TCP e 2 na fixture de browser. O socket de
  envio solicita SO_SNDBUF de 16 KiB; o SO pode ajustar/dobrar o valor. Cada frame
  tem limite de 3 MiB e deadline absoluto de tentativa de escrita, sem reiniciá-lo
  a cada write parcial. Não é garantia de tempo real do scheduler.
- Para acelerar os cenários, heartbeat em 100 ms e revalidação de sessão em polls
  de 25 ms. Testes usam deadline de escrita de 500 ms. O teste de overflow
  usa uma pausa controlada de consumidor, limitada a três segundos, antes do I/O.
  Esses valores não substituem defaults do servidor do produto.
- O binário de exemplo recebe comandos limitados em stdin para inserir fixtures,
  consultar contadores, revogar e encerrar. Esse driver local não é endpoint de
  administração. Token é entregue em arquivo temporário privado, nunca em stdout,
  argumentos, URL ou HTML. stdout contém só endereço/caminho de entrega e metadados.

## Evidências observadas

### TCP real, seis testes

1. Dois sockets independentes recebem os mesmos eventos. GET de documento depois
   do evento vê o estado confirmado. Content-Type, no-store e sequência são verificados.
2. Auth ausente/duplicada, origem inválida e Last-Event-ID não criam assinatura.
   Quatro streams ocupam o teto e o seguinte recebe 429. 401 inclui challenge;
   request ID fornecido pelo cliente não aparece no header/envelope de resposta.
3. Consumidor com SO_RCVBUF solicitado de 4 KiB para de ler após ready. Frames de
   250 mil bytes de payload geram timeout de escrita e liberação da assinatura;
   seis commits e o consumidor rápido continuam, incluindo escrita posterior.
4. Outro cenário pausa o worker de stream após retirar um evento da fila e antes
   de escrevê-lo. Preenche a fila de uma posição, provoca overflow e libera o
   consumidor; recebe o evento em trânsito e depois stream_end/overflow. O hook
   local é one-shot, espera no máximo três segundos e não pausa o coordenador.
   Isso prova a entrega terminal quando o socket está disponível sem depender
   da recuperação da janela TCP de um kernel específico.
5. Revogação aplicada por reload fecha stream como interrupted, libera sua vaga e
   recusa nova conexão com 401, sem reverter dados confirmados.
6. Shutdown com stream ativo envia terminal quando possível e faz join de todos
   os workers; contadores de conexões/streams voltam a zero. Desconexão também
   libera vaga via heartbeat/erro do socket, sem esperar novo documento.

Os testes usam portas/arquivos próprios e não silenciam frames incompletos.
Overflow e timeout são distintos: o frame terminal é best-effort. Quando o socket
não aceita escrita no prazo, há fechamento, não garantia de entregar stream_end.

### Chromium real, um cenário de ponta a ponta

Executado com Node 22.23.4, Playwright 1.57.0 e Chromium 143.0.7499.4 em Linux.
Não usa Response/ReadableStream mocks no transporte nem rotas com respostas falsas.
O teste carrega o módulo fetch de referência servido pela fixture na mesma origem.

- Dois pages mantêm streams autenticados e recebem alteração após commit.
- Header Bearer é observado com request.allHeaders; o token não está na URL.
- Cookie de controle foi de fato enviado no request sem autenticação, que recebeu
  401. Nos requests autenticados de stream, credentials omit remove o cookie.
- Last-Event-ID recebe 400 antes de alocar assinatura. ready indica ausência de
  snapshot/replay. Unicode e tokens numéricos grandes permanecem no data textual.
- GET autenticado observa o documento confirmado sem arredondar sua representação.
- AbortController encerra um stream e a vaga é liberada; o outro continua.
  Reconexão explícita não recebe os documentos passados, somente próximo evento.
- Revogação encerra streams ativos; nenhuma reconexão automática é criada e a
  tentativa seguinte com o token antigo falha como unauthorized.

O teste mantém respostas reais; a interceptação Playwright somente deixa passar
requests da própria fixture e aborta recursos externos. Traces e screenshots
contendo credenciais não são gerados.

## Limites e consequência para o gate

O objetivo local M0-F1 foi demonstrado. Não houve HTTPS/proxy, browser distinto,
WebSocket real, teste de carga, crash durante escrita de stream ou CI remoto.
O teste não certifica desempenho ou segurança de um servidor de produção.

A fixture implementa só GET e a coleção de teste usa o índice sintético status
do spike de storage. Não promove esse requisito ao modelo JSON do produto.
Contadores são locais para asserções, não um endpoint de métricas público.

A revalidação ocorre antes do frame e durante espera; não recolhe bytes já
enviados ou cancela write em andamento instantaneamente. Shutdown de rede é
limitado nos testes, mas commit/I/O de storage continua síncrono e sem deadline
de cancelamento. Essa abordagem com threads/HTTP restrito não é a escolha do
framework/runtime M1; a comparação de primitivas async permanece para implementação.

Os snapshots de auth são carregados explicitamente na inicialização da fixture;
revogação do teste usa reload em memória. Bootstrap/ponteiro ativo/restart de auth
continuam G3. As decisões, formatos e limites do gate continuam pendentes de revisão.

## Ajuste após primeira execução da CI — 2026-11-14

O job Linux do checkpoint 5aea314 truncou o frame no antigo teste de recuperação
de overflow: aumentar SO_RCVBUF não garantiu escoamento dos 250 mil bytes antes
do deadline de dois segundos. O cenário foi substituído pela pausa controlada
acima, mantendo o teste separado de timeout com consumidor TCP lento real e
recusa de frames incompletos. A mudança é exclusiva do fixture/spike, sem
alterar prazos ou protocolo do produto. A matriz CI agora usa fail-fast false
para concluir verificações Linux e macOS independentemente.
