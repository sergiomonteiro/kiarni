# 0008 — Tokens locais, rotação e revogação

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md))
- Data: 2026-11-10
- Requisitos e marco: M0; RF-07, preparação para RF-06/RF-08/RF-09.

## Contexto e alternativas

O ADR 0005 propõe Bearer em Authorization, e o ADR 0007 protege todas as rotas da
API. Falta definir provisionamento e lifecycle sem serviço externo obrigatório.
Alternativas: senha humana, token aleatório compartilhado, JWT, sessão de cookie
ou provedor externo. Propor tokens aleatórios opacos com verificação local: não
há necessidade de usuários, claims distribuídas, OAuth ou sessões persistentes.

## Decisão proposta para o experimento

- Token: `kiarni_v1_` + ID público de 8 bytes em hex + `_` + segredo aleatório
  de 32 bytes em hex. Fonte: getrandom/SO, sem fallback para relógio ou contador.
  São 256 bits aleatórios de segredo; o ID público não autentica nada.
- O servidor armazena SHA-256 do token completo, ID, início de validade e expiração.
  Hash não substitui entropia: o mecanismo não aceita senhas humanas. Primitivas
  [SHA-2](https://docs.rs/sha2/),
  [comparação de tamanho fixo](https://docs.rs/subtle/latest/subtle/trait.ConstantTimeEq.html)
  e [zeroização](https://docs.rs/zeroize/) são fornecidas por bibliotecas.
- Manter até 2 verificadores. Comparar todos os candidatos de tamanho fixo com
  subtle, sem encerrar o loop no primeiro hash igual. Isso não promete autenticação
  inteira em tempo constante; parsing, relógio, locks e configuração não o são.
- Aceitar exatamente um header Authorization, esquema Bearer case-insensitive e
  um espaço, token canônico ASCII. Credencial ausente, duplicada, malformada,
  desconhecida, expirada ou revogada retorna o mesmo unauthorized/401.
- Sem token em URL/cookie/subprotocolo; sem credenciais em logs, Debug, exemplos,
  stdout, HTML ou localStorage. Tipos de segredo não têm Display/Serialize/Clone.
  Zeroizing reduz retenção dos buffers próprios, sem prometer limpeza de cópias
  externas, registradores, swap, core dumps ou buffers do futuro transporte.
- Lifetime explícito de 1 s até 30 dias; início inclusivo, expiração exclusiva.
  Não congelar default de CLI ainda. `now` é injetado em segundos Unix para testes;
  servidor futuro deve obtê-lo de relógio confiável, nunca do request.
- Rotação gera novo token, incrementa epoch e pode manter um token anterior por
  até 300 s, limitado à expiração original. Não estender validade do token antigo.
  Mais de um token ativo impede nova sobreposição; rotação com grace zero substitui
  todos imediatamente. Colisões de ID têm até 8 tentativas; falha de entropia aborta.
- Revogação local de todos os tokens publica keyset vazio com epoch incrementado.
  Recuperação é local: gerar novo token por rotação grace zero. Não criar endpoint
  remoto de administração de credenciais no MVP.
- Epoch deve ser crescente. Reload idempotente do mesmo conteúdo/epoch é permitido;
  rollback de epoch ou conteúdo diferente com mesmo epoch é recusado. Reload
  inválido torna a autoridade indisponível, sem fallback silencioso ao keyset antigo.
- Sessão autenticada retém somente ID/fingerprint, não token. Revalidar contra o
  keyset atual e a expiração invalida sessões antigas após aplicar revogação.
  Tokens antigos mantidos no grace continuam válidos até seu prazo.
- Relógio observado não pode retroceder durante a instância: recusar autenticação
  quando now < máximo já observado. Isso não protege contra rollback de relógio
  e arquivos após reinício nem contra o administrador que controla o host.

## Arquivos e ativação

- Implementar somente Unix (alvos Linux/macOS atuais). Diretório existente, dono
  igual ao UID efetivo, modo 0700; arquivos regulares 0600, mesmo dono e um link.
  Abrir o diretório sem seguir symlink final e operar com openat/NOFOLLOW ancorado
  no descriptor. Não há unsafe no código do projeto.
- Token em claro é escrito somente em arquivo novo 0600 em diretório privado
  separado para o operador/cliente. Mesmo inode de diretório é recusado; o
  diretório do servidor recebe somente verificadores. Isso não isola processos
  executados sob o mesmo UID; backups e entrega do arquivo do cliente precisam
  preservar sua confidencialidade.
  Snapshots de verificadores `auth-{epoch}.json` são novos, até 4 KiB, versionados
  e contêm hashes/metadados, nunca segredo. Não sobrescrever arquivos existentes.
- Escrita: token primeiro, snapshot depois, sync dos arquivos e diretório antes
  de retornar sucesso. Falha pode deixar token órfão ou snapshot parcial; load de
  snapshot parcial deve falhar fechado. Não há transação atômica entre arquivos,
  seleção automática da maior epoch ou ponteiro ativo persistido.
- Ativação é explícita: carregar o snapshot escolhido, validar e aplicar reload.
  O protótipo não implementa comandos `kiarni auth`, watcher, publicação atômica
  de configuração ativa ou sincronização durável entre CLI e processo servidor.
  Esses itens e sua recuperação após crash são pendências antes do uso operacional.
- Rotação/revogação efetivas no modelo começam no reload. O futuro servidor deve
  observar configuração e revalidar streams antes de emitir e em timer limitado;
  propor intervalo de 1 s a testar. Não alegar revogação de rede instantânea.

## Rede e fronteiras de segurança

Bind permanece 127.0.0.1, sem bypass de auth por peer loopback ou headers de proxy.
Acesso remoto requer HTTPS por proxy configurado e autenticação. O framework,
origem/Host permitidos, headers encaminhados, CSP do dashboard, rate limits,
provisionamento da CLI e políticas de logs ainda exigem integração/testes.

Métricas e endpoints sensíveis compartilham os controles. O HTML estático de login
pode ser servido sem segredo, mas dados/administração requerem Bearer; assets e
API não podem embutir tokens. Sem cookies, permissões por coleção ou multi-tenancy.

Revogar credencial não desfaz requests/commits já aceitos, nem recolhe bytes já
enviados ao cliente. Streams revogados devem encerrar como interrupted do ADR 0005;
nova conexão sem credencial válida recebe 401 antes de alocar assinatura.

## Evidências e limites

O spike `spikes/auth` deve testar formato, sintaxe de Authorization, expiração,
rotação/grace, revogação, sessão, reload inválido/rollback, arquivo privado e
falhas de provisionamento. Sem sockets, TLS, browser ou integração de logs.
Unidade de teste não prova timing constante, entropia, resistência a XSS ou
durabilidade frente a queda de energia. Não houve auditoria de segurança.
Resultados e limites estão no [relatório do spike](../spikes/auth.md). Nenhum
ADR foi marcado aceito por esta implementação experimental.

## Incremento M0-F2

O [ADR 0009](./0009-bootstrap-e-ativacao-auth.md) propõe uma camada separada sobre
esse modelo: bootstrap local, journal/seleção ativa e recuperação após kill/restart.
O [relatório](../spikes/auth-activation.md) registra evidências. O helper não é o
servidor de produto; esse incremento não altera o status de proposta deste ADR.
