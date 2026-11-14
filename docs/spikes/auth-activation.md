# M0-F2 — bootstrap e ativação persistida de auth

Data: 2026-11-10. RF-01/RF-07, bloqueador G3 da revisão M0. Implementação em
`crates/auth/src/activation.rs`, proposta no [ADR 0009](../adr/0009-bootstrap-e-ativacao-auth.md).
Nenhuma mudança no binário de produto `apps/kiarni` ou aceite de ADR.

## Reprodução

```bash
cargo test -p kiarni-auth-spike --test activation --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

O helper `auth-bootstrap-spike` recebe `serve --data-dir <diretório>`, ou rotate/
revoke com o mesmo argumento. É uma fixture que simula startup, imprime somente
metadados e termina: **não abre servidor e usa relógio determinístico de 1000 s
Unix para os testes**. Não fornece credenciais para o servidor real. Não introduz
argumento obrigatório adicional ao fluxo mínimo proposto nem serviço externo.

Os testes usam diretórios temporários privados; nenhuma credencial é incluída no
repositório, stdout ou argumentos. Novo token é entregue em arquivo 0600, no
diretório irmão `data.credentials` por padrão. O loader normal não regenera token
por estar expirado/revogado, e não seleciona snapshot de maior número.

## Estado e publicação

Layout experimental em `data/auth`: marker.json, intent.json, active.json,
writer.lock e snapshots state-SHA256.json. O snapshot contém apenas keyset com
verificadores. A intenção registra predecessor/próximo com epoch e checksum.

O writer segura flock exclusivo não bloqueante. Entrega token, sincroniza o
snapshot e publica intenção por staging/rename/sync. Marca inicialização completa
antes do primeiro active e só confirma sucesso após a publicação/sync de active.
Erro após publicar intenção é commit_uncertain; não simula rollback da credencial.

No restart, intenção válida pendente é concluída antes de disponibilizar o keyset.
Metadados corrompidos, active incompatível ou seleção ausente após inicialização
falham fechados. O início pode ser repetido com credencial nova somente se marker
indicar tentativa inicial incompleta, sem intenção/active: nenhuma credencial
dessa tentativa anterior havia sido selecionada. Órfãos não são ativados.

## Evidências locais

Execução em Linux x86_64 com Rust/Cargo 1.99.1, sem novas dependências externas.
Os dez testes de integração de activation incluem:

- Comando mínimo em processo separado cria layout e entrega privada; segundo
  startup não emite novo token. Expiração também não reinicializa auth.
- Revogação confirmada na CLI continua selecionada em um novo processo, recusando
  o token antigo. Rotação local posterior restaura acesso apenas com token novo.
- Erros injetados antes da intenção mantêm a seleção anterior; depois da intenção
  retornam commit_uncertain e o restart conclui a seleção nova.
- Snapshot órfão com epoch alta é ignorado. Hash incorreto, intenção corrompida,
  active conflitante e lock inseguro são recusados, sem fallback nem reemissão.
- Intenção de revogação reconstrói active ausente sem recuperar token revogado.
  Se intenção e seleção estiverem ausentes após inicialização, falha fechado.
- Layout rejeita entrega dentro de dados, symlink final e diretório existente
  com modo inseguro, sem corrigir permissões do operador silenciosamente.
- Refresh da autoridade aplica somente estado validado; erro de seleção torna
  auth indisponível, sem manter credencial antiga como fallback.

A execução completa também revelou retenção transitória de flock durante testes
paralelos que criam subprocessos. A operação agora libera o lock explicitamente
via RAII antes de fechar o arquivo. Um teste unitário com descritor duplicado
verifica que a próxima operação não depende do fechamento dessa cópia herdada.

### Kill/restart em 27 cenários

| Operação | Checkpoints de kill | Resultado observado |
|---|---|---|
| Bootstrap inicial, 11 cenários | marker, entrega, snapshot, antes de intenção, intenção renomeada/durável, marker completo, antes de active, active renomeado/durável e Complete | Startup termina com exatamente uma credencial aceita; órfãs não são aceitas |
| Rotação grace zero, 8 cenários | snapshot, antes da intenção, intenção renomeada/durável, antes de active, active renomeado/durável e Complete | Antes da decisão fica epoch anterior; após decisão termina em epoch nova e recusa token antigo |
| Revogação, 8 cenários | mesmos oito checkpoints | Após decisão, restart mantém keyset revogado; não gera substituto automático |

O pai aguarda marcador READY do filho, testa lock Busy e/ou envia kill, espera o
processo e abre estado novamente. Marcadores não contêm segredo. O checkpoint
Complete ocorre após os syncs e antes de retornar; o caso de sucesso da CLI é
testado separadamente por saída com status zero e outro processo de startup.

## Limites e pendências

- Testes são de kill do processo em checkpoints sincronizados, não queda de energia,
  erro real de disco cheio, corrupção durante syscall, filesystem de rede ou auditoria
  de segurança. Erros retornáveis de publicação são injetados nos hooks indicados.
- fsync/rename são usados, mas as garantias físicas dependem de filesystem/dispositivo;
  hierarquias/volumes de produção e macOS não foram certificados nesta execução.
- Lock é advisory. Dono do host/UID que altera estado fora do protocolo continua
  no domínio de confiança. Não há proteção contra restauração maliciosa de journal
  e snapshots antigos após restart.
- Existem snapshots/temporários/entregas órfãos após falha; não há GC automático.
  Preservar snapshots referenciados pela intenção, inclusive predecessor. Estado
  inicial desconhecido sem marker não é limpo/resetado automaticamente.
- Canonicalização impede nesting por caminhos comuns e symlinks finais são recusados.
  Isso não certifica topologias especiais de bind mounts/backups sob controle do host.
- A entrega irmã padrão exige pai gravável. Volume montado como /data sob pai
  somente leitura precisa de destino privado explícito; o layout de containers
  e o provisionamento por secret externo ainda não são decisões de distribuição.
- Seleção ativa e refresh são síncronos/locais. Não há watcher, reload automático na
  fixture de transporte ou comando auth integrado ao produto; essas integrações
  continuam para M1 com os contratos aceitos.
- Lifetime de 30 dias, entrega irmã e formato do journal são propostas experimentais.
  Este formato não decide o armazenamento de documentos/coleções, ainda em G4.

M0-F2 tem evidência local para discutir G3. O gate geral continua aberto; próximo
incremento recomendado é M0-F3: contratos de escrita/no-op, formato inicial do banco
e limites/precedência de configuração.
