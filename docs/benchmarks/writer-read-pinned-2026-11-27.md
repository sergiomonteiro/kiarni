# Leitura histórica com gerador e servidor separados — 2026-11-27

Terceiro incremento da issue #9 / requisito não funcional de desempenho, após
M3. Continuação da [revisita dos artefatos exatos](./writer-read-revisit-2026-11-26.md).
**32 rodadas válidas e 8.033.821 leituras confirmadas**, sem recusa, falha de
transporte, resposta incorreta, retry ou exclusão de rodada.

A coleta amplia a janela de 3 para 10 segundos, aumenta o warmup de 5 para
100 requests por cliente e separa os processos medidos em conjuntos de cores
físicos distintos. Integrado/anterior teve razões medianas pareadas de
**0,99995 com oito clientes** e **1,00485 com 32**. A queda de 4%–6% novamente
não foi confirmada de forma consistente; não demonstramos equivalência nem
causa no writer.

O limite mais relevante surgiu no próprio gerador: com 32 clientes, a mediana
de CPU agregada dos clientes/tempo de janela ficou em **7,94 CPUs lógicas-
equivalentes**, de oito disponíveis no conjunto. É sinal de orçamento de CPU
próximo de saturação no gerador, que pode limitar a carga oferecida e mascarar
diferenças no servidor. **Não usar esse throughput como capacidade do Kiarni**
nem interpretar razão próxima de 1 como ausência de regressão.

## Condições e validação

- Mesmos executáveis históricos verificados por SHA-256; não recompilados e não
  substituídos pelo produto atual. [Manifesto](./writer-read-artifacts-2026-11-26.json).
- Servidor em CPUs lógicas **0–7**, cores físicos 0–3; processos clientes em
  **8–15**, cores físicos 4–7. SMT em pares 0–1, 2–3, ..., 14–15. O controlador
  mantém a afinidade normal. Isso não reserva cores contra outras aplicações.
- Launcher aplica afinidade antes do exec do servidor; seu PID é conferido
  após readiness. Cada worker cliente aplica e verifica sua máscara antes do
  warmup. Todas as máscaras observadas coincidiram com o solicitado nas 32
  rodadas. Teste específico demonstra herança no exec/threads sem prender o pai.
- GET de documentos existentes, closed-loop, uma conexão nova/request, processos
  Python separados. Dataset 256 × 1 KiB, mesmo hash inicial original:
  `85fb688b70f90d073c5fe1bda93328937f85c4489435f2f9d90fda9aefa01ac4`.
  JSON completo conferido em todo GET/warmup e todos os documentos após a carga.
- Quatro repetições × quatro modos × dois níveis de clientes. Modos 0/2 usam
  anterior, 1/3 integrado, ordem Williams; 16 pares A/B e controles A/A que
  compartilham rodadas. Não são amostras independentes adicionais.
- Nova instalação por membro, mesma porta loopback, configuração HTTP original,
  sem assinantes e sem flags recentes no servidor histórico. Sem alterações de
  kernel, governor, protocolo, configuração pública ou código Rust do produto.

Esse particionamento é controle do experimento, **não avaliação de escala por
cores** ou implementação de escalabilidade vertical. Somente uma divisão fixa
foi medida. CPU/RSS do servidor e otimizações amplas continuam na etapa separada.

## Resultados

RPS e percentis da tabela são medianas de oito rodadas por versão/nível de
clientes, não percentis de amostras reunidas. CPU/tempo soma tempo de CPU dos
processos clientes durante a carga e divide pelo tempo de janela agregado;
inclui trabalho Python, validação e syscalls. Não mede CPU do servidor nem
quantidade de cores físicos úteis. Aproximar de oito indica pressão no orçamento
lógico; ficar abaixo não prova folga em recursos compartilhados do SMT.

| Clientes | Versão | Leituras/s | p50 (ms) | p95 (ms) | p99 (ms) | CPU clientes / tempo |
| --- | --- | --- | --- | --- | --- | --- |
| 8 | before | 25466.5 | 0.2965 | 0.3917 | 0.5299 | 6.307 |
| 8 | writer | 25475.7 | 0.2960 | 0.3982 | 0.5342 | 6.304 |
| 32 | before | 24770.8 | 1.2260 | 2.1368 | 2.9015 | 7.936 |
| 32 | writer | 24958.3 | 1.2202 | 2.1204 | 2.8763 | 7.940 |

Razões de throughput são resumidas por par, não pela divisão das medianas acima.
Min–max é amplitude observada, sem intervalo de confiança ou teto de ruído.

