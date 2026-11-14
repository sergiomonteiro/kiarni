# Spike M0 — autenticação e tokens

Atualização: [M0-F2](./auth-activation.md) adicionou uma camada de bootstrap,
ativação persistida e testes de kill/restart. As limitações abaixo descrevem o
primeiro modelo de snapshots explícitos; integração ao produto permanece pendente.

O [ADR 0008](../adr/0008-autenticacao-e-tokens.md) continua em proposta. O spike
`spikes/auth` implementa um modelo local de lifecycle e arquivos Unix. O modelo
HTTP autentica antes do roteamento, mas não existe servidor HTTP, TLS ou CLI de
credenciais implementada no binário `kiarni`. O incremento [M0-F1](./transport.md)
usa essa autoridade em sockets/Chromium reais e fecha streams após reload de
revogação em memória; bootstrap/seleção ativa persistida continuam pendentes.

## Reprodução

```bash
cargo test -p kiarni-auth-spike -p kiarni-http-contract-spike --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Execução local em Linux x86_64, Rust/Cargo 1.99.1. As verificações entram no CI Rust;
execução remota/macOS ainda não demonstrada. Dependências novas e transitivas estão
no [inventário de licenças](../dependencies.md), sem serviço externo obrigatório.

## Evidências

- Geração usa 40 bytes do SO: 8 para ID público e 32 para segredo. Lifetime e
  validade são explícitos; os testes não esperam relógio real para simular expiração.
- Snapshot contém somente SHA-256/metadados; testes verificam ausência do token
  e de sua parte secreta. SecretToken e Session têm Debug redigido, sem Display,
  Serialize ou Clone do segredo.
- Headers ausentes, duplicados, malformados, desconhecidos, expirados e revogados
  retornam Unauthorized. Testes cobrem início inclusivo, fim exclusivo, scheme
  case-insensitive e recusa de whitespace extra/bytes inválidos.
- Rotação mantém até dois verificadores, com overlap até 300 s, sem estender a
  expiração anterior. Grace zero invalida anteriores; revogação completa permite
  recuperação local em nova epoch.
- Sessões retêm fingerprint, não token. Quatro consumidores concorrentes recusam
  token/sessão após revogação aplicada. Reload inválido ou rollback de epoch falha
  fechado, sem fallback; o mesmo conteúdo/epoch é idempotente.
- Retrocesso do relógio observado impede autenticação até alcançar o último valor.
  Entropia indisponível, colisões, excesso de tentativas, overflow de epoch, TTL e
  overlap inválidos são exercitados sem fallback.
- Arquivos novos 0600 sob diretórios 0700 do mesmo UID, usando descriptor/openat/
  NOFOLLOW. Modos excessivos, symlinks, hardlinks e snapshots parciais, oversized
  ou com epoch divergente são recusados.
- Provisionamento entrega o token a outro diretório privado; recusa mesmo inode
  ou token incompatível com o verificador. O teste verifica que o diretório de
  verificadores contém somente o snapshot. Nenhuma função imprime o token.
- Dois provisionadores da mesma epoch têm um vencedor sem sobrescrita; o snapshot
  final autentica o token entregue por esse vencedor.
- O guard do modelo HTTP autentica inclusive health, rota inexistente e caminho
  inválido antes de expor detalhes. Infraestrutura de auth recebe 503 genérico;
  credencial inválida recebe 401.

## Limites e pendências

1. Arquivos são novos por API, não imutáveis contra seu dono. O teste negativo de
   UID diferente não foi executado por exigir outro usuário/privilégios. O dono do
   host e processos do mesmo UID continuam no domínio de confiança.
2. O operador deve escolher entrega fora do volume/backup do servidor. Descriptors
   distintos não provam ausência de ancestralidade entre diretórios; validar esse
   layout e a entrega ao cliente na futura CLI.
3. Há sync de arquivos/diretórios, sem transação multi-file ou ponteiro ativo
   persistido. Erros podem deixar token órfão ou snapshot parcial. Não descobrir
   automaticamente a maior epoch. Crash durante provisionamento, recuperação e
   publicação atômica da configuração ativa ainda não foram testados.
4. Revogação começa no reload aplicado. Watcher/seleção de snapshot e timer de
   revalidação dos streams ainda não integram servidor. Obter now do relógio do
   processo, não do request. Bytes enviados e commits aceitos não são desfeitos.
5. Não houve teste de rede, HTTPS/proxy, origem/Host, browser, CSP/XSS ou logging
   middleware. Testes de Debug/erros não comprovam ausência de segredos em logs
   de infraestrutura. Após 200 SSE, fechar stream revogado como interrupted.
6. Subtle compara arrays fixos e todos os verificadores, sem comprovar tempo
   constante do autenticador completo. Zeroizing cobre buffers próprios de segredo,
   sem garantir eliminação de cópias externas, registradores, swap ou core dumps.
7. Não houve auditoria de segurança ou teste de queda de energia. Não há defesa
   contra rollback de filesystem/relógio após restart, usuários, roles, JWT/OAuth,
   permissões por coleção, cookies ou multi-tenancy.

Próximo incremento: revisar o gate M0 com matriz de evidências, bloqueadores e
ADRs a aceitar/revisar antes de iniciar o servidor M1.
