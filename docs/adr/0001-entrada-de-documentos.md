# 0001 — Entrada de documentos e IDs explícitos

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md))
- Data: 2026-11-09
- Requisitos e marco: M0; RF-02 e preparação para RF-03/RF-06.

## Contexto

Precisamos de fixtures e validação de entrada independentes de storage e HTTP.
Este ADR propõe um contrato parcial; não congela geração de IDs, AST, comparação
numérica, atualização parcial ou o formato persistido. O protótipo não é CRUD.

## Opções consideradas

- Aceitar qualquer valor JSON como documento ou exigir objeto na raiz.
- IDs numéricos/Unicode ou strings ASCII adequadas a rotas e chaves.
- Aceitar a última ocorrência de uma chave duplicada ou rejeitar ambiguidades.
- Converter números para ponto flutuante ou preservar sua representação JSON
  até decidir a semântica de comparação.

## Decisão proposta

- A raiz é um objeto JSON UTF-8. Objetos vazios são válidos.
- `_id` explícito é string de 1 a 128 bytes: `[A-Za-z0-9_-]+`, sensível a caixa,
  sem coerção, trim ou normalização. A restrição se aplica somente à raiz.
- `_id` ausente significa que o servidor deverá gerar um ID antes de persistir.
  `_id: null` é inválido. O protótipo retorna uma entrada validada com ID opcional;
  não gera IDs e não garante unicidade. Formato gerado e colisões exigem decisão
  posterior e teste transacional por coleção.
- Campos ausentes e `null` permanecem distintos. Campos arbitrários, arrays,
  booleanos e strings Unicode são preservados.
- Números são preservados como JSON bruto, sem conversão a `f64`, inclusive
  inteiros grandes e expoentes. Isso não define igualdade, ordenação ou coerção.
- Rejeitar chaves duplicadas em qualquer nível, inclusive escapes equivalentes.
  Não normalizar Unicode de nomes de campos.
- Limite proposto: 1 MiB de entrada UTF-8, configurável pelo chamador. Conta
  whitespace e escapes; igualdade com o limite é aceita. A camada HTTP deverá
  limitar leitura do corpo antes da alocação. O tamanho final após gerar `_id`
  deverá ser verificado no caminho de escrita, ainda não implementado.
- Profundidade proposta: até 64 containers, incluindo a raiz. O parser mantém
  também sua proteção interna de recursão; entradas que a excedam podem retornar
  `invalid_json` antes da verificação de profundidade do domínio.
- Precedência de erros: tamanho, sintaxe/duplicatas, profundidade, raiz e ID.
- Erros de domínio: `document_too_large`, `invalid_json`,
  `document_too_deep`, `document_must_be_object` e `invalid_id`.
  Mensagens não incluem entrada, valores de ID nem detalhes internos do parser.
  Status HTTP, request IDs e envelope de erro ficam para o contrato de API.

## Consequências

O crate experimental `kiarni-core` valida entrada sem acoplamento a rede ou disco.
`serde` e `serde_json` são bibliotecas incorporadas; não exigem serviços externos.
Valores brutos evitam perda numérica e colisão entre nomes de campos do usuário
e representações internas do parser. A validação faz mais de uma passagem no
corpo limitado; medir custo no baseline antes de otimizar.

Não há atualização/substituição, índices, geração de ID ou recuperação nesta
entrega. Não confundir rejeição de chave JSON duplicada com ID já persistido.
Esses contratos serão implementados em incrementos posteriores.

## Evidências e validação

- Corpus: `crates/core/tests/fixtures/documents.json`.
- Testes: `cargo test -p kiarni-core --locked`.
- Cobertura adicional: limites de bytes/ID/profundidade, UTF-8 inválido,
  números sem arredondamento e nomes que coincidem com detalhes internos de Serde.
- Aceitação futura: revisar as restrições e limites antes de tornar este contrato
  público. A implementação experimental não altera o status de proposta.

## Incremento posterior

O [ADR 0002](./0002-geracao-de-ids.md) propõe e exercita a geração de IDs e a
preparação do documento final. As limitações acima descrevem o primeiro recorte;
unicidade e persistência continuam pendentes.
