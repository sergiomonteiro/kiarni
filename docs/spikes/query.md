# Spike M0 — AST e semântica de consultas

O [ADR 0006](../adr/0006-ast-e-comparacao-json.md) propõe o contrato; continua em
revisão. O código em `spikes/query` é referência de semântica sobre fixtures, não
engine de produção, integração ao storage ou endpoint HTTP.

## Reprodução

```bash
cargo test -p kiarni-query-spike --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Execução local em Linux x86_64 com Rust/Cargo 1.99.1. Sem novas bibliotecas além
de Serde/serde_json já presentes no workspace; o spike habilita derive de Serde.
Não exige serviço externo nem executa código do cliente. macOS depende do CI.

## Evidências

- Corpus fixo de 12 documentos e 19 consultas em `tests/fixtures`. IDs esperados
  verificam filtro, composição, busca por ID, limites, sort e project.
- O corpus usa RawValue também nas queries para preservar os literais numéricos.
  A AST é serializada e validada novamente, com os mesmos resultados.
- A ordem de entrada é invertida: o resultado mantém sort e desempate por `_id`.
  Sem sort explícito, os resultados usam `_id` crescente.
- Null, ausente, false, true, números, strings e campos compostos exercitam as
  regras de tipos. Ponto no nome do campo é chave literal, não travessia.
- Comparação decimal preserva inteiros acima de 2^53 e casas que f64 arredondaria.
  5.776 pares adicionais são comparados com um oráculo racional independente,
  calculado por multiplicação inteira cruzada, sem ponto flutuante.
- Testes específicos verificam -0, expoentes extremos aceitos, limite de token,
  expoente fora do contrato, Unicode decodificado sem normalização e falta de
  coerção entre strings/booleanos/números.
- Projeção mantém tokens numéricos fora do domínio do comparador. Comparar ou
  ordenar esses números retorna erro; filtros all/any fazem curto-circuito.
- Campos desconhecidos, atributos incompatíveis com operadores, null em campos
  opcionais estruturais, duplicatas JSON e literais compostos são rejeitados.
- Limites inclusivos de bytes, resultado, quantidade de documentos, profundidade,
  nós, sort, projeção e nomes são exercitados. O avaliador não retorna saída
  parcial em caso de erro. Documentos sem ID ou com ID duplicado são inválidos
  mesmo se um filtro/lookup os excluiria do resultado.
- Debug da Query e mensagens de erro não incluem literais ou documentos.

## Limites e decisões em aberto

O avaliador recebe uma única coleção do chamador, sem consultar `from` no storage.
Há teto de 256 documentos e 8 MiB em chaves/valores brutos, não um orçamento
completo do alocador. A saída tem teto de 4 MiB. Buffers temporários e objetos
fornecidos pelo chamador também ocupam memória; este harness não é admission
control de servidor nem garante latência de consulta.

Comparador só aceita tokens de até 256 bytes e expoente explícito absoluto até
10000. Não expande zeros pelo expoente. A persistência de outros números JSON
não foi restringida. Esse limite do comparador e a política de erros dependente
dos campos examinados precisam de revisão antes de uma API pública.

Comparação de objetos/arrays, caminhos aninhados, paginação, planejamento,
explain, escolha de índice e equivalência de scans/índices com esta semântica
não foram implementados. O índice sintético do spike de storage não é um índice
numérico compatível com este contrato. Assinaturas filtradas também continuam
pendentes; este avaliador não resolve entrada/saída de conjuntos reativos.

Próximo incremento: consolidar o contrato HTTP e os erros, seguido do modelo
completo de autenticação e da revisão das pendências/ADRs para o gate M0.
