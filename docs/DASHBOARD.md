# Dashboard local — incrementos M3

Inicie o servidor conforme o [guia da alpha](./ALPHA_LOCAL.md) e abra
`http://127.0.0.1:8711/` (ou o endereço exato informado pelo processo).
Cole no formulário a credencial do arquivo privado gerado no bootstrap;
não a coloque na URL. Use a mesma origem numérica do endereço informado,
pois Host/Origin são validados estritamente.

A interface mostra liveness, readiness e nomes de coleções, até 100 por página.
**Atualizar status** informa versão, uptime do serviço HTTP e ocupação dos
limites de conexões. [Semântica dos valores](./SERVER_STATUS.md).
**Atualizar** volta à primeira página e refaz as leituras; **Próxima página**
continua por nome. A paginação não é snapshot entre requisições. Selecione uma
coleção para consultar documentos: são até 25 inicialmente, com editor de AST
e limite obrigatório. [Uso das consultas e limites](./DASHBOARD_QUERIES.md).
**Contar documentos** lê a coleção inteira dentro de um orçamento; mostra total
exato ou pelo menos N. [Contrato da contagem](./DOCUMENT_COUNTS.md).
Selecione **Acompanhar mudanças** para observar a coleção inteira, com início e
parada manuais. [Uso do feed e garantias](./DASHBOARD_FEEDS.md). Não há edição
de documentos. **Atualizar métricas** coleta os indicadores agregados HTTP, de consultas, storage e feeds
sem parar o acompanhamento. [Uso e semântica](./DASHBOARD_METRICS.md).
**Atualizar erros** mostra até oito conclusões com erro, correlacionadas por
request ID e sem payloads. [Retenção e limites](./RECENT_ERRORS.md).
[Revisão de fechamento do dashboard](./DASHBOARD_REVIEW.md).

O HTML de entrada e seus assets estáticos são públicos e não contêm dados do banco.
Todas as leituras administrativas continuam autenticadas. A credencial fica
somente em memória, é removida do campo após conectar e precisa ser informada
novamente após recarregar. Desconectar limpa esta página; não revoga o token.
Falha ao atualizar a visão geral limpa os dados e solicita nova conexão. Erros
de consulta permitem corrigir a AST; 401 encerra a sessão local.
Não use HTTP público para transmitir credenciais.

## Verificar

Além de fmt/clippy/test do workspace:

```bash
cargo build -p kiarni --locked
npm ci --prefix spikes/transport --ignore-scripts --no-audit --no-fund
spikes/transport/node_modules/.bin/playwright install chromium
node scripts/dashboard-metrics.test.mjs
node scripts/dashboard-feed.test.mjs
node scripts/dashboard-browser.test.mjs
```

Playwright é ferramenta de teste, não dependência do binário. O teste cria um
servidor real em porta loopback temporária e dados privados temporários, e
encerra ambos ao terminar. A biblioteca usa a versão já fixada no spike de
transporte. Não imprime credenciais nem captura screenshots com segredos.

[Decisão e limites](./adr/0020-dashboard-embutido.md).
