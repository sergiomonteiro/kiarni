# Validação operacional do candidato

RF-08/09/10, issue #7. A decisão de release fica na
[revisão M3](./RELEASE_REVIEW.md); executar este roteiro não a aprova automaticamente.
Usa somente fixtures públicas, diretórios privados e recursos temporários próprios.
Não executa otimização de CPU/RSS, teste de TPS ou mudança de limites do produto.

## Instalação e quickstart do pacote

O pacote nativo contém `QUICKSTART.md` e `examples/`. Nenhum arquivo do checkout é
necessário para o usuário seguir o [roteiro](./QUICKSTART.md). O CI baixa o mesmo
artifact construído no workflow, verifica hashes e o executa em runner novo:

```bash
python3 scripts/quickstart_smoke.py --artifact-dir ./download \
  --expected-target x86_64-unknown-linux-gnu --report /tmp/quickstart-linux.json
```

No macOS use target `aarch64-apple-darwin`. O harness requer Python 3.11+, Bash,
curl e cat como ferramentas de teste/cliente, não dependências do servidor.
Verifica estrutura/checksums antes de executar; copia o binário para instalação
privada nova, fora do checkout. O servidor recebe ambiente vazio. O cliente tem
PATH somente com kiarni/curl/cat e shell selecionado explicitamente, sem Rust,
Cargo, Node, proxy ou serviços do banco. Executa os quatro comandos do próprio
Markdown empacotado: coleção, documento, consulta e patch, com eventos SSE reais.

O relatório registra hashes de arquivo/binário, commit/target/versão, sistema,
libc, marcos monotônicos até primeiro documento/consulta/evento e reinício com
mesmos dados/credencial. Todos os marcos devem ficar abaixo de 600 s. Não mede
rede/download/autenticação do GitHub, leitura humana, instalação prévia de
ferramentas ou Gatekeeper/quarentena. Não tratar automação como teste de usabilidade
humana nem como homologação de instalação pelo navegador no macOS.

O modo --allow-dirty é somente QA local e não passa no CI. O CI exige artifact
limpo do commit do workflow e publica relatório `quickstart-TARGET`, retido por
14 dias. Um erro/timeout falha o job; não fabrica relatório aprovado.

## Coleta externa e observação contínua

Em Docker Engine Linux x86_64, com um único pacote nativo limpo baixado:

```bash
python3 scripts/operational_soak.py --artifact-dir ./download \
  --duration-seconds 900 --report /tmp/operational-observation.json
```

O teste inicia o binário distribuído em ambiente vazio e configura **um Prometheus
real externo ao processo Kiarni**, somente para esse ensaio. A imagem é fixada em
Prometheus 3.16.0 por digest no script. O coletor é opcional e não entra no binário,
imagem do banco, quickstart ou dependências obrigatórias de runtime.

O contêiner temporário usa UID/GID do operador, config privado read-only,
credentials_file separado 0600, rede host com listener **127.0.0.1 em porta livre**,
sem capacidades/elevação, root filesystem read-only, limite 512 MiB/1 CPU/128 PIDs
e TSDB temporário limitado por tmpfs. São limites do teste, não sizing do produto.
`promtool check config` valida a configuração; scrape HTTP autenticado a cada
segundo prova que o Prometheus aceitou a exposição real. A credencial não passa
em argumentos, ambiente, URL ou relatório. Não há endpoint anônimo novo no banco.

O ensaio faz:

1. Carregar oito documentos com aproximadamente 700 KB de padding cada e um
   documento de progresso. Tudo cabe no orçamento de scan e nos tetos de formato.
2. Executar cinco lookups por ID e cinco scans limitados/projetados. Registrar
   delta de histogramas execution/wait e request IDs dos logs HTTP. O fixture
   precisa demonstrar execution de scan maior que ID; não há sleep/injeção de
   latência no produto nem promessa de uma razão de desempenho específica.
