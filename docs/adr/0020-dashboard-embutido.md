# ADR 0020 — Primeiro recorte do dashboard embutido

Status: aceito para alpha local. Data: 2026-11-13.

## Contexto

O M3/RF-08 exige interface no mesmo binário. O ADR 0005 já determina mesma
origem, Bearer em header e token somente em memória. Precisamos permitir que o
navegador apresente o formulário antes de possuir uma credencial, sem expor
estado administrativo.

## Decisão

Incorporar HTML, CSS e JavaScript fixos em `crates/server/dashboard` por
`include_bytes!`, servidos em `/`, `/dashboard/style.css` e `/dashboard/app.js`.
Somente esses três recursos sem dados são públicos. Health, coleções, documentos,
streams e métricas continuam exigindo a mesma autenticação da API. A página
pública não interpola configurações, credenciais ou dados do storage.

GET/HEAD são permitidos, sem query ou corpo. Host e Origin continuam estritos.
Respostas usam no-store, nosniff, no-referrer e CSP sem scripts/estilos inline,
sem framing, submissão de formulário ou conexão com outra origem. Não há CORS,
cookies de sessão ou nova forma de autenticação. O formulário é tratado por JS.

A credencial fica em variável de fechamento e é retirada do input ao conectar.
Não há URL, localStorage, sessionStorage ou cookie para persistir sessão.
Desconectar, falha de consulta e pagehide limpam o estado local e cancelam fetch.
Isso não revoga a credencial no servidor nem elimina o risco de extensões/XSS
no navegador. Recarregar exige autenticar novamente.

O primeiro recorte lê liveness, readiness e até 100 nomes de coleção por página,
sequencialmente para funcionar com admissão de uma requisição. Há atualização
manual e deadline de dez segundos por atualização. Não há polling automático,
contagens de documentos, edição, consultas ou streaming neste incremento.

## Consequências e validação

Sem dependências novas de produto nem build frontend/Node em runtime. Os assets
são pequenos e constantes, dentro da reserva mínima de resposta existente;
não há cache de dados ou fila adicional no servidor. O token é enviado somente
para rotas relativas da mesma origem, com redirects recusados.

Teste black-box Rust verifica assets, GET/HEAD, métodos, corpo, query, Host,
Origin e preservação da autenticação da API. O teste Chromium usa o binário
real, 101 coleções e a biblioteca Playwright já fixada como ferramenta de
desenvolvimento no spike de transporte. Cobre credencial inválida, navegação,
recarga, limpeza de sessão, viewport móvel e falha de rede. O CI passa a executar
esse teste, mas a execução remota não foi observada nesta entrega.

Não conclui RF-08/M3: faltam documentos, consultas limitadas, atividade de feeds,
versão/uptime, métricas e demais informações previstas no PRD. Acesso remoto,
proxy/TLS e hardening de release permanecem fora deste recorte local.

Validação local desta entrega: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`,
`cargo test --workspace --locked --offline` (235 passaram, dois workers de
fixture ignorados diretamente), `cargo build -p kiarni --release --locked --offline`,
fronteira de nove pacotes e teste Chromium real passaram. Sintaxe JavaScript,
links locais e `git diff --check` também passaram.

Durante a suíte, o teste existente de expiração SSE revelou que exigia um
terminal garantido. Foi alinhado ao contrato já documentado: exige encerramento,
nenhuma mudança e recusa da credencial expirada; aceita EOF e, quando presente,
verifica o motivo `interrupted` do terminal. Não houve alteração do streaming.

## Evolução incremental

O [segundo recorte M3](../DASHBOARD_QUERIES.md) adiciona seleção de coleção e
editor da AST existente, com consulta inicial limitada a 25 documentos. Corpo
e resultado permanecem como JSON original para preservar precisão numérica.
Não há alteração de autenticação, contrato da API ou dependências de produto.

O [terceiro recorte M3](../DASHBOARD_FEEDS.md) adiciona acompanhamento manual de
coleção via fetch/SSE, sem filtrar pelo editor ou atualizar seu resultado.
Um novo asset estático `/dashboard/feed.js` contém o parser de produto; o
script principal passa a módulo ES. Mantêm-se CSP, autenticação e ausência de
dependências de runtime. Cliente limita frame a 3 MiB/8192 linhas e mantém
somente o último evento, com interrupção explícita e sem reconexão automática.

O [quarto recorte M3](../DASHBOARD_METRICS.md) apresenta as métricas de feeds
do ADR 0019 com coleta manual e tabela fixa. O módulo `/dashboard/metrics.js`
é incorporado ao binário; leitor limitado a 64 KiB/dez segundos, sem histórico,
taxas ou percentis derivados. Valores originais preservam precisão u64. Erros
limpam a tabela; 401 limpa a sessão; cancelamento invalida respostas antigas.
Não altera o endpoint, suas séries, autenticação ou dependências de runtime.
