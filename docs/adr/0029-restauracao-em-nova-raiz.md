# ADR 0029 — Restauração validada em nova raiz

- Estado: aceito para issue #2, M3
- Data: 2026-11-14
- Complementa ADRs 0013, 0028 e 0009. Formatos de backup/store inalterados.

## Contrato

`kiarni restore --from BACKUP --data-dir NOVA_RAIZ` valida e restaura o artefato
v1. `--from` é obrigatório e único; destino segue configuração comum CLI/env/JSON.
Não faz bind, não inicia o servidor e não emite credenciais. Destino e diretório
irmão NOVA_RAIZ.credentials devem ser novos, inclusive vazios/symlinks existentes
são recusados. Pai existente deve pertencer ao usuário e não ter escrita por
grupo/outros; raiz/entrega 0700, arquivos 0600. Não restaurar dentro do artefato.

Abrir artefato/arquivos regulares privados com NOFOLLOW e descriptors ancorados;
recusar entradas extras, symlinks/hardlinks e tipos/permissões impróprios. Ler
backup.json em até 1 KiB, com schema estrito (sem duplicados/campos desconhecidos),
versões/engine conhecidos, contagens/tamanhos decimais u64 em strings, dois
caminhos fixos em ordem e hashes hexadecimais minúsculos de 64 caracteres.

Copiar somente os dois arquivos esperados para staging privado no pai do destino,
com buffer de 64 KiB e tamanho limitado pelo manifesto. Fechar/sincronizar e
calcular hashes relendo os bytes copiados, antes de abrir o engine. Não abrir o
banco original do backup para escrita; origem e manifesto permanecem intactos.

Abrir a cópia verificada e reexportar um snapshot usando os lotes e validação
estrutural do backup: tabelas/metadados, nomes, unicidade e referências de IDs,
JSON bruto, contador e correspondência de _id. Comparar contagens/bytes com o
manifesto. A reexportação reaproveita a validação limitada em memória e exige
espaço temporário para duas cópias do store, além do artefato. Remover somente a
cópia intermediária de staging após fechar o engine. Publicar apenas store final
validado e bootstrap.json em store_ready, com serve.lock; sem auth da origem.

## Publicação e interrupções

Dois diretórios não podem ser publicados como uma única operação atômica. Criar
a entrega vazia dentro de staging e publicá-la primeiro no nome irmão final,
com rename sem substituição e sync do pai. Publicar então a raiz por rename sem
substituição e confirmar sync do pai. Manter lock da raiz preparada até concluir.
O próximo serve usa bootstrap existente e cria novas credenciais naquela entrega.

Falha antes das publicações pode deixar staging privado, sem destino final.
Interrupção entre as duas publicações pode deixar somente a entrega vazia;
nenhum token foi emitido. Falha depois de publicar a entrega retorna resultado
incerto, inclusive disputa do nome da raiz. Não remover automaticamente a
entrega nem destinos existentes. Operador verifica ambos os caminhos antes de
repetir ou escolhe outro nome. Nova execução recusa qualquer entrega existente.
`restore_publication_uncertain` não implica ausência da raiz. Plataforma ou FS
sem rename atômico sem substituição é recusado, sem fallback check-then-rename.

Não importar credenciais, auth, replay ou sequência de changefeed. Iniciar a raiz
restaurada é escolha explícita do operador; a instalação original permanece
independente. Restauração não muda automaticamente a configuração de serve.

## Evidência

Testes de artefatos truncados, incompatíveis, adulterados, schema/árvore inválida,
symlinks e inconsistências estruturais mesmo com hashes recalculados. Provar
contador e JSON após restaurar, preservação de destinos concorrentes e
interrupção real nas três janelas de publicação. Servidor real deve recusar
token antigo e revogado e aceitar somente a nova credencial. Sucesso em testes
de processo não demonstra queda de energia, falha física ou desempenho de disco.
