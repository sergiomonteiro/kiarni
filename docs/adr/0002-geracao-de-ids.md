# 0002 — Geração de IDs e preparação de inserções

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md))
- Data: 2026-11-09
- Requisitos e marco: M0; RF-02, preparação do spike de storage.

## Contexto

O ADR 0001 permite `_id` ausente, mas a entrada precisa receber um identificador
antes de persistir. Precisamos testar geração, falhas e limite do documento final
sem escolher storage. Este experimento complementa o ADR 0001; ambos continuam
propostas, sem estabilizar a API pública ou o formato persistido.

## Opções consideradas

- Contador persistente: compacto, mas exige coordenação transacional e revela ordem.
- UUID v4: formato reconhecido, com bits de versão/variante e biblioteca específica.
- UUID v7/ULID: ordenáveis por tempo, mas adicionam relógio e semântica desnecessária
  para o contrato atual.
- 128 bits aleatórios codificados em hexadecimal: formato pequeno de implementar,
  sem relógio nem estado compartilhado; colisões ainda exigem controle transacional.

## Decisão proposta

- Gerar 16 bytes usando `getrandom`, com a fonte do sistema operacional; codificar
  em 32 caracteres hexadecimais minúsculos, sem prefixo. Não é um UUID.
- Gerar somente quando `_id` estiver ausente. Preservar IDs explícitos, inclusive
  caixa, e não consultar aleatoriedade nesse caminho.
- Não prometer ordenação temporal, segredo, autenticação ou unicidade absoluta.
  IDs explícitos e gerados compartilham o mesmo namespace por coleção.
- Falha de aleatoriedade retorna `id_generation_failed`, sem fallback para tempo,
  contador, ID vazio ou gerador não criptográfico. Não incluir detalhes do SO.
- Preparar o documento inclui `_id`, serializa um objeto JSON e verifica novamente
  o limite escolhido no parse. A entrada e a representação final devem caber no
  mesmo limite. A ordem dos campos da raiz e seu whitespace podem mudar; valores
  de campos permanecem como JSON bruto, preservando números e dados aninhados.
- `document_encoding_failed` representa falha de serialização sem expor payloads.
- O resultado é um candidato à inserção, não confirmação de persistência. Expõe
  ID, origem gerada/explícita e bytes JSON somente para o caminho de escrita.
- O storage futuro deve testar existência e inserir documento/índices na mesma
  transação. ID explícito existente retorna `duplicate_id`, sem sobrescrita.
- Para colisões de IDs gerados, propor no máximo 8 tentativas totais. Esgotamento
  retorna `id_generation_exhausted`, sem loop infinito. Esses dois erros e retries
  ainda não são implementados; exigem testes determinísticos no spike de storage.

## Consequências

Geração não exige relógio ou estado compartilhado. `getrandom` é uma biblioteca
incorporada, sem serviço externo obrigatório. Não adicionar PRNG próprio.
Valores gerados ainda podem colidir; somente geração e preparação são demonstradas
neste incremento. O servidor deverá rejeitar colisões antes de confirmar escrita.
O documento preparado não tem contrato de durabilidade nem de emissão de eventos.

## Evidências e validação

- Testes unitários com bytes determinísticos verificam o encoding completo.
- Injeção privada da fonte de bytes permite testar falha sem depender do SO.
- Testes cobrem ID explícito sem geração, revalidação do limite final e payload
  numérico sem arredondamento; smoke test exercita a fonte real do SO.
- Comando: `cargo test -p kiarni-core --locked`.
- Próxima evidência: conflito explícito, colisão gerada, tentativas limitadas e
  concorrência sob transações reais no spike de persistência. Não inferir garantia
  de unicidade de uma amostra de IDs aleatórios.
