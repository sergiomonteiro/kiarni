# Referência de CLI e configuração

RF-06/07/10, M3. Descreve a alpha implementada; não aprova release estável.
[Instalação nativa](./DISTRIBUTION.md), [contêiner](./CONTAINERS.md),
[API](./API.md) e [diagnóstico](./TROUBLESHOOTING.md).

## Comandos

Use o binário instalado como `kiarni`; no checkout, `./target/debug/kiarni`
após `cargo build -p kiarni --locked`. Sem argumentos, imprime o resumo de uso.
`--version` imprime `kiarni VERSÃO`. Não há `--help`, daemonize, comando stop,
cliente de documentos, migração ou atualização automática implementados.

| Comando | Efeito e argumentos próprios |
| --- | --- |
| `kiarni --version` | Informa versão Cargo; não inicializa servidor. |
| `kiarni serve` | Abre raiz, valida/recupera bootstrap, ativa auth e atende HTTP. |
| `kiarni healthcheck` | Consulta ready autenticado no loopback; opcional `--token-file CAMINHO`. Não abre banco nem altera auth. |
| `kiarni auth rotate` | Offline: emite nova credencial, com validade de 30 dias, sem sobreposição com a anterior. |
| `kiarni auth revoke-all` | Offline: revoga todas as credenciais ativas; reiniciar não cria outra automaticamente. Use rotate para recuperar acesso. |
| `kiarni backup --output DESTINO` | Offline: exporta snapshot consistente para destino novo, sem credenciais. |
| `kiarni restore --from BACKUP` | Offline: valida e publica em raiz nova selecionada por `--data-dir`; novas credenciais no primeiro serve. |

Os cinco comandos operacionais (serve, healthcheck, auth, backup, restore) usam a
configuração abaixo. As opções exclusivas `--token-file`, `--output`, `--from`
não são campos de configuração JSON/env. Mesmo opções sem efeito no comando
offline são validadas; não use combinações inválidas. Exit code 0 indica sucesso,
1 indica erro; erros locais são códigos curtos em stderr, não envelopes HTTP.

Pare serve com **Ctrl+C ou SIGTERM**, aguarde encerrar e preserve a mesma raiz.
Não envie SIGKILL como procedimento normal. O lock exclusivo recusa outro serve
ou administração offline na mesma raiz. Timeout de cliente não prova rollback de
mutação. Para persistência/recuperação, veja [backup](./BACKUP.md) e
[restore](./RESTORE.md); copiar o arquivo redb aberto não é backup consistente.

A raiz de dados é criada se o pai existir. Raiz/credenciais privadas exigem UID
atual e modo 0700; arquivos privados, 0600, sem links indevidos. Nunca corrigir
corrupção apagando arquivos de bootstrap/auth/lock enquanto o processo estiver
vivo. Use backup e diagnóstico antes de qualquer intervenção.

## Precedência e formatos

Por campo: **CLI > ambiente KIARNI_* > arquivo JSON explícito > default**.
Não há descoberta automática de config, expansão de `~`/variáveis dentro de JSON,
watcher ou reload. Caminho relativo usa o diretório de trabalho do processo.
O shell pode expandir expressões antes de chamar o binário; o Kiarni não o faz.

CLI aceita pares `--nome-com-hifens VALOR`, não `--nome=VALOR`, flags booleanas ou
unidades. JSON usa nomes com underscores e inteiros para campos numéricos;
ambiente usa nome uppercase com underscores e número decimal sem unidade.
Valores desconhecidos, vazios, tipos inválidos e duplicatas em uma fonte falham,
**mesmo se sobrescritos** por fonte de maior prioridade. Sem prefixo KIARNI_,
variáveis de ambiente são ignoradas; com esse prefixo, erros de nome falham.
Não use KIARNI_TOKEN, KIARNI_CONFIG ou KIARNI_IMAGE como ambiente do binário.
KIARNI_IMAGE/KIARNI_BIND usados pelo Compose têm papéis próprios; só campos
válidos devem ser encaminhados ao processo do servidor.

Arquivo opcional com `--config CAMINHO`, até 16384 bytes, objeto JSON UTF-8,
sem chaves duplicadas, desconhecidas ou comentários. Até 32 pares relevantes
por fonte, chave até 64 bytes e valor até 4096 bytes. Não há opção para gravar ou
imprimir configuração efetiva contendo segredos; credenciais não são campos aceitos.

| Campo JSON | CLI | Ambiente | Default | Intervalo/validação |
| --- | --- | --- | --- | --- |
| `bind` | `--bind` | `KIARNI_BIND` | `127.0.0.1:8711` | Endereço numérico loopback IPv4/IPv6; porta 1–65535. Sem hostname ou wildcard. |
| `public_origin` | `--public-origin` | `KIARNI_PUBLIC_ORIGIN` | ausente | Origem HTTPS canônica, até 272 bytes; não aceita null para remover. |
| `data_dir` | `--data-dir` | `KIARNI_DATA_DIR` | `./kiarni-data` | String não vazia, sem controles; até 4096 bytes. |
| `max_document_bytes` | `--max-document-bytes` | `KIARNI_MAX_DOCUMENT_BYTES` | `1048576` | 1–1048576; entrada e novo estado escrito. |
| `max_query_bytes` | `--max-query-bytes` | `KIARNI_MAX_QUERY_BYTES` | `16384` | 1–16384. |
| `max_result_bytes` | `--max-result-bytes` | `KIARNI_MAX_RESULT_BYTES` | `4194304` | 1049600–4194304 no binário, mesmo reduzindo limite de escrita. |
| `max_requests` | `--max-requests` | `KIARNI_MAX_REQUESTS` | `32` | 1–128; inclui conexões incompletas/streams. |
| `max_streams` | `--max-streams` | `KIARNI_MAX_STREAMS` | `8` | 1–32 e não maior que max_requests. |
| `feed_queue` | `--feed-queue` | `KIARNI_FEED_QUEUE` | `4` | 1–16 eventos por assinante. |
| `read_timeout_ms` | `--read-timeout-ms` | `KIARNI_READ_TIMEOUT_MS` | `5000` | 100–30000; prazo de leitura do request, não vida do feed. |
| `write_timeout_ms` | `--write-timeout-ms` | `KIARNI_WRITE_TIMEOUT_MS` | `5000` | 100–30000; prazo por resposta/frame, não prova de rollback. |
| `heartbeat_ms` | `--heartbeat-ms` | `KIARNI_HEARTBEAT_MS` | `15000` | 1000–60000; heartbeat não estende prazo de frame bloqueado. |
| `log_level` | `--log-level` | `KIARNI_LOG_LEVEL` | `info` | off, error, warn, info; sem debug/trace. |

