# M0-F3b — Formato inicial e catálogo

Data: 2026-11-10. RF-01/02/04, ainda M0.
[ADR 0011](../adr/0011-formato-inicial-e-catalogo.md): **proposta**.

## Resultado

O [spike isolado original](https://github.com/sergiomonteiro/kiarni/tree/f355eef/spikes/format) exercita layout candidato com manifesto
`format.json`, banco `data.redb` e tabelas metadata, collections e documents.
Nenhuma mudança no servidor/binário, storage antigo, feeds ou handlers. redb 4.5.0
continua candidato, fixado apenas neste experimento. Não há migração de arquivos
anteriores ou promessa de compatibilidade física.

O manifesto identifica magic, versão lógica 1 e engine exato, em JSON estrito de
até 1 KiB. É validado antes da abertura de redb: formato incompatível ou estado
parcial não dispara inicialização, reparo ou migração automática. Criação exige
diretório novo e publica o manifesto depois do schema confirmado/fechado e dos
syncs. Arquivos novos são 0600 e diretório 0700 em Unix. Erro pode deixar diretório
parcial; reabrir não o reinicializa. Não há operação de limpeza automática.

Catálogo atribui ID u64 monotônico a cada criação, sem reutilização ou wraparound.
Documentos usam chave composta `(collection_id, _id)` e JSON bruto validado, sem
exigir status string. Nome duplicado e ID duplicado não sobrescrevem. Excluir
coleção não vazia falha; a verificação e exclusão usam a mesma transação. Handles
antigos falham após exclusão/recriação do nome. São identidades internas ao banco,
não IDs globais nem credenciais; não transferir handles entre bancos distintos.

## Verificação reproduzível

```bash
cargo test -p kiarni-storage --test format --locked --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
```

Dez testes do novo crate passaram:

- Catálogo/documentos genéricos e números extensos preservados após reabertura;
  mesmo `_id` permitido em coleções distintas, conflitos sem sobrescrita.
- Exclusão de coleção não vazia, identidade nova após recriação e rejeição de
  leitura/escrita/exclusão usando handle antigo.
- Manifestos de versões anterior/futura, magic/engine incompatível, duplicatas,
  campos desconhecidos, truncamento, raiz inválida e mais de 1 KiB. Comparação
  byte a byte demonstra banco inalterado nos casos recusados antes do engine.
- Diretório ausente/vazio e arquivos ausentes não geram bootstrap; arquivo legado
  permanece intacto. Estados parciais são construídos, não crashes de inicialização.
- Nomes/JSON inválidos, lock de writer, modos Unix e symlinks finais recusados.
- Processo filho confirma coleção e documento, anuncia COMMITTED e espera. Pai
  mata o processo, verifica recusa de manifesto futuro sem modificar o arquivo
  deixado pelo kill e então restaura manifesto conhecido para reabrir os dados.
  Esse teste cobre kill **depois** dos commits retornarem, não dentro de fsync.
- Corrida de duas threads entre excluir coleção vazia e inserir: somente inserção
  confirmada com exclusão recusada, ou exclusão confirmada com inserção recusada.
- Contador esgotado sem catálogo parcial; conflito não consome contador. Schema
  faltante e versão interna inconsistente são recusados, sem recriar tabelas.

Fmt, clippy e a suíte completa de 117 testes Rust passaram (testes TCP fora do
sandbox). Node/Chromium/SQLite não foram repetidos neste incremento.
Não foram adicionadas bibliotecas: dependências já estavam
resolvidas, com licenças no [inventário](../dependencies.md).

## Limites

Manifesto conhecido autoriza abrir o engine para recovery. A recusa sem alteração
de bytes foi demonstrada para manifesto ausente/inválido/incompatível; não para
banco adulterado ou schema inconsistente sob manifesto conhecido. Abrir/fechar
redb nesses casos pode escrever metadados de recuperação. Manifesto não autentica
o conteúdo nem protege contra dono do diretório malicioso. Não há varredura de
todos os documentos/catálogos no startup para detectar corrupção lógica completa.

Checagem de symlink final não elimina corrida de troca de caminho, não ancora
diretórios em descritores e não valida proprietário/hardlinks/permissões existentes.
O harness pressupõe diretório local confiável; hardening de produção continua
pendente. Não executar I/O síncrono deste spike diretamente em tarefa async.

Não são demonstrados queda de energia, disco cheio, falha física, crash durante
publicação do manifesto ou commit, backup operacional, migração ou desempenho.
Inserção/leitura são fixtures com ID explícito, sem CRUD completo, listagem HTTP,
geração de IDs, PUT/PATCH, consultas ou eventos. O formato permanece sujeito a
revisão antes de escrever dados do produto.

Próximo incremento: M0-F3c, consolidar defaults, limites/admissão e precedência
CLI/variáveis de ambiente, sem implementar o servidor M1 nem aceitar ADRs.

O wrapper de compatibilidade do spike foi removido na consolidação do Store.
Os testes de contrato e o worker de crash estão agora em
[crates/storage/tests/format.rs](../../crates/storage/tests/format.rs), adaptados
ao formato atual. Resultados e formato descritos acima são evidência histórica.
