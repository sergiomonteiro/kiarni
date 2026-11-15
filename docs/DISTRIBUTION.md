# Binários de desenvolvimento

O CI prepara arquivos nativos experimentais para o RF-10/M3. Eles identificam
versão e commit; `0.1.0` no executável ainda não significa release estável ou
MVP aprovado. A publicação de uma release permanece sujeita ao gate M3.
Decisão e limites: [ADR 0031](./adr/0031-artefatos-nativos-verificados.md).

## Plataformas da matriz

| Target | Construção e teste do pacote em runner novo | Baseline |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | `ubuntu-24.04` x86_64 | Ubuntu 24.04, glibc 2.39 |
| `aarch64-apple-darwin` | `macos-15` Apple Silicon | macOS 15, ARM64 |

A evidência válida é uma execução verde do CI para o **mesmo commit**, incluindo
os dois jobs `Package` e os dois `Artifact`, além dos testes Rust, Chromium e
HTTPS. Consulte a [lista de execuções](https://github.com/sergiomonteiro/kiarni/actions/workflows/ci.yml).
Falha ou cancelamento não validam a plataforma. Não há suporte declarado a
macOS Intel, Linux ARM, musl, Windows ou versões anteriores desses sistemas.
O binário Linux usa bibliotecas do sistema: não é um executável estático nem
uma promessa de compatibilidade com qualquer distribuição Linux. O pacote
registra a lista de bibliotecas em `BUILD_INFO.json`.

Os arquivos macOS são de desenvolvimento, sem assinatura de distribuidor nem
notarização. Gatekeeper pode impedir uma transferência pelo navegador. O teste
em runner não comprova a experiência de download sob quarentena; a pipeline
não remove quarentena nem desativa proteções do sistema. Para ambientes que
exigem notarização, a distribuição ainda tem essa limitação; compilar a partir
do código é uma alternativa sujeita à política local.

## Obter e verificar

Abra uma execução verde, confira seu commit e baixe o artifact
`binary-x86_64-unknown-linux-gnu` ou `binary-aarch64-apple-darwin`. GitHub pode
exigir autenticação para o download. Os artifacts expiram em **14 dias**.
Também é possível usar GitHub CLI, substituindo `RUN_ID` pelo número da execução:

```bash
gh run download RUN_ID --repo sergiomonteiro/kiarni \
  --name binary-x86_64-unknown-linux-gnu --dir ./download
cd download
```

O download contém um `.tar.gz` e seu `.tar.gz.sha256`. Substitua `ARQUIVO` abaixo
pelo nome completo recebido. Verifique **antes** de extrair em um diretório novo:

```bash
# Linux
sha256sum -c ARQUIVO.tar.gz.sha256
# macOS, no lugar do comando anterior
shasum -a 256 -c ARQUIVO.tar.gz.sha256

tar -xzf ARQUIVO.tar.gz
cd ARQUIVO
# Linux; no macOS use shasum -a 256 -c CHECKSUMS.sha256
sha256sum -c CHECKSUMS.sha256
```

Hashes detectam corrupção; eles não autenticam o publicador. Obtenha pacote e
checksum da execução de origem confiável e confira commit/target em
`BUILD_INFO.json`. Não reutilize diretório de extração de outro pacote.

## Instalar e iniciar

O executável precisa somente das bibliotecas do sistema. Rust, Cargo, Python,
Node, Nginx e serviços externos não são necessários para a operação local.
O dashboard já está embutido.

```bash
mkdir -p "$HOME/.local/bin"
install -m 755 ./kiarni "$HOME/.local/bin/kiarni"
"$HOME/.local/bin/kiarni" --version
"$HOME/.local/bin/kiarni" serve --data-dir ./data
```

O diretório pai dos dados deve existir. O bind padrão é `127.0.0.1:8711`;
Ctrl+C ou SIGTERM encerra o servidor. No primeiro início, a credencial é entregue
em arquivo privado no diretório irmão `data.credentials`. Preserve esse arquivo;
não coloque o token em URL, argumentos ou logs. Siga a [alpha local](./ALPHA_LOCAL.md)
a partir da preparação do cliente para criar documento, consultar e assinar feed.

O pacote inclui um [quickstart](./QUICKSTART.md) com examples/ para executar
sem checkout. O [teste de instalação isolada](./OPERATIONAL_VALIDATION.md)
registra tempo até documento/consulta/evento, com escopo explícito.
O `INSTALL.md` do pacote contém os comandos publicados e links fixados no commit
de origem. Os contratos completos estão em [backup](./BACKUP.md),
[restauração](./RESTORE.md) e [HTTPS opcional](./REMOTE_HTTPS.md).
Rotação/revogação de credenciais, backup e restore exigem servidor parado.
Destino de backup e raiz restaurada precisam ser novos; prepare pais privados.

## Conteúdo e licenças

- `kiarni`: executável nativo; `INSTALL.md`: instruções do pacote.
- `QUICKSTART.md` e `examples/`: roteiro/fixtures autossuficientes de cliente.
- `BUILD_INFO.json`: versão, commit completo, target, baseline, compilador,
  bibliotecas do sistema, comando de build e indicação de árvore modificada.
- `LICENSE`: MIT do Kiarni.
- `DEPENDENCIES.json` e `THIRD_PARTY_NOTICES.txt`: inventário e textos integrais
  dos avisos/licenças das dependências normais e de build do produto, filtradas
  pelo target. Dependências de testes, spikes, browser e proxy ficam fora.
- `toolchain/COPYRIGHT-library.html` e `toolchain/licenses/`: catálogo upstream
  da biblioteca padrão Rust e seus textos de licença. Esse catálogo também
  descreve componentes condicionais; sua inclusão não afirma que todos estejam
  ligados ao executável nem muda a licença MIT do Kiarni.
- `CHECKSUMS.sha256`: hashes de todos os demais arquivos internos.

Licença desconhecida, material ausente, dependência de sistema inesperada ou
checkout modificado fazem o build de distribuição falhar. Nenhuma credencial,
ambiente completo ou diretório local de usuário é incluído na proveniência.

## Construir e verificar

O repositório fixa Rust **1.99.1**, componentes e `Cargo.lock`; Python **3.11+** é
uma ferramenta de empacotamento/teste. Compile nativamente em uma das baselines
acima, com checkout limpo, sem flags externas de release/linker e sem dependências
novas no lockfile:

```bash
rustup show active-toolchain
python3 -m unittest discover -s scripts -p test_release.py
python3 scripts/package_release.py --target x86_64-unknown-linux-gnu
python3 scripts/release_smoke.py --artifact-dir dist \
  --expected-target x86_64-unknown-linux-gnu
```

No macOS, troque ambos os targets por `aarch64-apple-darwin`. O empacotador define
`MACOSX_DEPLOYMENT_TARGET=15.0`. A receita é reproduzível, mas não há promessa de
binários idênticos bit a bit entre máquinas/SDKs. O script recusa sobrescrever
arquivos; use `--output` para escolher um diretório novo.

Para QA local de alterações ainda não commitadas, os dois scripts aceitam
`--allow-dirty`; o pacote recebe `-dirty` no nome e na proveniência. Esse modo
não é aceito pelo empacotador no CI e não serve como artifact de publicação.

O CI primeiro executa testes Rust e integrações existentes. Depois constrói os
dois pacotes e os transfere para runners novos. O smoke confere hashes externos
e internos, extrai com limites de tamanho/quantidade e recusa links/traversal.
Executa o binário fora do checkout com ambiente vazio: versão/ajuda, autenticação,
coleção/documento, precisão JSON/consulta, SSE futuro, reinício, rotação/revogação,
backup e restauração com nova credencial. Não instala Rust ou serviços nesse job.
Isso valida o pacote e seus comandos; não substitui instalação humana, Gatekeeper,
operação prolongada ou todo o checklist de release da issue #7.

## Fluxo de publicação

Push, pull request e execução manual de CI produzem somente artifacts temporários;
não criam tags ou GitHub Releases. Antes de publicar, mantenedores precisam:

1. Fechar o gate M3 e registrar limitações/evidências, incluindo instalação limpa.
2. Revisar versão, changelog, licenças, commit e todos os jobs da execução escolhida.
3. Preparar tag e release de forma explícita, reutilizando os `.tar.gz` e checksums
   já verificados desse commit. Não reconstruir silenciosamente os arquivos.
4. Revisar um draft com notas e assets antes de publicá-lo. Quando for uma alpha,
   identificá-la como prerelease; não inferir estabilidade do número no Cargo.toml.

Assinatura/notarização, retenção permanente e política de upgrade ainda precisam
ser tratadas no fechamento de release; não são garantidas pelos artifacts de CI.
