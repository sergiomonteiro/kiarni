# Backup offline consistente

M3, operação/RF-01, issue [#1](https://github.com/sergiomonteiro/kiarni/issues/1).
Implementado conforme [ADR 0013](./adr/0013-backup-e-restauracao.md) e
[ADR 0028](./adr/0028-artefato-de-backup-offline.md).

## Executar

Encerre o servidor e aguarde sua saída: Ctrl+C no terminal ou SIGTERM
(`kill -TERM PID`, substituindo PID pelo número do processo) em segundo plano. Prepare um diretório de backups sob seu
usuário, sem escrita por grupo/outros. O pai deve existir; o destino deve ser novo.
Na raiz do checkout, usando o binário de release:

```bash
cargo build -p kiarni --release --locked
mkdir -m 700 ./backups
./target/release/kiarni backup --data-dir ./data --output ./backups/primeiro
```

O diretório `backups/primeiro` é publicado somente quando o artefato está completo.
Sucesso informa apenas quantidades, nunca credenciais ou payloads. O comando não
abre rede, não emite token, não altera a seleção de auth e não inicia o servidor.
O lock exclusivo da origem permanece até fechar o engine e concluir a operação.
Servidor ativo ou outro comando offline com o lock provoca `data_directory_locked`.
A origem precisa ter bootstrap `initialized`; backup não inicializa uma instalação.

Origem segue a configuração já existente: flags, ambiente (`KIARNI_DATA_DIR`),
arquivo `--config` e defaults. `--output` é obrigatório, único e somente CLI.
Opções de rede são validadas pela resolução comum, mas backup não faz bind.
Destino dentro da instalação ou da entrega padrão de credenciais é recusado.
O pai deve pertencer ao usuário atual e não permitir escrita por grupo/outros;
pode ser 0700 ou, por exemplo, 0755. Artefato é 0700 e arquivos são 0600.

## Conteúdo e integridade

```text
primeiro/
  backup.json
  store/
    format.json
    data.redb
```

Uma única transação de leitura fornece catálogo, documentos, metadados e contador
de identidades das coleções. JSON bruto e precisão numérica são preservados. A
exportação valida nomes, IDs, referências de coleções e correspondência de `_id`
com a chave; dados inconsistentes provocam erro, não um backup parcial publicado.

O novo banco é materializado incrementalmente. Escritas do destino são confirmadas
em lotes de até 256 registros lógicos ou aproximadamente 4 MiB de payloads e
chaves, com overshoot máximo de uma entrada (1 MiB de JSON mais até 136 bytes de
chave). Um índice temporário em disco valida
identidades sem carregar o catálogo inteiro em RAM; ele é removido antes do
fechamento. Cache configurado é 8 MiB por engine (origem e destino). A validação
retém um documento de cada vez; hash usa buffer de 64 KiB. Esses limites não são
uma medição ou promessa de RSS total do processo.

`backup.json` contém magic `kiarni-backup`, versão 1, versão lógica do store 1,
engine `redb-4.5.0`, contagens de coleções/documentos/bytes JSON em strings decimais
e uma lista fixa de dois arquivos. Cada registro inclui caminho relativo, tamanho
em string e SHA-256 hexadecimal minúsculo. Manifesto tem menos de 1 KiB.

Auth, verificadores, chaves, entrega de credenciais, bootstrap e locks da origem
não são exportados. **O backup contém os documentos e exige a mesma proteção da
origem.** Hash detecta alteração acidental, não autentica quem forneceu o artefato.
Abertura da origem pode executar recovery do engine, autorizado pelo manifesto;
backup não promete que nenhum byte físico da origem seja tocado.

Não abra `store/data.redb` do artefato com ferramentas de escrita: isso pode mudar
seu hash. Não inicie `serve --data-dir` no artefato. A [restauração operacional](./RESTORE.md) usa
`kiarni restore --from BACKUP --data-dir NOVA_RAIZ`, valida o artefato e prepara
uma instalação independente. Novas credenciais são emitidas no primeiro `serve`.

## Falhas e interrupção

Staging privado `.kiarni-backup-<aleatório>` é criado no mesmo pai do destino.
Depois da exportação, banco é fechado explicitamente, arquivos/diretórios são
sincronizados, hashes calculados, manifesto gravado e sincronizado. Publicação
usa rename atômico sem substituição. Destino existente (diretório, arquivo ou
symlink), inclusive criado durante a exportação, nunca é sobrescrito.

- `backup_destination_exists`: escolha outro destino; o existente foi preservado.
- `backup_destination_overlaps_source`: escolha um destino fora da instalação e
  de sua entrega padrão de credenciais.
- `insecure_backup_parent`: ajuste a escolha/permissões do pai explicitamente;
  o comando não altera permissões de diretórios existentes.
- `backup_publication_unsupported`: plataforma/filesystem não ofereceu publicação
  atômica sem substituição; não há fallback inseguro.
- `backup_publication_uncertain`: publicação ocorreu, mas não houve confirmação
  completa posterior. Preserve/valide o artefato antes de repetir; não presuma
  ausência do destino nem sobrescreva seus arquivos.

Interrupção/falha anterior à publicação pode deixar staging privado; o destino
final não existe. Depois da publicação, o destino completo pode existir mesmo
sem mensagem de sucesso. Staging não é reutilizado nem removido automaticamente;
inspecione e trate esses diretórios durante a manutenção, sem usá-los como destino
final. Não existe timeout global da exportação: janela de manutenção depende do
volume de dados e disco. Não há backup online, compressão ou criptografia nesta
entrega.

## Evidências e verificações

Testes de storage exportam mais de 600 documentos, inclusive registros grandes,
em múltiplos lotes. Capturam snapshot, alteram a origem e comprovam que o export
preserva o snapshot, JSON numérico, catálogo e próximo ID. Casos de layout inválido
recusam IDs duplicados, contador inconsistente, referência órfã e documentos
inválidos. Export vazio e destino existente também são exercitados.

Testes do servidor verificam hashes/tamanhos/permissões, árvore sem auth, origem
utilizável, lock, sobreposição e pai inseguro. Injeção entre preflight e rename
prova recusa de destino concorrente; erro depois de rename retorna resultado
incerto. Processos reais recebem kill antes/depois de publicar: destino ausente
antes, artefato completo depois, fonte reabre e lock é liberado.

Teste black-box usa o binário real: recusa servidor ativo, exporta após shutdown,
funciona com endereço de rede ocupado e preserva a credencial original ao reiniciar.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
python3 scripts/check_product_dependencies.py
```

Testes de rede precisam de loopback. Workers ignorados são subprocessos invocados
pelos testes pais. A evidência cobre interrupção de processo e chamadas de sync;
não simula queda de energia, falha física de disco ou ENOSPC. Nenhuma conclusão de
throughput de backup ou durabilidade física decorre desses testes.

Validação local em Linux x86_64, Rust/Cargo 1.99.1: fmt, clippy com e sem
profiling, 272 testes Rust com e sem profiling (três workers ignorados invocados
pelos testes pais), build release e fronteira de nove pacotes passaram. Testes
TCP foram executados com permissão de loopback. CI macOS/Linux deve ser
consultada separadamente no commit publicado.
