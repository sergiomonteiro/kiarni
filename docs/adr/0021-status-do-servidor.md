# ADR 0021 — Status autenticado do serviço HTTP

Status: aceito para alpha local. Data: 2026-11-13. Marco: M3/RF-08, RF-09 parcial.

## Decisão

Adicionar GET/HEAD `/v1/status`, sem query ou corpo, com Bearer, Host/Origin,
request ID, no-store e erros existentes. Não alterar `/v1/health/live` ou
`/v1/health/ready`: status é informação operacional, não substitui readiness.

Resposta JSON:

```json
{
  "version": "0.1.0",
  "uptime_seconds": "12",
  "connections": {"admitted": 2, "streaming": 1, "limit": 8, "stream_limit": 4}
}
```

`version` vem da versão Cargo do crate servidor, compartilhada com o binário
pelo workspace. `uptime_seconds` é string decimal de segundos inteiros medidos
com Instant monotônico desde a entrada em `http::serve`; exclui bootstrap e
reinicia ao reiniciar o serviço. Não é hora de início, tempo de CPU ou uptime do
sistema operacional.

`connections` usa uma leitura do contador atômico já existente da admissão:
`admitted` são vagas gerais ocupadas e `streaming` o subconjunto promovido a
stream. Inclui esta requisição de status e requisições com headers incompletos;
exclui conexões pendentes no backlog do kernel. Uma vaga compartilhada com o
writer pode permanecer ocupada após o fechamento do socket até a operação
terminar. Portanto, não são clientes únicos, sockets TCP necessariamente vivos
ou assinaturas elegíveis do dispatcher. `limit` e `stream_limit` são os tetos
configurados, não contadores históricos. O par ocupado vem da mesma palavra
atômica; sua observação pode mudar logo após a leitura.

Não adicionar contadores independentes, varredura de workers, acesso ao storage
ou mensagens ao writer. O status respeita a admissão normal; sob saturação pode
não ser atendido imediatamente. A leitura não altera a política de admissão.

Dashboard mostra versão, segundos e ocupação dos limites, em coleta manual.
Atualizar status não fecha um stream nem exige atualizar coleções. Sem polling,
histórico ou credencial persistida. Erro limpa os valores anteriores; 401 limpa
a sessão local. Cancelamento impede uma resposta antiga de repovoar a página.

## Escopo

Adição compatível de API, sem dependências ou mudanças de formato persistido.
Não conclui RF-09, não mede CPU/memória e não amplia o perfil de acesso remoto.
As métricas Prometheus de feeds permanecem com as séries atuais. Contagem de
assinaturas do dispatcher continua distinta de ocupação de stream na admissão.

## Evidências

[Verificação local e testes do contrato](../SERVER_STATUS.md): 237 testes Rust
e integração Chromium real passaram, incluindo promoção/retenção da admissão,
conexão incompleta, stream, liberação e reinício. Sem validação de CI remoto ou
outras plataformas nesta entrega.
