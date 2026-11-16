# RF-04 — Índice secundário de campo simples

Avaliação isolada da issue #8, complementando o
[spike inicial de storage](./storage.md). Decisão:
[ADR 0033](../adr/0033-avaliacao-indice-secundario.md). Não é uma funcionalidade
indexada disponível no servidor; o formato de produto continua 1.

## O que mudou em relação ao M0

O protótipo original exigia status string em todos os documentos e mantinha um
índice em todas as escritas. O novo módulo de avaliação aceita os documentos JSON
do domínio e compara dois bancos independentes: documentos apenas, e documentos
mais índice parcial de string. Ambos usam os mesmos bytes de entrada, engine
real de query/escrita, cache 8 MiB, Immediate e uma transação por documento.
Não há nova biblioteca transitiva ou dependência do produto para o spike.

Apenas `filter:{op:eq,field:status,value:STRING}` direto, sem id e com valor até
128 bytes UTF-8, escolhe o índice. ID usa primário; predicados compostos, range,
null, outros tipos/campos ou string longa usam scan. A string vazia é indexável;
escapes equivalentes são decodificados e Unicode não é normalizado. Missing não
é null. Nenhuma dessas restrições limita o JSON armazenável do produto.

O índice retorna IDs e busca o documento no mesmo snapshot; filter/sort/limit/
project são sempre executados pelo engine de produto. Um índice de baixa
seletividade ainda lê muitos documentos; limit 25 não o torna automaticamente
barato. Candidatos/bytes têm teto explícito de 10000/8 MiB. O experimento não decide
como mudar limites de uma query antes recusada por scan ao usar índice no produto.

## Correção e falhas

```bash
cargo test -p kiarni-storage-spike --locked
```

Seis testes de avaliação e um de crash, além dos cenários antigos, verificam:

- Igualdade de resultados serializados entre controle sem índice, scan forçado
  e índice; números brutos, sort/limit/project, composição/fallback e isolamento
  de coleções com o mesmo ID.
- Status ausente, null, booleano, número, array, objeto, vazio, Unicode composto/
  decomposto, escape equivalente, NUL e fronteira 128/130 bytes.
- PUT/PATCH/delete, remoção do campo, transições entre indexável/não indexável,
  no-op, conflitos, JSON duplicado, imutabilidade de ID e reabertura.
- Snapshot único de dados/índice durante quatro writers e snapshot antigo retido.
- Abort nos estágios de documento, remoção antiga, inserção nova e pré-commit;
  read snapshots nesses pontos veem estado coerente. Um erro depois do commit
  não é tratado como rollback: o estado novo coerente permanece.
- Kill de subprocesso em update/delete nos mesmos pontos e após commit retornado,
  seguido de reabertura/auditoria. Somente o filho criado pelo teste é morto.
- Auditoria detecta entrada faltante/espúria e referência inconsistente. Entrada
  faltante pode ser invisível ao lookup indexado; backfill/rebuild/recuperação
  precisam de contrato antes de integrar.

Não há injeção dentro do fsync/commit, energia, corrupção arbitrária ou teste de
lifecycle/migração de índice no formato real. A criação experimental recusa
arquivo existente, e a reabertura exige metadados próprios da avaliação.

## Reprodução da medição

Com dependências já baixadas, na raiz do checkout:

```bash
python3 benches/secondary_index.py --output /tmp/secondary-index-evaluation.json
```

O script chama exatamente o cargo run registrado no JSON: release, --locked,
--offline, três repetições pareadas e 30 queries medidas por variante/cenário.
Recusa destino existente e interrompe após dez minutos. O source fingerprint
precisa permanecer igual durante a medição; os arquivos e a receita estão no
[JSON versionado](../benchmarks/secondary-index-evaluation.json).

A fonte estava dirty em relação ao commit base 409e29f. O hash
`01e72ac0f62fbd83efd96dac9f637125413b6093b8b5d5eba6bf22247aa6e3b8`
identifica o recorte efetivamente medido; não atribuir esses números ao commit
base sozinho. Esse recorte foi versionado junto das evidências. Para conferir
correspondência no commit desta avaliação:

