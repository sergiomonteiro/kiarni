# ADR 0030 — HTTPS por proxy local e origem pública explícita

- Estado: aceito para issue #3, M3/RF-07
- Data: 2026-11-14
- Complementa ADRs 0005/0008/0012/0020. Sem mudança de autenticação ou streaming.

## Decisão e alternativas

Manter Kiarni HTTP restrito a endereço numérico loopback, inclusive com configuração
explícita de origem pública. TLS nativo adicionaria dependências, gestão de chaves
e certificados ao binário. O perfil remoto usa proxy HTTPS opcional no mesmo host;
Nginx é a referência a validar. Execução local continua sem serviços obrigatórios.
Não usar o proxy como bypass de autenticação nem remover Origin para fazer o
browser funcionar. API, métricas, dados do dashboard e SSE continuam com Bearer.

Adicionar `public_origin` opcional (CLI `--public-origin`, ambiente
`KIARNI_PUBLIC_ORIGIN`, JSON string), seguindo precedência/validação existentes.
Ausente: Host e Origin HTTP derivam do bind, como hoje. Presente: exigir Host da
origem pública e, se recebido, Origin exatamente igual à origem HTTPS configurada,
inclusive em assets públicos sem dados. Não aceitar simultaneamente Host local.
Ausência de Origin continua permitida para clientes não-browser autenticados.
Forwarded, X-Forwarded-* e IP do peer nunca autenticam, escolhem Host/Origin ou
habilitam um modo confiável. Proxy deve fixar Host canônico e limpar esses headers.

Aceitar somente uma origem `https://HOST[:PORT]`, sem caminho (nem slash final),
query, fragmento, userinfo ou escapes. DNS ASCII, IPv4 estrito ou IPv6 entre
colchetes; porta 1–65535. Normalizar caixa DNS, IPv6 e porta (443 omitida). Recusar
nomes vazios, labels inválidos, ponto final, IPv4 abreviado/numérico ambíguo,
endereços não especificados/multicast, IPv6 mapeado em IPv4 e zonas IPv6. Unicode requer representação
ASCII válida; sem conversão IDNA no servidor. Limite de 272 bytes; sem nova
biblioteca de URL. Essa configuração descreve o frontend; não oferece TLS no backend.

## Perfil de proxy

Somente frontend HTTPS, TLS 1.2/1.3 e certificado válido para o nome escolhido;
upstream fixo em loopback, sem retry de mutações. Encaminhar Authorization e Origin
sem transformar a autenticação, sem cookies/CORS ou token na URL. Recusar Host
externo desconhecido; headers encaminhados pelo cliente são removidos. Nenhum
header consegue habilitar exposição HTTP pública do backend.

Respostas sem cache, compressão ou buffering de proxy para permitir flush de SSE;
requests com buffering limitado a 1 MiB e teto de corpo igual ao formato. Definir
limites explícitos de conexões/taxa/buffers e timeouts. Read timeout do proxy é de
inatividade, maior que o heartbeat máximo de 60 s; não é duração total do stream.
Disconnect deve fechar upstream. Terminal continua best-effort; timeout HTTP não
implica rollback e proxy não deve repetir mutações automaticamente.

Logs do backend continuam tipados. Perfil de access log do proxy inclui somente
status, duração e request ID gerado pelo backend: sem URL, query, payload,
Authorization, IP ou campos livres do cliente. Error log de requests fica
suprimido nesse perfil; diagnóstico detalhado do proxy exige operação consciente
sobre arquivos privados, pois pode incluir request/URI. Não documentar debug com
credenciais. Certificados/chaves permanecem fora do Git; nenhuma chave está no app.

## Fronteira operacional e evidências

HTTP entre proxy/backend é local; processos do mesmo host com token ainda podem
usar o backend com Host público. Não é isolamento contra usuário local malicioso,
firewall, TLS ponta a ponta entre hosts ou suporte a ingress/cluster. Proxy e
Kiarni devem compartilhar o host/namespace de rede loopback deste perfil. Expor
backend em wildcard/contêiner público não é autorizado por public_origin.

Validar parser e precedência, default local, auth e Host/Origin em conexões reais,
spoof de Forwarded, HTTPS com verificação de certificado, dashboard, métricas,
CRUD e dois SSEs com flush/heartbeat/cancelamento. Testar limite, erro e ausência
de token/payload em logs. Registrar ferramentas/ambiente; CI local não é prova de
certificado público, renovação ACME, desempenho de Internet ou homologação de todos
os proxies. Esse recorte não aprova gate M3/release.

Referências primárias: [Nginx proxy](https://nginx.org/en/docs/http/ngx_http_proxy_module.html),
[SSL](https://nginx.org/en/docs/http/ngx_http_ssl_module.html),
[conexões](https://nginx.org/en/docs/http/ngx_http_limit_conn_module.html),
[taxa](https://nginx.org/en/docs/http/ngx_http_limit_req_module.html).
