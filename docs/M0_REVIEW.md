# Revisão do gate M0

- Data: 2026-11-10.
- Resultado final: **gate M0 aprovado para entrada no M1**. [Deliberação M0-F4](./M0_DELIBERACAO.md)
- Base: [PRD v0.1, §10](../KIARNI_PRD_v0.1.md), [arquitetura](../ARCHITECTURE.md)
  e [ADRs](./adr/README.md).
- Este documento preserva o levantamento de evidências anterior à deliberação.

O PRD exige decisões documentadas e riscos bloqueadores resolvidos. Os spikes
oferecem evidência útil, mas não equivalem ao produto funcional ou à aceitação
das decisões. Não é necessário construir todo M1–M3 para fechar M0; é necessário
resolver as incertezas que impedem escolher os contratos e iniciar M1.

## Evidências verificadas

| Frente de M0 | Evidência existente | Limite da conclusão |
|---|---|---|
| Documentos/IDs, RF-02 | [Core e testes](../crates/core/), ADRs [0001](./adr/0001-entrada-de-documentos.md)/[0002](./adr/0002-geracao-de-ids.md): JSON, duplicatas, limites, geração e erros | Contratos experimentais; nenhum CRUD público |
| Storage, RF-01/02/04 | [Relatório](./spikes/storage.md): transações com documento/índice, rollback, conflitos, lock, leitores/writers e kill/reabertura | redb permanece candidato; índice status é sintético e não suporta todo o modelo JSON |
| Alternativa de persistência | Comparação SQLite passou em conflito, rollback, snapshot WAL, scan/índice, backup e reopen | Comparação funcional, não benchmark entre engines ou validação de crash SQLite |
| Commit → evento, RF-05 | [Relatório](./spikes/feeds.md): contraexemplo da publicação ingênua, serialização, pausas e consumidores independentes | Caminho em memória; lacuna pós-commit injetada sem kill nesse intervalo |
| Backpressure | Referência com filas limitadas; [M0-F1](./spikes/transport.md) demonstra timeout TCP e overflow com recuperação | Buffers e prazos do experimento não são orçamento global de produção; proxy não testado |
| Streaming, RF-05/06 | [Comparação](./spikes/streaming.md) e [transporte real](./spikes/transport.md): dois sockets e dois streams Chromium autenticados | Fixture local GET, sem produto, proxy, HTTPS ou handshake WS |
| Query, RF-03 | [Relatório](./spikes/query.md): AST, 12 documentos/19 consultas, ordem determinística e 5.776 pares contra oráculo racional | Referência limitada a uma coleção fornecida; sem scan do storage ou planner de produção |
| API/erros, RF-06 | [Modelo](./spikes/http-contract.md) e erros/GET de documento exercitados na fixture TCP | Sem handlers do produto, merge PATCH, paginação de coleções ou readiness real |
| Escrita, RF-02/05/06 | [M0-F3a](./spikes/writes.md): PUT, Merge Patch e decisão no-op; sete testes com 17 fixtures | Modelo puro; sem commit/evento, integração concorrente ou handlers |
| Formato/catálogo, RF-01/02/04 | [M0-F3b](./spikes/format.md): manifesto antes do engine, versão lógica, catálogo/IDs e kill após commits | Formato candidato; sem migração, crash dentro de commit ou hardening de diretórios |
| Configuração/admissão, RF-01/02/03/05/06 | [M0-F3c](./spikes/config.md): precedência, defaults, relações, reserva de buffers e vagas atômicas com RAII | Modelo sem bootstrap/servidor; reserva não é teto de RSS nem prova de admissão antes de alocar |
| Auth, RF-01/07 | [Modelo local](./spikes/auth.md), rede em [M0-F1](./spikes/transport.md) e [ativação persistida M0-F2](./spikes/auth-activation.md) | CLI é fixture com relógio fixo; sem watcher ou integração de produção |
| Backup, confiabilidade | Snapshot lógico único restaura ambas as tabelas apesar de commits posteriores | Ainda não é procedimento operacional com publicação atômica/recuperação de backup parcial |
| Baseline | Dataset, hardware, versões, comando e latência/crescimento registrados no [spike de storage](./spikes/storage.md) | Amostra de 256 documentos/uma execução; sem meta de TPS ou inferência de capacidade de produção |

O baseline e a comparação funcional atendem à existência de evidência inicial,
com os limites descritos. Não exigir uma competição completa de engines nem uma
meta arbitrária de performance para encerrar M0. A deliberação seleciona redb para M1 e registra as verificações restantes.

## Verificações desta revisão

Executadas sobre o workspace local em Linux, com dependências já disponíveis:

| Comando | Resultado |
|---|---|
| `cargo fmt --all -- --check` | passou |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | passou |
| `cargo test --workspace --locked --offline` | 129 testes passaram após M0-F3c, incluindo doze de configuração/admissão, dez de formato/catálogo, sete de escrita, seis TCP e dez de integração de ativação |
| `node spikes/streaming/client.test.mjs` | 12 testes passaram, com streams simulados |
| `node spikes/transport/browser.test.mjs` | um cenário Chromium real passou, com dois streams |
| `python3 spikes/storage/sqlite_comparison.py` | comparação funcional passou, SQLite 3.45.1 |

O benchmark não foi repetido nesta revisão; os números no relatório são da
execução histórica de 2026-11-09. Rede loopback e Chromium foram verificados em
M0-F1, com execução autorizada fora do sandbox. CI remoto, macOS e HTTPS/proxy não
foram verificados. A configuração de CI não é evidência de execução remota.

Em M0-F3c foram repetidos fmt, clippy e os 129 testes Rust (loopback fora do
sandbox). Node, Chromium e SQLite não foram repetidos nesse incremento; os
resultados correspondentes acima pertencem às verificações anteriores.

