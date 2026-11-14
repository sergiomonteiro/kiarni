# ADR 0015 — Coordenação concorrente e admissão com espera limitada

- Status: aceita para o perfil local, após testes e medição.
- Marco: otimização do servidor após M1, RF-01/02/03/06.
- Evolui a política de admissão dos ADRs 0004 e 0012.
- Escrita evoluída no [ADR 0017](./0017-integracao-coordenador-de-escrita.md); gate de query preservado.

## Contexto

O baseline mostrou latência próxima de 25 ms em baixa concorrência e rejeições
503 mesmo em cargas de leitura. A troca isolada de sleep por poll no listener
melhorou a baixa concorrência. A contenção do mutex compartilhado continua
limitando o uso produtivo das conexões.

## Decisão

Leituras usam diretamente os snapshots do storage. O store compartilhado é
imutável em Rust, e redb mantém a consistência das transações. As mutações
continuam serializadas por uma permissão exclusiva, cobrindo leitura anterior,
commit e o ponto futuro de publicação de eventos.

A permissão de escrita e a vaga única de query admitem espera FIFO de até 100 ms.
O número de threads à espera permanece limitado pelas vagas de conexão já
admitidas. O gate usa condition variable, verifica shutdown a cada 25 ms e
libera a permissão por RAII. Cada waiter tem sua condition variable, evitando
acordar toda a fila a cada liberação. Esgotamento do prazo retorna 503/write_busy para
escritas ou 503/service_unavailable para queries. Shutdown retorna shutting_down
antes de iniciar uma operação que ainda aguardava admissão.

A consulta mantém uma única vaga ativa e a reserva global de 32 MiB. A espera
acontece antes de capturar o snapshot. Leituras por ID e listagens podem avançar
durante uma transação de escrita e observam um snapshot confirmado.

O prazo de espera limita a admissão. Commit iniciado preserva o contrato de
durabilidade e pode deixar resultado desconhecido se a conexão for interrompida.
O gate é liberado antes de escrever a resposta no socket. O tempo de um cliente
lento continua limitado pelo transporte. A ordem FIFO é verificada por teste.

O listener aguarda uma vaga de worker antes de accept quando a capacidade está
ocupada. Conexões pendentes ficam no backlog limitado do SO. A conclusão de
worker sinaliza o acceptor. Shutdown continua sendo verificado a cada 25 ms.
Respostas que já consumiram o corpo encerram o socket diretamente. Erros
anteriores à leitura completa mantêm o descarte limitado de bytes.

## Evidências

Comparar o baseline, a alteração isolada de poll e a coordenação revisada com
o mesmo benchmark. Conferir confirmações, rejeições, latência de sucesso e
visibilidade das escritas. A suíte deve verificar snapshot durante escrita,
admissão, timeout, shutdown, concorrência e recuperação.

O ensaio final completou 378.550 operações sem 503, falhas de transporte ou
escritas de resultado desconhecido. Os 161 testes Rust passaram. O relatório
registra a queda de throughput de escrita com 32 clientes e a melhoria na
admissão, com a medição de latência que inclui o tempo de espera.
[Comparação](../benchmarks/http-performance-2026-11-12.md)