| Clientes | Comparação | Pares | Mediana RPS | Min–max | Mediana razão p95 |
| --- | --- | --- | --- | --- | --- |
| 8 | AB | 8 | 0.99995 | 0.95077–1.00382 | 1.01316 |
| 8 | before_AA | 4 | 0.99971 | 0.92385–1.00228 | 1.00449 |
| 8 | writer_AA | 4 | 0.98746 | 0.96212–1.00696 | 1.03188 |
| 32 | AB | 8 | 1.00485 | 0.97598–1.01411 | 0.99794 |
| 32 | before_AA | 4 | 0.99951 | 0.98994–1.00720 | 1.00437 |
| 32 | writer_AA | 4 | 1.00394 | 0.99931–1.02861 | 0.98647 |

Há menor amplitude em alguns controles, especialmente 32 clientes, mas o A/A
anterior com oito clientes ainda teve razão mínima de 0,92385. Todos os pares
permanecem nos resumos. Afinidade, duração, warmup, horário e carga ambiente
mudaram juntos em relação ao ensaio anterior: não atribuir a diferença só ao
particionamento ou só à janela, nem tratar as duas campanhas como fatoriais.

## Ambiente, diagnósticos e reprodução

Linux 7.2.6-76070206-generic x86_64, Ryzen 7 6800H, oito cores físicos/16 threads,
15.898.722.304 bytes de RAM; ext4/bloco 4.096 bytes. Python 3.12.3 e Rust/Cargo
1.99.1 no ambiente. Governor observado `powersave`, sem fixar frequência ou
controlar temperatura. Frequências pontuais, topologia e loadavg antes/depois
constam dos JSON, sem inferência de médias de frequência ou CPU/RSS do servidor.
Host compartilhado; não é máquina dedicada nem ambiente de capacidade certificado.
UTC 2026-11-28, noite local de 2026-11-27 (America/Sao_Paulo).

TIME_WAIT máximo observado 14.157; sem novos TCPTimeWaitOverflow, ListenDrops ou
ListenOverflows nos ciclos. TCPTimeouts global cresceu 62; não atribuir esse total
do host aos clientes medidos, que não tiveram falhas de transporte.

Fonte do medidor: `94d89b4f3a1f25418b5084e7e789a3179f72194b`, checkout limpo.
[JSON bruto](./writer-read-pinned-2026-11-27.json) copiado byte a byte da saída.
Fingerprints de fontes/binários conferidos antes/depois; nenhum build/teste local
concorrente à coleta. Metadados do checkout atual identificam o medidor, não a
fonte que produziu os executáveis históricos de workspace então alterado.

```bash
python3 benches/writer_history.py --before .bench-artifacts/writer-2026-11-26/before --writer .bench-artifacts/writer-2026-11-26/writer --output /tmp/writer-pinned-NOVO.json --clients 8 32 --repeat 4 --seconds 10 --warmup 100 --server-cpus 0 1 2 3 4 5 6 7 --client-cpus 8 9 10 11 12 13 14 15
```

Usar caminhos novos e conjuntos de CPU compatíveis com o host. As máscaras são
opcionais e validadas antes da execução; sem elas mantém-se o comportamento
normal do medidor, incluindo macOS. O launcher é ferramenta de benchmark,
sem dependência runtime do banco. Executáveis históricos preservados somente
localmente; checkout novo precisa recebê-los com os hashes exatos do manifesto.
[Metodologia e limites](../../benches/README.md).

## Atualização — conclusão da #9

A [calibração e comparação finais com gerador Rust](./writer-read-rust-2026-11-28.md)
concluem a investigação, com margem de CPU no cliente. A queda histórica não
foi confirmada consistentemente; a intervenção justificada foi no ferramental,
sem patch especulativo no runtime. O estado abaixo preserva o histórico deste
incremento, não uma pendência atual de localização ou medição.

## Decisão e próximo passo

Não aplicar uma correção no GET com base no percentual antigo: a queda continua
sem confirmação consistente e agora há sinal concreto de limitação do gerador.
Não repetir indefinidamente o mesmo teste Python para buscar um percentual.
Próximo incremento recomendado: um gerador de carga mais leve, limitado e com
validação equivalente, usado primeiro em A/A. Registrar seu consumo de CPU e
reavaliar se deixa margem antes de usá-lo para comparar servidores. Isso muda
a ferramenta de diagnóstico, sem trocar protocolo, garantias ou arquitetura.

A #9 permanece aberta; esse resultado não fecha a reprodução histórica nem
homologa uma otimização Rust. CPU/memória amplas do servidor continuam no
planejamento separado. Não se obteve evidência para alegar liderança de mercado
ou capacidade geral do Kiarni com esses dados.

Verificações locais: 372 testes Rust aprovados (13 workers exercitados pelos
pais), 16 testes do medidor, fmt/clippy, documentação/inventários e diff. O teste
real de afinidade é específico de Linux e é ignorado no macOS; validações puras
continuam portáveis. Smoke: oito rodadas curtas válidas e máscaras conferidas,
sem uso como evidência de desempenho. Nenhuma dependência runtime nova ou
mudança de API/formato persistido.
