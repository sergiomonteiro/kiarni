# 0009 — Bootstrap mínimo e ativação persistida de auth

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md))
- Data: 2026-11-10
- Requisitos: M0-F2/G3, RF-01/RF-07. Complementa ADR 0008 sem aceitá-lo.


Atualização do formato atual (2026-11-21): O protocolo de fases e o journal de auth permanecem. Conforme ADR 0034, o
registro de bootstrap da instalação atual é JSON estrito versão 2, com
store_format 2 e phase, limitado a 1 KiB; o caminho legado foi removido. Não
altera as versões próprias dos registros do journal de autenticação.

## Proposta e alternativas

Alternativas: exigir provisionamento prévio com argumento adicional; regenerar
token em todo startup; escolher o snapshot de maior epoch; ou bootstrap local com
intenção persistida e seleção explícita. Propor a última opção para preservar
`kiarni serve --data-dir ./data`, sem serviço externo ou segredo em stdout.

Layout experimental: `data/auth` guarda verificadores/metadados; `data.credentials`
é irmão de data e guarda somente a entrega ao cliente. Ambos privados, mesmo UID,
sem symlink final. Caminhos são canonicalizados e os diretórios não podem conter
um ao outro; rejeitar diretório existente inseguro sem alterar suas permissões.
Opcionalmente o operador pode escolher outro destino privado fora dos dados.
O exemplo do spike simula serve, não altera o binário kiarni nem abre rede.

No primeiro startup, gerar token com lifetime experimental de 30 dias; entregar
arquivo novo 0600 no diretório do cliente e informar somente seu caminho. Auth
revogada/expirada no startup seguinte nunca provoca geração automática de token.
Recuperação de acesso exige rotação local explícita. Esse default é proposta.

## Protocolo de publicação

### Composição com o storage no M1

`--data-dir` aponta para uma raiz privada. A raiz contém `serve.lock`,
`bootstrap.json`, `store/` e `auth/`. O diretório de entrega padrão continua
irmão da raiz. O processo valida a raiz e adquire `serve.lock` de forma
exclusiva e não bloqueante antes de classificar o estado ou abrir o banco.
Mantém o lock até fechar o storage no shutdown. O lock segue as mesmas
regras de abertura e propriedade descritas abaixo para o lock de auth.
O lock do engine e o do journal de auth continuam protegendo suas operações.
Quando uma operação exigir ambos os locks de arquivo, adquirir primeiro o da raiz.

O bootstrap usa um registro de fase versionado na raiz, publicado por staging,
sync do arquivo, rename e sync do diretório. Fase ausente autoriza iniciar apenas
em raiz sem artefatos de instalação, exceto o lock. Fase inválida ou artefatos
inesperados exigem diagnóstico, sem inicialização automática.

| Fase durável | Ação na retomada |
|---|---|
| `initializing_store` | Criar `store/` se ausente. Se existir, abrir e validar conforme 0011. Estado parcial ou incompatível interrompe o startup antes da entrega de credenciais. |
| `store_ready` | Reabrir o store existente e validar. Preparar `auth/` e persistir o marker inicial `initialized=false`. Diretório vazio ou contendo apenas o lock de auth permite concluir essa preparação. |
| `initializing_auth` | Reabrir o store e exigir marker de auth válido. Retomar pelo journal abaixo. Marker ausente ou inválido interrompe o startup. |
| `initialized` | Reabrir store e exigir marker de auth com `initialized=true` e seleção recuperável pelo journal. Ausência ou inconsistência interrompe o startup. Um keyset revogado ou expirado permanece nessa condição até operação explícita. |

Publicar `initializing_store` antes de criar o store e `store_ready` após a
abertura e validação do banco. Durante `store_ready`, aceitar apenas a preparação
de auth, com marker inicial e sem intenção ou seleção ativa. Persistir
`initializing_auth` após o marker e antes de gerar ou entregar qualquer token.
Assim, a ausência posterior do marker exige intervenção em vez de nova emissão.
Publicar `initialized` depois da confirmação durável da seleção ativa e antes
de sinalizar readiness. Se houver interrupção entre a confirmação de auth e
esse último passo, carregar a seleção já confirmada e concluir a fase.

O primeiro incremento M1 deve testar dois processos disputando a raiz e
interrupções nas transições de fase, incluindo confirmação de auth antes de
`initialized`, ausência de auth após instalação concluída e manifesto
incompatível sem entrega de token. A composição é uma proposta para o produto.
O spike M0-F2 exercita o journal de auth isolado.

### Journal de autenticação

