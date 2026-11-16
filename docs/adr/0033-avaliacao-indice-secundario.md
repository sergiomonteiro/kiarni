# ADR 0033 — Avaliação de índice secundário de campo simples

- Estado: aceito para o resultado da avaliação RF-04, issue #8.
- Data: 2026-11-15.
- Decisão: viabilidade demonstrada no recorte abaixo; integração adiada para um
  incremento próprio. O produto desta alpha mantém índice primário e scan limitado.
- Não modifica PRD, API, autenticação, changefeeds ou formato persistido 1.

## Requisito e hipótese

O RF-04 exige **avaliar** índice secundário simples condicionado ao spike, comparar
correção com/sem índice e registrar custos de leitura/escrita. Não determina que
uma tabela experimental seja integrada automaticamente ao servidor. A avaliação
faz parte do MVP; o adiamento de integração não remove essa avaliação ou o índice
primário obrigatório. Não declarar índice secundário disponível na API.

O M0 já testou duas tabelas em uma transação redb, rollback/kill e equivalência de
um pequeno dataset string-only. Faltavam controle sem índice, o modelo JSON geral,
uso do engine real e medições pareadas. O novo experimento fica em
`spikes/storage/src/secondary.rs`, com layout isolado e destinos novos; não abre ou
migra uma instalação Kiarni. Usa redb 4.5.0, cache 8 MiB e Immediate explícito.

Hipótese estreita: acelerar somente igualdade direta de **status string** na raiz,
decodificada, até 128 bytes UTF-8. Índice parcial `(coleção, valor, _id)` referencia
o documento. Valores maiores, null, ausente, booleanos/números/compostos continuam
válidos; queries fora desse predicado usam scan. ID tem prioridade. Sem coerção,
normalização Unicode, indexação numérica, range/composição indexados ou índice
reutilizável como contrato de produto.

Dados/índice são lidos em um snapshot. O engine de produto continua responsável
por filter/sort/limit/project e comparação exata; a seleção só reduz candidatos.
Amostras ficam dentro dos orçamentos de 10000 documentos/8 MiB, evitando atribuir
a um caminho novo a equivalência de erros de orçamento ainda não especificada.

## Evidências

[Relatório](../spikes/secondary-index.md) e
[medições brutas](../benchmarks/secondary-index-evaluation.json) registram fonte,
hardware, toolchain, comandos e todos os resultados. Há três pares por cenário,
ordem alternada, input/AST preparados antes do timer e caches aquecidos:

| Documentos / valores status | Candidatos scan → índice | Mediana pareada de ganho de leitura |
| --- | --- | --- |
| 256 / 16 | 256 → 16 | 8,02× |
| 4096 / 1 | 4096 → 4096 | 0,54×; índice aproximadamente 1,84× mais lento |
| 4096 / 16 | 4096 → 256 | 5,87× |
| 4096 / 128 | 4096 → 32 | 43,12× |

Nas medianas pareadas do fixture, inserir com índice custou aproximadamente
10–13% a mais; mudar status, 13–23% a mais. Mudanças fora do campo tiveram razões
0,98–1,04 e deletes 1,05–1,14. Não inferir melhoria de escrita por razão abaixo de 1
nesta amostra. Arquivo lógico variou com alocação COW; não equivale a espaço físico
ou a tamanho isolado do índice. Resultados não são throughput HTTP, SLA ou prova
universal; host compartilhado, governor powersave, cache quente e um writer.

Testes exercitam missing/null/tipos mistos, vazio/Unicode/escapes, limites de bytes,
precisão numérica, sort/projeção/composição via fallback, alteração e remoção do
campo, no-op, conflitos, reabertura, snapshots com quatro writers, abort e kill
nos checkpoints de update/delete antes/depois do commit. Auditoria compara todas
as entradas esperadas e efetivas. Não testa energia, falha física ou corrupção
arbitrária em produção.

Um teste remove uma entrada do índice: o lookup sozinho perde o documento,
enquanto scan o encontra; auditoria detecta a divergência. Isso demonstra uma
limitação concreta de recuperação/build, não um defeito introduzido no produto.

## Alternativas e decisão

1. Integrar imediatamente o índice em todo eq string: melhora buscas seletivas,
   mas piora o caso sem seletividade e exige lifecycle persistente ainda ausente.
2. Introduzir também administração/backfill/migração/rebuild/estatísticas agora:
   amplia esta avaliação para uma mudança de formato e API sem evidência de todo
   esse lifecycle. Não é a menor entrega correta da issue #8.
3. **Concluir a avaliação, manter primário/scan nesta alpha e planejar integração
   separada**, com o ganho seletivo como justificativa concreta. Escolhida.

A decisão não é que índices são desnecessários. Os ganhos justificam prioridade
para uma iteração de integração; o experimento não homologa um planner sempre
indexado nem o lifecycle de um índice de produto. Para integrar, requerer ADR de
formato/catálogo, criação/backfill consistente, coleção/geração/drop, recuperação
e rebuild, manutenção PUT/PATCH/delete/no-op, backup/restore e escolha de plano
baseada em custo/seletividade ou seleção explícita documentada. Preservar buffers,
orçamentos e contratos de erro; não habilitar sucesso além dos limites por acaso.

## Consequências para o gate

A avaliação condicionada do RF-04 está atendida com testes e medições; índice
primário continua obrigatório e implementado. PRD permanece inalterado. A
[revisão M3](../RELEASE_REVIEW.md) deve considerar essa decisão junto das demais
evidências, sem transformar o ganho do spike em ganho já entregue pelo servidor.
Nenhuma promessa de compatibilidade, endpoint ou migração de dados foi adicionada.