HEAD local: `674ac71` (Initial commit). Há código/documentação não rastreados e
README alterado. Os resultados acima são do workspace com essas mudanças, não
de um checkout limpo desse commit. Antes de vincular evidência a uma revisão/CI,
é necessário versionar o conjunto pretendido; não houve stage, commit ou push.

## Encerramento dos bloqueadores

| ID | Disposição |
|---|---|
| G1 | Escolhas e verificações futuras registradas na deliberação M0-F4. |
| G2 | Transporte local demonstrado no M0-F1. Integração de produção fica no M2. |
| G3 | Ativação persistida demonstrada no M0-F2. Startup integrado fica no M1. |
| G4 | Contratos de escrita, formato e configuração exercitados em M0-F3. As revisões de contrato serão verificadas na integração. |

## Plano de fechamento em incrementos

São tarefas locais propostas, não issues remotas criadas.

### M0-F1 — Transporte real mínimo — executado localmente

Relaciona-se a G2 e aos ADRs 0004/0005/0008. O [relatório](./spikes/transport.md)
registra implementação, seis testes TCP, cenário Chromium e limites observados.
O incremento seguinte M0-F2 também foi executado; o foco passa a ser M0-F3.

- Bind explícito em 127.0.0.1 com porta efêmera; dados/credenciais temporários.
- Dois clientes independentes recebem eventos do storage real após ready/commit.
- Sem credencial válida, recusar antes de criar assinatura; erros estáveis e
  Last-Event-ID recusado antes dos headers de stream.
- Demonstrar fluxo fetch autenticado em pelo menos um browser real, sem token
  em URL/cookies e sem reconexão automática após interrupção.
- Consumidor TCP lento não bloqueia writer ou cliente rápido; filas, conexões,
  buffers e prazo de escrita limitados, inclusive após desconexão/revogação.
- Encerrar limpo sem vazar worker/handle; registrar exatamente quais condições
  foram simuladas e quais foram observadas em sockets.

Não implementar dashboard, SDK, todos os endpoints ou integrar o spike ao binário
de produto. Não escolher framework/transporte definitivo sem registrar o resultado.

### M0-F2 — Bootstrap e configuração ativa de auth — executado localmente

Relaciona-se a G3 e ao ADR 0008. Definir fluxo compatível com inicialização mínima,
layout de entrega fora dos dados/backups, seleção de snapshot e comportamento de
reload. Testar falhas nos pontos de publicação/restart e garantir que revogação
reportada como concluída é usada no próximo startup normal. Não prometer defesa
contra administrador malicioso ou rollback físico do host.

Evidência: [relatório M0-F2](./spikes/auth-activation.md) e
[ADR 0009](./adr/0009-bootstrap-e-ativacao-auth.md). A fixture manteve revogação
concluída após novo processo e recuperou decisões explícitas em 27 cenários
de kill/restart, sem varrer snapshots por maior epoch. Próximo incremento: M0-F3.

### M0-F3 — Fechar contratos de escrita, formato e limites

Executado em três incrementos. [M0-F3a](./spikes/writes.md) propõe substituição, merge e
no-op com testes de transformação/limite final. Não demonstra efeitos em commits
ou feeds. [M0-F3b](./spikes/format.md) propõe formato/versionamento e catálogo com
dez testes, incluindo manifesto recusado antes do engine e kill após commits.
[M0-F3c](./spikes/config.md) consolida defaults, precedência CLI/env/arquivo e
admissão limitada com doze testes. A evidência de contratos de G4 está disponível;
a decisão está registrada no M0-F4.

Relaciona-se a G4. Definir no-op/PATCH e fixtures com null, arrays, `_id`, números
brutos e limite final. Registrar formato inicial de metadados/documentos/coleções,
versão e recusa segura de formato incompatível antes de gravar no produto.
Consolidar defaults, admissão e precedência de configuração sem confundir tetos
dos harnesses com limites do servidor. Não criar migrações ou abstrações para
cluster, plugins ou requisitos hipotéticos.

### M0-F4 — Deliberação e registro do gate

O [registro da deliberação](./M0_DELIBERACAO.md) aprova a entrada no M1 e
lista as verificações por ADR. A evidência disponível é do workspace local.

## Pendências posteriores que não exigem construir o MVP em M0

| Marco/condição | Entrega a preservar |
|---|---|
| M1 | Servidor real, coleções/CRUD, consultas sobre storage, índice primário, health efetivo, configuração/shutdown e testes multi-cliente/recovery do produto |
| M2 | Dispatcher/streaming de produção, métricas, desconexões e backpressure integrados; filtragem reativa só com entrada/saída corretas |
| M3 | Dashboard, distribuição, Docker, quickstart, backup operacional/restauração e avisos de terceiros |
| Antes de qualquer exposição remota | Autenticação, HTTPS/proxy, controles de origem/Host, dashboard/endpoints sensíveis e testes de logs; não adiar segurança só por estar listada em M3 |
| Antes do primeiro SDK | Congelar wire contracts necessários e tratar números JSON sem arredondamento involuntário |

Snapshot inicial consistente e consultas reativas filtradas continuam sem solução
implementada. A primeira proposta é somente assinatura de coleção sem snapshot;
isso respeita o recorte permitido pelo PRD e não obriga implementar query reativa
completa para iniciar M1. Índice secundário também não é requisito de entrada de
M1: o roadmap pede índice primário, e o índice sintético não deve ser promovido.

Depois do gate, o primeiro incremento M1 recomendado é serve local com
configuração/credencial e storage inicializados, lock, health e shutdown
verificados. CRUD, consultas, feeds e dashboard devem vir em issues separadas.
