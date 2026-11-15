# Status do serviço HTTP

Recorte M3/RF-08, com observabilidade parcial do RF-09. GET `/v1/status` exige
Bearer e Host/Origin exatos; não aceita query ou corpo. HEAD retorna os mesmos
headers, sem corpo. Usa JSON, no-store, request ID e erros estáveis da API.
Não substitui `/v1/health/ready`, que verifica o storage.

```json
{
  "version": "0.1.0",
  "uptime_seconds": "12",
  "connections": {
    "admitted": 2,
    "streaming": 1,
    "limit": 8,
    "stream_limit": 4
  }
}
```

Os limites acima são ilustrativos; o servidor informa a configuração efetiva.
`version` corresponde à versão Cargo compartilhada pelo workspace e pelo
binário. `uptime_seconds` é string decimal para preservar precisão: segundos
inteiros monotônicos desde a entrada no serviço HTTP, depois do bootstrap.
Não é tempo de CPU, uptime do host ou relógio civil. Reiniciar o serviço zera
essa contagem.

`admitted` conta vagas HTTP gerais ocupadas; `streaming` é seu subconjunto
promovido a stream. A própria leitura de status ocupa uma vaga e entra na
contagem, assim como conexões aceitas com headers ainda incompletos. Backlog do
kernel não entra. Uma operação que retém uma reserva compartilhada pode manter
uma vaga ocupada depois de seu socket fechar. Não são clientes únicos nem um
inventário de sockets vivos. Slots de stream também não são a mesma coisa que
assinaturas elegíveis para receber eventos no dispatcher; consulte as
[métricas dos feeds](./CHANGEFEED_METRICS.md) para esse diagnóstico.

O par de ocupação vem de uma única leitura do contador atômico da admissão.
Os valores podem mudar imediatamente depois. `limit` e `stream_limit` são
limites de configuração, não totais históricos. O status não lê storage,
consulta writer, varre conexões ou adiciona contadores ao caminho de requisição.
Está sujeito à admissão normal: saturação pode impedir uma resposta imediata.

## Dashboard

Após autenticar, use **Atualizar status** no painel Serviço HTTP. A coleta não
fecha a consulta ou o stream. Versão, segundos e ocupação/limites são exibidos
como valores pontuais. Não há polling, relógio incrementado localmente ou
histórico. O botão Atualizar da visão geral limpa esses valores; uma nova
coleta é explícita. Com apenas uma vaga geral configurada, pare o stream antes
de consultar status ou outros endpoints.

Cada coleta tem espera de dez segundos. Erro limpa os valores anteriores;
erros HTTP mostram código/request ID, quando disponíveis. 401 limpa a sessão
local. Atualizar a visão geral, desconectar ou sair cancela a leitura e impede
que uma resposta antiga repovoe a página. Credencial permanece somente em
memória, com os mesmos controles de origem e segurança do dashboard.

## Verificação e limites

Testes de contrato cobrem GET/HEAD/405. A admissão tem teste de promoção,
retenção de Permit compartilhado e liberação. A integração black-box valida
acesso negado, Origin/Host, corpo/query, HEAD, versão, conexão incompleta, stream,
liberação das vagas e reinício do uptime. Chromium consulta o binário real com
stream ativo; respostas injetadas cobrem formato inválido, 503/request ID,
401, texto inerte, precisão e cancelamento de resposta tardia.

Não adiciona dependências de runtime nem métricas de CPU/memória. A [contagem limitada de documentos](./DOCUMENT_COUNTS.md) já está disponível.
[Logs de requisições](./REQUEST_LOGS.md) já estão disponíveis.
Métricas de [HTTP](./HTTP_METRICS.md) e [consultas](./QUERY_METRICS.md) já estão
na API. [Storage](./STORAGE_METRICS.md) também expõe primitivas/commits e
tamanho lógico. A [revisão do dashboard](./DASHBOARD_REVIEW.md) inclui
apresentação de storage e erros recentes. M3 permanece aberto.
[Decisão do contrato](./adr/0021-status-do-servidor.md).

Validação local em 2026-11-13: 237 testes Rust passaram (dois workers de fixture
ignorados diretamente e executados pelos testes pais), incluindo os dois novos
testes de status/admissão. Passaram também fmt, clippy, integração Chromium real,
fronteira de nove pacotes, build release, sintaxe JS, links locais e
`git diff --check`. CI remoto e outras plataformas não foram executados.
