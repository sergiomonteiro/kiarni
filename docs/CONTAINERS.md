# Docker, Compose e Kubernetes

Entrega experimental do RF-10/M3, [issue #5](https://github.com/sergiomonteiro/kiarni/issues/5)
e [ADR 0032](./adr/0032-container-single-node.md). O mesmo binário fornece banco,
dashboard e healthcheck. Não há cluster, replicação ou alta disponibilidade.
O gate de release do MVP permanece aberto.

## Plataforma e imagem

A imagem é **Linux amd64**, baseada em Ubuntu 24.04 fixado por digest no
[Dockerfile](../deploy/docker/Dockerfile). Ela incorpora o pacote nativo verificado,
seus avisos/licenças e proveniência, sem Rust, Node, curl ou proxy obrigatório.
Os avisos do Kiarni ficam em `/usr/local/share/kiarni`; os materiais da base
permanecem na imagem. Não há homologação de ARM ou Docker Desktop nesta entrega.

O CI constrói e testa a imagem sem publicar em registry. Uma execução verde para
o commit precisa incluir `Container and Kubernetes`, além dos jobs nativos e
integrações anteriores. O artifact `container-linux-amd64`, retido por 14 dias,
contém imagem exportada, checksums e exemplos. Para obtê-lo, troque `RUN_ID`:

```bash
gh run download RUN_ID --repo sergiomonteiro/kiarni \
  --name container-linux-amd64 --dir ./kiarni-container
cd kiarni-container
sha256sum -c CHECKSUMS.sha256
docker image load --input kiarni-development-linux-amd64.tar.gz
docker image inspect kiarni:development --format '{{index .Config.Labels "org.opencontainers.image.revision"}}'
```

Confira a revisão com a execução escolhida. Checksums detectam corrupção, não
identidade do publicador. A tag local `kiarni:development` é substituída ao carregar
outro artifact; preserve a imagem/revisão escolhida antes de atualizar uma instalação.
Nenhuma release estável ou imagem oficial em registry foi publicada por esse CI.

## Compose no Docker Engine Linux

Execute o `compose.yaml` recebido com o artifact ou o
[exemplo do repositório](../deploy/docker/compose.yaml) da mesma revisão:

```bash
docker compose -p kiarni -f compose.yaml up -d
docker compose -p kiarni -f compose.yaml ps
```

Abra `http://127.0.0.1:8711`. O exemplo usa rede **host**: o servidor permanece
restrito ao loopback do host, sem portas publicadas ou bind wildcard. Isso é
necessário porque mapear uma porta de bridge não alcança um listener preso ao
loopback interno. Não acrescentar `ports:` a esse exemplo. O perfil foi testado
no Docker Engine Linux; não presumir comportamento equivalente em Docker Desktop.
Para uma porta local diferente, defina `KIARNI_BIND=127.0.0.1:8712` ao executar
Compose e mantenha essa configuração nos comandos de operação.

O contêiner roda como **10001:10001**, com raiz read-only, todas as capacidades
removidas, sem elevação de privilégios e limites de processos/memória/CPU.
Os valores do exemplo são limites operacionais iniciais, não sizing ou benchmark.
Ele não configura reinício automático; escolha sua política de operação conscientemente.

O volume nomeado `kiarni_state` monta **`/var/lib/kiarni` inteiro**:

- `data/`: store, locks, metadados e verificadores de autenticação;
- `data.credentials/`: arquivos privados de entrega de credenciais;
- diretórios de backup/restauração criados pelo operador, quando usados.

Docker inicializa o volume vazio com as permissões do diretório da imagem. Não
use `volume-nocopy` nesse primeiro uso nem monte somente `data/`. Diretórios
internos exigem proprietário UID 10001 e modo 0700; arquivos privados, 0600.
Um bind mount exige preparar o pai com essas permissões no host; a imagem não
executa `chown -R` nem corrige silenciosamente uma instalação existente.

### Obter a credencial e usar a API

O startup registra o **caminho**, nunca o token. Para entregar o arquivo ao cliente,
use uma pasta privada e stdout redirecionado, sem valores de segredo nos argumentos:

```bash
umask 077
mkdir -p ./client
chmod 700 ./client
docker compose -p kiarni -f compose.yaml exec -T kiarni \
  sh -ec 'cat /var/lib/kiarni/data.credentials/token-*.txt' > ./client/token.txt
```

Esse glob é apropriado **somente no primeiro bootstrap**, com um único arquivo.
Depois de rotação, use o caminho específico informado pelo comando; concatenar
credenciais não produz uma credencial válida. Use o arquivo com o cliente do
[quickstart](./ALPHA_LOCAL.md#2-preparar-o-cliente), que transmite Authorization
por stdin. Não imprima o token, use `curl -v` ou o coloque em variável do contêiner.

### Healthcheck

`kiarni healthcheck` consulta `/v1/health/ready` com autenticação. O comando não
abre o banco ou altera auth. Sucesso retorna código 0 sem saída; erro retorna 1
com `healthcheck_failed`, sem reproduzir resposta ou segredo.

```bash
docker compose -p kiarni -f compose.yaml exec -T kiarni kiarni healthcheck
```

Ele usa a configuração normal de bind, data-dir e public-origin. Automaticamente
procura arquivos `token-ID.txt` no diretório irmão privado, até **64 entradas**,
e aceita somente arquivos regulares do UID atual, modo 0600, um link, sem symlink.
Cada token deve ter exatamente 91 bytes; respostas têm teto de 4 KiB e o cliente
aplica prazo de rede total de 3 segundos. Um arquivo candidato inválido ou excesso
de entradas faz a descoberta falhar. Para escolher uma credencial ativa, use
`--token-file CAMINHO`; não passe o valor do token.

Tokens expiram conforme o contrato atual (bootstrap de até 30 dias). Rotação ou
revogação exige servidor parado; o probe tenta os arquivos entregues, sem ativá-los
ou renovar tokens. Preserve a credencial ativa e remova arquivos revogados apenas
após conferir seus IDs e necessidades dos clientes. Se houver mais de 64 entradas,
configure o healthcheck com caminho explícito ou faça essa manutenção privada.

Docker marca `unhealthy` após falhas; isso por si só não reinicia o contêiner.
Não é teste de carga, disponibilidade externa ou recuperação completa do banco.

### Parar, recriar e administrar

```bash
docker compose -p kiarni -f compose.yaml stop
# Depois de parar, quando necessário:
docker compose -p kiarni -f compose.yaml run --rm --no-deps kiarni auth rotate
# Ou revoke-all. Ambos preservam a semântica offline existente.
docker compose -p kiarni -f compose.yaml up -d
```

`stop` envia SIGTERM e permite 30 segundos de encerramento. `down` remove o
contêiner, preservando o volume nomeado; subir novamente com o mesmo projeto
reutiliza dados e credenciais. **`down --volumes` remove os dados desse projeto**;
não é um comando de reinício. Não execute duas instâncias sobre o mesmo diretório:
o lock recusa a segunda, mas não fornece coordenação distribuída.

### Backup e restore

Com o serviço ainda iniciado, prepare pais privados no volume, uma única vez:

```bash
docker compose -p kiarni -f compose.yaml exec -T kiarni \
  mkdir -m 700 /var/lib/kiarni/backups /var/lib/kiarni/restores
docker compose -p kiarni -f compose.yaml stop
docker compose -p kiarni -f compose.yaml run --rm --no-deps kiarni \
  backup --output /var/lib/kiarni/backups/first
docker compose -p kiarni -f compose.yaml run --rm --no-deps kiarni \
  restore --from /var/lib/kiarni/backups/first --data-dir /var/lib/kiarni/restores/data
```

Destinos precisam ser novos; pais já existentes não precisam ser recriados.
Para iniciar a raiz restaurada, altere **KIARNI_DATA_DIR do serviço** para
`/var/lib/kiarni/restores/data` e recrie o contêiner. A mudança também orienta o
healthcheck. Primeiro serve emite novas credenciais em `restores/data.credentials`;
as originais não autenticam essa raiz. Leia [backup](./BACKUP.md) e
[restore](./RESTORE.md), incluindo janelas de publicação e locks.

O backup nesse mesmo volume é apenas uma etapa de exportação: copie o artefato
concluído para armazenamento independente, confira seus hashes e teste restore.
Perder o volume perde também qualquer backup deixado somente nele. Não copie o
arquivo redb aberto como substituto do comando de backup. A imagem não oferece
conversão de formato, migração automática ou rollback de dados entre versões.

## Kubernetes básico

O [manifesto](../deploy/kubernetes/kiarni.yaml) cria PVC de 5 GiB e Deployment com
**uma réplica**, estratégia **Recreate** e nó Linux amd64. Não cria Service,
Ingress, LoadBalancer ou hostNetwork. O acesso inicial é port-forward local.
Prepare uma StorageClass compatível; ReadWriteOnce não equivale a lock de banco
nem alta disponibilidade. Filesystem precisa oferecer locks e fsync adequados;
o teste com kind não homologa todos os drivers CSI ou armazenamento remoto.

Escolha uma imagem da mesma revisão, disponibilize-a no cluster e substitua
**as duas ocorrências** de `image:` (initContainer e servidor) por referência
imutável adequada. O CI não publica em registry. No kind local, por exemplo:

```bash
kind load docker-image kiarni:development --name SEU_CLUSTER
kubectl apply -f kiarni.yaml
kubectl rollout status deployment/kiarni
kubectl port-forward --address 127.0.0.1 deployment/kiarni 8711:8711
```

Use o `kiarni.yaml` recebido com o artifact ou o manifesto da mesma revisão.
O initContainer usa root e somente capacidades CHOWN/DAC_OVERRIDE para preparar
um subdiretório fixo 0700/UID 10001 no PVC vazio. Não altera recursivamente dados
existentes e recusa layout incompatível. O servidor opera sem root e sem
capacidades. Clusters com política que proíbe initContainer root exigem preparar
o volume fora do pod; adapte o exemplo à política, sem afrouxá-la silenciosamente.
Não usar fsGroup recursivo que transforme arquivos 0600/diretórios 0700.

Startup e readiness executam o probe autenticado dentro do pod. Não há liveness
dependente de credencial: expiração ou revogação deve retirar readiness sem causar
um ciclo de reinícios. Logs, entrega de token, rotação e backup seguem o mesmo
contrato do Docker; comandos administrativos exigem o servidor parado e um pod de
manutenção com o mesmo UID/volume. Não usar `kubectl exec ... backup` com writers
ativos nem escalar o Deployment para duas réplicas. PVC e seus snapshots não
substituem validação de backup consistente do Kiarni.

O proxy HTTPS opcional deve compartilhar o namespace de rede do backend
(sidecar no pod ou mesmo host no perfil Docker). Um Ingress comum apontando para
IP do pod não alcança o listener loopback. Esse exemplo não implementa proxy,
certificados, réplica de banco, eleição ou failover.

## Construção e evidências

A partir de um único pacote Linux amd64 verificado da mesma revisão do repositório:

```bash
python3 scripts/prepare_container.py --artifact-dir ./download \
  --output /tmp/kiarni-image-context
docker build --platform linux/amd64 -t kiarni:development /tmp/kiarni-image-context
python3 scripts/container_smoke.py --image kiarni:development
python3 scripts/install_container_test_tools.py --output /tmp/kiarni-test-tools
python3 scripts/kubernetes_smoke.py --image kiarni:development --tools /tmp/kiarni-test-tools
```

O contexto novo contém somente pacote e Dockerfile; nunca credenciais, dados,
`.git` ou todo o checkout. `--allow-dirty` no preparo é apenas QA local; CI exige
pacote limpo do commit da execução. O comando de [empacotamento nativo](./DISTRIBUTION.md)
fixa Rust 1.99.1/Cargo.lock. Ferramentas Kubernetes de teste são fixadas por versão
e SHA-256 e não entram na imagem: kind 0.33.0, kubectl/Kubernetes 1.37.0,
kindest/node por digest.

Evidência local em 2026-11-15: Docker Engine 29.1.3, Compose 5.6.1, Linux x86_64;
smoke real Compose passou para UID, health, auth, documentos exatos, restart,
recriação com volume, rotação/revogação, lock, backup e restore com nova chave.
Cluster kind temporário passou validação no API server, inicialização do PVC,
probes, acesso por port-forward e substituição de pod preservando documento e
credencial. Testes removem somente projetos/volumes/clusters temporários próprios.
Essa evidência e CI não aprovam operação prolongada, upgrade, outros clusters,
registry público ou o gate M3.

Referências: [rede host Docker](https://docs.docker.com/engine/network/drivers/host/),
[volumes Docker](https://docs.docker.com/engine/storage/volumes/),
[probes Kubernetes](https://kubernetes.io/docs/concepts/workloads/pods/probes/).
