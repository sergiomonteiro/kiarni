# Experimentar a alpha reativa local

O workspace já permite experimentar banco JSON persistente e changefeeds de
coleção. É uma alpha local: não é uma release publicada nem o MVP completo.
Métricas HTTP, de consultas, storage e feeds estão em `/v1/metrics`; o
[dashboard local](./DASHBOARD.md) inclui erros recentes. Validação de
operação/distribuição continua no roadmap. A avaliação de CPU/memória e a regressão de leitura registrada
também permanecem pendentes. Não exponha este perfil loopback como serviço público.

## 1. Iniciar

Na raiz do repositório, no primeiro terminal:

```bash
cargo build -p kiarni --release --locked --offline
./target/release/kiarni serve --data-dir ./data-alpha
```

O processo informa o endereço e, na primeira inicialização, o caminho do arquivo
de credencial. Guarde esse caminho: ele não é reemitido em reinícios normais.
O diretório pai precisa existir. Ctrl+C encerra o servidor; dados confirmados
permanecem na mesma raiz para a próxima execução.

## 2. Preparar o cliente

Nos terminais usados para chamadas, ajuste o caminho e defina esta função.
Ela passa a credencial ao curl por stdin, sem token nos argumentos ou na URL:

```bash
token_file='./data-alpha.credentials/token-EXEMPLO.txt'
kc() {
  { printf 'Authorization: Bearer '; cat "$token_file"; printf '\n'; } |
    curl --silent --show-error --header @- "$@"
}
```

Use o nome real exibido no startup, não o placeholder. O arquivo deve continuar
privado. Não use `curl -v` com credenciais.

## 3. Criar coleção e observar

No segundo terminal, com a função definida:

```bash
kc -H 'Content-Type: application/json' --data '{"name":"orders"}' \
  http://127.0.0.1:8711/v1/collections

kc --no-buffer -H 'Accept: text/event-stream' \
  http://127.0.0.1:8711/v1/collections/orders/changes
```

O stream começa com `event: ready`. Isso confirma inscrição, **não** um snapshot.
Abra outro stream em outro terminal para observar as mesmas alterações.

## 4. Alterar documentos

Em outro terminal, repita a preparação da função e execute:

```bash
kc -H 'Content-Type: application/json' \
  --data '{"_id":"first","status":"new","total":9007199254740993}' \
  http://127.0.0.1:8711/v1/collections/orders/documents

kc -X PATCH -H 'Content-Type: application/merge-patch+json' \
  --data '{"status":"paid"}' \
  http://127.0.0.1:8711/v1/collections/orders/documents/first

kc -X DELETE http://127.0.0.1:8711/v1/collections/orders/documents/first
```

Os streams recebem `event: change` para insert, update e delete, somente após
commit confirmado. Os dados incluem type, collection, id, sequence, before e
after. sequence é string decimal local ao processo; números de documentos são
preservados como tokens JSON. Clientes JavaScript que precisam de precisão
integral devem usar parsing lossless dos documentos.

PATCH vazio/no-op e operações rejeitadas não geram change. Uma consulta por ID
usa GET na mesma URL de documento; consultas AST usam POST em `/query`.

## Limites da experiência

- O stream usa Bearer, Host/Origin da mesma origem e Accept `text/event-stream`.
- Last-Event-ID é recusado; não há campos SSE id/retry, replay ou reconexão automática.
- Nova conexão recebe apenas mudanças futuras. EOF não comprova encerramento
  limpo; estado derivado pode estar incompleto. Releitura e nova assinatura não
  constituem um protocolo de snapshot atômico.
- Fila cheia termina como overflow; shutdown, expiração ou interrupção encerram
  o stream. stream_end é best-effort e pode dar lugar a EOF, especialmente após
  frame parcial ou consumidor lento. O dado confirmado não é desfeito.
- Defaults: 32 conexões totais, até oito streams, fila quatro, heartbeat 15 s e
  prazo de escrita de 5 s por frame. Streams também ocupam conexões totais.
- Um frame tem teto de 3 MiB codificados. Se whitespace expandir o framing além
  do teto, o stream termina interrupted; o commit permanece.
- Rotação/revogação de credenciais continuam offline, com servidor parado;
  expiração é revalidada durante o stream. Não existe reload automático de auth.

Os testes locais usam processos reais, dois streams, escritas concorrentes,
desconexão, expiração, consumidor que não lê e shutdown. Não houve teste de
proxy/TLS, exposição remota. O dashboard tem teste Chromium real para autenticação,
consultas e SSE de coleção do produto. Não há promessa de replay,
exactly-once, queda de energia ou falha física.

Para acompanhar assinaturas, backlog e encerramentos, consulte as
[métricas de changefeeds](./CHANGEFEED_METRICS.md).

## Dashboard inicial

Abra a raiz do endereço informado pelo servidor para autenticar, consultar health
e listar coleções. Selecione uma coleção para consultar documentos com limites.
O painel também acompanha mudanças da coleção, sem atualizar a consulta
automaticamente. O botão Atualizar métricas mostra HTTP, consultas, storage e feeds. Os tempos de query
(total, espera e execução) ficam em seções expansíveis, separados de respostas
HTTP, streams encerrados e distribuição interna. Storage tem resumo de commits/
grupo/arquivo e um seletor de primitivas que reutiliza a coleta atual.
Atualizar status informa versão, uptime HTTP e ocupação das vagas de conexão.
Contar documentos informa total exato dentro do orçamento ou um limite inferior.
A interface continua sem edição de documentos.
[Uso e limites do dashboard](./DASHBOARD.md).

## Logs de requisições

Use `--log-level off|error|warn|info` (default info). Os registros JSON em
stderr permitem correlacionar request ID, operação e duração sem payloads ou
credenciais. [Semântica e limites de saída](./REQUEST_LOGS.md).

## Backup offline

Com o servidor parado, `kiarni backup --data-dir ./data --output ./backups/primeiro`
exporta um artefato consistente para destino novo. Prepare antes um pai privado
para backups. [Comando, segurança, integridade e interrupções](./BACKUP.md).
Use `kiarni restore --from BACKUP --data-dir NOVA_RAIZ` para preparar uma
instalação independente. [Restauração, novas credenciais e interrupções](./RESTORE.md).
Não inicie o servidor diretamente no artefato.
