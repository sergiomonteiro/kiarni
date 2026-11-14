# M0-F3c — Configuração e admissão

Data: 2026-11-10. RF-01/02/03/05/06, ainda M0.
[ADR 0012](../adr/0012-configuracao-e-admissao.md): **proposta**.

## Resultado

O [modelo isolado](../../spikes/config/) resolve fontes fornecidas pelo chamador:
pares CLI normalizados, pares de ambiente UTF-8 e bytes de JSON opcional.
Precedência por campo: CLI > KIARNI_* > arquivo > defaults. Não acessa disco,
ambiente real, argv, relógio ou sockets; não altera `kiarni` nem os outros spikes.

Todas as fontes são validadas, inclusive valores sobrescritos. Desconhecidos,
duplicatas, tipos errados e tamanhos excessivos são erros estáticos, sem valores
ou caminhos. Relações entre campos são verificadas depois da composição final.
Segredos/tokens não são configurações aceitas. Debug omite conteúdo. Limites de
fonte são verificados antes de copiar chaves/valores CLI/env relevantes.

Defaults: bind 127.0.0.1:8711, dados ./kiarni-data, documento 1 MiB, consulta
16 KiB, resultado 4 MiB, 32 vagas gerais, oito streams, quatro eventos por fila,
read/write timeout 5 s e heartbeat 15 s. O ADR define intervalos, tetos fixos,
escopo e interações. O perfil local recusa bind público; não implementa modo remoto.

Exemplo de arquivo **proposto**, ainda sem comando de servidor que o carregue:

```json
{"data_dir":"./data","max_requests":16,"max_streams":4,"feed_queue":2}
```

Equivalentes nas outras fontes: `KIARNI_MAX_REQUESTS=16` ou par normalizado
`--max-requests`, `16`. CLI/env usam inteiros decimais sem unidades; arquivo usa
números inteiros JSON. Caminhos são literais, sem expansão de shell/tilde.

Plano aritmético de buffers cobra requests, streams, filas e cache candidato;
default 353 MiB, combinações acima de 512 MiB recusadas. **Não é teto de RSS**:
exclui overhead, árvores JSON, kernel, stacks e outros custos. Não mede memória
nem altera limites ativos de storage/HTTP/feed. Frame proposto limitado a 3 MiB,
com checagem de tamanho real; whitespace SSE requer atenção no encoder futuro.

Admissão compartilha contadores atômicos num Arc. Guard não clonável ocupa uma
vaga geral e opcionalmente uma de stream; Drop libera ambas. Conexão admitida
pode ser promovida após auth/rota sem segunda vaga geral. Promoção recusada mantém
o guard original. No máximo 32 tentativas CAS, sem fila, espera, spawn ou alocação
na recusa. Limite geral/contenção = Busy, limite de stream = StreamLimit. O servidor
futuro deverá mapear para 503/429 quando ainda puder responder HTTP.

## Verificação reproduzível

```bash
cargo test -p kiarni-config-spike --locked --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
```

Doze testes do novo crate passaram, incluindo:

- Seis fixtures de precedência/fallback por campo e defaults explícitos.
- Valor inválido sobrescrito, desconhecidos, duplicatas inclusive JSON escapado,
  sintaxe/tipos/intervalos numéricos, overflow e tamanho de fontes.
- Relações finais (streams ≤ vagas, resultado ≥ documento + 1024), orçamento
  excedido e aritmética protegida contra overflow.
- IPv4/IPv6 loopback, recusa de wildcard/remoto/hostname/porta zero; caminhos
  vazios/com controles, ausência de expansão, Debug/erros sem conteúdo.
- Stream ocupando ambos os limites, liberação por Drop, clones compartilhando
  o mesmo orçamento e oito threads concorrentes retendo no máximo duas vagas.
- Promoção no limite geral, idempotência, recusa sem perder vaga e liberação.

Fmt, clippy e os 129 testes Rust do workspace passaram, com testes TCP fora do
sandbox. Node/Chromium/SQLite não foram repetidos neste incremento.
Não há novas bibliotecas; core/serde_json já constavam no
lockfile e no [inventário de licenças](../dependencies.md).

## Limites e próximo incremento

Este modelo não prova admissão antes de alocar no servidor, timeout em sockets,
backpressure real, orçamento de RAM ou segurança do processo. É necessário criar
**uma** instância compartilhada de Admission por servidor; criar uma por conexão
anularia o limite global. Manter guard durante conexões ociosas/keep-alive e não
duplicar admissão ao promover para stream. O modelo não faz auth, HTTP nem reload.

Bootstrap futuro precisa tratar argv/env não UTF-8, ler arquivo com limite antes
de acumular, aplicar caminhos/permissões e resolver colisão de porta. Esses erros
não foram testados aqui. Configuração é imutável por instância; nenhuma integração
ao storage, encoder ou handlers. O orçamento não habilita valores acima dos tetos
atuais dos contratos de JSON/AST.

Com F3a/F3b/F3c, as propostas de contrato de G4 têm evidência local; isso não aceita
os ADRs nem entrega M1. Próximo incremento: M0-F4, preparar deliberação dos ADRs,
com decisões necessárias, riscos residuais e critérios de integração posteriores.
