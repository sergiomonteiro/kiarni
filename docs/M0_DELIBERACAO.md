# M0-F4 — Deliberação do gate

**Decisão. M0 aprovado para iniciar M1.** Os 13 ADRs foram aceitos como decisões de arquitetura com o recorte e as verificações abaixo. A decisão se apoia nas evidências locais de transações, recuperação, contratos e streaming. O gate M1 continua exigindo operações multi-cliente e recuperação no servidor do produto. [PRD, §10](../KIARNI_PRD_v0.1.md) · [Evidências dos spikes](./M0_REVIEW.md)

No workspace local, `cargo fmt --all -- --check` e `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` passaram. `cargo test --workspace --locked --offline` passou com 129 testes após a execução fora do sandbox, que permitiu abrir os sockets loopback dos seis testes TCP.

| ADR | Decisão aceita | Verificação no produto |
|---|---|---|
| 0001 | Adotar objeto UTF-8, ID ASCII, rejeição de duplicatas e números preservados como JSON bruto. | M1 CRUD aplica limites antes de ler e após serializar. |
| 0002 | Adotar geração aleatória de 128 bits e até oito tentativas, com unicidade por coleção. | M1 CRUD testa colisão gerada, conflito explícito e rollback. |
| 0003 | Adotar redb 4.5.0 para M1. O spike demonstra as operações transacionais e a recuperação necessárias à escolha. A comparação funcional com SQLite sustenta sua viabilidade como alternativa. | M1 testa lock e recuperação do layout real. M3 valida backup operacional. |
| 0004 | Adotar ordenação por coordenador e overflow terminal. Usar os limites de produção do 0012. | M2 verifica ordem, lag, falhas entre commit e publicação e liberação de vagas. |
| 0005 | Adotar SSE via fetch com Bearer, assinatura por coleção e reconexão sem replay. | M2 exercita dois clientes reais e revogação. TLS/proxy entra antes da exposição remota. |
| 0006 | Adotar AST e comparação escalar exata do recorte proposto. | M1 confirma resultados sobre o storage e limita scan e saída. SDK valida números sem perda. |
| 0007 | Adotar rotas e catálogo com a precedência de erros do 0010 e os limites do 0012. | M1 testa handlers, request IDs, paginação e envelopes por HTTP. |
| 0008 | Adotar Bearer local e ciclo de vida junto do 0009, que descreve a ativação durável. | M1 testa permissões, autenticação e logs. M2 testa revalidação de streams. |
| 0009 | Adotar a raiz comum, o lock e as fases de retomada em conjunto com 0011. | Primeiro incremento M1 testa novo diretório, reinício, concorrência e estados parciais. |
| 0010 | Adotar PUT/PATCH transacional e no-op com igualdade estrutural e representação numérica preservada conforme os erros do 0007. | M1 confirma leitura e alteração na mesma transação. M2 confirma que no-op preserva a sequência e omite o evento. |
| 0011 | Adotar manifesto anterior à abertura do engine e catálogo com IDs sem reutilização, com o store em subdiretório conforme a composição proposta em 0009. | Primeiro incremento M1 testa manifesto incompatível, schema parcial e recuperação. |
| 0012 | Adotar precedência estrita, loopback e vagas compartilhadas com reserva de buffers pelo teto do formato. | M1 valida configuração real e admissão antes de alocar. M2 mede buffers sob carga e aplica limites de filas. |
| 0013 | Adotar snapshot lógico offline e restauração em raiz nova com novas credenciais. | M3 testa publicação, integridade, recuperação e rejeição de credenciais antigas. |

## Ajustes incorporados

1. **Raiz de dados e retomada.** Os ADRs 0009 e 0011 agora propõem `auth/` e `store/` sob um lock comum e registram as fases duráveis de retomada. A fase anterior à primeira emissão exige o marker de auth persistido. A ausência posterior desse marker interrompe o startup. [ADR 0009](./adr/0009-bootstrap-e-ativacao-auth.md) · [ADR 0011](./adr/0011-formato-inicial-e-catalogo.md)
2. **Precedência e limites de escrita.** Os ADRs 0007, 0010 e 0012 agora descrevem validação do corpo antes do lookup, preservação de bytes no no-op e reserva de buffers pelo teto do formato. Os modelos F3a/F3c mantêm os limites do experimento original. O M1 deve verificar o contrato revisado. [ADR 0007](./adr/0007-http-e-erros.md) · [ADR 0010](./adr/0010-substituicao-merge-patch-e-no-op.md) · [ADR 0012](./adr/0012-configuracao-e-admissao.md)
3. **Backup e restauração.** O ADR 0013 propõe snapshot lógico offline, publicação atômica sem sobrescrita e restauração em raiz nova com credenciais novas. A implementação operacional e os testes de restauração pertencem ao M3. [ADR 0013](./adr/0013-backup-e-restauracao.md)

## Primeiro incremento M1

O fluxo implementado e os comandos de operação estão no [servidor local do M1](./M1_SERVE.md).

Implementar `kiarni serve --data-dir ./data` com configuração resolvida, lock, abertura do store, ativação de auth, bind loopback e health autenticado. A readiness depende da abertura bem-sucedida do store e da configuração de auth carregada. Shutdown fecha a admissão, encerra conexões e libera o lock. Exercitar com subprocessos independentes o primeiro startup, restart, segundo servidor recusado, manifesto incompatível, interrupção entre store e auth e shutdown. CRUD e consultas seguem em incrementos próprios.
