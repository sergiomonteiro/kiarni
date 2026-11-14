# Dependências do protótipo M0

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
| redb (somente spike) | MIT OR Apache-2.0 |
| tempfile, bitflags, errno, once_cell (testes/exemplo) | MIT OR Apache-2.0 |
| fastrand (testes/exemplo) | Apache-2.0 OR MIT |
| rustix, linux-raw-sys (spikes/testes) | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| windows-link, windows-sys (targets Windows) | MIT OR Apache-2.0 |
| sha2, digest, crypto-common, block-buffer, cpufeatures, hybrid-array, typenum (auth spike) | MIT OR Apache-2.0 |
| const-oid, zeroize (auth spike) | Apache-2.0 OR MIT |
| subtle (auth spike) | BSD-3-Clause |
| httparse 1.10.1 (transport spike) | MIT OR Apache-2.0 |
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
