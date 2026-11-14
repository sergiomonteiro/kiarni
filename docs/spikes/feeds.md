# Spike M0 — commit → evento

Código descartável em `spikes/feeds`; nenhuma integração ao servidor. O
[ADR 0004](../adr/0004-ordenacao-e-overflow.md) permanece em proposta.
Usa o storage redb experimental, sem novas dependências externas.

## Reprodução

```bash
cargo test -p kiarni-feed-spike --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Testes locais em Linux x86_64, Rust/Cargo 1.99.1, redb 4.5.0, com diretórios
temporários e dataset sintético. O [relatório do storage](./storage.md) descreve
o ambiente. Não há benchmark de throughput deste coordenador; não inferir metas
de desempenho dos tempos de execução dos testes. macOS depende do CI remoto.

## Experimentos e resultados

| Experimento | Evidência |
|---|---|
| Caminho ingênuo commit seguido de publish independente | Atraso sincronizado depois do primeiro commit faz o segundo evento chegar primeiro |
| Seção crítica única | Pausas antes e depois do commit retornam `busy` para outro writer e não emitem antes da liberação |
| Quatro writers, duas escritas cada | Dois consumidores recebem a mesma ordem dos oito commits, com sequências 1–8 |
| Insert/update/delete | Dois assinantes recebem os estados anterior/posterior esperados; outra coleção não recebe esses eventos |
| Escopo de coleção | Sequência é global à instância; assinantes podem observar lacunas por commits em outras coleções |
| Erro antes de commit | Aborto injetado, ID duplicado e documento ausente não geram eventos nem consomem sequência |
| Falha injetada após commit | Dado confirmado permanece, evento é omitido e assinaturas são interrompidas explicitamente |
| Consumidor lento | Fila de capacidade 1 encerra com overflow; writer e consumidor rápido seguem sem esperar espaço |
| Retenção de handles encerrados | Handles com overflow ocupam vagas até Drop; não é possível acumular backlogs sem limite por reconexões |
| Desconexão/reconexão | Drop libera vaga; nova assinatura recebe somente eventos futuros |
| Shutdown/drop do coordenador | Motivo terminal acorda consumidor, descarta backlog e rejeita novo trabalho após shutdown |
| Reabertura | Dados permanecem; sequência reinicia e não há replay |
| Limites | Capacidade inválida, metadados inválidos, payload acima de 1 MiB e esgotamento da sequência impedem nova escrita |

As pausas usam canais de sincronização, não sleeps para presumir interleavings.
Há timeouts de 3 segundos nos pontos de sincronização. Os writers concorrentes
usam retry de `busy` apenas no teste, com deadline; o coordenador não implementa
uma fila interna ou política pública de retry.

## Limites observados

- A injeção de lacuna após commit é uma simulação em memória. O teste confirma o
  estado persistido e a interrupção das assinaturas, não executa kill neste
  intervalo. O spike de storage testa kill antes/depois de commit separadamente.
- Não existe atomicidade durável entre commit e evento. Um crash real pode perder
  notificações de dados confirmados, inclusive antes de a resposta chegar ao cliente.
- Ordem é a dos commits aceitos pelo coordenador; writers concorrentes podem
  receber `busy`. Não há garantia de ordem de chegada, fairness ou sequência
  durável. Escrita que contorne o coordenador não faz parte da garantia.
- Entrega é best-effort enquanto conectado. Não há replay, ACK, exactly-once,
  snapshot inicial ou query reativa filtrada.
- O sinal terminal fica fora da fila de dados; ao observá-lo, o consumidor descarta
  backlog. O handle conserva sua vaga até Drop, mesmo após observar o erro. O
  transporte futuro deve descartar esse handle ao fechar a conexão.
- Há até 8 handles de assinatura, fila máxima de 8 eventos e estados de documento
  até 1 MiB cada. O teto conservador de payload nas filas é 128 MiB, além de
  metadados, buffers de trabalho e eventos já entregues mantidos pelo chamador.
- Fanout é síncrono e limitado. Writers não aguardam assinantes, mas I/O lento ou
  processo suspenso ainda pode reter a seção crítica. Subscribe/shutdown/drop
  podem esperar esse lock. Integração async e limites de admissão da rede não
  estão implementados.
- São assinantes em memória, não conexões HTTP independentes. RF-05 e M2 ainda
  precisam dos testes de rede, transporte, autenticação e observabilidade.

## Próximo incremento

Comparar SSE e WebSocket incluindo autenticação no browser, reconexão sem replay,
sinalização terminal e limites de conexão. A comparação deve registrar trade-offs
em ADR antes de escolher transporte. O marco M0 continua aberto.
