# Revisão de fechamento do dashboard local

Data: 2026-11-14. Marco M3, RF-08 e instrumentação do RF-09.

**O recorte funcional do dashboard local está concluído e validado em Linux.**
O M3 e o gate de release continuam abertos. Não há publicação de release,
validação de macOS ou homologação de exposição remota implícitas nesta revisão.

## Cobertura

| Requisito | Comportamento e evidência |
| --- | --- |
| Mesmo executável | HTML/CSS/JS compilados no servidor; nenhum frontend ou serviço obrigatório separado. Testes HTTP de assets, CSP e Host/Origin. |
| Acesso administrativo | Bearer em memória; token fora de URL, armazenamento do navegador e logs. 401 limpa sessão. Shell público sem dados. Testes com servidor e Chromium. |
| Status e coleções | Health, versão, uptime, ocupação HTTP/streams, coleção paginada e contagem exata ou limite inferior dentro do orçamento. |
| Documentos e consultas | AST validada e limitada, JSON original com precisão numérica preservada, resultados como texto, sem edição de documentos. |
| Mudanças | SSE autenticado, manual, último evento limitado; não mistura eventos com snapshot. Testes de início/parada, falhas e limpeza. Sem replay/reconexão automática. |
| Métricas | Séries HTTP, consultas, storage e feeds; tempos de processamento/espera/execução, histogramas cumulativos, filas/overflow, tamanho lógico e resultados de commit. Parser validado, u64 preservado e ausência de tamanho sinalizada. |
| Erros recentes | Até oito conclusões com metadados seguros, request ID correlacionável, perdas por contenção explícitas, independente de logs. API real com logs off e reinício. |
| Consistência da interface | Estados inicial, carregando, vazio, sucesso, falha e 401; descarte de respostas obsoletas; limpeza ao atualizar visão geral, desconectar e sair. Falha de painel não preserva valores antigos como atuais. |
| Navegação e tela pequena | Labels, status acessíveis, foco visível e tabelas com scroll local; Chromium em viewport de 375 px sem overflow da página. Não equivale a auditoria completa de acessibilidade. |

RF-09 dispõe de logs estruturados e métricas para distinguir falhas HTTP,
validação/admissão/execução de queries, primitivas/commits e consumidor atrasado.
Tempos e códigos são agregados ou metadados, sem AST/payload. O painel e os logs
ajudam o diagnóstico, mas não fornecem profiling de CPU, planos completos de
queries, auditoria durável ou rastreamento distribuído. Validação prolongada com
coletor externo continua no fechamento operacional; ela não justifica ampliar o
dashboard antes das entregas restantes do MVP.

## Verificação local

Ambiente: Linux x86_64, Rust/Cargo 1.99.1, Chromium da versão fixada do Playwright
em `spikes/transport`. Testes TCP e navegador executados com permissão de loopback;
o sandbox comum recusa a abertura dessas portas.

| Comando | Resultado |
| --- | --- |
| `cargo fmt --all -- --check` | passou |
| `cargo clippy --workspace --all-targets -- -D warnings` | passou |
| `cargo clippy --workspace --all-targets --features kiarni-server/profiling -- -D warnings` | passou |
| `cargo test --workspace` | 262 passaram; dois workers ignorados executados pelos testes pais |
| `cargo test --workspace --features kiarni-server/profiling` | 262 passaram; mesmos dois workers |
| `cargo build -p kiarni --release --locked` | passou |
| `node scripts/dashboard-metrics.test.mjs` | 11 passaram |
| `node scripts/dashboard-feed.test.mjs` | oito passaram |
| `node scripts/dashboard-browser.test.mjs` | passou, servidor real e cenários de erro/cancelamento |
| `python3 scripts/check_product_dependencies.py` | nove pacotes do produto, sem dependência de spikes |
| `python3 -m unittest discover -s benches -p test_http_load.py` | três passaram |
| `node spikes/streaming/client.test.mjs` | passou |

A reserva conjunta de telemetria permanece abaixo de 4 KiB incluindo anel e
snapshot; teste cobre esse limite. Não houve benchmark novo de throughput ou
latência nesta entrega. A regressão de leitura anteriormente medida e custo de
telemetria continuam como investigação de performance, sem alegação de ausência
de regressões. CI remoto e operação prolongada não são substituídos por testes
locais. Não foram adicionadas dependências de runtime.

## Encaminhamento

Consolidar checkpoint no Git e issues verificáveis antes de novos recursos.
Depois, backup/restauração operacional conforme ADR 0013, operação remota segura
e distribuição. O [plano do MVP](./MVP_REMAINING.md) mantém essa ordem. Dashboard
volta a receber mudanças por defeito ou requisito concreto, sem novo ciclo de
expansão visual. Isso não congela API pública nem declara estabilidade de release.

## Checkpoint versionado

A implementação e as evidências desta revisão foram publicadas no commit
[5aea314](https://github.com/sergiomonteiro/kiarni/commit/5aea314). As pendências
seguintes estão nas [issues do plano do MVP](./MVP_REMAINING.md#issues-de-acompanhamento).
A CI remota deve ser observada separadamente dos resultados locais acima.

A primeira CI do checkpoint passou no Chromium e falhou num teste histórico de
recuperação de janela TCP do spike. O cenário foi tornado determinístico com
pausa limitada no fixture; o teste separado de consumidor lento real permanece.
[Diagnóstico e ajuste](./spikes/transport.md#ajuste-após-primeira-execução-da-ci--2026-11-14).
Os resultados remotos finais devem ser consultados no workflow do commit atual.

Na execução do ajuste de transporte, Linux e Chromium passaram; macOS identificou
uma comparação textual de caminhos temporários no spike de autenticação
(`/var` versus `/private/var`). O teste passou a comparar o diretório irmão
canônico, preservando as recusas de symlinks e permissões do vault. Não houve
mudança de código ou contrato de autenticação do produto.