3. Abrir dois SSEs: leitor rápido e socket que deixa de ler, com janela pequena.
   Doze alterações confirmadas devem chegar ao rápido. O lento sofre overflow;
   métricas externas e log correlacionado pelo request ID precisam demonstrá-lo.
   As vagas dos streams devem ser liberadas em prazo limitado.
4. Durante a janela solicitada, a cada cerca de dois segundos, confirmar PUT,
   ler o documento com número exato e executar query por ID. O coletor deve
   continuar reportando up=1 e cobertura de pelo menos 80% das amostras esperadas.
5. Conferir ausência de token/padding nos logs, correlacionar query/overflow,
   parar o coletor, reiniciar Kiarni e verificar documento e credencial preservados.

O relatório sanitizado contém duração **observada**, versões/digest, ambiente,
dataset, contagens, deltas de execução/espera, cobertura de scrape e registros
seguros selecionados. Não contém tokens, payloads, caminhos privados ou dumps de
ambiente. O script não publica raw logs nem o TSDB. Remove somente o contêiner
com nome aleatório que criou e seus diretórios privados. Timeout/falha retorna
erro; não confundir um processo iniciado com uma observação concluída.

## Interpretar o diagnóstico

Para query, use `kiarni_query_duration_seconds_{sum,count}{stage="execution"}`
e stage wait, mais `kiarni_query_execution_total{access="id"|"scan"}`. O log com
request ID informa duração HTTP completa; ela não é CPU nem tempo isolado do
storage. A comparação deste fixture comprova o caminho de diagnóstico, não um
benchmark representativo de produção. [Semântica das queries](./QUERY_METRICS.md).

Para atraso, o contador
`kiarni_changefeed_subscriptions_ended_total{reason="overflow"}` permanece após
encerramento e pode ser capturado pelo coletor mesmo se o pico transitório da
fila não coincidir com o scrape. Correlacione stream_end_reason no log e observe
queue_events/queue_capacity. O terminal no socket continua best-effort; o teste
não exige que um consumidor que não lê receba seu aviso.
[Métricas dos feeds](./CHANGEFEED_METRICS.md).

No Prometheus local, os valores usados incluem `up{job="kiarni"}`,
`count_over_time(up{job="kiarni"}[1h])` e
`min_over_time(up{job="kiarni"}[1h])`. Scrape bem-sucedido não prova entrega de
evento ao usuário. Logs/métricas não são auditoria durável e não contêm AST/IDs de
documento; o harness usa fixtures conhecidas para correlacionar seus pedidos.

## Duração e limites da evidência

O CI roda uma observação funcional de **120 segundos**, com relatório no artifact
`operational-observation-linux-amd64`. A janela local planejada é **900 segundos**;
só conta como evidência depois de terminar com status passed e duração registrada.
O comando aceita 20–3600 s para QA/reprodução; 20 s é teste do harness, não a
janela longa. Resultados reais e fonte do pacote ficam na revisão/issue, sem
somar durações de execuções que falharam ou de binários diferentes.

Mesmo 15 minutos são uma janela curta e controlada: não certificam dias de carga,
RSS estável, exaustão de disco, energia/falha física, CSI, redes públicas,
renovação de certificado ou desempenho de produção. Esses limites não devem
virar alegações de HA, replay ou metas arbitrárias de TPS. Os
[benchmarks existentes](./PERFORMANCE.md) mantêm seus datasets e ambiente próprios.

O coletor e a observação contínua foram exercitados em Linux. Quickstart/CLI e
smokes nativos são separados por plataforma; não extrapolar ensaio Linux para
macOS. Também não substituir o [checklist de backup/segurança/licença e gate](./RELEASE_REVIEW.md)
por um único relatório aprovado.

Referências primárias de configuração do coletor:
[authorization/credentials_file](https://prometheus.io/docs/prometheus/latest/configuration/configuration/)
e [opções do Prometheus](https://prometheus.io/docs/prometheus/latest/command-line/prometheus/).
