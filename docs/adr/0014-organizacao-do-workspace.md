# ADR 0014 — Organização do workspace no M1

- Status: aceita.
- Data: 2026-11-11.
- Escopo: promover componentes usados pelo produto e preservar os harnesses M0.

## Decisão

O binário kiarni fica em apps/kiarni, com CLI, leitura das fontes de configuração
e composição do servidor. Os testes HTTP com subprocessos ficam nesse pacote.
Os contratos de biblioteca e suas fixtures acompanham as bibliotecas promovidas.

| Crate | Origem principal | Responsabilidade |
|---|---|---|
| kiarni-core | crates/core | Documentos e inserções preparadas |
| kiarni-engine | spikes/query e spikes/writes | Consultas e transformações puras |
| kiarni-storage | spikes/format | Layout persistido e transações |
| kiarni-auth | spikes/auth | Credenciais e ativação |
| kiarni-config | spikes/config | Resolução de fontes e admissão |
| kiarni-http | spikes/http-contract | Contratos HTTP |
| kiarni-server | apps/kiarni/src/serve | Ciclo de vida e handlers |

As dependências normais e de build do produto seguem de apps/crates para
apps/crates ou bibliotecas externas. O CI verifica o grafo de dependências
locais, incluindo arestas transitivas e declarações condicionais de plataforma.
[Checagem](../../scripts/check_product_dependencies.py)

Os pacotes legados de query, writes, format, auth, config e http-contract
permanecem como entradas de compatibilidade dos experimentos. Eles reexportam
as bibliotecas promovidas. Os helpers de kill/restart ficam junto dos harnesses.
As conversões de erros do storage sintético e do feed experimental para HTTP
ficam nos pacotes que definem esses erros.

As pastas usam nomes curtos, como crates/core e crates/server. Os nomes de
pacote mantêm o prefixo kiarni-, como kiarni-core e kiarni-server. Cargo
resolve o nome de pacote pelo manifest e a localização pelo campo path.

## Consequências

A implementação tem uma fonte por componente, usada pelo produto e pelos
experimentos. A reorganização preserva versões de dependências externas,
layout persistido, nomes de tabelas, fases de bootstrap e contratos HTTP.
O índice sintético de status e a fixture SSE permanecem experimentais.

O código de rede e coordenação agora pertence a kiarni-server. A separação
permite testar o binário como um consumidor dessa biblioteca. Funcionalidades
do M2, como dispatcher e streaming do produto, entram em incrementos próprios.

## Verificação local

Em Linux, os 151 testes Rust passaram após a movimentação, incluindo HTTP,
crash/restart e concorrência. Os 12 testes do cliente JavaScript e o cenário
Chromium com dois streams autenticados também passaram. Fmt, clippy e a
checagem de dependências dos oito pacotes do produto passaram. Os links
locais da documentação foram conferidos. CI remoto e macOS aguardam execução.
