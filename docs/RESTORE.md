# Restaurar um backup em nova instalação

M3, operação/RF-01. Issue [#2](https://github.com/sergiomonteiro/kiarni/issues/2),
[ADR 0013](./adr/0013-backup-e-restauracao.md) e
[ADR 0029](./adr/0029-restauracao-em-nova-raiz.md).

## Executar e iniciar

Use um artefato produzido por [kiarni backup](./BACKUP.md), sob seu usuário, com
permissões privadas preservadas. Escolha uma raiz nova e seu diretório irmão de
credenciais também novo. Mesmo um destino vazio existente é recusado.

```bash
mkdir -m 700 ./restores
kiarni restore --from ./backups/primeiro --data-dir ./restores/data
kiarni serve --data-dir ./restores/data --bind 127.0.0.1:8712
```

O pai `restores` precisa existir, pertencer ao usuário atual e não permitir
escrita por grupo/outros. Não é alterado automaticamente. O comando cria a raiz
0700 e `data.credentials` 0700 no mesmo pai. Antes do primeiro `serve`, a entrega
fica vazia, `auth/` não existe e o bootstrap está em `store_ready`.

`restore` não abre rede nem inicia servidor. O primeiro `serve` na raiz escolhida
emite uma credencial nova, entregue no arquivo privado informado pelo processo.
Reinícios preservam essa credencial; não geram outra automaticamente. Tokens
ativos ou revogados da instalação original recebem 401 no servidor restaurado.
A instalação original continua independente. Seu desligamento para criar o
backup é obrigatório; restaurar o artefato em outra raiz não altera um servidor
original que esteja rodando.

`--from` é obrigatório, único e somente CLI. O destino segue a resolução comum de
configuração: CLI, ambiente (`KIARNI_DATA_DIR`), arquivo `--config`, defaults.
Prefira informar explicitamente `--data-dir`. Opções de rede são validadas pela
resolução comum, mas somente `serve` faz bind. A configuração de outros comandos
não muda automaticamente; selecionar a raiz restaurada é uma ação do operador.

Não inicie `serve` diretamente no diretório do backup. O artefato não é uma
instalação. Não reutilize diretórios de dados ou de credenciais existentes, nem
restaure dentro do próprio artefato. Os caminhos de demonstração `backups/` e
`restores/`, assim como staging e diretórios `*.credentials`, ficam fora do Git.

## O que é validado

A árvore do artefato deve conter somente `backup.json` e `store/`, com apenas
`format.json` e `data.redb`. Diretórios são 0700, arquivos 0600, do usuário atual;
symlinks, hardlinks de arquivos, tipos impróprios e entradas extras são recusados.

O manifesto é lido em até 1 KiB, com schema estrito: sem campos desconhecidos ou
duplicados, versões e engine suportados, decimais u64 em strings, exatamente dois
caminhos fixos em ordem e SHA-256 hexadecimal minúsculo. Caminhos recebidos nunca
escolhem arquivos fora dessa lista.

Somente os arquivos esperados são copiados para staging privado, com buffer de
64 KiB e tamanho limitado pelo manifesto. Eles são fechados/sincronizados; hashes
são calculados relendo os bytes copiados. Só depois de tamanhos e hashes coincidirem
o engine abre a cópia. O arquivo do artefato original não é aberto para escrita,
recovery ou compactação e seus bytes permanecem intactos.

A cópia é reexportada para o store final usando um snapshot e os lotes limitados
do backup. Isso reaproveita a validação de tabelas/metadados, nomes, identidades
únicas, referências a coleções, JSON e correspondência de `_id` com a chave.
Contagens de coleções/documentos/bytes JSON precisam coincidir com o manifesto.
O contador de coleções, inclusive lacunas após exclusões, e o JSON bruto com
precisão numérica são preservados.

O método precisa temporariamente de espaço para duas cópias do store, além do
artefato. Cada engine tem cache configurado em 8 MiB, valida um documento de cada
vez e usa os lotes do backup; isso não é uma promessa de RSS total. A cópia
intermediária é removida após fechar o engine. O store final é fechado e
sincronizado antes da publicação. Não há benchmark desta operação ou timeout
global: o prazo depende dos dados e disco.

Hashes detectam corrupção acidental, sem autenticar a origem. Um artefato com
hashes recalculados ainda precisa passar pela validação estrutural. Dados são
sensíveis e exigem a mesma proteção da instalação de origem.

## Publicação e resultados incertos

Duas entradas distintas não são publicadas como uma única operação atômica:

1. Preparar o store final e bootstrap `store_ready` em `.kiarni-restore-<aleatório>`.
2. Publicar a entrega nova e vazia de credenciais com rename sem substituição e
   sincronizar o pai.
3. Publicar a raiz nova por rename sem substituição e sincronizar o pai, mantendo
   seu lock até concluir a operação.

Nenhum destino existente é sobrescrito, inclusive se surgir durante a cópia.
Plataforma/filesystem sem essa operação é recusado, sem fallback inseguro.

| Situação | Resultado e ação |
| --- | --- |
| `restore_destination_exists` | Raiz já existe; escolher outra, inclusive se a existente estiver vazia ou for symlink. |
| `restore_credentials_exists` | Entrega irmã já existe; preservada. Escolher outra raiz ou investigar a reserva de uma tentativa anterior. |
| `invalid_backup_manifest` / `unsupported_backup_format` | Schema, valores ou versão incompatíveis. Não editar versões para forçar aceitação. |
| `backup_size_mismatch` / `backup_hash_mismatch` | Arquivos não coincidem com manifesto; nenhuma raiz final é publicada. |
| `backup_counts_mismatch` / erro de layout do storage | Inconsistência dos dados mesmo com hashes válidos; nenhuma raiz final é publicada. |
| `insecure_restore_source` / `insecure_restore_parent` | Permissões/propriedade impróprias; não são corrigidas pelo comando. |
| `restore_publication_unsupported` | A publicação sem substituição não foi oferecida pelo sistema. |
| `restore_publication_uncertain` | Inspecionar raiz e entrega antes de repetir; a entrega pode existir sozinha ou a raiz completa pode estar publicada. |

Antes das publicações, uma interrupção pode deixar apenas staging privado. Entre
as publicações, pode restar somente o diretório final de credenciais vazio e
staging. Depois da publicação da raiz, ela está completa e pode existir mesmo sem
mensagem de sucesso. Não há token emitido pela restauração em nenhum desses casos.

O comando não reutiliza staging nem apaga automaticamente reservas ou destinos.
Uma nova tentativa com a entrega existente é recusada. Prefira outro nome de raiz
quando houver dúvida; não interprete erro como garantia de que ambos os caminhos
estão ausentes. Não use diretórios temporários como instalações.

## Feeds e limites

Auth e credenciais da origem não são importadas. A sequência local dos feeds
recomeça; não é cursor durável. Novas assinaturas recebem somente mudanças futuras,
com `snapshot:false` e `replay:false`; clientes precisam refazer leituras e assinar
na instalação selecionada. Não há retomada de streams do servidor original.

Testes cobrem manifesto/árvore inválidos, versões, truncamento, adulteração,
hashes recalculados com estrutura inconsistente, destino concorrente e falha
posterior à publicação. Processos reais recebem kill antes, entre e depois dos
renames. Teste black-box faz backup, restaura, inicia servidor real, valida 401
para tokens antigos/revogados, nova credencial, JSON numérico, reinício e feed
futuro com sequência reiniciada. Outro teste cria a próxima coleção e comprova
que seu ID usa o contador preservado.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
python3 scripts/check_product_dependencies.py
```

Workers ignorados são subprocessos invocados pelos testes pais. Testes de rede
precisam de loopback. A evidência cobre falha de processo e chamadas de sync;
não demonstra queda de energia, falha física, ENOSPC ou performance de disco.
O M3 e o gate de release continuam abertos para operação remota/distribuição.

Validação local em Linux x86_64, Rust/Cargo 1.99.1: fmt, clippy, build release,
fronteira de nove pacotes e 280 testes Rust com/sem profiling passaram (quatro
workers ignorados são invocados pelos testes pais). Testes TCP usam permissão de
loopback. A CI Linux/macOS/Chromium do commit publicado deve ser consultada como
evidência separada da execução local.