```bash
PYTHONPATH=benches python3 -c 'import json,secondary_index; r=json.load(open("docs/benchmarks/secondary-index-evaluation.json")); assert secondary_index.fingerprint()==r["source_fingerprint"]'
```

Mudanças posteriores podem produzir outro fingerprint; isso não invalida uma
medição histórica nem exige reescrever seu resultado. Não misturar números de
fontes ou ambientes diferentes como prova de ganho.

## Ambiente e protocolo

Execução local Linux x86_64, kernel 7.2.6-76070206-generic, glibc 2.39; AMD Ryzen 7
6800H, 16 CPUs lógicas, memória total informada 15898722304 bytes, governor
powersave. Host compartilhado, sem afinidade de CPU e sem derrubar caches.
Diretório temporário em ext4, origem /dev/nvme0n1p3[/tmp]; modelo do dispositivo,
cache/controladora e persistência física não foram certificados. Não houve
RUSTFLAGS/CARGO_PROFILE_RELEASE_* externos nessa execução. Rust/Cargo 1.99.1,
redb 4.5.0; versões completas no JSON. MacOS participa dos testes de correção
via CI; não há medição de desempenho macOS nesta avaliação.

Quatro cenários: 256 documentos/16 status e 4096 documentos/1,16,128 status. Cada
documento tem 512 bytes ASCII de payload, id determinístico, status=i mod C, n=i
e total=9007199254740993. Input/IDs da operação e AST são preparados fora do timer;
validação do domínio, transação/commit e manutenção de índice ficam dentro do
custo de escrita. Query mede planejamento, snapshot/candidatos, validação,
avaliação e serialização; comparação de resultados e descarte ficam fora do timer.

Cada repetição cria bancos novos; ordem controle/índice se alterna. Após insert,
auditoria e scan de referência aquecem o estado, seguidos de três warmups e 30
queries. A query filtra s000, ordena n desc, limita 25 e projeta status/n/total.
Mede também 128 mudanças de status, 128 mudanças fora do campo e 128 deletes,
com audits e comparação/reabertura fora dos timers. Chave inalterada não é
removida/reinserida; no-op do documento não confirma mutação.

Não mede HTTP, clientes concorrentes, group commit ou cache frio. Quantis usam
nearest-rank; p99 com 30 leituras é a amostra máxima, não estimativa de cauda de
produção. Resultados são três pares por cenário; não declarar significância ou
metas de TPS/RSS. O comando inteiro levou 120,502 s incluindo build.

## Resultados observados

| Documentos / cardinalidade | Matching | P50 scan, ms | P50 índice, ms | Ganho pareado scan/índice |
| --- | --- | --- | --- | --- |
| 256 / 16 | 16 | 0,522 | 0,065 | 8,02× |
| 4096 / 1 | 4096 | 9,417 | 17,360 | 0,54× |
| 4096 / 16 | 256 | 9,656 | 1,644 | 5,87× |
| 4096 / 128 | 32 | 8,795 | 0,205 | 43,12× |

P50s da tabela são medianas dos três p50s de cada variante; ganho é mediana dos
três quocientes pareados, não necessariamente quociente dessas duas medianas.
O cenário 4096/16 variou de 5,34× a 8,84× entre pares; 4096/128, de 42,13× a
44,31×. Todos os resultados brutos, operações e tempos ficam no JSON.

| Operação | Faixa das medianas pareadas índice/controle dos p50s, nos quatro cenários |
| --- | --- |
| Inserção | 1,10–1,13 |
| Atualização de status | 1,13–1,23 |
| Atualização fora do campo | 0,98–1,04 |
| Delete | 1,05–1,14 |

A faixa é dos quatro resultados por cenário, não intervalo de confiança.
Razão abaixo de 1 em alteração fora do campo não demonstra otimização de escrita.
Comprimento lógico de arquivo com índice/controle variou de 0,89 a 1,40 depois de
inserir: alocação COW/ordem de commits afetam esse tamanho. Não inferir economia
ou tamanho físico do índice; não foram medidos blocos alocados ou RSS.

O ganho seletivo é relevante, e o caso denso impede recomendar “usar sempre
índice”. A integração precisa de seleção de plano e lifecycle durável, além da
manutenção transacional demonstrada. [Decisão e consequências](../adr/0033-avaliacao-indice-secundario.md).
