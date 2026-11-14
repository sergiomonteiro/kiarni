# ADR 0011 — Formato inicial e catálogo de coleções

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md)).
- Data: 2026-11-10; M0-F3b, RF-01/02/04.
- Complementa ADRs 0001/0003/0007. O ADR 0003 seleciona redb 4.5.0 para M1.

## Contexto

O layout antigo do spike exige status string e não tem catálogo/versionamento.
Não deve ser interpretado como formato do produto. Validar apenas metadados
internos após abrir um engine para escrita pode ser tarde: a abertura pode reparar
o arquivo. A API read-only de redb 4.5.0 recusa arquivos que precisam de recovery,
conforme seu código/documentação local em `src/db.rs`, `open_read_only_with_backend`.

## Proposta experimental

O diretório exclusivo do engine contém `data.redb` e `format.json`. Na composição
proposta para M1, ele fica em `<data-dir>/store`, ao lado de `auth/`. O bootstrap
usa o lock da raiz e o registro de fase descritos no [ADR 0009](./0009-bootstrap-e-ativacao-auth.md).
Somente a fase `initializing_store` permite criar um store ausente. Em qualquer
fase, um store existente com manifesto ausente, inválido ou incompatível deve
ser recusado antes de qualquer entrega de credencial. Após `store_ready`, store
ausente também é erro. O spike recebe diretamente o diretório exclusivo do engine.

O manifesto é pequeno
(até 1 KiB), JSON estrito sem campos desconhecidos/duplicados, com:

```json
{"magic":"kiarni","version":1,"engine":"redb-4.5.0"}
```

`version` descreve o layout lógico Kiarni; `engine` fixa a implementação física
avaliada. Uma futura atualização do engine requer avaliação, não troca silenciosa
da string. Não prometer compatibilidade física com outras versões de redb.

Inicialização explícita exige diretório inexistente, cria o banco, confirma schema
com Immediate, fecha o banco, sincroniza arquivo/diretório, grava e sincroniza o
manifesto por último. Sucesso só após sync do diretório. Erro pode deixar diretório
parcial, que não é reutilizado automaticamente. Não há publicação atômica de toda
a árvore nem defesa contra queda de energia demonstrada neste incremento.

Abertura exige ambos os arquivos regulares e manifesto válido/suportado **antes**
de chamar redb. Ausente/inválido/incompatível: recusar sem inicializar, reparar,
atualizar ou migrar. Não reinterpretar dados do spike antigo. O manifesto é
imutável para esta versão; não há upgrade/downgrade in-place.

Layout candidato, tabelas tipadas em uma transação:

| Tabela | Chave | Valor |
|---|---|---|
| metadata | string | u64: format_version=1 e next_collection_id |
| collections | nome ASCII 1–128 `[A-Za-z0-9_-]` | u64 identificador interno |
| documents | (collection_id u64, `_id` string) | bytes JSON UTF-8 validados |

Contador inicia em 1, aumenta somente no commit de criação e nunca reutiliza ID.
Overflow recusa criação sem wraparound. Coleção duplicada é conflito. Nenhuma
coleção implícita. Exclusão só se não houver documento no intervalo da coleção,
verificado na mesma transação; recriar nome produz identidade nova. Handles antigos
não podem ler ou escrever a nova coleção. Não há sequência/cursor durável aqui.

Índice primário é a chave composta de documents. JSON não exige status nem esquema
fixo, e preserva números brutos. Somente `_id` validado, presente e consistente com
a chave. Não adicionar índice secundário sintético nem event log. Nome, catálogo,
contador e documento são verificados sob a transação correspondente.

Após abertura do engine, validar tabelas esperadas e metadados, sem criar tabelas
faltantes. Manifesto conhecido autoriza recovery do engine: **não** garante zero
escritas se o banco estiver corrompido ou tiver sido trocado por outro arquivo.
Não prometer proteção contra troca maliciosa do manifesto/arquivo pelo dono do
diretório; manifesto não é assinatura nem checksum dos dados. Validação estrutural
não varre todos os documentos no startup. Backup/restauração futuros precisam
transportar manifesto e banco de um snapshot consistente, não copiar arquivo vivo.

## Alternativas e limites

Somente header interno é simples, mas pode exigir recovery antes da leitura.
Preflight read-only funciona com fechamento limpo, mas não resolve recovery normal
de arquivos dirty. Manifesto externo torna a versão declarada verificável antes do
engine, ao custo de um artefato adicional e estados de inicialização incompleta.

Spike em `spikes/format`, sem servidor, migrations, CLI, TTL, índices secundários,
CRUD completo ou feeds. As evidências do formato estão no
[relatório](../spikes/format.md). Segurança operacional de diretórios, backup e
integração de startup continuam requisitos posteriores, não comprovados aqui.
