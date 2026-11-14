# Consultar documentos no dashboard

Recorte M3/RF-08 sobre a API de consultas já existente (RF-03). Não altera AST,
comparação, persistência ou limites do servidor definidos no
[ADR 0006](./adr/0006-ast-e-comparacao-json.md).

Selecione uma coleção: a interface executa uma consulta inicial com `limit: 25`,
ordenada por `_id` conforme o contrato existente. Edite a consulta JSON e use
**Executar consulta**. `from` deve manter a coleção selecionada e `limit` é
obrigatório, inteiro de 1 a 1000. `id` faz lookup primário; `filter`, `sort` e
`project` seguem a AST da API. Exemplo:

```json
{
  "from": "orders",
  "filter": {"op": "eq", "field": "status", "value": "open"},
  "sort": [{"field": "total", "direction": "desc"}],
  "project": ["status", "total"],
  "limit": 20
}
```

O resultado mostra o envelope JSON original: `documents` e `count` (quantidade
retornada, não o total da coleção). O [botão de contagem](./DOCUMENT_COUNTS.md)
consulta a coleção inteira, independentemente da AST. Não há paginação de documentos nem promessa
de coleção inteira quando o resultado atinge o limite. Para restringir a
leitura, use `id` ou filtros; um limite pequeno ainda pode exigir scan e falhar
nos limites de 10.000 documentos/8 MiB de entrada do servidor.

A interface confere sintaxe, coleção, limite e tamanho UTF-8 de até 16 KiB;
a validação completa continua no servidor, inclusive rejeição de chaves
repetidas, operadores inválidos e campos desconhecidos. O corpo original é
transmitido sem reserialização. A resposta é exibida como texto, sem parse de
números ou interpretação de HTML. Isso preserva inteiros acima de 2^53 e
decimais precisos. Não há execução de código fornecido pelo usuário.

A resposta tem o limite de até 4 MiB do servidor (ou menor, conforme config).
Cada execução tem espera de dez segundos no navegador. Cancelar a espera não
promete cancelar uma consulta já iniciada no servidor. Há um resultado atual
por painel, sem histórico, polling ou armazenamento persistente. Selecionar
outra coleção, fechar o painel, atualizar a visão geral, desconectar ou sair da
página aborta a leitura anterior e limpa seus dados. Respostas antigas não
substituem a seleção atual.

Erro de consulta limpa o resultado anterior; erros do servidor apresentam
código/request ID. A interface
permite corrigir a AST. Resposta 401 encerra a sessão local e pede nova
credencial. O dashboard continua somente leitura; o POST de query não é uma
mutação. Não há edição de documentos. O [painel de mudanças](./DASHBOARD_FEEDS.md) é
independente da consulta e observa a coleção inteira.

## Verificação

O teste Chromium de `scripts/dashboard-browser.test.mjs` usa o servidor real e
fixtures determinísticas: coleção vazia, 30 documentos, limite inicial, lookup,
filtro numérico exato, sort/projeção, JSON inválido, limites e coleção divergente,
AST recusada pelo servidor, texto com HTML e cancelamento de resposta antiga.
Os comandos permanecem no [guia do dashboard](./DASHBOARD.md).

Validação local em 2026-11-13: 235 testes Rust passaram (dois workers de fixture
ignorados diretamente e executados pelos testes pais), fmt, clippy, fronteira
de nove pacotes, build release, sintaxe JS, links locais e teste Chromium real
passaram. O caso de tamanho usa texto multibyte para conferir o orçamento em
bytes, além do maxlength em caracteres do campo. CI remoto não foi executado.
