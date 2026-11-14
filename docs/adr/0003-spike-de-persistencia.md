# 0003 — Avaliação inicial de redb para persistência

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md))
- Data: 2026-11-09
- Requisitos e marco: M0; RF-01, RF-02, RF-04 e preparação para RF-05.

## Contexto

O PRD exige evidências de transações, índices, recuperação, concorrência e backup
antes de escolher o storage. O experimento está isolado em `spikes/storage`, não
ligado ao binário `kiarni`. Não define API pública nem formato de arquivo Kiarni.

## Opções consideradas

| Aspecto | redb 4.5.0 | SQLite 3.45.1 via Python local |
|---|---|---|
| Integração | Biblioteca Rust; tabelas tipadas | Biblioteca C incorporável; binding Rust ainda não avaliado |
| Dados e índices | Duas tabelas atualizadas na mesma transação | Chave primária e índice secundário mantidos pelo engine |
| Conflito de ID | Verificar e inserir dentro da transação de escrita | Restrição composta `(collection, id)` |
| Leitor durante escrita | Snapshot de leitura retido enquanto outro thread confirma | Snapshot retido em outra conexão com WAL |
| Backup exercitado | Exportação lógica das duas tabelas de um único snapshot | API de backup online via biblioteca padrão Python |
| Recuperação exercitada | Reabertura normal e kill de subprocesso antes/depois de commit | Apenas reabertura normal nesta comparação |
| Resultado | Testes do experimento passam no Linux local | Comparação funcional passa no Linux local |

Fontes primárias: [redb 4.5.0](https://docs.rs/crate/redb/4.5.0),
[isolamento SQLite](https://www.sqlite.org/isolation.html) e
[backup SQLite](https://www.sqlite.org/backup.html).
O comparativo verifica semântica de um dataset pequeno. Não compara performance,
durabilidade em crash entre engines, custo de distribuição nem integração Rust
do SQLite. Python serve apenas ao experimento; não é dependência de runtime.

## Decisão proposta

Adotar redb 4.5.0 para o M1, com o layout do ADR 0011.
A alternativa SQLite permanece viável: as duas opções atendem aos casos básicos
exercitados, e não há evidência suficiente de superioridade geral.

Protocolo do experimento:

- `Durability::Immediate` explícito em cada transação de escrita.
- Cache de 8 MiB por instância; 4 writers no teste concorrente, sem spawn ilimitado.
- Documentos por chave composta `(coleção, id)` e índice sintético
  `(coleção, status, id)`. Somente documentos com `status` string participam deste
  dataset; isso não restringe o modelo JSON do produto.
- Inserção verifica ID existente e escreve documento/índice na mesma transação.
  Substituição remove a entrada antiga e adiciona a nova; exclusão remove ambas.
- IDs gerados têm até 8 tentativas de inserção, com colisões injetadas de forma
  determinística. IDs explícitos duplicados não são sobrescritos.
- Backup lógico usa um snapshot único e cria destino novo, sem copiar arquivo
  aberto. O backup restaurado não inclui commits posteriores ao snapshot.

## Consequências e limites

O protótipo demonstra consistência nos cenários testados, não durabilidade sob
queda de energia, falha física, disco cheio ou corrupção. Os kills são feitos
após marcadores sincronizados: documento sem índice, documento+índice sem commit,
e commit retornado. Não há injeção durante as chamadas internas de commit/fsync.

O backup é experimental: mantém um snapshot por toda a exportação, pode aumentar
retenção de páginas e deixa arquivo parcial em caso de erro. Não faz publicação
atômica/`fsync` de diretório nem oferece comando de backup ao usuário.

O lock entre processos foi demonstrado. O protótipo não valida diretórios nem
impõe limites de concorrência na camada de servidor, ainda inexistente. As
operações são síncronas; sua integração futura não deve bloquear tarefas async.

Nenhum evento é emitido. A relação commit → publicação e a ordem concorrente
continuam pendentes de spike próprio. O índice sintético não decide adoção de
índice secundário no MVP nem sua comparação de tipos. Crescimento de arquivo e
latência têm somente um baseline pequeno, sem meta de throughput.

## Evidências e validação

- `cargo test -p kiarni-storage-spike --locked`: inserção, substituição, exclusão,
  escopo de coleção, rollback, índices, concorrência, conflitos e retries,
  snapshot, backup, restore, lock de processo e kill/reabertura.
- `python3 spikes/storage/sqlite_comparison.py`: comparação funcional alternativa.
- [Relatório reproduzível](../spikes/storage.md): ambiente, comandos e baseline.
- O M1 verifica recuperação do formato real e carga representativa. O M2
  verifica a integração commit → evento.