Public origin descreve o frontend HTTPS, não muda bind, não oferece TLS e não
confia em Forwarded. Sem caminho, slash final, credenciais, query ou fragmento;
DNS ASCII/IPv4/IPv6 conforme [perfil remoto](./REMOTE_HTTPS.md).

Limites são combinados: nem toda combinação de máximos é válida. O binário exige
pelo menos 1 MiB + 1024 para respostas, max_streams ≤ max_requests e valida um
plano conservador de buffers de até 512 MiB, cobrando o teto de formato de 1 MiB
mesmo com max_document_bytes menor. A configuração default passa nesse plano;
seus 385 MiB de reserva conservadora não são alocação imediata ou teto de RSS.
O cálculo inclui uma reserva global para query e custos dos componentes, conforme
[ADR 0012](./adr/0012-configuracao-e-admissao.md). Não aumente concorrência sem
considerar filas, payloads e memória. Esse cálculo não dimensiona hardware.

Exemplo completo: [config.json](./examples/config.json). Na raiz do checkout:

```bash
kiarni serve --config docs/examples/config.json
```

Com override por campo, mantendo valores inferiores válidos:

```bash
KIARNI_MAX_REQUESTS=12 kiarni serve --config docs/examples/config.json --max-requests 8
```

O arquivo define 16 requests/4 streams; o ambiente muda para 12 e a CLI prevalece
com 8. Todos continuam válidos. O teste documental inicia o servidor com esse
arquivo e confirma a precedência por status HTTP, com porta/raiz temporárias.

## Credenciais e probe

Primeiro serve informa o caminho do arquivo em `DATA_DIR.credentials`, irmão da
raiz. Não reemite esse caminho nem token em reinícios normais. Entregue o arquivo
privadamente, sem colocar seu conteúdo em argumento, URL, log ou config.
Não há diferentes usuários/roles implementados; a credencial tem acesso à API.
Bootstrap emite validade de 30 dias; rotação offline também. Sem renovação/reload
ou ativação de credencial por editar/copiar um arquivo manualmente.

```bash
kiarni healthcheck --data-dir ./kiarni-data
# Opcional: selecionar arquivo específico, nunca passar o valor do token.
kiarni healthcheck --data-dir ./kiarni-data --token-file ./kiarni-data.credentials/token-EXEMPLO.txt
```

O probe usa a mesma configuração de bind/Host, inclusive public_origin. Automaticamente
examina até 64 entradas no diretório privado irmão e tenta arquivos token-ID.txt
regulares de 91 bytes, dono atual, modo 0600 e um link, sem seguir symlinks. Um
candidato inválido faz falhar. `--token-file` evita a descoberta; confirme permissões.
Prazo de rede de 3 s e resposta até 4 KiB; código 0 silencioso, código 1 com
healthcheck_failed. Expiração/revogação gera falha legítima; ele não renova tokens.
[Operação em contêiner](./CONTAINERS.md#healthcheck).

## Falhas locais frequentes

| Código/mensagem | O que conferir |
| --- | --- |
| `missing_config_value`, `duplicate_config_setting`, `invalid_config_value` | Sintaxe em pares, flags repetidas ou texto não UTF-8. |
| `config_file_unavailable` | Caminho, arquivo regular e permissão de leitura. |
| `invalid_configuration` | Campos, tipos, limites e relações nas fontes; inclui valor inválido sobrescrito e KIARNI_* desconhecido. |
| `config_source_too_large` | Tamanho/quantidade dos argumentos ou ambiente antes da resolução. |
| `config_result_below_format_limit` | max_result_bytes inferior a 1049600. |
| `config_buffer_plan_exceeded` | Combinação supera o plano de buffers permitido. |
| `bind_failed` | Porta ocupada, indisponibilidade do endereço ou permissão de rede. |
| `data_directory_locked` | Outro servidor ou comando offline usa essa raiz; não remova locks à força. |
| `insecure_data_directory`, `insecure_data_file` | Dono, permissões, tipo e links do layout existente. |
| `unsupported_auth_command`, `missing_backup_output`, `missing_restore_source` | Comando/opção obrigatória ausente ou não implementado. |
| `healthcheck_failed` | Servidor, configuração/Host, validade e arquivo privado da credencial. Sem detalhes/segredos na saída. |

Algumas validações são agregadas como invalid_configuration antes das verificações
específicas do binário; não dependa de um diagnóstico mais específico em todos os
casos. [Troubleshooting](./TROUBLESHOOTING.md) e guias de backup/restore detalham
falhas de integridade e publicação sem instruir remoção de dados.
