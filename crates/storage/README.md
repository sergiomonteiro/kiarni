# Storage do produto

O crate mantém catálogo e documentos no formato do ADR 0011, com redb 4.5.0.
Não depende dos spikes. O HTTP usa o coordenador comum: grupos para PUT/PATCH
e operações isoladas para as demais mutações.

## Atualizações em grupo

`Store::update_group(&[updates::Update], max_bytes)` é uma primitiva interna para
o ADR 0017. Não cria endpoint de transação multi-documento.
Cada Update informa coleção, ID, operação Replace/MergePatch
e corpo emprestado. A coleção é resolvida dentro da transação, sem handle antigo.

O grupo aceita 1–32 operações e até 8 MiB de corpos. `max_bytes` deve estar entre
1 byte e 1 MiB, teto do formato. Lookup, transformação e índice primário usam
a mesma transação Immediate. Atualizações observam as anteriores do grupo.
Preservam-se as regras do engine: números exatos, ID reservado e no-op com bytes
originais, inclusive quando o documento excede um limite configurado reduzido.

O Receipt contém outcomes na ordem de entrada, com JSON próprio por operação
bem-sucedida, inclusive estados intermediários de um ID alterado várias vezes.
Só retorna após commit confirmado ou aborto de grupo sem alterações.
`committed` informa se houve commit; cada candidato informa se mudou o documento.

| Resultado | Significado |
|---|---|
| `outcomes[i] = Ok(PreparedWrite)` | JSON daquela operação; no-op não provoca commit sozinho |
| `outcomes[i] = Err(Error)` | Rejeição individual de domínio; outras operações válidas podem confirmar |
| `GroupError::InvalidGroup` | Grupo vazio, mais de 32 operações ou mais de 8 MiB; transação não iniciada |
| `GroupError::Aborted(error)` | Limite inválido ou falha anterior ao commit; grupo não confirma alterações |
| `GroupError::CommitUncertain` | Falha de commit ou confirmação perdida após commit; não implica rollback ou retry seguro |

Schema e metadados são conferidos antes de abrir tabelas para escrita, pois
open_table poderia criar uma tabela ausente. Schema ausente, metadados inválidos,
identidade de coleção inconsistente ou documento atual corrompido interrompem
o grupo inteiro, inclusive mudanças já preparadas. Não há reparo ou migração.
Snapshots de leitores não mostram mudanças anteriores à confirmação do grupo.

Os limites restringem entradas e até 32 JSONs de resposta de 1 MiB cada, além
do trabalho transitório do engine. Não constituem limite superior de RSS nem
substituem a reserva de buffers do coordenador. Cache, política de CPU, admissão,
threads HTTP e eventos permanecem fora desta etapa.

## Verificação

```bash
cargo test -p kiarni-storage --offline
cargo test --workspace --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
```

Dez testes novos cobrem PUT/PATCH misturados, JSON individual, no-op, rejeições,
limites, grupo máximo de 32, resolução após drop/recreate, snapshot durante staging,
schema/corrupção, rollback e grupos concorrentes. O teste de recuperação executa
um worker de fixture ignorado, mata-o antes/depois do commit e verifica ambas
as coleções após reabrir o formato real.

Falhas antes do commit e perda de confirmação após ele são injetadas em hooks
privados de teste. O método público não expõe os hooks. Erros reais de commit
são mapeados conservadoramente para resultado incerto. Não foram simulados EIO,
disco cheio, queda de energia ou falha física; demonstrou-se recuperação de processo.

## Estados para changefeeds

`update_group_captured` recebe flags por operação e retorna before/after em Arc
apenas para mudanças observadas. Os estados são da mesma transação, incluindo
transições intermediárias de um ID repetido; no-op/rejeição não captura. O JSON
after é compartilhado com PreparedWrite, evitando uma segunda imagem retida.
`delete_document_captured` devolve o estado removido somente após commit. Nenhum
destes métodos publica eventos: a responsabilidade pós-commit é do writer.
