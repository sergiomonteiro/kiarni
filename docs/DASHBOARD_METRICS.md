# Métricas HTTP, consultas, storage e feeds no dashboard

Incremento M3/RF-08 sobre o recorte já implementado do RF-09. Após autenticar,
use **Atualizar métricas**. A coleta usa GET `/v1/metrics` com o mesmo Bearer,
origem e controles da API. Não requer selecionar uma coleção e não interrompe
um stream ativo. Consome uma vaga HTTP comum; com somente uma vaga configurada,
pare o stream antes de coletar.

A coleta valida 238 séries fixas. São onze tabelas, com seletor para as doze
primitivas de storage, em vez de abrir todas ao mesmo tempo:

- HTTP: cinco classes de status e três resultados de transporte (oito valores).
- Respostas comuns: sete buckets, soma e contagem (nove valores).
- Streams encerrados: sete buckets, soma e contagem (nove valores).
- Consultas: dez outcomes, caminhos ID/scan/secondary e totais de documentos/bytes de
  respostas montadas (quinze valores).
- Processamento de consultas: três histogramas de total, wait e execution,
  cada um com sete buckets, soma e contagem (27 valores).
- Storage: arquivo/commits/grupos (nove valores), histograma de commit (nove) e
  primitiva selecionada (duas contagens e nove valores do histograma). Todas as
  150 séries de storage são validadas; somente as onze da primitiva selecionada
  ficam nessa tabela. Trocar a seleção não faz nova requisição.
- Feeds: assinaturas ativas e vagas retidas, backlog/capacidade, inscrições,
  publicações, mudanças sem observadores, enfileiramentos, quatro motivos de
  encerramento e histograma de distribuição interna (vinte valores).

HTTP contabiliza conclusões de todo o servidor, inclusive erros, assets e
scrapes. 4xx/5xx classificam o status pretendido, não garantem entrega. Written
indica escrita local, write_failed pode ocorrer com 2xx e stream_closed não
confirma entrega de eventos. O scrape aparece só na coleta seguinte.

Respostas comuns medem recepção, execução e tentativa de escrita no handler;
streams medem sua vida inteira e só têm amostra ao encerrar. Streams ativos
não entram nesses totais nem no histograma de conclusões: veja assinaturas e
status de admissão para atividade atual. Recusas de assinatura são respostas
comuns. Esses histogramas não são tempo isolado do query engine/storage nem
latência de entrega de eventos. [Semântica HTTP](./HTTP_METRICS.md).

Consultas têm uma tabela de resultados/caminhos e três seções expansíveis para
os tempos. Rejeição de entrada pode ter amostra total sem espera ou execução;
recusa do gate tem total/espera, sem execução. Não interpretar as contagens de
estágios como necessariamente iguais. Caminho ID/scan/secondary significa seleção na execução admitida, inclusive
se ela falhar depois; antes da seleção mantém a classificação inicial id/scan; não é quantidade
de documentos examinados. Documentos e bytes são de respostas montadas com
sucesso, não prova de entrega. Contagem de documentos usa o gate, mas não entra
nessas métricas de query. Veja [semântica de consultas](./QUERY_METRICS.md).

Total de query exclui recepção de headers, autenticação e escrita no socket;
wait mede admissão; execution inclui leitura/snapshot, avaliação, serialização
e limite do envelope. Não são tempos isolados de storage ou CPU. Consultas em
andamento só aparecem ao finalizar. Os histogramas são expansíveis para manter
os detalhes disponíveis sem abrir todas as tabelas de tempo por padrão.

Storage distingue primitivas, commit do engine e resultados de grupos. Uma
chamada bem-sucedida não prova entrega nem alteração de documento; erros também
incluem conflitos/validação. Commit confirmado pode existir sem resultado de
grupo devolvido se uma falha acontecer depois. Grupos sem alterações não tentam
commit. A duração de commit já está incluída na primitiva de escrita; não some
as duas como etapas disjuntas. [Semântica de storage](./STORAGE_METRICS.md).
Para PUT/PATCH pela API HTTP, selecione **Atualização em grupo**; a primitiva
individual tem seu próprio contador e não representa essas chamadas agrupadas.

O tamanho lógico do arquivo não é uso físico de disco, espaço livre ou payload
dos documentos. Metadata com disponibilidade zero e tamanho NaN é um estado
válido: o painel mostra **Indisponível** e mantém os outros valores da coleta.
Não transforma ausência em zero. NaN com disponibilidade um, disponibilidade
inválida ou outro NaN/Inf em contadores/somas invalida a coleta inteira.

Os valores pertencem ao servidor inteiro, não à coleção selecionada ou a esta
aba. Contadores reiniciam com o processo. Enfileiramento não comprova entrega,
leitura ou confirmação do cliente. Vagas retidas incluem assinaturas terminais;
backlog exclui eventos em processamento. Gauges podem variar durante a coleta,
que não é snapshot transacional global.

O histograma dos feeds mede do retorno da confirmação do storage até concluir a tentativa
de distribuição interna observada. Não inclui duração do commit, espera do
writer, socket ou confirmação do cliente. Seus buckets são cumulativos em
segundos; não são percentis. Sem observadores não há amostra. Não calculamos
média, taxas ou percentis no navegador. Veja a
[semântica completa](./CHANGEFEED_METRICS.md).

