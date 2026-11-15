# ADR 0031 — Artefatos nativos verificados

- Estado: aceito para issue #4, M3/RF-10
- Data: 2026-11-15
- Não aprova release nem amplia funcionalidades do MVP.

## Decisão

Produzir inicialmente Linux x86_64 GNU em Ubuntu 24.04 e macOS arm64 em macOS 15.
Compilar e executar nativamente: validar o host do compilador antes do build e
retestar o pacote baixado em um job novo da mesma plataforma. Não declarar Linux
musl/ARM, Intel Mac, Windows ou plataformas antigas como suportadas.
Rust 1.99.1 fica fixado em rust-toolchain.toml, com Cargo.lock e --locked.
O procedimento é reproduzível; igualdade bit a bit entre rebuilds não foi provada.

Workflow de desenvolvimento em push/PR/dispatch, somente contents:read, publica
artefatos de Actions com retenção limitada. Nome inclui versão, commit e target.
Nenhuma tag, GitHub Release ou promoção estável é criada automaticamente. Release
pública depende de revisão/gate próprios; documentação deve descrever promoção dos
mesmos arquivos já validados, sem rebuild silencioso.

Tar.gz contém executável, MIT, avisos completos das dependências normal/build do
produto, inventário sem caminhos locais, avisos da biblioteca padrão do toolchain,
proveniência e guia de instalação. Excluir dev dependencies e spikes. Licença
não aprovada ou material ausente interrompe empacotamento. Preservar textos de
licenças/NOTICE/COPYRIGHT sem substituir a licença MIT do projeto. O catálogo do
Rust std inclui componentes condicionais; não presumir que todos foram linkados.

Hashes internos cobrem os arquivos; checksum externo cobre o tar.gz. Hash detecta
corrupção, não autentica publicador. Proveniência registra commit, dirty flag,
compiler e plataforma; nunca tokens, ambiente completo ou diretórios do usuário.
Build sujo só com opção explícita para QA local e identificação dirty, nunca CI.
Flags de compilação externas são recusadas para evitar artefato rotulado incorretamente.

Validar pacote em diretório privado novo, conferir checksums e estrutura antes de
executar. Binário filho roda com ambiente vazio e fora do checkout, sem Rust,
Node, proxy ou serviços auxiliares em runtime. Exercitar versão, CRUD/query/SSE,
reinício, rotação/revogação, backup e restore com novas credenciais. Limitar tamanho
extraído e recusar links/traversal no teste de pacote.

## Limites

Binários experimentais sem assinatura/notarização; download via CI não homologa
Gatekeeper/Safari ou instalação por todo gerenciador de pacotes. Linux usa libc
GNU do baseline; sem promessa de binário estático ou ABI para distros antigas.
Bibliotecas de sistema não são redistribuídas no pacote. Python é ferramenta de
build/validação, não dependência do executável. Sem benchmark novo nesta entrega.

Referências: [runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
[upload-artifact](https://github.com/actions/upload-artifact),
[download-artifact](https://github.com/actions/download-artifact).
