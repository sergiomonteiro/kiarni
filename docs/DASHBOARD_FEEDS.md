# Acompanhar mudanças no dashboard

Incremento M3/RF-08 usando o changefeed de coleção do RF-05, já integrado ao
produto. Selecione uma coleção e clique em **Acompanhar mudanças**. O status
fica conectado somente após `ready`. **Parar** cancela a conexão e limpa o
último evento. Não há reconexão automática.

O feed observa a coleção inteira: não aplica `filter`, `id`, `project` ou
`limit` do editor. Exibe o JSON original do último evento insert/update/delete,
com before/after e sequência decimal em string. O contador é de eventos
recebidos nesta conexão, não de documentos, commits ou entregas confirmadas.
Não há histórico: cada evento substitui o anterior, como texto sem interpretar
HTML ou converter os números dos documentos. A consulta exibida acima permanece
uma leitura pontual; não é atualizada nem reconciliada pelo feed.

A assinatura recebe somente mudanças futuras. `ready` não fornece snapshot e
a consulta anterior não é atômica com a inscrição. Mudanças podem ocorrer entre
as duas operações. Não há replay, exactly-once ou retomada por sequência.
Interrupção, EOF, overflow ou encerramento exigem nova consulta para reconciliar
estado; uma nova assinatura continua apenas dali em diante. O último evento
pode permanecer visível junto do aviso de interrupção; iniciar outra conexão
limpa esse evento e reinicia o contador.

O cliente usa fetch com Bearer no header, mesma origem, sem cookies, cache,
redirects ou token na URL. Resposta 401 limpa toda a sessão local. Outros erros
recusam/interrompem o acompanhamento com aviso, sem retry automático.
Trocar coleção, fechar o painel, atualizar a visão geral, desconectar ou sair
cancela o stream, limpa seus dados e impede atualização por uma conexão antiga.

## Limites e recursos

- Uma assinatura por página, iniciada explicitamente. Cada aba usa sua própria
  assinatura e está sujeita à admissão do servidor. Com somente uma conexão
  admitida, pare o stream antes de executar outras leituras no mesmo servidor.
- Até dez segundos para obter `ready`; depois aguarda eventos/heartbeats sem
  timeout de inatividade próprio. Não há garantia de detectar imediatamente
  uma falha de rede silenciosa; EOF e erros do fetch são interrupções.
- Parser do dialeto SSE do produto, com LF/CRLF e UTF-8 estrito. Frame até 3 MiB
  e no máximo 8192 linhas físicas, incluindo delimitador. O limite de linhas é
  uma restrição adicional do dashboard: um evento aceito pelo servidor pode ser
  recusado por esse cliente se exceder esse limite. `id`/`retry` são recusados.
- Buffer fixo de frame e apenas um evento no DOM, sem lista/histórico crescente.
  Esses limites de payload não equivalem a um teto de RSS do navegador: strings,
  decodificação e DOM têm custos próprios. Não houve benchmark do frontend.
- Cancelar no navegador libera o leitor; a liberação da vaga no servidor depende
  da observação do fechamento TCP. Escritores permanecem sujeitos aos limites
  e à política de overflow existentes, sem alteração neste incremento.

## Implementação e verificação

`crates/server/dashboard/feed.mjs` é o cliente de produto, incorporado como
`/dashboard/feed.js`, importado pelo módulo principal. Não importa código de
spikes e não adiciona dependências em runtime. Autenticação e CSP seguem o
[ADR 0020](./adr/0020-dashboard-embutido.md); protocolo e garantias seguem o
[ADR 0005](./adr/0005-streaming-e-autenticacao.md).

```bash
node scripts/dashboard-feed.test.mjs
cargo build -p kiarni --locked
node scripts/dashboard-browser.test.mjs
```

Os oito testes do parser cobrem fragmentação UTF-8, dados multilinha, precisão,
heartbeat, limites, framing, estados inválidos, autenticação e cancelamento.
O Chromium usa o servidor real para eventos ordenados, no-op, isolamento por
coleção, preservação numérica, texto inerte, assinatura sem snapshot/replay,
parada, troca de coleção, recarga e desconexão com liberação de capacidade.
Overflow, EOF e 401 são injetados no navegador para testar seus avisos/limpeza;
o overflow real do servidor continua coberto pelos testes Rust.

Validação local em 2026-11-13: 235 testes Rust passaram (dois workers de fixture
ignorados diretamente e executados pelos testes pais), oito testes do parser
e integração Chromium real passaram. Também passaram fmt, clippy, fronteira
de nove pacotes, build release, sintaxe JS, links locais e `git diff --check`.
CI remoto, proxy/TLS, navegador em outra plataforma e benchmark de UI não foram
executados nesta entrega.
