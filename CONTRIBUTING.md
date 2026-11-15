# Contribuir com Kiarni

Kiarni é um banco JSON reativo em Rust, single-node, sob [MIT](./LICENSE).
Comece pelo [índice de documentação](./docs/README.md), pelo
[PRD](./KIARNI_PRD_v0.1.md) e pela [arquitetura](./ARCHITECTURE.md).
O [plano do MVP](./docs/MVP_REMAINING.md) mostra entregas e pendências;
nenhuma contribuição deve inferir cluster, replay ou compatibilidade ReQL.

## Preparar o ambiente

Use Linux ou macOS conforme a matriz do CI. `rust-toolchain.toml` fixa Rust
1.99.1, rustfmt e clippy; `Cargo.lock` fixa as dependências. É necessário rustup
instalado; a primeira execução pode baixar o toolchain e dependências pela rede.
Python 3.11+ é usado nos scripts de verificação; Bash e curl nos exemplos testados.
Node 22 e Chromium/Playwright são necessários somente para testes de navegador.
Docker Engine Linux amd64/Compose e kind/kubectl são opcionais para testar deploy.
Nenhuma dessas ferramentas adicionais é serviço obrigatório do banco nativo.

Na raiz do checkout:

```bash
rustup show active-toolchain
cargo build -p kiarni --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 scripts/check_product_dependencies.py
python3 scripts/check_docs.py
python3 scripts/docs_smoke.py --binary target/debug/kiarni
python3 -m unittest discover -s scripts -p test_release.py
python3 -m unittest discover -s benches -p test_http_load.py
```

Os testes black-box precisam abrir sockets loopback e criar diretórios privados
temporários. Se o sandbox impedir, registre esse impedimento e use a autorização
apropriada do ambiente; não apresente falha de permissão como teste aprovado.
Os workers de crash fixture aparecem como ignored na execução direta e são
acionados pelos testes pais. Não remova esses workers por parecerem testes esquecidos.

Para o dashboard e transporte, instale as ferramentas de teste sem serviços:

```bash
npm ci --prefix spikes/transport --ignore-scripts --no-audit --no-fund
spikes/transport/node_modules/.bin/playwright install chromium
node scripts/dashboard-metrics.test.mjs
node scripts/dashboard-feed.test.mjs
node spikes/streaming/client.test.mjs
cargo build -p kiarni-transport-spike --example fixture_server --locked
node spikes/transport/browser.test.mjs
node scripts/dashboard-browser.test.mjs
```

Linux pode precisar das bibliotecas do Chromium; o CI usa install --with-deps.
O teste HTTPS usa Nginx/OpenSSL opcionais, sem serviço instalado; siga o job
[HTTPS do CI](./.github/workflows/ci.yml) e [perfil remoto](./docs/REMOTE_HTTPS.md).
Para pacotes/imagens, siga [distribuição](./docs/DISTRIBUTION.md) e
[contêiner](./docs/CONTAINERS.md); não publique registry/tag/release automaticamente.

## Organização do código

| Diretório | Responsabilidade |
| --- | --- |
| apps/kiarni | Entrypoint e CLI do binário. |
| crates/core | Documentos JSON, IDs e invariantes independentes de HTTP/storage. |
| crates/engine | AST, comparação exata e transformações de escrita. |
| crates/storage | Persistência, catálogo, índice primário e snapshots. |
| crates/auth, config, http | Credenciais locais, limites/configuração e contratos HTTP. |
| crates/feeds | Dispatcher pós-commit, filas limitadas e framing SSE. |
| crates/server | Composição, handlers, coordenação, observabilidade e dashboard embutido. |
| spikes/ | Experimentos e evidências; não são dependências do produto. |
| docs/adr, docs/spikes | Decisões e relatórios, preservando contexto histórico. |
| benches, scripts, deploy | Medições reproduzíveis, verificações e exemplos operacionais. |

Fixtures determinísticas ficam nos testes de core/engine/http e nos spikes.
Testes HTTP reais ficam em apps/kiarni/tests. Exemplos para usuários ficam em
[docs/examples](./docs/examples/); seus comandos marcados na referência API são
executados pelo smoke documental. O checker de links e inventários entra no CI.

## Fazer uma mudança revisável

Escolha uma issue pequena e identifique RF/marco, comportamento esperado e fora
de escopo. Leia ADRs aceitos relevantes e inspecione o código antes de alterar.
Spikes ajudam a testar hipóteses antes de decisões caras; integração ao produto
precisa de evidências e fronteiras claras, não cópia automática do experimento.

Preserve publicação pós-commit, precisão JSON, loopback default, limites de filas
/memória/concorrência e mensagens sem dados sensíveis. Não use código arbitrário
na query ou consumidores lentos para bloquear writers indefinidamente. Não
introduza serviços obrigatórios, dependências centrais ou abstrações hipotéticas.

Atualize testes e docs junto com o comportamento. Use ADR para decisões de
storage, IDs, comparação, AST, streaming, auth, backup ou formato persistido;
preserve decisões anteriores ao substituí-las. Mudança de escopo, quebra de API,
licença, dependência externa obrigatória ou risco de perda de dados requer decisão
explícita. Orientações específicas para agentes: [AGENTS.md](./AGENTS.md).

No PR, explique problema, resultado, requisitos afetados e verificações executadas.
Informe falhas/impedimentos e limitações reais; links de CI devem corresponder ao
commit revisado. Não declare pronta uma plataforma ou durabilidade só por compilar.
Não inclua banco real, backups privados, credenciais ou payloads sensíveis no PR.

## Medir desempenho e preservar licenças

Use [guia de performance](./docs/PERFORMANCE.md) e registre dataset, hardware,
sistema, versões, parâmetros, concorrência, throughput confirmado, latências e
rejeições. Não compare números de cenários/ambientes diferentes como prova de ganho.
CPU/memória e a investigação já registrada seguem seu planejamento próprio.

Antes de adicionar dependência, avalie necessidade, manutenção, tamanho e licenças
transitivas. Preserve [MIT](./LICENSE), [inventário](./docs/dependencies.md) e
avisos nos artifacts. O empacotamento falha em licença não revisada ou texto ausente.
Não marque o gate M3 ou publique uma release apenas porque um incremento passou.
