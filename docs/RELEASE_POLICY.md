# Canal experimental e compatibilidade pré-1.0

Esta política registra o canal **já operado** pelo repositório e o limite do
aceite técnico M3. Não cria tag, GitHub Release ou publicação de registry.
[Revisão do gate](./RELEASE_REVIEW.md), [changelog](../CHANGELOG.md) e
[distribuição](./DISTRIBUTION.md).

## Canal desta alpha

O executável informa a versão Cargo 0.1.0. Os entregáveis desta etapa são
artifacts experimentais de CI do repositório oficial, identificados pelo commit:

- Pacotes nativos Linux x86_64 GNU/glibc 2.39 e macOS 15 Apple Silicon, com
  executável, quickstart, exemplos, proveniência, checksums e licenças.
- Imagem Docker Linux amd64 exportada, checksum, Compose e manifesto Kubernetes
  de uma instância; carrega por docker image load, sem registry obrigatório.
- Relatórios sanitizados de instalação e observação, vinculados ao commit.

Artifacts expiram em 14 dias e o download pode exigir login GitHub. Esse é o
canal oficial **experimental**, não uma promessa de disponibilidade permanente,
instalação sem quarentena macOS ou release estável. Cada entrega deve ser obtida
da execução verde do commit correspondente e conferida por hash/proveniência.
SHA-256 detecta corrupção, não autentica sozinho o publicador.

Aprovar o gate técnico atesta o recorte testado e documentado do MVP. Não transforma
0.1.0 em estabilidade de produção, certificação de qualquer hardware ou garantia
de desempenho/HA. Prometheus, Rust, Node e serviços auxiliares não são exigidos
pelo servidor local. O coletor externo é ferramenta opcional de operação/teste.

## Compatibilidade

Antes de 1.0, não há garantia automática de compatibilidade entre artifacts ou
APIs Rust internas. A API HTTP implementada continua em /v1; consultar a
referência do commit. Não há compatibilidade prometida com ReQL/drivers externos.
Mudanças devem atualizar contratos, fixtures, documentação e changelog. As decisões
que exigem ADR seguem AGENTS/CONTRIBUTING; isso não autoriza quebra silenciosa.

Formato persistido atual: 2, em um único backend Store. O layout 1 era de
desenvolvimento e não é suportado pelo binário atual. Não há instalações em uso
a preservar, conforme decisão registrada no ADR 0034. Não há migração ou downgrade automáticos, nem teste
de atualização entre duas releases distintas. Formato desconhecido deve ser
recusado; não editar metadados para fazê-lo parecer compatível. Antes de ensaiar
outra versão, preservar backup consistente, validar restauração e usar raiz de
teste independente. Arquivos antigos são recusados e nunca apagados automaticamente. Não editar
versões em metadados para contornar essa recusa.

A avaliação de índice secundário atende ao RF-04 conforme
[ADR 0033](./adr/0033-avaliacao-indice-secundario.md); a plataforma desta alpha
usa Store único e planner HTTP com ID, secundário seletivo e scan limitado.
Administração de índices usa a [CLI offline](./INDEXES.md). As
[medições HTTP do produto](./benchmarks/secondary-index-http-2026-11-22.md)
registram benefícios seletivos e custo de escrita nos fixtures locais.
Não são promessa de capacidade universal nem extrapolação dos ganhos do spike. SDK inicial continua questão do roadmap do PRD;
a documentação HTTP e os clientes genéricos usados no aceite não são um SDK
publicado. Esta revisão não acrescenta requisito de SDK ou compatibilidade.

## Publicação futura

Uma tag/prerelease/Release ou imagem em registry exige ação explícita de
publicação, versão/notas correspondentes e reutilização dos arquivos validados.
Não reconstruir silenciosamente assets após validar outro commit. A pipeline
atual tem apenas contents:read e não publica esses canais automaticamente.

O checklist técnico fica na revisão M3. Limitações de produto permanecem:
single-node, tokens locais e administração offline, query com orçamentos,
changefeeds futuros best-effort, sem replay/exactly-once, sem cluster/replicação.
Nenhuma aprovação remove limitações conhecidas de benchmark, backup ou segurança.
