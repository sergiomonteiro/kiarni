# Spike M0 — persistência

Relato histórico do experimento M0 em `spikes/storage`; o produto posteriormente
adotou storage no M1. Decisão: [ADR 0003](../adr/0003-spike-de-persistencia.md),
aceito na deliberação M0-F4. A [avaliação RF-04 posterior](./secondary-index.md)
compara campo string parcial com um controle sem índice e o engine do produto.

## Reprodução

```bash
cargo test -p kiarni-storage-spike --locked
python3 spikes/storage/sqlite_comparison.py
cargo run --release -p kiarni-storage-spike --example baseline --locked
```

Python 3 com `sqlite3` é necessário somente para a comparação opcional.
Os testes Rust fazem parte de `cargo test --workspace` no CI; comparação Python
e benchmark são execuções separadas. Dados sintéticos são criados em diretórios
temporários e removidos ao final. O teste de crash mata somente seu subprocesso
`fault-worker`, com timeout de 15 segundos para o marcador de sincronização.
Esse helper e a biblioteca de spike não integram o binário `kiarni`.

## Ambiente da execução local

- Data: 2026-11-09; Linux x86_64, kernel 7.2.6-76070206-generic, glibc 2.39.
- CPU informada pelo host: AMD Ryzen 7 6800H with Radeon Graphics.
- Memória total informada pelo host: 15.526.096 kB.
- Diretório temporário: `/tmp`, ext4 em `/dev/nvme0n1p3`; modelo do dispositivo,
  caches/controladora e garantias de persistência física não foram verificados.
- `rustc 1.99.1 (58cf8f9c3 2026-10-13)`;
  `cargo 1.99.1 (5f94df478 2026-08-27)`.
- redb 4.5.0; SQLite 3.45.1 na biblioteca padrão Python.
- Testes em debug; baseline em release. macOS ainda depende de execução do CI.

## Resultados

Os testes demonstram que alterações de documento e índice permanecem coerentes
após sucesso, rollback e encerramento forçado nos pontos injetados. Quatro
writers disputando o mesmo ID produzem um vencedor e três conflitos. O mesmo
ID em coleções distintas é permitido. Um leitor com snapshot retido não impede
o writer de concluir dentro do timeout de 10 segundos.

O processo de crash mantém os handles abertos até o pai executar kill. A
reabertura mantém o documento previamente confirmado e recupera também a escrita
marcada após commit; escritas sem commit não deixam documento nem índice. Enquanto
o filho mantém o banco aberto, outro processo recebe `DatabaseAlreadyOpen`.

O backup das duas tabelas a partir do snapshot antigo restaura os dados antigos,
mesmo após novas escritas no original. Destinos existentes são recusados.

A comparação SQLite passou em conflito de ID, rollback, snapshot WAL, equivalência
de scan/índice, backup online e reabertura normal. Não executou crash SQLite e não
serve como comparação de velocidade entre Rust e Python.

### Baseline observado

Uma execução, sem aquecimento controlado ou repetições estatísticas. Dataset:
256 documentos com 1.024 bytes ASCII de payload mais `_id`, `status` e overhead
JSON. Um writer, um índice secundário, uma transação por documento, cache de 8 MiB,
durabilidade Immediate. Preparação da entrada fora do timer; parse do campo
indexado e trabalho da transação dentro do timer de inserção. Os percentis são
obtidos ordenando 256 durações em microssegundos e tomando os índices 128/243/253.

```text
insert_us p50=721 p95=1456 p99=1973
point_reads=256 total_us=309 warm_cache=true
file_bytes empty=1056768 inserted=565248 after_delete_half=565248
normal_reopen_us=913
```

As leituras são 256 lookups individuais com novos snapshots após escrita, sem
limpar caches. O tamanho registrado é comprimento lógico do arquivo, não blocos
alocados. Nesta amostra o arquivo ficou menor que o tamanho inicial após inserir;
não há inferência de crescimento monotônico ou de devolução de espaço após delete.
O reopen inclui a inicialização das tabelas feita pelo protótipo. Esses números
não são metas, comparação com SQLite ou garantia de latência/durabilidade.

## Pendências

- Cargas maiores, retenção prolongada de snapshots, scans e crescimento/reuso de
  páginas sob ciclos longos de substituição e exclusão.
- Falha durante commit, disco cheio, erros de I/O e corrupção; crash não equivale
  a queda de energia nem certifica hardware.
- Backup com publicação atômica e tratamento de arquivos parciais antes de uso
  operacional; formato persistido e migrações ainda sem contrato.
- Spike de commit → evento sob concorrência e falha entre commit e publicação.
- Decisão de engine após revisão das evidências e limitações.
