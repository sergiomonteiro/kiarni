# Servidor local do M1

`kiarni serve --data-dir ./data` inicia o servidor em Linux/macOS. O diretório
pai deve existir. A raiz do servidor deve pertencer ao usuário do processo e
ter modo 0700. O startup guarda o store em `data/store/` e os verificadores em
`data/auth/`. A entrega inicial usa um arquivo 0600 em `data.credentials/`.
Leia o caminho na saída do primeiro startup e guarde o arquivo em local privado.

Precedência por campo é CLI, ambiente `KIARNI_*`, JSON e defaults. Selecione o
arquivo com `--config caminho.json`. O bind aceita endereço numérico loopback
com porta explícita e usa `127.0.0.1:8711` por padrão. A reserva de buffers usa
o teto do formato mesmo quando o limite de escrita for menor.

Em outro terminal, forneça o caminho exibido no startup. O exemplo envia a
credencial pelo stdin do curl, sem inseri-la nos argumentos do processo:

```bash
token_file='./data.credentials/token-EXEMPLO.txt'
{ printf 'Authorization: Bearer '; cat "$token_file"; printf '\n'; } |
  curl --header @- http://127.0.0.1:8711/v1/health/ready
```

O lock da raiz dura até o fim do servidor. O registro de bootstrap permite
retomar um store já criado ou carregar auth já confirmada. Store incompleto ou
incompatível e auth ausente após a instalação exigem diagnóstico. As rotas de
health ficam disponíveis após abrir e validar o store e carregar auth. O HTTP
local atende uma requisição por conexão, com limite de cabeçalho, prazo de
leitura/escrita e vagas concorrentes. Quando os workers estão ocupados, a escuta aguarda uma vaga e aplica
backpressure pelo backlog do SO. SIGINT/SIGTERM interrompem a espera por novos requests, fecham conexões
e aguardam os workers antes de soltar o lock. [ADRs 0009 e 0011](./adr/README.md)

Readiness consulta os metadados e tabelas do store a cada requisição.
Erro retorna 503/storage_unavailable. Leituras usam snapshots, permitindo que
readiness avance durante uma escrita. Liveness responde sobre o processo.

O servidor compõe bibliotecas nas pastas server, config, storage, auth, http,
core e engine dentro de crates. Os comandos locais de autenticação permitem
recuperar acesso após expiração ou revogação.

`cargo test -p kiarni --locked` usa subprocessos e sockets loopback. Os testes
montam estados persistidos intermediários para exercitar a retomada. O teste de
kill abrupto ocorre depois da ativação do servidor, enquanto os testes
anteriores de auth exercitam pontos internos do journal isoladamente.

## Coleções

As operações usam a mesma autenticação Bearer e o mesmo Host do health.

| Método | Rota | Resposta |
|---|---|---|
| POST | /v1/collections | 201 com nome e Location |
| GET/HEAD | /v1/collections?limit=100&after=nome | 200 com collections e next_after |
| DELETE | /v1/collections/nome | 204 se vazia |

POST recebe JSON `{"name":"orders"}`, Content-Type application/json e
Content-Length, com corpo até 1 KiB. A leitura do corpo compartilha o prazo
absoluto com o cabeçalho. Corpo incompleto expira antes de executar a mutação.
Transfer-Encoding e Content-Encoding são recusados neste perfil HTTP.

Nomes seguem ASCII `[A-Za-z0-9_-]`, de 1 a 128 bytes. Nome duplicado retorna
409, nome ausente retorna 404 e exclusão de coleção não vazia retorna 409.
O coordenador serializa mutações e admite espera de até 100 ms antes de iniciar
a operação, em ordem FIFO. Esgotamento retorna 503/write_busy. A espera ocupa uma vaga de
conexão já admitida. Um erro de transporte após commit pode deixar resultado desconhecido.

