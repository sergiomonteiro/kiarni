# Operação remota por HTTPS e proxy local

M3/RF-07, issue [#3](https://github.com/sergiomonteiro/kiarni/issues/3),
[ADR 0030](./adr/0030-https-por-proxy-local.md).

O perfil remoto usa um proxy HTTPS opcional no mesmo host/namespace de rede do
Kiarni. O binário continua funcionando sozinho no perfil local. **Kiarni não faz
TLS nativo e continua recusando bind fora de loopback**, mesmo com origem pública.
Este guia não publica nenhum serviço automaticamente.

## Configurar o backend

Escolha um nome DNS e um certificado válido para ele. O exemplo `db.example.test`
é ilustrativo: substitua pelo nome sob seu controle. Execute o Kiarni com origem
pública explícita e mantenha o backend local:

```bash
kiarni serve --data-dir ./data --bind 127.0.0.1:8711 \
  --public-origin https://db.example.test:8443
```

Alternativas: `KIARNI_PUBLIC_ORIGIN` ou string `public_origin` no arquivo JSON.
Precedência permanece CLI > ambiente > JSON > default, com validação de todas as
fontes, inclusive valores sobrescritos. Ausente, Host/Origin derivam do bind HTTP
local como antes. Para voltar ao perfil local, remova a chave de todas as fontes
ativas e reinicie; string vazia, null e origem HTTP não desabilitam essa validação.

O valor admite somente `https://HOST[:PORT]`, sem caminho (nem `/` final), query,
fragmento, credenciais ou escapes. DNS ASCII, IPv4 estrito ou IPv6 entre colchetes,
porta 1–65535; até 272 bytes. Caixa DNS, IPv6 e porta são normalizados; porta 443 é
omitida. IPv4 abreviado/numérico ambíguo, labels inválidos, ponto final,
endereços não especificados/multicast, IPv6 mapeado em IPv4 e zonas IPv6 são
recusados. Não há conversão de Unicode/IDNA no servidor. O perfil de referência
abaixo usa nome DNS com SNI; não declara homologação de todo proxy/endereço IP.

`public_origin` descreve o frontend, não valida certificado nem cria listener TLS.
Backend exige exatamente seu Host canônico; se Origin estiver presente, deve ser
exatamente a origem HTTPS configurada. Isso vale também para os assets públicos.
Host HTTP local deixa de ser aceito nesse modo. Origin ausente continua permitido
para clientes autenticados não-browser. Não há autenticação especial por IP local.

## Proxy Nginx de referência

[Template completo](../deploy/nginx/kiarni.conf.template), validado com Nginx da
distribuição Ubuntu 24.04. Não incluir dentro de outro bloco http: é uma configuração
standalone. Requer os módulos padrão usados no template, incluindo SSL, proxy,
limites e os módulos de temp paths do pacote de referência. Ajustes em outro build
exigem `nginx -t` e a integração correspondente.

Instale/gerencie o proxy e um certificado com cadeia confiável conforme seu ambiente.
Certificado e chave ficam fora do repositório. O processo do proxy precisa ler a
chave privada; clientes mantêm verificação de certificado e hostname habilitada.
Renovação/ACME é responsabilidade operacional, não um recurso do Kiarni.

Este exemplo prepara um prefixo privado novo e substitui todos os placeholders.
Edite nome, porta e caminhos de certificados antes de executar. Os caminhos deste
recorte não usam espaços, aspas ou variáveis Nginx. `LISTEN` é uma exposição
explícita **do proxy TLS**; nunca é um valor de bind aceito pelo Kiarni.

```bash
python3 - <<'PY'
from pathlib import Path
import os
import re

prefix = Path('proxy-run').resolve()
prefix.mkdir(mode=0o700)  # exige destino novo
values = {
    'PREFIX': str(prefix),
    'LISTEN': '0.0.0.0:8443',
    'SERVER_NAME': 'db.example.test',
    'AUTHORITY': 'db.example.test:8443',
    'UPSTREAM': '127.0.0.1:8711',
    'CERT': '/caminho/privado/fullchain.pem',
    'KEY': '/caminho/privado/privkey.pem',
}
text = Path('deploy/nginx/kiarni.conf.template').read_text()
for name, value in values.items():
    assert re.fullmatch(r'[A-Za-z0-9_./:-]+', value), 'Valor inválido no template'
    text = text.replace('@' + name + '@', value)
assert not re.search(r'@[A-Z_]+@', text)
fd = os.open(prefix / 'nginx.conf', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, 'w') as file:
    file.write(text)
PY
nginx -t -p "$PWD/proxy-run/" -c "$PWD/proxy-run/nginx.conf" \
  -e "$PWD/proxy-run/startup.log"
nginx -p "$PWD/proxy-run/" -c "$PWD/proxy-run/nginx.conf" \
  -e "$PWD/proxy-run/startup.log" -g 'daemon off;'
```

O frontend recusa SNI/Host desconhecidos. Host encaminhado ao backend é uma
constante canônica igual à configuração pública. Authorization e Origin seguem
para o app; o proxy não injeta token, apaga Origin, cria cookies ou habilita CORS.
Forwarded, X-Forwarded-For/Host/Proto/Port e X-Real-IP são removidos. O backend
ignora esses campos para confiança, autenticação e seleção de origem.

TLS 1.2/1.3 ficam explícitos, com early data e tickets desabilitados neste perfil.
Não há listener HTTP de redirecionamento. Não transmita token por HTTP, URL,
argumentos, logs ou `curl -v`. Abra o dashboard em `https://db.example.test:8443/`;
a mesma página e API usam HTTPS. O shell de login continua público e sem dados;
leituras administrativas e SSE exigem Bearer.

Para cliente CLI, preserve a validação TLS e passe o header por stdin:

```bash
kiarni_token_file='./data.credentials/token-EXEMPLO.txt'
kiarni_remote() {
  { printf 'Authorization: Bearer '; cat "$kiarni_token_file"; printf '\n'; } |
    curl --silent --show-error --header @- "$@"
}
kiarni_remote https://db.example.test:8443/v1/health/ready
kiarni_remote --no-buffer -H 'Accept: text/event-stream' \
  https://db.example.test:8443/v1/collections/orders/changes
```

Use o arquivo privado real emitido no bootstrap. Uma CA privada deve ser confiada
explicitamente pelo cliente (`--cacert` no curl, ou mecanismo do navegador);
não use `-k`. Não há EventSource com token na URL nem credencial no proxy config.

## Limites, streams e erros

O template limita um worker a 128 conexões de sockets (inclui upstream), até 16
requests ativos por IP e 32 no servidor, taxa de 20 requests/s por IP e burst 40
sem fila de atraso. Limites por IP compartilham a cota entre clientes atrás de NAT.
Não são metas de throughput nem configuração universal. Requisições recusadas por
conexão/taxa recebem 429. As zonas de limites e sessão TLS têm tamanho explícito.
Buffers/caches do proxy são adicionais ao orçamento do processo Kiarni.

Corpo máximo de 1 MiB, buffering de request em até 1 MiB por request e headers
limitados. O app continua aplicando seus tetos menores de coleção/query e limites
de admissão/stream. Requests com transferência chunked são normalizados pelo
proxy com buffering de request; resposta não deve ser bufferizada para SSE.
O perfil não usa cache, gzip, spool de respostas ou retry de upstream.

[Buffering desabilitado permite encaminhar respostas conforme chegam; read timeout
mede inatividade entre leituras, não a vida total do stream](https://nginx.org/en/docs/http/ngx_http_proxy_module.html).
O valor de 75 s fica acima do heartbeat máximo configurável do Kiarni (60 s).
Timeouts de headers/corpo/escrita do proxy são de 10 s de inatividade; conexão ao
upstream tem 2 s. Os prazos absolutos internos do app continuam independentes.
Cliente lento/cancelamento não cria fila ilimitada no writer. O teste inclui
assinante HTTPS sem consumir enquanto outras escritas são confirmadas.

Timeout ou desconexão HTTP não prova rollback. Não repetir mutações automaticamente;
um commit pode ter sido confirmado mesmo sem resposta ao cliente. Feeds permanecem
futuros, best-effort, sem replay/reconexão automática; terminal pode não chegar em
socket bloqueado. Consulte novamente antes de assinar após interrupção.

Erros gerados pelo app preservam JSON, códigos e request IDs. Erros anteriores ao
app (TLS, Host do frontend, limites do proxy) podem ser respostas Nginx, sem JSON
ou request ID. Métricas/conexões do backend contam upstreams, não clientes únicos.

## Logs e fronteira de confiança

Access log do template contém somente status, duração e request ID retornado pelo
backend. Não registra URI, query, headers, IP, payload ou método livre do cliente.
Error log de requests fica suprimido em `/dev/null`; startup/syntax usa arquivo
privado no prefixo. Isso reduz diagnóstico detalhado do proxy deliberadamente.
Investigações com outro formato/debug exigem tratar saída como sensível; não
habilitar dumps com credenciais. Logs tipados do Kiarni permanecem disponíveis.
Retenção e rotação de `access.log` são responsabilidades do operador; o template
não instala um serviço de rotação nem define retenção durável.

O proxy termina TLS e vê credenciais/dados em trânsito: deve ser confiável.
Backend HTTP permanece local; processo do mesmo host com token e Host público
pode acessá-lo diretamente. O desenho não isola usuários locais maliciosos,
não oferece TLS entre hosts nem permite backend HTTP em rede pública.
Kiarni/proxy precisam compartilhar o mesmo host/namespace de loopback; dois
contêineres em rede bridge não compartilham 127.0.0.1. Não publicar a porta do
backend nem deduzir suporte a ingress/cluster a partir deste exemplo.

## Verificação reproduzível

```bash
cargo build -p kiarni --locked
node scripts/https-proxy.test.mjs
```

Dependências de teste: Nginx, OpenSSL e Playwright fixado em `spikes/transport`.
`KIARNI_TEST_NGINX=/caminho/nginx` escolhe um executável extraído sem instalar
serviço. O servidor de teste recebe ambiente limpo, sem essa variável privada.
O job HTTPS da CI baixa o pacote autenticado da distribuição, extrai em diretório
temporário e usa o template do repositório, sem instalar serviço persistente.

Fixture cria CA/chave/certificado temporários, valida cadeia e nome no cliente
Node, recusa CA não confiável e SNI errado e exercita TLS 1.2/1.3. Chromium usa
um perfil isolado com uma exceção específica para a chave pública temporária via
[allowlist SPKI do Chromium](https://chromium.googlesource.com/chromium/src/+/HEAD/services/network/ignore_errors_cert_verifier.h);
a validação normal permanece para as demais chaves. Essa exceção de teste
não valida PKI pública nem é configuração de produção. Não há ignoreHTTPSErrors ou bypass global de certificado no teste.

São exercitados API, dashboard/consultas/métricas no navegador, auth ausente,
Host/Origin inválidos/vazios/duplicados, spoof de headers, dois SSEs, flush de
heartbeat, cancelamento, consumidor parado, payload excessivo e recusa por limite.
Logs são examinados para ausência de token, payload e marcadores de query/header.
Todos os processos e arquivos de teste são encerrados/removidos ao terminar.

Perfil validado localmente em Linux com Nginx 1.24.0-2ubuntu7.19 (Ubuntu), OpenSSL
3.0.13 e Chromium do Playwright fixado. Isso não comprova certificado público,
renovação, operação prolongada, compatibilidade com todos os proxies, macOS/Nginx
ou desempenho de Internet. CI Rust mantém verificações Linux/macOS separadas;
o M3/release continuam abertos para distribuição e validação operacional.

Referências: [SSL Nginx](https://nginx.org/en/docs/http/ngx_http_ssl_module.html),
[limites de conexões](https://nginx.org/en/docs/http/ngx_http_limit_conn_module.html),
[limites de taxa](https://nginx.org/en/docs/http/ngx_http_limit_req_module.html).

Validação local em 2026-11-15, Linux x86_64, Rust/Cargo 1.99.1: fmt, clippy,
build release, fronteira de nove pacotes e 286 testes Rust com/sem profiling
passaram (quatro workers ignorados invocados pelos pais). A integração real
Nginx/OpenSSL/Chromium também passou, incluindo corpo chunked normalizado e
logs privados em recusas do proxy. Testes TCP/browser precisam de permissão de
loopback. CI do commit publicado é evidência separada da execução local.
