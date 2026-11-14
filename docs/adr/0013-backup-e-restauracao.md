# ADR 0013 — Backup consistente e restauração em nova raiz

- Status: **aceita** ([deliberação M0-F4](../M0_DELIBERACAO.md)).
- Marco: deliberação M0-F4, entrega operacional M3.
- Depende da escolha do engine no ADR 0003 e do layout dos ADRs 0009 e 0011.

## Backup

O primeiro procedimento é local e offline. Com o servidor parado, a operação
adquire o lock exclusivo da raiz e valida o manifesto antes de abrir o banco.
Uma única transação de leitura fornece documentos, catálogo e metadados,
incluindo o contador de IDs de coleção. A exportação percorre as tabelas
incrementalmente, com memória limitada, e materializa um banco novo com
manifesto compatível. A janela de manutenção inclui o tempo da exportação.

O artefato contém somente o store e um manifesto de backup versionado que
registra os tamanhos e SHA-256 dos arquivos esperados. Os hashes detectam
alterações acidentais. A origem do artefato deve ser confiável. Documentos no
backup têm a mesma sensibilidade dos documentos no banco.

Criar o artefato em diretório privado temporário no mesmo filesystem do
destino. Fechar e sincronizar o banco exportado, os manifestos e o diretório
temporário. Publicar o diretório completo com operação atômica que recuse um
destino existente e sincronizar o diretório pai antes de reportar sucesso.
Se a plataforma não oferecer publicação com essa garantia, recusar a operação.
Uma interrupção anterior à publicação pode deixar temporários privados. Uma
falha após a publicação e antes da confirmação do sync deixa o resultado
incerto. Validar o destino antes de repetir a operação.

## Restauração

Restaurar para uma raiz nova, com entrega de credenciais em um diretório irmão
novo. Validar a versão do backup e do formato, os arquivos esperados, tamanhos
e hashes sobre os bytes copiados para staging. Validar tabelas, referências de
documentos a coleções e o contador de IDs antes de publicar a raiz. Usar o mesmo
protocolo de publicação sem sobrescrita. A seleção da raiz restaurada é uma
operação explícita do operador.

A raiz restaurada contém o store validado e `bootstrap.json` na fase
`store_ready`. O próximo startup segue o ADR 0009 e emite uma credencial
nova. O artefato exclui verificadores, chaves, journal de auth, arquivos de
entrega e locks da instalação original. O servidor restaurado aceita apenas
credenciais da nova instalação. A sequência de changefeed começa novamente e
os clientes estabelecem novas assinaturas.

## Evidência e verificações M3

O spike exportou duas tabelas a partir de um snapshot único e demonstrou
reabertura com o estado desse snapshot. A composição do catálogo real, a
publicação atômica do destino e o bootstrap pós-restauração serão exercitados
no M3. [Relatório de storage](../spikes/storage.md)

- Restaurar documentos, catálogo e contador de IDs. Confirmar que o próximo
  ID de coleção mantém o avanço do snapshot.
- Recusar fonte bloqueada pelo servidor, backup truncado ou incompatível,
  referências inconsistentes e destino existente. Verificar que o destino
  existente conserva seus bytes.
- Interromper antes e depois da publicação. Verificar ausência de destino
  final antes da publicação e validação do artefato completo após ela.
- Criar e revogar uma credencial na instalação de origem. Após restaurar,
  verificar que credenciais antigas, inclusive a revogada, recebem 401 e que
  a credencial recém-provisionada autentica.
