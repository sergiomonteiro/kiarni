# M0-F3a — Contratos de substituição, Merge Patch e no-op

Atualização M1. A transformação passou a validar o documento atual pelo teto
de formato e a preservar no-op antes de verificar o limite final. O servidor
aplica PUT/PATCH dentro da transação e oferece DELETE.
O relato abaixo registra a evidência original do spike.
[API implementada](../M1_SERVE.md)

Data: 2026-11-10. Marco M0; RF-02, efeitos propostos em RF-05/06.
[ADR 0010](../adr/0010-substituicao-merge-patch-e-no-op.md) permanece **proposta**.

## Pergunta e resultado

O [modelo puro](../../crates/engine/src/writes.rs) prepara PUT/PATCH para documento
existente e distingue alteração de no-op. Não abre storage ou rede. Não altera
o core, o binário, o coordenador de feeds nem os handlers do modelo HTTP.

PUT substitui campos, injetando o ID da rota quando ausente. PATCH aceita objeto
sem `_id` na raiz, mescla objetos, remove membros com null e substitui arrays.
São regras do [RFC 7396](https://www.rfc-editor.org/rfc/rfc7396.html), com restrição
de raiz objeto e identidade reservada. Não se trata de JSON Patch.

A comparação ignora espaços, ordem de membros e escapes equivalentes de strings.
Arrays são ordenados; números mantêm seus tokens exatos, incluindo expoente e
zero negativo. `1` → `1.0` é update pela proposta, mesmo sendo numericamente igual
em consultas. Nenhum valor passa por float ou por `serde_json::Value`, preservando
também objetos comuns com chave `$serde_json::private::Number`.

No-op devolve os bytes originais. Resultado alterado é serializado e revalidado
contra limite final e profundidade 64; inputs também são validados. O candidato
não é certificado de commit. Erros e Debug não incluem payloads.

## Evidência reproduzível

```bash
cargo test -p kiarni-write-spike --locked --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
```

Fmt e clippy passaram; o workspace passou com 107 testes Rust, incluindo os sete
do novo crate. Os testes de rede executaram fora do sandbox. A suíte usa
[17 fixtures determinísticas](../../crates/engine/tests/fixtures.json), comparação
independente da árvore esperada e reaplicação idempotente. Cobre objetos/arrays,
null ausente/presente/aninhado, `_id` raiz/aninhado/escapado, strings Unicode,
números extensos/fora do domínio comparável de query, expoentes e zero negativo.
Outros testes cobrem bytes originais no no-op, saída numérica exata, Debug sem
payload, ID de rota inválido, estado anterior inválido, JSON/duplicatas inválidos,
tamanho final após merge/injeção e profundidade na fronteira 64/65.

Não há novas bibliotecas no lockfile: o crate reutiliza core, serde e serde_json,
cujas licenças constam no [inventário](../dependencies.md).

## Limites e próximo incremento

- A regra proposta é sucesso 200 sem commit de mutação, evento ou sequência em
  no-op. Isso exige integração posterior e testes do caminho real; não foi
  demonstrado pelo modelo puro. O spike anterior de feeds ainda emite update
  para substituições idênticas.
- Leitura, transformação e comparação devem usar a mesma transação/exclusão de
  escrita; a função pura não evita lost updates se chamada com estado antigo.
- Limites de bytes/profundidade não constituem orçamento global de RAM ou
  admissão. Árvores e cópias têm overhead; consolidar isso antes do servidor.
- Nenhuma alegação nova de persistência, recovery, crash ou desempenho.

M0-F3 continua parcial: faltam formato persistido/versionamento, catálogo e
consolidação de configuração/limites. Próximo incremento: propor formato inicial
e recusa segura de formato incompatível, sem promover redb a engine definitiva.