## Segurança, precisão e limites

- Atualização somente manual, sem polling, histórico ou séries temporais. Há uma
  coleta em andamento e uma coleta atual por página, com horário local de coleta.
- Contadores u64 e valores do histograma são exibidos como texto original; não
  são convertidos para Number para renderização. A validação de contadores usa
  BigInt. Números acima de 2^53 são preservados.
- O leitor limita a resposta a 64 KiB e usa UTF-8 estrito, com espera de dez
  segundos. O parser reconhece as 237 séries fixas do produto: exige todas,
  rejeita duplicadas/valores inválidos e confere buckets cumulativos e count nos dezenove histogramas.
  Não é um parser Prometheus genérico. Séries desconhecidas são ignoradas.
- Uma falha limpa todas as tabelas anteriores; não exibe zeros inventados nem dados antigos
  como atuais. Erros HTTP mostram código/request ID quando disponíveis. 401
  limpa toda a sessão local; outros erros permitem tentar uma nova coleta.
- Atualizar a visão geral, desconectar ou sair da página cancela a coleta e limpa
  seus dados. Respostas antigas não repovoam uma sessão encerrada. A credencial
  continua somente em memória e não aparece em URL ou armazenamento local.
- O novo módulo estático `/dashboard/metrics.js` é incorporado ao mesmo binário.
  Não altera autenticação, CSP, formato do endpoint ou dependências em runtime.

## Verificação

```bash
node scripts/dashboard-metrics.test.mjs
cargo build -p kiarni --locked
node scripts/dashboard-browser.test.mjs
```

Os onze testes do parser/leitor cobrem precisão u64/decimal, séries ausentes
ou duplicadas, valores não finitos, os histogramas HTTP, queries e feeds,
tamanho e UTF-8. Os parsers HTTP/query recusam famílias incompletas sem renderização parcial.
O Chromium coleta métricas do servidor real com stream ativo e após mudanças,
no-op e cancelamento. Respostas injetadas verificam u64 máximo, formato inválido,
503/request ID, resposta tardia após desconectar e 401 com limpeza da sessão.
Também valida as onze tabelas, resultados reais de query, conclusão de stream sem incluir o ativo,
precisão HTTP u64, ausência de família HTTP e histograma HTTP inconsistente,
com limpeza conjunta dos dados e layout em viewport móvel.

O recorte não conclui RF-08/RF-09: o [status do serviço](./SERVER_STATUS.md) já mostra versão, uptime e ocupação
da admissão. A [contagem limitada de documentos](./DOCUMENT_COUNTS.md) também está disponível.
[Logs de requisições](./REQUEST_LOGS.md) já correlacionam operação e duração
por request ID. As métricas HTTP estão disponíveis na API e no dashboard. Continuam pendentes
diagnóstico interno necessário e erros recentes no dashboard. As métricas de
storage já estão apresentadas neste painel. As
métricas de query já estão apresentadas neste painel. Não houve servidor Prometheus externo ou
promtool nesta entrega.

Validação do primeiro recorte (feeds), em 2026-11-13: 235 testes Rust passaram (dois workers de fixture
ignorados diretamente e executados pelos testes pais), quatro testes novos de
métricas, oito do parser SSE, doze do cliente de referência e integração
Chromium real passaram. Também passaram fmt, clippy, fronteira de nove pacotes,
build release, sintaxe JS, links locais e `git diff --check`. CI remoto e outras
plataformas não foram executados nesta entrega.

Validação do incremento HTTP no dashboard, em 2026-11-13:
`cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`,
`cargo test --workspace --locked --offline`, build release e fronteira de nove
pacotes passaram. Passaram seis testes do parser/leitor e Chromium real, incluindo
as quatro tabelas, stream ativo/encerrado, limpeza conjunta e precisão HTTP u64.
Sintaxe JS, links locais e `git diff --check` passaram. Não houve nova execução
de CI remoto, outras plataformas ou benchmark de frontend; as séries da API
permaneceram inalteradas.

Validação do incremento de consultas no dashboard, em 2026-11-14:
255 testes Rust passaram no perfil padrão (dois workers de fixture ignorados
diretamente e executados pelos testes pais). Passaram fmt, clippy, build release,
fronteira de nove pacotes, oito testes do parser/leitor e Chromium real. Foram
validados os 41 valores de consultas, estágios expansíveis, consultas reais,
precisão u64, famílias ausentes e falha de cada histograma com limpeza conjunta.
Sintaxe JS, links locais e `git diff --check` passaram. Backend e séries
permaneceram inalterados; não houve benchmark, CI remoto ou validação de outras
plataformas neste incremento.

Validação do incremento de storage no dashboard, em 2026-11-14: 258 testes Rust
passaram no perfil padrão (dois workers de fixture ignorados diretamente e
executados pelos testes pais). Passaram fmt, clippy, build release, fronteira de
nove pacotes, onze testes do parser/leitor e Chromium real. A validação cobre
seleção sem novo fetch, contexto da primitiva, u64, tamanho indisponível válido,
pares inconsistentes, famílias ausentes e histogramas inválidos com limpeza das
onze tabelas e do cache da seleção. Sintaxe JS, links locais e
`git diff --check` passaram. Backend e séries não mudaram; sem benchmark novo,
CI remoto ou validação de outras plataformas neste incremento.
