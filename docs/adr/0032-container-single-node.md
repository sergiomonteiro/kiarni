# ADR 0032 — Contêiner single-node e probe autenticado

- Estado: aceito para issue #5, RF-10/M3
- Data: 2026-11-15
- Complementa 0009/0012/0030/0031; sem mudança de protocolo, storage ou auth.

Preservar bind loopback. Compose usa network_mode host em Docker Engine Linux;
não publica portas nem habilita wildcard. Kubernetes usa namespace de rede do
pod, uma réplica e estratégia Recreate, sem Service/Ingress. Acesso inicial por
port-forward; proxy HTTPS futuro deve compartilhar esse namespace. Exemplos não
homologam Docker Desktop, clusterização ou alta disponibilidade do banco.

Imagem Linux amd64 construída de pacote nativo com notices/checksums, base Ubuntu
24.04 fixada por digest. Não incluir toolchain ou curl. UID/GID 10001, filesystem
raiz read-only e capacidades removidas. Um volume em /var/lib/kiarni contém
`data` e `data.credentials`, preservando o layout existente. Não montar somente
`data`, que perderia a entrega privada da credencial. Docker inicializa permissões
do volume vazio a partir da imagem; Kubernetes exige preparação explícita do
volume para UID 10001 via initContainer com privilégios mínimos e caminho fixo.
Não corrigir recursivamente permissões de uma instalação existente.

Adicionar `kiarni healthcheck` como cliente local somente leitura: resolver bind
e Host da configuração existente, conectar somente a loopback e consultar ready
com Bearer de arquivo privado. `--token-file` aceita caminho, nunca token. Sem
essa opção, examinar no máximo 64 entradas no diretório irmão de credenciais e
usar somente arquivos token-ID.txt de proprietário atual, modo 0600 e um link,
sem seguir symlinks; diretório 0700 do mesmo proprietário. Teto de token 91 bytes,
resposta 4 KiB e prazo total de 3 s, sem retry automático após falha de rede.
Não abrir banco, inicializar auth ou modificar credenciais. Mensagens genéricas,
sem imprimir arquivo, resposta ou token. Credencial expirada/revogada falha probe;
rotação offline e limpeza consciente dos arquivos revogados são operação normal.
Limite 64 pode ser evitado com caminho explícito de token ativo.

Docker healthcheck e probes exec Kubernetes usam esse cliente. Kubernetes terá
startup/readiness autenticados, sem liveness dependente de token para evitar
reinícios infinitos após expiração/revogação. Docker unhealthy não é garantia de
restart automático. Não introduzir endpoint de health anônimo ou permissão nova.

Publicação externa da imagem requer ação explícita; CI constrói/testa sem push
para registry. Preservar licenças de base e avisos do binário. Validar criação,
reinício/recriação, volume, UID, health, rotação, backup e restore com Docker real.
Executar também o manifesto em cluster kind temporário: validação no API server,
PVC e recriação do pod com dados e credenciais preservados. Essa evidência não
homologa outros provisionadores, políticas de segurança ou produção.

Referências: [rede host](https://docs.docker.com/engine/network/drivers/host/),
[volumes](https://docs.docker.com/engine/storage/volumes/),
[probes](https://kubernetes.io/docs/concepts/workloads/pods/probes/).
