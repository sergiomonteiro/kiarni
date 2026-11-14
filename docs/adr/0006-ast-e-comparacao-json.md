# 0006 — AST de consultas e comparação JSON

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md))
- Data: 2026-11-10
- Requisitos e marco: M0; RF-02/RF-03, preparação para RF-04.

## Contexto

Precisamos de contrato serializável e fixtures determinísticas antes de integrar
queries ao storage ou publicar SDK. Os exemplos do PRD são ilustrativos. O spike
deve preservar números JSON brutos e não executar código enviado pelo cliente.

## Opções consideradas

- Filtros abreviados por nome de campo versus AST explicitamente discriminada:
  propor `op` explícito para evitar confundir operador e campo.
- Coerção implícita versus comparação tipada: propor ausência de coerção.
- Números como f64 versus decimais exatos: f64 arredonda inteiros/decimais aceitos
  pelo contrato de documentos; comparar coeficiente e expoente sem expandir zeros.
- Ordenação parcial de tipos versus ordem total de escalares: propor ordem total
  somente para sort, mantendo operadores de faixa entre tipos compatíveis.

## Decisão proposta

AST JSON ilustrativa do experimento:

```json
{
  "from": "orders",
  "filter": {
    "op": "all",
    "args": [
      {"op": "eq", "field": "status", "value": "paid"},
      {"op": "gte", "field": "total", "value": 10.00}
    ]
  },
  "sort": [{"field": "total", "direction": "desc"}],
  "limit": 20,
  "project": ["status", "total"]
}
```

- `from` obrigatório, nome ASCII `[A-Za-z0-9_-]` de 1–128 bytes. `id` opcional,
  com a mesma validação dos IDs explícitos; busca por ID pode combinar filtro.
- `limit` obrigatório, inteiro de 1–1000. `filter`, `sort`, `project` opcionais.
  Campos desconhecidos e chaves JSON duplicadas em qualquer nível são erros.
- Filtros: `eq`, `lt`, `lte`, `gt`, `gte` com field/value; `exists` com field;
  `all`/`any` com args não vazio; `not` com arg. Sem funções arbitrárias ou scripts.
- Campos são chaves literais da raiz, UTF-8 de 1–128 bytes, sem controle ASCII.
  Ponto e barra não são caminhos: `a.b` acessa a chave literal `a.b`.
- Literais de comparação são somente null, boolean, número e string.
  Objetos/arrays podem ser armazenados, projetados e testados com exists; sua
  comparação por valor fica fora deste recorte. Tentar compará-los como valor do
  documento produz false; como literal da query é rejeitado.
- `exists` distingue ausente de null: null existe. `eq null` só seleciona null
  explícito. Operadores de comparação retornam false para campo ausente.
- Eq entre tipos distintos é false. Eq de bool/string/null usa valor exato;
  strings decodificadas, sensíveis a caixa e sem normalização Unicode.
- Faixa opera somente sobre dois números ou duas strings; demais pares são false.
  `not` é negação booleana: `not(eq missing ...)` pode ser true; não há lógica SQL
  de três valores ou operador `ne` implícito.
- Números comparados em decimal exato: 1 = 1.0 = 1e0; -0 = 0. Sem coerção de
  strings. Limite do comparador: token numérico até 256 bytes e expoente explícito
  de valor absoluto até 10000; excesso retorna `unsupported_number`, não arredonda.
  Documentos continuam aceitando números fora disso, que podem ser projetados
  sem comparação. Valores fora do limite dão erro somente quando examinados em
  comparação/sort, sujeitos ao curto-circuito dos filtros.
- Query até 16 KiB, filtro até 64 nós e profundidade 8; sort até 4 chaves únicas;
  project até 32 nomes únicos. Direção obrigatória asc/desc. Nenhum limite ilimitado.
- Execução de referência: ID → filter → sort → limit → project. Sort asc usa
  ausente < null < bool (false < true) < número < string; desc inverte essa ordem.
  Sort de objeto/array em documento selecionado retorna `unsupported_sort_value`.
  Desempate final é `_id` crescente, mesmo com desc; sem sort, `_id` crescente.
- Project preserva `_id` sempre e copia somente campos pedidos presentes. Lista
  vazia retorna apenas `_id`; ausência de project retorna o documento inteiro.
  Ordenação/filtragem precedem a projeção, incluindo campos não projetados.
- O avaliador recebe documentos de uma única coleção fornecida pelo chamador;
  não roteia coleções. Recusa IDs ausentes/duplicados, até em documentos que não
  passariam pelo filtro. Não é o engine ou planner de produção.
- Harness limitado a 256 documentos, 8 MiB de entrada contada pelas chaves/valores
  e 4 MiB de saída JSON serializada. Erros abortam a consulta, sem resposta parcial.
  Não garante performance, paginação, índice ou snapshot do storage.

## Consequências e validação

### Executor inicial do M1

O endpoint de query usa um snapshot da coleção persistida. Quando a AST inclui
id, a leitura usa a chave primária. As demais consultas têm orçamento de
10.000 documentos examinados e 8 MiB de JSON bruto. Esses limites são conferidos
antes de acumular e parsear cada documento. Exceder o orçamento retorna
422/query_too_complex, sem resultados parciais. Um limit pequeno continua
sujeito ao orçamento de scan. O avaliador aplica filter, sort, limit e project
sobre os documentos desse snapshot, preservando as regras de comparação.

O perfil admite uma query em execução por servidor, com espera FIFO de até
100 ms conforme o ADR 0015. Esgotamento retorna 503/service_unavailable. O lock do store cobre
a captura dos documentos e é liberado antes de filtrar e ordenar os dados
capturados. A codificação da resposta preserva números como RawValue e confere
o limite do envelope inteiro antes de enviar headers de sucesso.

O experimento `spikes/query` mantém tipos de wire privados e retorna uma Query
validada; não expõe Deserialize que permita contornar validação. A representação
pode ser serializada novamente. Debug não inclui filtros/literais/documentos.

Não se estabiliza API pública nem se aceita este ADR automaticamente pelo código.
Ainda faltam semântica de caminhos, comparações de compostos, política numérica de
SDK e integração ao storage, quando exigidos por incrementos futuros.

Executar `cargo test -p kiarni-query-spike --locked`. Fixtures e casos-limite
devem cobrir igualdade numérica, null/ausência, tipos mistos, ordenação/desempate,
projeção, composição, ID, schemas desconhecidos e limites. Relatório em
[relatório do spike](../spikes/query.md), com execução local em Linux e limites
identificados. Nenhuma decisão foi marcada aceita por este incremento.
