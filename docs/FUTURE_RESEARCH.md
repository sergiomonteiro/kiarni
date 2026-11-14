# Investigações futuras, fora do MVP

## Escalabilidade horizontal

Registrada a pedido do usuário em 2026-11-13. Inspiração: a simplicidade
operacional desejada ao adicionar servidores, associada pelo usuário ao modelo
de distribuição da VM Erlang. É uma hipótese para spike futuro, não uma decisão
de usar Erlang/BEAM, distribuir o runtime Rust ou modificar a arquitetura atual.

O MVP continua single-node. Não introduzir cluster, replicação ou sharding por
antecipação deste estudo. A investigação só começa numa etapa futura explícita;
qualquer mudança no escopo do PRD exige decisão própria.

### Pergunta do spike

É possível oferecer uma experiência simples de adicionar servidores e obter
capacidade útil, com garantias de banco de dados compreensíveis e custo aceitável?
Separar a distribuição da execução da distribuição/durabilidade dos dados.

### Hipóteses e comparações a investigar

- Entrada/saída de servidores, descoberta, balanceamento e redistribuição.
- Propriedade dos dados e das escritas; custo de mover dados e aquecer cache.
- Consistência, replicação, disponibilidade em partições e recuperação.
- Localidade de queries, índices, coleções, skew e documentos muito disputados.
- Ordem e fronteiras dos changefeeds ao mover dados ou perder um nó; retomada
  só se for contratada num roadmap futuro, sem herdar promessas inexistentes.
- Comparar propostas inspiradas em processos/mensagens distribuídas com outras
  topologias de persistência; não presumir que um runtime distribuído resolva
  coordenação de escrita, consenso, replicação ou particionamento dos dados.

### Evidência esperada

Spike isolado, cenários de falha e partição, redistribuição sob carga,
throughput/latência confirmados, amplificação de rede/disco, recuperação e
passos operacionais reproduzíveis. A experiência de adicionar servidores deve
ser avaliada junto das garantias e do custo, antes de propor ADR ou produto.

Este registro não bloqueia M2/M3 nem autoriza implementação distribuída agora.
