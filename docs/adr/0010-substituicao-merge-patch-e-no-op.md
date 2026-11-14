# ADR 0010 — Substituição, Merge Patch e no-op

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md)).
- Marco: M0-F3, RF-02 e efeitos em RF-05/06.
- Complementa: ADRs 0001, 0006 e 0007; não os aceita automaticamente.

## Contexto

Validar o envelope HTTP não define a transformação nem quando publicar update.
Precisamos preservar números JSON sem arredondamento e separar substituição de
atualização parcial antes de implementar handlers ou persistir dados do produto.

## Proposta

- PUT substitui todos os campos, preservando o `_id` da rota. Se ausente no
  corpo, injetá-lo; se diferente, `id_mismatch`. Não cria documento ausente:
  lookup anterior deve retornar `document_not_found`.
- PATCH aceita somente objeto na raiz, com media type já validado pelo HTTP.
  Qualquer `_id` na raiz, inclusive null ou idêntico, retorna `immutable_id`.
  `_id` aninhado é campo comum.
- Merge Patch recursa objetos, remove membros com null e substitui arrays
  inteiros; objeto aplicado sobre escalar/array começa como objeto vazio.
  O recorte de raiz objeto é deliberado; não implementa todo patch raiz permitido
  pelo [RFC 7396](https://www.rfc-editor.org/rfc/rfc7396.html).
  Para armazenar null diretamente em um membro, usar PUT.
- Duplicatas (inclusive chaves escapadas), JSON inválido, profundidade acima de
  64 contêineres e corpos acima do limite são recusados pelo contrato existente.
  Para uma alteração efetiva, revalidar tamanho e profundidade do resultado
  depois de injetar ID ou mesclar. Erros preservam o estado anterior.
- No-op compara a estrutura inteira: ordem de membros, espaços e escapes
  equivalentes de strings não contam; ordem de arrays e tokens numéricos contam.
  Assim, `1` → `1.0` e `0` → `-0` são alterações, embora a igualdade numérica
  de consultas do ADR 0006 os considere iguais. Nenhum número passa por float.
- No-op retorna o documento original (inclusive bytes) com sucesso 200 no futuro
  handler, sem commit de mutação, evento ou incremento de sequência local.
  Alteração real produz candidato completo, mas só pode gerar evento após commit.
- Lookup, transformação e comparação precisam ocorrer sob a mesma exclusão ou
  transação de escrita; não executar read-modify-write com leitura antiga fora
  dessa fronteira. Falha de commit não pode ser reportada como escrita concluída.

### Redução do limite configurado

Validar o corpo contra o limite vigente antes do lookup. Sob a transação,
validar o documento atual pelo teto do formato de 1 MiB e verificar que seu
`_id` corresponde à chave. Uma inconsistência persistida retorna erro interno.
Depois da transformação, comparar a estrutura antes de impor o limite de
escrita ao resultado. Um no-op retorna 200 com os bytes persistidos, inclusive
quando excedem o limite vigente. Uma alteração efetiva acima do limite retorna
413/`document_too_large`, ainda que reduza o tamanho anterior.

Por exemplo, com um documento persistido de 40 KiB e limite reduzido a 16 KiB,
um PATCH vazio retorna o original. Um PATCH que reduza o documento a 20 KiB
retorna 413. Um PATCH que o reduza a 16 KiB pode confirmar normalmente.
Um PUT com corpo de 40 KiB falha na validação de entrada, mesmo se for idêntico
ao documento atual. O modelo de transformação foi atualizado no M1 para validar
o estado anterior pelo teto do formato e decidir no-op antes do limite final.
O adapter de storage aplica a transformação dentro da transação de escrita.

## Alternativas e consequências

Comparar bytes brutos geraria updates por espaços/ordenação. Comparar todos os
números como decimais iguais apagaria alterações de representação e exigiria
normalização sem limites de expoente compatível com todo JSON aceito. A proposta
preserva tokens e torna essa diferença da linguagem de consultas explícita.

O modelo aloca árvores limitadas pelo tamanho e profundidade de entrada; não é
um orçamento de memória do servidor. Admissão global pertence ao restante de F3.
Não introduz upsert, JSON Patch, revisão otimista, transação multi-documento ou
formato persistido. O storage/feeds anterior ainda não aplica essas regras.

## Evidência e integração

Modelo puro e fixtures em `spikes/writes`, relatório em
[writes.md](../spikes/writes.md). Testam transformação e decisão no-op; não
demonstram ausência de commit/evento em um servidor. Integração transacional,
concorrência, falha de commit e publicação pertencem à implementação posterior.
