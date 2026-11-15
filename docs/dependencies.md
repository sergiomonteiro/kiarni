# Dependências do workspace

Licenças Rust verificadas nos manifests locais resolvidos por `Cargo.lock`.
Dependências de browser constam nos manifests npm e no package-lock indicado abaixo.
As bibliotecas são incorporadas ao programa, sem serviços externos obrigatórios.

| Pacote | Licença declarada |
|---|---|
| getrandom, cfg-if, libc | MIT OR Apache-2.0 |
| serde, serde_core, serde_derive | MIT OR Apache-2.0 |
| serde_json | MIT OR Apache-2.0 |
| itoa | MIT OR Apache-2.0 |
| memchr | Unlicense OR MIT |
| zmij | MIT |
| proc-macro2, quote, syn | MIT OR Apache-2.0 |
| unicode-ident | (MIT OR Apache-2.0) AND Unicode-3.0 |
| r-efi (target UEFI, fora dos builds Linux/macOS) | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| redb (storage e spikes) | MIT OR Apache-2.0 |
| tempfile, bitflags, errno, once_cell (testes/exemplo) | MIT OR Apache-2.0 |
| fastrand (testes/exemplo) | Apache-2.0 OR MIT |
| rustix, linux-raw-sys (Unix, spikes/testes) | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| windows-link, windows-sys (targets Windows) | MIT OR Apache-2.0 |
| sha2, digest, crypto-common, block-buffer, cpufeatures, hybrid-array, typenum (auth/backup) | MIT OR Apache-2.0 |
| const-oid, zeroize (auth) | Apache-2.0 OR MIT |
| subtle (auth) | BSD-3-Clause |
| httparse 1.10.1 (HTTP e transporte) | MIT OR Apache-2.0 |
| signal-hook 0.3.18, signal-hook-registry 1.4.8 (shutdown Unix) | MIT OR Apache-2.0 |
| playwright, playwright-core 1.57.0 (testes browser) | Apache-2.0 |
| fsevents 2.3.2 (npm opcional macOS, metadata do lockfile) | MIT |

Para pacotes com alternativa MIT, essa alternativa permite distribuição sob os
termos MIT, preservando os avisos exigidos.

O spike de queries habilita `serde/derive`, compilando também serde_derive,
proc-macro2, quote, syn e unicode-ident. Consultar
`cargo tree --workspace --locked` para o grafo realmente compilado.
Preservar licenças e avisos dos pacotes na distribuição. Este inventário não
substitui o bundle de avisos de terceiros necessário para uma release.

O spike de auth habilita também `rustix/fs` e `rustix/process`; sua licença consta
acima. Subtle tem BSD-3-Clause: preservar copyright, condições e disclaimer BSD
na distribuição. Isso não altera a licença MIT do projeto. SHA-2/zeroize e suas
dependências foram conferidas nos manifests das versões travadas.

O spike de transporte habilita `rustix/net`. O teste browser tem package.json e
package-lock privados em `spikes/transport`; não adiciona Node/Chromium ao runtime
do produto. Chromium de teste é instalado pela ferramenta, não vendorizado ou
redistribuído como parte do binário. Preservar também os avisos Apache ao distribuir
artefatos das ferramentas, se isso vier a ocorrer.

O servidor habilita também `rustix/event` para esperar prontidão do listener
com poll. A versão 1.1.6 e as licenças já constam no inventário acima.

O comando de backup no servidor usa diretamente `sha2 0.11` (hashes SHA-256) e
`getrandom 0.4` (nome de staging). Ambos já estavam no grafo de auth e tiveram
suas licenças MIT OR Apache-2.0 verificadas nos manifests locais resolvidos.
Não entram novos pacotes transitivos nem serviços obrigatórios.

Os testes de restauração usam `redb =4.5.0` como dependência de desenvolvimento
do servidor para construir corrupção estrutural com hashes recalculados. É a
mesma versão/licença já usada no storage; não adiciona dependência de runtime ou
pacotes transitivos novos.

O perfil remoto não adiciona crates: a origem HTTPS usa uma gramática limitada
com std. Nginx/OpenSSL são ferramentas externas opcionais do perfil/teste, não
incorporadas ao binário ou fornecidas como serviços obrigatórios no perfil local.
A CI extrai o pacote Nginx da distribuição em área temporária; Playwright já
permanece fixado como dependência de desenvolvimento.

A [distribuição nativa](./DISTRIBUTION.md) gera o inventário por target a partir
do grafo normal/build do executável, sem dev dependencies ou spikes. Os pacotes
incluem textos integrais de LICENSE/NOTICE/COPYRIGHT e os avisos da biblioteca
padrão Rust do toolchain fixado. Licença não revisada ou aviso ausente interrompe
o empacotamento. `DEPENDENCIES.json` e `THIRD_PARTY_NOTICES.txt` do artifact são
a evidência da versão distribuída; a tabela acima descreve o workspace inteiro.

A imagem Docker usa o mesmo pacote nativo e preserva seus avisos em
`/usr/local/share/kiarni`, além dos materiais da base Ubuntu fixada por digest.
O healthcheck usa std/rustix/zeroize já presentes; nenhuma crate nova.
Kind/kubectl são ferramentas de teste obtidas de fontes oficiais, com versões
e checksums fixados; não são incluídas na imagem nem exigidas pelo servidor.