PUT/PATCH consecutivos podem compartilhar um commit Immediate, sem esperar para
formar lote. Cada resposta mantém o JSON daquela operação. Inserções, exclusões
e catálogo são barreiras na mesma fila. A permissão da conexão acompanha o comando
e o recibo até o descarte dos buffers; um consumidor lento não bloqueia o worker.
Timeout antes do início remove o comando; commit iniciado não é cancelado pelo
prazo de admissão. O encerramento cancela pendências e conclui grupos iniciados.
[ADR 0017](./adr/0017-integracao-coordenador-de-escrita.md).

A listagem exige limit de 1 a 1000. after é opcional e exclusivo, com a mesma
validação de nomes. A ordem é lexical crescente. next_after contém o último
nome retornado quando há outra página. Cada página usa um snapshot próprio.
Nomes removidos entre requests continuam válidos como limite lexical do cursor.
Parâmetros duplicados, desconhecidos e escapes percentuais são recusados.
HEAD preserva os headers da representação JSON e envia corpo vazio.

Os testes HTTP exercitam paginação, conflitos, exclusão, concorrência,
autenticação, corpo fragmentado/incompleto e persistência de coleções depois
de kill/restart. A coleção não vazia usada no teste é preparada offline pelo
adapter de storage.

## Inserção e leitura de documentos

POST em `/v1/collections/orders/documents` recebe objeto JSON com
Content-Type application/json e Content-Length. A coleção deve existir.
O servidor devolve 201 com o documento completo e Location após commit
confirmado. Um `_id` explícito deve ter 1 a 128 caracteres ASCII
`[A-Za-z0-9_-]`. Se ausente, o servidor gera 32 dígitos hexadecimais usando
128 bits aleatórios. Colisões de IDs gerados têm até oito tentativas.
ID explícito duplicado retorna 409 sem sobrescrever o documento anterior.

GET e HEAD em `/v1/collections/orders/documents/{id}` retornam o documento
por chave primária. Coleção ausente e documento ausente retornam 404 com
códigos distintos. O índice é composto pela identidade interna da coleção
e pelo ID, permitindo o mesmo ID em coleções diferentes.

O limite de escrita é aplicado ao corpo antes da alocação e ao documento final
depois da geração de ID. JSON inválido, chaves duplicadas e identidade inválida
são recusados. Números extensos e expoentes são preservados como JSON bruto.
GET continua aceitando documentos persistidos até o teto de formato de 1 MiB
quando o processo reinicia com limite menor. O adapter confere que o ID
persistido corresponde à chave. Os testes cobrem IDs explícitos/gerados,
colisões injetadas, erros, limite final, concorrência e kill/restart.

## Substituição, Merge Patch e exclusão

PUT em `/v1/collections/orders/documents/{id}` substitui o documento existente.
Se o corpo omitir `_id`, o servidor usa o ID da rota. Divergência retorna
422/id_mismatch. Documento ausente retorna 404.

PATCH na mesma rota usa Content-Type application/merge-patch+json e objeto na
raiz. Objetos mesclam recursivamente, null remove membros e arrays são
substituídos inteiros. Qualquer `_id` na raiz retorna 422/immutable_id,
inclusive null ou o mesmo ID. Campos `_id` aninhados seguem as regras comuns.
Para gravar um null literal como membro, use PUT.

PUT/PATCH retornam 200 com o documento completo. A comparação de no-op ignora
espaços, ordem dos membros e escapes equivalentes de strings. Ordem de arrays
e representação numérica contam como alteração. No-op devolve os bytes
persistidos e aborta a transação. Uma alteração efetiva confirma a transação
antes da resposta. Os testes injetam falha antes do commit para verificar rollback
e exercitam patches concorrentes sobre campos diferentes sem perda de atualização.

Corpo e resultado alterado precisam caber no limite vigente. Com limite menor
após restart, PATCH vazio pode devolver documento antigo acima desse limite.
Um PATCH que ainda produza documento grande retorna 413 e preserva o anterior.

