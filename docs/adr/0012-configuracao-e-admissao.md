# ADR 0012 — Configuração, limites e admissão inicial

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md)); 2026-11-10; M0-F3c, RF-01/02/03/05/06.
- Complementa ADRs 0004/0007/0009; não os aceita automaticamente.

## Contrato proposto

Precedência por campo: **CLI > ambiente KIARNI_* > arquivo JSON opcional > default**.
Arquivo não é descoberto automaticamente: o modelo recebe seus bytes já lidos;
seleção/leitura do arquivo e parsing de argv pertencem ao futuro bootstrap.
CLI é uma lista normalizada de pares `--nome-com-hifens`, valor; ambiente usa
`KIARNI_NOME_COM_UNDERSCORES`. JSON usa `nome_com_underscores`, números inteiros
sem strings/unidades, e strings para bind/data_dir. Fonte ausente mantém fallback.

Cada fonte é estrita: desconhecidos, duplicatas, vazios, tipos/valores inválidos
falham mesmo se sobrescritos por fonte prioritária. Variáveis sem prefixo KIARNI_
são ignoradas. Arquivo até 16 KiB; no máximo 32 pares relevantes por fonte; chaves
até 64 bytes, valores até 4096 bytes. Segredos/tokens não são parâmetros aceitos.
Erros e Debug não expõem valores nem caminhos. Sem expansão de shell, tilde ou
interpolação de ambiente; caminho relativo é relativo ao cwd do futuro processo.

| Campo | Default | Intervalo aceito |
|---|---|---|
| bind | 127.0.0.1:8711 | SocketAddr numérico loopback, porta 1–65535 |
| data_dir | ./kiarni-data | string não vazia, sem controles, até 4096 bytes |
| max_document_bytes | 1048576 | 1–1048576 |
| max_query_bytes | 16384 | 1–16384 |
| max_result_bytes | 4194304 | 1049600–4194304 |
| max_requests | 32 | 1–128, inclui streams/conexões ociosas admitidas |
| max_streams | 8 | 1–32, no máximo max_requests |
| feed_queue | 4 | 1–16 eventos por assinante |
| read_timeout_ms | 5000 | 100–30000 |
| write_timeout_ms | 5000 | 100–30000 por frame/resposta, prazo absoluto |
| heartbeat_ms | 15000 | 1000–60000 |
| log_level | info | off, error, warn, info (string; ADR 0023) |

Porta 8711 é uma convenção proposta, não reserva de serviço nem garantia de porta
disponível. IPv6 loopback explícito é aceito. Este perfil local recusa wildcard e
IP remoto mesmo explícitos: exposição remota segura exige incremento próprio,
sem exceção de autenticação para loopback. Resolver configuração não faz bind.

Tetos fixos herdados: path 2 KiB, headers 8 KiB, corpo de coleção 1 KiB,
profundidade de documento 64, AST até 64 nós/profundidade 8, sort até quatro
campos, project até 32, resultado de consulta até 1000 documentos. Cache candidato
8 MiB. O adapter deverá aplicar os limites resolvidos, inclusive no corpo final,
envelope de resultado e frame SSE. Frame máximo proposto: 3 MiB; conferir tamanho
real antes de enviar, inclusive overhead de linhas SSE. Payload válido não garante
que toda representação SSE caiba: o encoder futuro deverá compactar whitespace
fora de strings ou terminar explicitamente o stream se ultrapassar o teto.
Esses valores não reconfiguram os harnesses existentes.

O teto de formato é 1 MiB por documento persistido. `max_document_bytes`
limita corpos de escrita e novos estados persistidos. O teto de formato rege
GET/HEAD, o estado anterior de PUT/PATCH/DELETE e os estados de changefeed.
Reduzir o limite na configuração de startup preserva os dados existentes.
Resultados de consulta continuam sujeitos ao limite do envelope, mesmo quando
incluem documentos gravados sob um limite anterior. Reservar pelo menos
1 MiB + 1024 bytes para resposta permite acomodar um documento no teto com
margem para envelope. A codificação deve conferir os bytes reais.

## Admissão e orçamento

A admissão conta cada conexão/request ativo em uma vaga geral; stream
ocupa simultaneamente uma vaga geral e uma de stream. Limite de stream → 429;
geral ou contenção pode resultar em 503. O ADR 0015 define a espera no backlog
do SO quando os workers estão ocupados. Conexão geral já admitida pode ser promovida a stream
depois de autenticação/roteamento, sem adquirir outra vaga geral. Falha na promoção
mantém a vaga original até o encerramento. Transporte ainda precisa definir rejeição antes de
parser/headers, onde pode haver apenas fechamento. Não aceitar conexões ilimitadas
enquanto se espera vaga. Futuro HTTP keep-alive precisa manter a vaga entre requests;
não multiplicar slots por requests pipelined. Sem fila adicional de sockets neste
perfil. Permissão liberada ao encerrar conexão/stream, inclusive erro/cancelamento.

Plano de reserva de buffers, com aritmética verificada e teto fixo 512 MiB:

`requests*(4*format_document + result + 2*query) + streams*(2*queue*format_document + 3 MiB) + 8 MiB`

`format_document` vale 1 MiB mesmo quando a escrita usa um limite menor. Os
estados anteriores podem entrar em respostas e ocupar filas de stream.

O executor de consultas do M1 acrescenta uma reserva global de 32 MiB para
snapshot carregado, chaves de ordenação e buffers de resultado. Há uma vaga
compartilhada de query, com admissão limitada pelo ADR 0015 e mantida até concluir a resposta
em memória. O plano total continua sujeito ao teto de 512 MiB.

Default no servidor M1: 385 MiB. O modelo F3c original calcula 353 MiB.
A soma cobra streams também no termo geral, deliberadamente
conservador. É uma restrição de combinações de configuração, **não** limite de RSS
nem prova de memória máxima: árvores JSON, stacks, engine, alocador, sockets/kernel,
snapshots e overhead não são medidos. O servidor precisará de admissão antes das
alocações, contabilidade de bytes reais e medição com carga representativa.
Não reduzir um valor inválido silenciosamente nem aceitar overflow aritmético.

Fila de feed cheia segue ADR 0004: terminal explícito, limitado e best-effort,
sem bloquear writer indefinidamente; não reter capacidade infinita após cancelamento.
Read timeout vale para request, não vida total do stream; write timeout não prova
rollback de commit. Heartbeat não prolonga prazo de frame bloqueado. Commit nunca
é cancelado presumindo que falhou; não há retry automático de mutação.

## Evidência e limites

`spikes/config` resolve dados fornecidos pelo chamador e modela admissão atômica
com guard RAII, sem rede, disco, relógio, parsing de argv, leitura de env real,
watcher/reload, tokens ou servidor. Configuração inicial é imutável; reload exige
contrato separado. Alternativa de defaults espalhados nos componentes foi evitada
para permitir checagem de relações; não se introduz framework genérico de config.

Fixtures/testes e limites em [relatório](../spikes/config.md). A implementação deve aplicar o contrato aceito na deliberação M0-F4.

O modelo F3c usa o limite configurado de escrita no cálculo de buffers e na
relação mínima com o resultado. No M1, validar a reserva com o teto fixo e
testar GET, DELETE e PUT/PATCH após reiniciar com limite menor. No M2, verificar
que eventos confirmados preservam o estado anterior e respeitam o teto do frame.

A configuração de logs e suas reservas foram adicionadas no
[ADR 0023](./0023-logs-de-requisicoes.md), preservando o orçamento de admissão.