Um lock de arquivo exclusivo, não bloqueante, serializa bootstrap, publicação e
recuperação entre processos. Arquivo de lock regular/0600/mesmo dono/um link;
NOFOLLOW e descriptor ancorado. No fim normal a liberação é explícita via RAII;
em kill, depende do fechamento das descrições abertas pelo SO. Descritores herdados
antes do exec não devem prolongar a posse de uma operação já terminada.

1. Em estado virgem, publicar marker versionado initialized=false. Diretório auth
   desconhecido sem marker é erro, não justificativa para resetar credenciais.
2. Entregar token quando necessário e sincronizar arquivo/diretório do cliente.
3. Gravar keyset em snapshot content-addressed `state-{sha256}.json`, até 4 KiB,
   via staging completo e rename; sincronizar. Não promover snapshots órfãos.
4. Publicar intent.json atomicamente: versão, ponteiro anterior e próximo, com
   epoch/hash. O próximo deve ser epoch anterior + 1; bootstrap começa em 1.
   A intenção só referencia snapshot já completo. Este é o ponto de decisão.
5. Publicar marker initialized=true antes de qualquer active.json inicial.
6. Publicar active.json por staging/rename e sync do diretório. Só depois retornar
   sucesso. Erro após publicar intenção pode ser commit_uncertain; nunca desfazer
   uma revogação para simular rollback.

No startup, lock + validação de marker/intent/active e do hash do snapshot.
Intent igual ao active carrega esse estado. Intent pendente válido com predecessor
igual ao active completa a publicação antes de disponibilizar auth. Active ausente
também pode ser concluído a partir dessa intenção explícita; não por varredura.
Active maior/incompatível, metadados corrompidos ou inicialização marcada completa
sem intenção falham fechados. Não retornar ao snapshot antigo para ganhar disponibilidade.

Se houver marker initialized=false sem intenção nem active, o primeiro bootstrap
foi interrompido antes da decisão: pode emitir nova credencial e tentar novamente.
Snapshots/entregas órfãos não foram ativados. initialized=true sem seleção válida
nunca pode entrar nesse caminho, mesmo após revogar todos os tokens.

Intenção/ponteiros têm JSON estrito e tamanho limitado; hash e epoch precisam
corresponder ao keyset. Cada substituição usa arquivo novo 0600, fsync do arquivo,
rename no mesmo diretório e fsync do diretório. Temporários órfãos não são seleção
ativa. Leitura/publicação não sobrescreve symlink, hardlink ou arquivo inseguro.

## Recuperação e limites

### Comandos locais do M1

`kiarni auth rotate --data-dir ./data` e
`kiarni auth revoke-all --data-dir ./data` exigem instalação com fase
`initialized`. Ambos adquirem o lock da raiz antes de validar store e auth.
Servidor em execução retorna erro de lock. Diretório ausente ou bootstrap
incompleto exige diagnóstico e preserva a ausência de credenciais.

A rotação usa lifetime de 30 dias e sobreposição zero, permitindo recuperar
acesso quando a credencial anterior expirou ou foi revogada. O caminho da
nova entrega e a epoch são informados após publicação durável. A revogação
informa a epoch confirmada. Os arquivos de entrega anteriores permanecem
sob controle do operador. O próximo startup carrega a seleção confirmada.
O journal e sua recuperação seguem o protocolo descrito acima.

Revogação concluída seleciona keyset vazio no próximo startup normal. Rotação com
grace segue ADR 0008. Reload é explícito no modelo; helper carrega a seleção e
aplica Authority.reload, falhando fechado em erro. Watcher e integração automática
com o servidor ainda não existem, nem cancelamento de requests já aceitos.

O protocolo visa crash de processo e falhas nos checkpoints testados; não prova
queda de energia, durabilidade física, filesystem de rede ou defesa contra o dono
do host que restaura metadados antigos. Registros iniciais são para o spike e não
definem o formato persistido de documentos do banco. Retenção/GC de snapshots e
entregas órfãs exigem política futura; não apagar arquivos do operador por heurística.

As garantias de rename e a necessidade de sync do diretório são descritas em
[rename(2)](https://man7.org/linux/man-pages/man2/rename.2.html) e
[fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html);
o lock é advisory conforme [flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html).
Isso não substitui testes do filesystem/hardware da distribuição.

Testar erros injetados e kill/restart antes/depois de intenção, publicação de active
e sucesso, além de permissões/layout, lock de processo, estado revogado/expirado,
snapshots órfãos e corrupção. Evidências no
[relatório M0-F2](../spikes/auth-activation.md). A integração ao servidor será verificada no M1.