DELETE na mesma rota confirma a remoção e retorna 204 com corpo vazio.
Documento ausente retorna 404. Após excluir o último documento, a coleção
pode ser excluída. Os testes verificam updates e exclusões após kill/restart.

## Métricas de changefeeds

GET/HEAD `/v1/metrics` usa a mesma autenticação e retorna Prometheus texto.
Não aceita query ou corpo. [Semântica](./CHANGEFEED_METRICS.md).

## Streams de coleção (SSE)

O stream de coleção está disponível em
`GET /v1/collections/{collection}/changes`, com Bearer e Accept
`text/event-stream`. Não aceita corpo, query ou Last-Event-ID. Começa com ready
após registrar a assinatura e envia changes confirmados, heartbeats e terminais
best-effort, sem snapshot/replay. [Roteiro da alpha local](./ALPHA_LOCAL.md).

## Consultas sobre documentos persistidos

POST em `/v1/collections/orders/query` recebe AST JSON com Content-Length.
O campo from deve coincidir com a coleção da rota e limit deve estar entre
1 e 1000. Corpo até 16 KiB, sujeito ao limite de query configurado.

```json
{
  "from": "orders",
  "filter": {"op": "gt", "field": "total", "value": 100},
  "sort": [{"field": "total", "direction": "desc"}],
  "limit": 20,
  "project": ["total", "status"]
}
```

A resposta 200 tem documents e count. A projeção preserva _id. Números JSON
mantêm sua representação e a comparação decimal é exata. A AST também aceita
id para lookup primário. Documento ausente nesse lookup produz lista vazia.
Cada execução usa um snapshot. Escritas posteriores aparecem na próxima query.

O primeiro executor examina até 10.000 documentos e 8 MiB de JSON por scan.
Exceder esses limites retorna 422/query_too_complex. Use id para lookup direto
quando a identidade já for conhecida. Cada servidor admite uma query ativa,
com espera FIFO limitada a 100 ms e resposta 503 se o prazo esgotar. O limite configurado de resultado inclui o
envelope. Erros de comparação, scan e tamanho encerram a consulta sem resultado
parcial. Os testes cobrem composição, precisão numérica, projeção, erros,
orçamentos e consultas após kill/restart.

## Administração de credenciais

Encerre o servidor com SIGINT ou SIGTERM antes de executar os comandos.

```bash
cargo run -p kiarni --locked -- auth rotate --data-dir ./data
cargo run -p kiarni --locked -- auth revoke-all --data-dir ./data
```

rotate cria um token válido por 30 dias e invalida os tokens anteriores
na seleção persistida. O comando informa o caminho do arquivo privado novo.
Use essa credencial ao iniciar novamente o servidor.

revoke-all invalida todas as credenciais. O servidor reiniciado exige
autenticação e recusa os tokens anteriores. Para restabelecer acesso,
pare o servidor, execute rotate e inicie o servidor com a nova credencial.
O startup conserva a seleção de autenticação, inclusive revogada ou expirada.

Os comandos usam a mesma precedência CLI/env/arquivo de serve e exigem
instalação concluída. Enquanto serve mantém o lock, a administração retorna
data_directory_locked. A saída contém somente caminho de entrega e epoch,
ou confirmação da revogação. Arquivos de tokens anteriores ficam no diretório
de entrega sob controle do operador.

Os testes verificam recusa durante serve, preservação de tokens em startup
revogado/expirado, rotação, revogação e recuperação após restart. O token
expirado é preparado pelo journal com um prazo no passado.

## Organização do produto

O binário e os testes HTTP ficam em apps/kiarni. A implementação do servidor
fica em crates/server. Storage, engine, core, auth, config e http são
bibliotecas próprias em crates. As dependências de runtime do produto ficam
inteiramente nessa árvore. Os harnesses de crash e transporte em spikes usam
as bibliotecas promovidas. [ADR 0014](./adr/0014-organizacao-do-workspace.md)
