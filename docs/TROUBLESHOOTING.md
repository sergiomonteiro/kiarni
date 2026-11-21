# Diagnóstico operacional

Use esta tabela com a [API](./API.md), [CLI/configuração](./CLI.md) e seus limites.
Antes de intervir, registre versão, plataforma, configuração sem segredos,
comando sanitizado e request ID. Não inclua token, URL com credencial, payload
real de documento, `.credentials`, banco ou backup privado em uma issue pública.

| Sintoma | Verificação e ação |
| --- | --- |
| Serve não inicia | Leia o código curto em stderr. Confira pai da raiz, UID/permissões, porta e configuração. Nunca resolva apagando bootstrap/auth/banco. |
| data_directory_locked | Localize o processo que usa a raiz e encerre-o normalmente antes de executar administração offline. Arquivo de lock existir não significa que o lock está ocupado. |
| 401/unauthorized | Confira arquivo entregue, validade e rotação/revogação. Mesmo health exige Bearer. Reconcilie consumidores antes de trocar chave; tokens não são recarregados em processo vivo. |
| 403/forbidden | Confira Host literal e Origin; localhost não equivale a 127.0.0.1. No proxy use Host público configurado. Spoof de Forwarded não corrige origem. |
| 400/invalid_path ou unexpected_body | Use caminho canônico, sem percent encoding, corpo em GET ou parâmetros fora da listagem. Não envie Transfer-Encoding/chunked diretamente ao backend. |
| 415/unsupported_media_type | Use application/json, ou application/merge-patch+json para PATCH; somente charset UTF-8 opcional. Não envie Content-Encoding. |
| 422/id_mismatch ou immutable_id | PUT deve omitir ID ou manter o da rota. PATCH não pode incluir `_id` na raiz. |
| 413/document_too_large | Confira corpo e estado final depois de ID/merge. Diminuir payload de PATCH não garante que o documento final caiba. |
| 422/query_too_complex | Confira tamanho/estrutura da AST e orçamento da coleção. limit, filtro e índice seletivos não ampliam esse orçamento. Se o ID for conhecido, use id. [Índices e manutenção offline](./INDEXES.md). |
| 422/unsupported_number ou unsupported_sort_value | Comparação exata tem token/expoente limitados; sort só aceita escalares. Não converta silenciosamente números em float. |
| 422/result_too_large | Projete menos campos ou retorne menos documentos; limite cobre envelope inteiro. Não é resposta parcial. |
| Query lenta ou 503/service_unavailable | Compare espera de admissão e execução nas métricas de consultas. Query/count compartilham uma execução; a espera FIFO é limitada. Confira também concorrência HTTP. |
| 503/write_busy | Writer tem admissão/budget limitados. Reduza pressão concorrente e meça commits/grupos/latência. Retry-After não torna retry de mutação seguro. |
| API demora sem JSON de erro | Conexões incompletas e streams ocupam vagas; saturação pode aguardar no backlog do SO ou fechar transporte. Confira status quando houver vaga. Não pressupor sempre 429/503. |
| 429/subscription_limit | Pare streams não usados e confira max_streams/max_requests. Uma aba pode ocupar uma assinatura. |
| SSE com overflow | Consumidor não acompanha fila limitada. Descarte estado derivado não confiável, reconcilie por leitura e abra assinatura para mudanças futuras. Não há cursor/replay para recuperar lacuna. |
| SSE encerra com EOF/interrupted | Verifique rede, expiração, shutdown e limites de frame. EOF não certifica entrega completa; terminal também é best-effort. Não repetir escrita para compensar evento ausente. |
| SSE não recebe nada | Aguarde ready; é futuro, sem snapshot. Use --no-buffer no curl e confira buffering/timeouts do proxy. Um heartbeat não é alteração de documento. |
| Count exibe exact:false | É limite inferior, não total ou estimativa. Contagem tem orçamento e pode ser cara; não confundir com count do resultado de query. |
| Logs não mostram uma requisição | Nível/fila de logs pode descartar; dropped_before informa perdas quando há registro posterior. Erros recentes têm só oito entradas e perdas próprias. Não são auditoria. |
| Dashboard perde sessão | 401 limpa sessão; token vive só em memória. Navegação/desconexão cancela leituras; resposta antiga não deve atualizar nova seleção. |
| Docker unhealthy / pod não Ready | Execute o probe e confira validade/permissões/descoberta até 64 entradas. Não desative auth ou publique HTTP para tornar probe verde. |
| Contêiner recriado perdeu credencial | Confira se o mesmo volume/projeto cobre o pai de data e data.credentials. Não faça bootstrap novo sobre diretório incompleto. |
| Proxy/port-forward não alcança backend | Backend é loopback. Compose usa host no Linux; no pod, proxy precisa compartilhar namespace. Ingress para IP do pod não alcança loopback. |
| Restore recusa destino | Destino/credenciais devem ser novos e fora de origem/backup. Confira hashes, espaço, permissões e janelas de publicação descritas no guia. Não apague colisões automaticamente. |

Para uma consulta lenta, colete [métricas de consultas](./QUERY_METRICS.md) e
[HTTP](./HTTP_METRICS.md) antes/depois da carga. Separe espera, execução e escrita
da resposta; um histograma global não atribui latência a um documento específico.
Para consumidor atrasado, observe [métricas de feeds](./CHANGEFEED_METRICS.md):
assinaturas, fila, overflow e encerramentos. Para storage, separe
[primitivas, commits e tamanho lógico](./STORAGE_METRICS.md); não inferir fsync
físico, RSS ou perda de dados a partir de um único contador.

Correlacione X-Request-ID com [logs](./REQUEST_LOGS.md) e
[erros recentes](./RECENT_ERRORS.md). O header enviado pelo cliente não é a
identidade confiável da resposta. Métricas são autenticadas e não incluem
coleção/ID/payload em labels. Reinício zera observabilidade em memória; a coleta
não é snapshot atômico de todos os componentes.

Em backup/restore, use os contratos de [backup](./BACKUP.md) e
[restauração](./RESTORE.md), não cópia de arquivo aberto. Verificações de hashes
não substituem restauração testada ou cópia independente do volume. Para problemas
que exigem mudança de formato/contrato ou risco de perda de dados, preserve a
instalação e registre a decisão antes de intervir.
