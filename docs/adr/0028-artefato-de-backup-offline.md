# ADR 0028 — Artefato e publicação do backup offline

- Estado: aceito para implementação da issue #1, M3
- Data: 2026-11-14
- Complementa ADRs 0013, 0011 e 0009; não altera formato do store.

## Contrato

`kiarni backup --data-dir ORIGEM --output DESTINO` exporta uma instalação
inicializada, com servidor parado e lock exclusivo da raiz. Configuração da
origem segue CLI/env/JSON/defaults existentes; output é obrigatório e exclusivo
pela CLI. Não abre rede, não emite credenciais e não inicia/recupera auth.

Destino deve ser novo, com pai existente, do mesmo usuário e sem escrita por
grupo/outros. Não pode ficar dentro da instalação ou de sua entrega padrão de
credenciais. Artefato 0700, arquivos 0600:

```text
backup.json
store/format.json
store/data.redb
```

`backup.json` é JSON versão 1, magic `kiarni-backup`, `format_version: 1`,
`engine: redb-4.5.0`, contagens `collections`, `documents`, `document_bytes`
em strings decimais, e `files` com exatamente dois registros em ordem fixa
(store/format.json e store/data.redb), cada um com `path`, `bytes` decimal em
string e `sha256` hexadecimal minúsculo. Não contém caminhos de origem,
credenciais, configurações ou timestamps que impliquem retenção/replay.
Hashes detectam corrupção acidental; não autenticam a origem do backup.

Capturar uma única transação de leitura. Exportar as três tabelas para um novo
store; preservar JSON bruto, identidades e contador de coleção. Transações de
escrita do destino têm no máximo 256 registros lógicos ou aproximadamente 4 MiB de
payloads e chaves, com overshoot máximo de uma entrada (1 MiB de JSON mais
até 136 bytes de chave). Não montar catálogo ou
banco inteiro em memória. Cada engine usa cache configurado em 8 MiB; isso não
é promessa de RSS total. Um índice temporário no banco de staging verifica
unicidade e referências de IDs sem conjunto proporcional em RAM; removê-lo antes
de publicar. Recusar layout inconsistente, documento inválido ou chave divergente.

Fechar explicitamente o banco exportado; sincronizar arquivos/store, calcular
hashes com buffer de 64 KiB, gravar/sincronizar backup.json e diretório de staging.
Staging privado com nome aleatório fica no pai do destino. Publicar com rename
atômico sem substituição (renameat2 NOREPLACE em Linux, renameatx_np EXCL em
macOS). Se plataforma/filesystem recusar essa garantia, falhar sem fallback para
check-then-rename. Sincronizar o pai antes de retornar sucesso.

Falha anterior à publicação pode deixar staging privado, nunca destino parcial.
Falha de sync depois da publicação retorna `backup_publication_uncertain`;
operador deve validar o artefato antes de repetir. Destino existente, inclusive
symlink, nunca é substituído. Erros são códigos seguros, sem payloads/segredos.
Lock da origem permanece até fechar o engine e concluir a operação. Abertura
pode executar recovery autorizado pelo manifesto conforme ADR 0011.

## Limites e evidência exigida

Sem restauração neste incremento (issue #2), backup online, compactação, criptografia
ou mudança de formato persistido. Artefato guarda documentos sensíveis e deve ser
protegido como a origem. Testes: snapshot com mutação concorrente, múltiplos lotes,
layout inválido, reabertura/contador, hashes, ausência de auth, lock, destino
existente e concorrente, permissões, interrupção real antes/depois da publicação.
Testes de processo não comprovam durabilidade sob queda de energia/disco físico.
