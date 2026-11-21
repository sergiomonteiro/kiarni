# Primeiro documento, consulta e mudança ao vivo

Este roteiro usa o pacote **experimental** distribuído, sem compilar Rust ou
instalar serviços do banco. É uma validação da alpha, não uma release estável.
Clientes dos exemplos precisam de Bash, curl e cat; o servidor precisa somente
das bibliotecas do sistema da plataforma indicada em BUILD_INFO.json.

## 1. Obter e iniciar o pacote

Escolha um artifact nativo de uma execução verde do CI, confira commit/target e
verifique seu `.tar.gz.sha256` **antes** de extrair num diretório novo:

```bash
# Linux; substitua ARQUIVO pelo nome completo recebido.
sha256sum -c ARQUIVO.tar.gz.sha256
# No macOS, no lugar do comando anterior:
# shasum -a 256 -c ARQUIVO.tar.gz.sha256
tar -xzf ARQUIVO.tar.gz
cd ARQUIVO
# No macOS, use shasum -a 256 -c CHECKSUMS.sha256.
sha256sum -c CHECKSUMS.sha256
./kiarni --version
./kiarni serve --data-dir ./data
```

Hashes detectam corrupção, não identidade. Obtenha pacote e checksum da origem
confiável. No macOS, artifacts sem notarização podem ser recusados por Gatekeeper;
o CI não homologa download pelo navegador/quarentena. Não desative proteções para
seguir este roteiro. Limites e obtenção:
[distribuição](https://github.com/sergiomonteiro/kiarni/blob/main/docs/DISTRIBUTION.md).

Execute serve no primeiro terminal. O pai dos dados já existe: é o diretório
extraído. Guarde o **caminho** de credencial informado no primeiro boot, por exemplo
`./data.credentials/token-EXEMPLO.txt`; o nome real é gerado e não deve ser adivinhado.
Bind padrão é 127.0.0.1:8711. Abra essa URL para ver o dashboard embutido.

## 2. Preparar o cliente e a assinatura

No segundo terminal, entre no mesmo diretório extraído. Ajuste o caminho real:

```bash
token_file='./data.credentials/token-EXEMPLO.txt'
source examples/client.sh
```

A função kc transmite Bearer por stdin, sem token em argumento, URL ou ambiente.
Não use set -x, curl -v ou publique esse arquivo. Ela desabilita configuração
pessoal do curl e ignora proxy para loopback. Os nomes/dados abaixo são fixtures
públicas, não credenciais ou documentos reais.

Crie a coleção, que ainda deve estar ausente. A resposta será 201, `{"name":"orders"}`:

<!-- verify:quickstart -->
```bash
kc -H 'Content-Type: application/json' --data-binary @examples/collection.json "$api_base/v1/collections"
```

Ainda no segundo terminal, abra a assinatura e aguarde `event: ready`:

```bash
kc --max-time 600 --no-buffer -H 'Accept: text/event-stream' "$api_base/v1/collections/orders/changes"
```

Ready diz `snapshot:false,replay:false`: não lista documentos anteriores. O prazo
de 600 s é do cliente, não um contrato de duração do servidor. Deixe esse terminal
aberto enquanto faz as operações seguintes.

## 3. Inserir, consultar e alterar

Num terceiro terminal, entre no mesmo diretório e repita a preparação de token_file
e source. Insira o documento (201):

<!-- verify:quickstart -->
```bash
kc -H 'Content-Type: application/json' --data-binary @examples/document.json "$api_base/v1/collections/orders/documents"
```

A assinatura recebe insert com after, ID first e sequence em string. O total
`9007199254740993` deve permanecer exato; não o converta para float em clientes.
Execute a consulta (200, count 1, somente `_id`, status e total):

<!-- verify:quickstart -->
```bash
kc -H 'Content-Type: application/json' --data-binary @examples/query.json "$api_base/v1/collections/orders/query"
```

O corpo usa from, filter, sort, limit e project. Limit é obrigatório e um valor
pequeno não amplia o orçamento da coleção. Índice secundário existente pode
reduzir candidatos quando elegível/seletivo; [administração de índices é offline](https://github.com/sergiomonteiro/kiarni/blob/main/docs/INDEXES.md). A consulta é pontual, não reconciliada automaticamente
pelo feed. Agora altere status para shipped e remova optional (200):

<!-- verify:quickstart -->
```bash
kc -X PATCH -H 'Content-Type: application/merge-patch+json' --data-binary @examples/patch.json "$api_base/v1/collections/orders/documents/first"
```

O stream recebe update com before/after. Eventos só são publicados após commit
confirmado. Não há snapshot+assinatura atômicos, replay ou exactly-once. Fila cheia
encerra consumidor atrasado; EOF/interrupção exige reconciliar por leitura.

## 4. Encerrar e retomar

Ctrl+C no cliente fecha a assinatura. Ctrl+C no primeiro terminal encerra o
servidor; SIGTERM também é aceito. Iniciar novamente com a mesma data-dir preserva
documentos confirmados e credenciais. Não remova arquivos privados, locks ou dados
para resolver erros. Para uma segunda execução deste roteiro, use uma instalação
nova de teste ou adapte a coleção: criar orders novamente retorna 409.

Depois do quickstart: [API](https://github.com/sergiomonteiro/kiarni/blob/main/docs/API.md),
[CLI](https://github.com/sergiomonteiro/kiarni/blob/main/docs/CLI.md),
[diagnóstico](https://github.com/sergiomonteiro/kiarni/blob/main/docs/TROUBLESHOOTING.md)
e [backup](https://github.com/sergiomonteiro/kiarni/blob/main/docs/BACKUP.md).
O pacote fixa esses links no commit de origem.

A automação mede desde o arquivo local disponível até verificar hashes, extrair,
copiar/iniciar o binário e obter documento, consulta e evento real. O limite é
600 segundos. Não inclui download/autenticação no GitHub, instalação de ferramentas,
quarentena macOS ou tempo de leitura humana. Esses limites de evidência devem
acompanhar qualquer alegação sobre o quickstart de até dez minutos do PRD.
